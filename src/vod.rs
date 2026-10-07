//! On-demand playback through the optional extra source: a fixed
//! allowlist of providers (see `extras::is_supported_provider`) — every
//! other provider in the catalogue only deep-links to a separate app and
//! is never shown. Manifests and segments come straight from each
//! provider's own CDN (which allows cross-origin requests); only the
//! Widevine license request goes through this server, since it needs the
//! extra source's login headers — including one provider's own license
//! server, an explicitly owner-approved exception to "only proxy this
//! app's own hosts".

use crate::extras::{PlaybackData, Rail, VodItem};
use crate::state::AppState;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PLAYBACK_TTL: Duration = Duration::from_secs(10 * 60);
const PLAYLIST_TTL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Default)]
pub struct VodState {
    playback: Mutex<HashMap<String, (PlaybackData, Instant)>>,
    playlist: Mutex<Option<(String, Instant)>>,
    /// Bumped by `clear` (account/product switch). A fetch captures it first
    /// and stores its result only if it is unchanged, so a request that
    /// outlived the switch cannot repopulate the cleared caches.
    generation: std::sync::atomic::AtomicU64,
}

impl VodState {
    /// A hit is served only in the generation it was captured for: `clear`
    /// bumps the generation under this same lock, so a request that crossed an
    /// account switch gets a miss, never the previous account's entry.
    fn get_playback(&self, id: &str, fresh: bool, generation: u64) -> Option<PlaybackData> {
        let map = self.playback.lock().unwrap();
        if self.generation() != generation {
            return None;
        }
        let (data, at) = map.get(id)?;
        if fresh || at.elapsed() > PLAYBACK_TTL {
            return None;
        }
        Some(data.clone())
    }

    fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Returns whether the entry was stored (false: the context changed).
    fn set_playback(&self, id: &str, data: PlaybackData, generation: u64) -> bool {
        let mut map = self.playback.lock().unwrap();
        if self.generation() != generation {
            return false;
        }
        map.retain(|_, (_, at)| at.elapsed() <= PLAYBACK_TTL);
        map.insert(id.to_string(), (data, Instant::now()));
        true
    }

    fn get_playlist(&self, generation: u64) -> Option<(String, Instant)> {
        let slot = self.playlist.lock().unwrap();
        if self.generation() != generation {
            return None;
        }
        slot.clone()
    }

    /// Returns whether the playlist was stored (false: the context changed).
    fn set_playlist(&self, xml: String, generation: u64) -> bool {
        let mut slot = self.playlist.lock().unwrap();
        if self.generation() != generation {
            return false;
        }
        *slot = Some((xml, Instant::now()));
        true
    }

    pub fn clear(&self) {
        // The bump happens while holding both locks, so every generation check
        // (made under the playback or the playlist lock) is atomic with it.
        let mut map = self.playback.lock().unwrap();
        let mut playlist = self.playlist.lock().unwrap();
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        map.clear();
        *playlist = None;
    }
}

fn valid_content_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

async fn vod_playback(
    state: &AppState,
    content_id: &str,
    fresh: bool,
) -> anyhow::Result<PlaybackData> {
    let generation = state.vod_state.generation();
    if let Some(cached) = state.vod_state.get_playback(content_id, fresh, generation) {
        return Ok(cached);
    }
    if let Err(e) = state.extras.ensure_token(false, &state.store).await {
        tracing::warn!("extras: token refresh failed: {e}");
    }
    let client = state.extras.client_for_vod().ok_or_else(|| {
        anyhow::anyhow!("connect the extra source in Settings to watch on-demand titles")
    })?;
    let resp = client.playback(content_id).await?;
    if !state
        .vod_state
        .set_playback(content_id, resp.data.clone(), generation)
    {
        anyhow::bail!("the active account changed while resolving this title; retry");
    }
    Ok(resp.data)
}

fn err(status: StatusCode, message: impl Into<String>) -> Response {
    (status, axum::Json(json!({"message": message.into()}))).into_response()
}

fn require_client(state: &AppState) -> Result<Arc<crate::extras::Client>, Box<Response>> {
    state.extras.client_for_vod().ok_or_else(|| {
        Box::new(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "connect the extra source in Settings to watch on-demand titles",
        ))
    })
}

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
}

