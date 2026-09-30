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
}

impl VodState {
    fn get_playback(&self, id: &str, fresh: bool) -> Option<PlaybackData> {
        let map = self.playback.lock().unwrap();
        let (data, at) = map.get(id)?;
        if fresh || at.elapsed() > PLAYBACK_TTL {
            return None;
        }
        Some(data.clone())
    }

    fn set_playback(&self, id: &str, data: PlaybackData) {
        let mut map = self.playback.lock().unwrap();
        map.retain(|_, (_, at)| at.elapsed() <= PLAYBACK_TTL);
        map.insert(id.to_string(), (data, Instant::now()));
    }

    pub fn clear(&self) {
        self.playback.lock().unwrap().clear();
        *self.playlist.lock().unwrap() = None;
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
    if let Some(cached) = state.vod_state.get_playback(content_id, fresh) {
        return Ok(cached);
    }
    if let Err(e) = state.extras.ensure_token(false, &state.store).await {
        tracing::warn!("extras: token refresh failed: {e}");
    }
    let client = state.extras.client_for_vod().ok_or_else(|| {
        anyhow::anyhow!("connect the extra source in Settings to watch on-demand titles")
    })?;
    let resp = client.playback(content_id).await?;
    state.vod_state.set_playback(content_id, resp.data.clone());
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

/// `GET /vod.m3u` — on-demand titles as an M3U playlist, cached 6h.
pub async fn vod_playlist_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let client = match require_client(&state) {
        Ok(c) => c,
        Err(r) => return *r,
    };

    let cached = { state.vod_state.playlist.lock().unwrap().clone() };
    let entries_xml = match cached {
        Some((xml, at)) if at.elapsed() <= PLAYLIST_TTL => xml,
        _ => {
            let entries = build_vod_playlist(&client).await;
            let base = headers
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .map(|h| format!("http://{h}"))
                .unwrap_or_default();
            let xml = render_vod_playlist(&entries, &base);
            if !entries.is_empty() {
                *state.vod_state.playlist.lock().unwrap() = Some((xml.clone(), Instant::now()));
            }
            xml
        }
    };

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