/// `GET /api/ott/search?q=`
pub async fn api_ott_search(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SearchQuery>,
) -> Response {
    let client = match require_client(&state) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let query = q.q.unwrap_or_default().trim().to_string();
    if query.is_empty() {
        return axum::Json(json!({"rails": Vec::<Rail>::new()})).into_response();
    }
    match client.search(&query).await {
        Ok(rails) => axum::Json(json!({"rails": rails})).into_response(),
        Err(e) => {
            tracing::warn!("extras search: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "search failed")
        }
    }
}

#[derive(serde::Deserialize)]
pub struct PageQuery {
    page: Option<i64>,
}

/// `GET /api/ott/screen/:id?page=`
pub async fn api_ott_screen(
    Path(id): Path<String>,
    Query(q): Query<PageQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let client = match require_client(&state) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let page = q.page.unwrap_or(0).max(0);
    match client.screen(&id, page).await {
        Ok((rails, more)) => axum::Json(json!({"rails": rails, "more": more})).into_response(),
        Err(e) => {
            tracing::warn!("extras screen: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "cannot load this page")
        }
    }
}

#[derive(serde::Deserialize)]
pub struct SeasonQuery {
    season: Option<i64>,
}

/// `GET /api/ott/show/:id?season=`
pub async fn api_ott_episodes(
    Path(id): Path<String>,
    Query(q): Query<SeasonQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let client = match require_client(&state) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if !valid_content_id(&id) {
        return err(StatusCode::BAD_REQUEST, "invalid id");
    }
    match client.episodes(&id, q.season.unwrap_or(0)).await {
        Ok(episodes) => axum::Json(json!({"episodes": episodes})).into_response(),
        Err(e) => {
            tracing::warn!("extras episodes: {e}");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "cannot load the episodes",
            )
        }
    }
}

/// `GET /api/ott/play/:id`
pub async fn api_ott_play(Path(id): Path<String>, State(state): State<Arc<AppState>>) -> Response {
    if require_client(&state).is_err() {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "connect the extra source in Settings to watch on-demand titles",
        );
    }
    if !valid_content_id(&id) {
        return err(StatusCode::BAD_REQUEST, "invalid id");
    }
    let d = match vod_playback(&state, &id, true).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("extras on-demand playback {id}: {e}");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "this title cannot be played",
            );
        }
    };
    if !crate::extras::is_supported_provider(&d.provider) {
        return err(StatusCode::FORBIDDEN, "this provider is not supported");
    }
    let (stream, dash) = d.vod_stream();
    if stream.is_empty() {
        return err(StatusCode::NOT_FOUND, "no stream for this title");
    }
    let license = if !d.key_url.is_empty() {
        format!("/api/ott/license/{id}")
    } else {
        String::new()
    };
    axum::Json(json!({
        "name": d.name,
        "provider": d.provider,
        "duration": d.total_duration,
        "url": stream,
        "dash": dash,
        "license": license,
    }))
    .into_response()
}

/// `POST /api/ott/license/:id` and (IPTV players) `POST /vod/license/:id`.
/// Forwards a Widevine license request to the title's own license server
/// (which provider hosts it varies) with the extra source app's headers.
pub async fn ott_license(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    method: Method,
    body: Bytes,
) -> Response {
    let client = match require_client(&state) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if !valid_content_id(&id) {
        return err(StatusCode::BAD_REQUEST, "invalid id");
    }
    let d = match vod_playback(&state, &id, false).await {
        Ok(d) if !d.key_url.is_empty() => d,
        _ => return err(StatusCode::NOT_FOUND, "no license for this title"),
    };
    if !crate::extras::is_supported_provider(&d.provider) {
        return err(StatusCode::FORBIDDEN, "this provider is not supported");
    }

    let mut req = state
        .http
        .request(method, &d.key_url)
        .header(header::USER_AGENT, crate::extras::PLAYER_USER_AGENT)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(body);
    for (k, v) in client.vod_license_headers(&d) {
        req = req.header(k, v);
    }

    match req.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let ct = resp.headers().get(header::CONTENT_TYPE).cloned();
            let bytes = resp.bytes().await.unwrap_or_default();
            let mut builder = Response::builder().status(status);
            if let Some(ct) = ct {
                builder = builder.header(header::CONTENT_TYPE, ct);
            }
            builder.body(Body::from(bytes)).unwrap()
        }
        Err(_) => err(StatusCode::BAD_GATEWAY, "license upstream request failed"),
    }
}

/// `GET /vod/:id` (`.mpd`/`.m3u8` suffix optional) — sends an IPTV player
/// straight to a fresh stream URL.
pub async fn vod_stream_handler(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    if require_client(&state).is_err() {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "connect the extra source in Settings to watch on-demand titles",
        );
    }
    let id = id.trim_end_matches(".mpd").trim_end_matches(".m3u8");
    if !valid_content_id(id) {
        return err(StatusCode::BAD_REQUEST, "invalid id");
    }
    let d = match vod_playback(&state, id, true).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("extras on-demand playback {id}: {e}");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "this title cannot be played",
            );
        }
    };
    if !crate::extras::is_supported_provider(&d.provider) {
        return err(StatusCode::FORBIDDEN, "this provider is not supported");
    }
    let (stream, _) = d.vod_stream();
    if stream.is_empty() {
        return err(StatusCode::NOT_FOUND, "no stream for this title");
    }
    Redirect::to(&stream).into_response()
}

const VOD_PLAYLIST_SCREENS: &[(&str, i64)] = &[
    ("1", 4),
    ("100021", 6),
    ("100023", 6),
    ("100025", 4),
    ("100097", 4),
];
const MAX_PLAYLIST_SHOWS: usize = 40;

async fn build_vod_playlist(client: &crate::extras::Client) -> Vec<(VodItem, String)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut shows = 0usize;

    for (screen_id, pages) in VOD_PLAYLIST_SCREENS {
        for page in 0..*pages {
            let (rails, more) = match client.screen(screen_id, page).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("extras on-demand playlist, screen {screen_id}: {e}");
                    break;
                }
            };
            for r in rails {
                for it in r.items {
                    let provider = it.provider.as_str();
                    if it.content_type != "Show" {
                        if seen.insert(it.content_id.clone()) {
                            let group = format!("{provider} · {}", r.title);
                            out.push((it, group));
                        }
                        continue;
                    }
                    if seen.contains(&it.content_id) || shows >= MAX_PLAYLIST_SHOWS {
                        continue;
                    }
                    seen.insert(it.content_id.clone());
                    shows += 1;
                    let Ok(episodes) = client.episodes(&it.content_id, 0).await else {
                        continue;
                    };
                    for mut ep in episodes {
                        if ep.show_name.is_empty() {
                            ep.show_name = it.name.clone();
                        }
                        let group = format!("{provider} · {}", it.name);
                        out.push((ep, group));
                    }
                }
            }
            if !more {
                break;
            }
        }
    }
    out
}

const BASE_PLACEHOLDER: &str = "@@JIOTV_BASE@@";

/// `GET /vod.m3u` — on-demand titles as an M3U playlist, cached 6h.
pub async fn vod_playlist_handler(
    State(state): State<Arc<AppState>>,
    https: Option<axum::Extension<crate::tls::Https>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let client = match require_client(&state) {
        Ok(c) => c,
        Err(r) => return *r,
    };

    // The cache holds the playlist with a placeholder base; the scheme and
    // host of the *current* request are substituted on every response, so
    // an http client and an https client never see each other's links.
    let generation = state.vod_state.generation();
    let cached = state.vod_state.get_playlist(generation);
    let template = match cached {
        Some((xml, at)) if at.elapsed() <= PLAYLIST_TTL => xml,
        _ => {
            let entries = build_vod_playlist(&client).await;
            let xml = render_vod_playlist(&entries, BASE_PLACEHOLDER);
            // A playlist built for the previous account is neither cached nor
            // returned if the context changed while it was being built.
            let stored =
                entries.is_empty() || state.vod_state.set_playlist(xml.clone(), generation);
            if !stored {
                return err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The active account changed while this request was running; retry",
                );
            }
            xml
        }
    };
    // Final synchronized check: a playlist cloned from the cache or built for the
    // previous account must not be returned once a switch has completed.
    if state.vod_state.generation() != generation {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "The active account changed while this request was running; retry",
        );
    }
    let scheme = if https.is_some() { "https" } else { "http" };
    let base = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| format!("{scheme}://{h}"))
        .unwrap_or_default();
    let entries_xml = template.replace(BASE_PLACEHOLDER, &base);

    Response::builder()
        .header(header::CONTENT_TYPE, "audio/x-mpegurl; charset=utf-8")
        .header(header::CONTENT_DISPOSITION, "inline; filename=\"vod.m3u\"")
        .body(Body::from(entries_xml))
        .unwrap()
}

fn render_vod_playlist(entries: &[(VodItem, String)], base: &str) -> String {
    let mut out = String::from("#EXTM3U\n");
    for (it, group) in entries {
        let mut name = it.name.clone();
        if it.content_type == "Episode" && !it.show_name.is_empty() {
            name = format!(
                "{} S{:02}E{:02} {}",
                it.show_name,
                it.season.max(1),
                it.episode_no,
                it.name
            );
        }
        let name = name.replace(['\n', ','], " ");
        let group = group.replace('"', "'");
        let duration = if it.total_duration > 0 {
            it.total_duration
        } else {
            -1
        };
        out.push_str(&format!(
            "#EXTINF:{duration} tvg-id=\"vod_{}\" tvg-logo=\"{}\" group-title=\"{group}\",{name}\n",
            it.content_id, it.thumbnail
        ));
        if it.provider == "MXPlayer" {
            out.push_str(&format!("{base}/vod/{}.m3u8\n", it.content_id));
            continue;
        }
        out.push_str("#KODIPROP:inputstream=inputstream.adaptive\n");
        out.push_str("#KODIPROP:inputstream.adaptive.manifest_type=mpd\n");
        out.push_str("#KODIPROP:inputstream.adaptive.license_type=com.widevine.alpha\n");
        out.push_str(&format!(
            "#KODIPROP:inputstream.adaptive.license_key={base}/vod/license/{}\n",
            it.content_id
        ));
        out.push_str(&format!("{base}/vod/{}.mpd\n", it.content_id));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hits_are_served_only_in_the_generation_they_were_captured_for() {
        let state = VodState::default();
        let old = state.generation();
        state.clear();
        let current = state.generation();
        assert!(state.set_playlist("<new/>".into(), current));
        // A request that captured the old generation and read after the switch
        // must see a miss, never the new (or the previous) account's entry.
        assert!(state.get_playlist(old).is_none());
        assert!(state.get_playlist(current).is_some());
        assert!(state.get_playback("title", false, old).is_none());
    }

    #[test]
    fn playback_and_playlist_writes_from_before_a_clear_are_dropped() {
        let state = VodState::default();
        let started = state.generation();
        assert!(state.set_playlist("<xml/>".into(), started));
        state.clear();
        assert!(state.playlist.lock().unwrap().is_none());
        // Fetches that began before the clear must not repopulate the caches.
        assert!(!state.set_playlist("<old/>".into(), started));
        assert!(state.playlist.lock().unwrap().is_none());
        assert!(state.playback.lock().unwrap().is_empty());
        assert!(state.set_playlist("<new/>".into(), state.generation()));
    }

    #[test]
    fn valid_content_id_rejects_bad_input() {
        assert!(valid_content_id("abc123-_XY"));
        assert!(!valid_content_id(""));
        assert!(!valid_content_id("has space"));
        assert!(!valid_content_id(&"x".repeat(65)));
    }

    #[test]
    fn playable_item_requires_known_provider_and_playback_type() {
        let mut it = VodItem {
            provider: "JioCinema".into(),
            playback_type: "playback".into(),
            content_type: "Movie".into(),
            ..Default::default()
        };
        assert!(it.playable());
        it.playback_type = "deeplink".into();
        assert!(!it.playable());
        it.playback_type = "playback".into();
        it.provider = "PrimeVideo".into();
        assert!(!it.playable());
    }

    #[test]
    fn playlist_render_uses_mx_player_hls_and_others_mpd() {
        let mx = VodItem {
            content_id: "1".into(),
            name: "MX Movie".into(),
            provider: "MXPlayer".into(),
            content_type: "Movie".into(),
            ..Default::default()
        };
        let jc = VodItem {
            content_id: "2".into(),
            name: "JC Movie".into(),
            provider: "JioCinema".into(),
            content_type: "Movie".into(),
            ..Default::default()
        };
        let entries = vec![
            (mx, "MX Player · Movies".to_string()),
            (jc, "JioCinema · Movies".to_string()),
        ];
        let out = render_vod_playlist(&entries, "http://host");
        assert!(out.contains("http://host/vod/1.m3u8"));
        assert!(out.contains("http://host/vod/2.mpd"));
        assert!(out.contains("http://host/vod/license/2"));
    }

    #[test]
    fn episode_name_includes_show_and_episode_number() {
        let ep = VodItem {
            content_id: "3".into(),
            name: "Pilot".into(),
            show_name: "A Show".into(),
            content_type: "Episode".into(),
            season: 2,
            episode_no: 5,
            provider: "Zee5".into(),
            ..Default::default()
        };
        let out = render_vod_playlist(&[(ep, "ZEE5 · A Show".to_string())], "http://host");
        assert!(out.contains("A Show S02E05 Pilot"));
    }
}
