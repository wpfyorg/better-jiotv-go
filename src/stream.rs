//! Live HLS proxying: `/live/:id`, `/live/:quality/:id`, and the
//! `/render.m3u8` / `/render.ts` / `/render.key` pipeline that rewrites a
//! fetched manifest's URLs to point back through this server. Mirrors
//! `LiveHandler`/`LiveQualityHandler`/`RenderHandler`/`RenderTSHandler`/
//! `RenderKeyHandler` in `internal/handlers/handlers.go`, at reduced fidelity
//! in a few places noted inline (and in the README's parity list) — the
//! biggest simplification is that concurrent requests for the same channel
//! are not deduplicated (no `singleflight` equivalent).

use crate::state::AppState;
use crate::television::{self, LiveUrlOutput};
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const HDNEA_CACHE_TTL: Duration = Duration::from_secs(60);
const DEAD_CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct RenderCaches {
    hdnea: std::sync::RwLock<HashMap<String, (String, Instant)>>,
    dead: std::sync::RwLock<HashMap<String, Instant>>,
    /// De-duplicates concurrent JioTV live-URL recovery fetches for the same
    /// channel (mirrors `refreshChannelToken`'s `singleflight.Group`); the
    /// extras path has its own dedup in `extras_state`.
    refresh_locks: crate::keyed_locks::KeyedLocks,
    /// Bumped, under both map locks, by `clear` (account/product switch). A
    /// request captures it before touching the caches and its writes are
    /// dropped if it changed, so a request that outlived the switch cannot
    /// write the previous account's token or dead-channel state back.
    generation: std::sync::atomic::AtomicU64,
}

impl RenderCaches {
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn hdnea_key(channel_id: &str, stream_url: &str, quality: &str) -> String {
        let quality = if quality.is_empty() { "auto" } else { quality };
        if stream_url.to_lowercase().contains("catchup") {
            format!("{channel_id}|catchup|{quality}")
        } else {
            format!("{channel_id}|hls|{quality}")
        }
    }

    pub fn get_hdnea(&self, key: &str) -> Option<String> {
        let map = self.hdnea.read().unwrap();
        let (token, at) = map.get(key)?;
        if at.elapsed() > HDNEA_CACHE_TTL {
            return None;
        }
        Some(token.clone())
    }

    /// Stores `token` unless the caches were cleared since `generation` was
    /// captured.
    pub fn set_hdnea(&self, generation: u64, key: &str, token: &str) {
        if token.is_empty() {
            return;
        }
        let mut map = self.hdnea.write().unwrap();
        if self.generation() != generation {
            return;
        }
        map.insert(key.to_string(), (token.to_string(), Instant::now()));
    }

    pub fn clear_hdnea(&self, key: &str) {
        self.hdnea.write().unwrap().remove(key);
    }

    pub fn is_dead(&self, channel_id: &str) -> bool {
        if channel_id.is_empty() {
            return false;
        }
        match self.dead.read().unwrap().get(channel_id) {
            Some(at) => at.elapsed() <= DEAD_CACHE_TTL,
            None => false,
        }
    }

    /// Marks the channel dead unless the caches were cleared since
    /// `generation` was captured.
    pub fn mark_dead(&self, generation: u64, channel_id: &str) {
        if channel_id.is_empty() {
            return;
        }
        let mut map = self.dead.write().unwrap();
        if self.generation() != generation {
            return;
        }
        map.insert(channel_id.to_string(), Instant::now());
    }

    pub fn clear_dead(&self, channel_id: &str) {
        self.dead.write().unwrap().remove(channel_id);
    }

    pub fn clear(&self) {
        let mut hdnea = self.hdnea.write().unwrap();
        let mut dead = self.dead.write().unwrap();
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        hdnea.clear();
        dead.clear();
    }
}

fn extract_hdnea_from_url(u: &str) -> Option<String> {
    let query = u.split('?').nth(1)?;
    for pair in query.split('&') {
        if let Some(v) = pair
            .strip_prefix("__hdnea__=")
            .or_else(|| pair.strip_prefix("hdnea="))
        {
            return Some(
                urlencoding::decode(v)
                    .map(|s| s.into_owned())
                    .unwrap_or_else(|_| v.to_string()),
            );
        }
    }
    None
}

pub fn strip_hdnea_from_url(u: &str) -> String {
    let Some((base, query)) = u.split_once('?') else {
        return u.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.starts_with("hdnea=") && !p.starts_with("__hdnea__="))
        .collect();
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

pub(crate) fn to_absolute_stream_url(stream_url: &str, base_from_live: Option<&str>) -> String {
    if stream_url.is_empty() {
        return String::new();
    }
    let lower = stream_url.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return stream_url.to_string();
    }
    if let Some(rest) = stream_url.strip_prefix("//") {
        return format!("https://{rest}");
    }
    let path = if stream_url.starts_with('/') {
        stream_url.to_string()
    } else {
        format!("/{stream_url}")
    };
    let base = base_from_live.unwrap_or("https://jiotvapi.cdn.jio.com");
    format!("{base}{path}")
}

pub(crate) fn absolute_base_from_live(live: &LiveUrlOutput) -> Option<String> {
    let candidates = [
        &live.bitrates.auto,
        &live.bitrates.high,
        &live.bitrates.medium,
        &live.bitrates.low,
        &live.result,
        &live.mpd.result,
    ];
    for c in candidates {
        let lower = c.to_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            if let Ok(parsed) = url::Url::parse(c) {
                return Some(format!(
                    "{}://{}",
                    parsed.scheme(),
                    parsed.host_str().unwrap_or("")
                ));
            }
        }
    }
    None
}

/// Fetches a fresh playback URL for `channel_id`, from extras when it routes
/// there and from JioTV otherwise — mirrors `getLiveResult`. JioTV's own
/// token refresh is skipped on a extras route (extras manages its own tokens).
/// JioTV calls have no singleflight dedup (see module docs); the extras path
/// does, via `ExtrasState::live`.
pub(crate) async fn fetch_live(
    state: &AppState,
    channel_id: &str,
) -> anyhow::Result<LiveUrlOutput> {
    if !state.channel_allowed(channel_id).await {
        anyhow::bail!("channel {channel_id} is not available for the active account");
    }
    let is_custom = state.custom_channels.contains(channel_id);
    if let Some(content_id) = state
        .extras
        .route(channel_id, state.tv.logged_in(), is_custom)
    {
        return state.extras.live(&content_id, &state.store).await;
    }
    crate::token_refresh::ensure_fresh(state).await;
    state.tv.live(channel_id).await
}

/// The recovery-path refetch used by `render_m3u8_handler`/
/// `render_ts_handler` on a 401/403/404: de-duplicates concurrent recovery
/// attempts for the same channel behind a per-channel lock, with a "is it
/// still dead" re-check inside the lock — mirrors `refreshChannelToken`'s
/// `singleflight.Group.Do("channelID", ...)`.
async fn refresh_channel_token(
    state: &AppState,
    channel_id: &str,
) -> anyhow::Result<LiveUrlOutput> {
    if channel_id.is_empty() {
        anyhow::bail!("empty channel ID");
    }
    let _guard = state.render_caches.refresh_locks.lock(channel_id).await;
    fetch_live(state, channel_id).await
}

fn channel_and_quality(id_with_ext: &str) -> String {
    id_with_ext.trim_end_matches(".m3u8").to_string()
}

pub async fn live_handler(
    axum::extract::Path(id): axum::extract::Path<String>,
    State(state): State<Arc<AppState>>,
    prefix: Option<axum::Extension<crate::api::KeyPrefix>>,
) -> Response {
    live_impl(&state, &channel_and_quality(&id), "auto", &prefix).await
}

/// Channels 1349/1322 output audio-only m3u8 when a quality is forced, so every
/// HLS source selection for them must stay on `auto`.
pub(crate) fn hls_quality_for_channel<'a>(id: &str, quality: &'a str) -> &'a str {
    if id == "1349" || id == "1322" {
        "auto"
    } else {
        quality
    }
}

pub async fn live_quality_handler(
    axum::extract::Path((quality, id)): axum::extract::Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    prefix: Option<axum::Extension<crate::api::KeyPrefix>>,
) -> Response {
    let id = channel_and_quality(&id);
    let quality = hls_quality_for_channel(&id, &quality).to_string();
    live_impl(&state, &id, &quality, &prefix).await
}

async fn live_impl(
    state: &Arc<AppState>,
    id: &str,
    quality: &str,
    prefix: &Option<axum::Extension<crate::api::KeyPrefix>>,
) -> Response {
    let epoch = state.secure.current_epoch();
    let response = live_impl_inner(state, id, quality, prefix).await;
    state.stable_since(epoch, response)
}

async fn live_impl_inner(
    state: &Arc<AppState>,
    id: &str,
    quality: &str,
    prefix: &Option<axum::Extension<crate::api::KeyPrefix>>,
) -> Response {
    // Captured before any cache read: writes from before an account switch are dropped.
    let cache_gen = state.render_caches.generation();
    if !state.channel_allowed(id).await {
        return (
            StatusCode::NOT_FOUND,
            format!("Channel {id} is not available for the active account"),
        )
            .into_response();
    }
    if let Some(ch) = state.custom_channels.get(id) {
        return Redirect::to(&ch.url).into_response();
    }

    let live = match fetch_live(state, id).await {
        Ok(l) => l,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let mut live_url = television::select_best_live_hls_url(&live, quality);
    if live_url.is_empty() {
        // A extras channel with only DASH: send the player to the MPD route.
        let is_custom = state.custom_channels.contains(id);
        let via_extras = state
            .extras
            .route(id, state.tv.logged_in(), is_custom)
            .is_some();
        if via_extras && television::has_dash(&live) {
            let prefix_str = prefix.as_ref().map(|p| p.0 .0.clone()).unwrap_or_default();
            return Redirect::to(&format!("{prefix_str}/live/mpd/{id}?q={quality}"))
                .into_response();
        }
        let message = format!(
            "No stream found for channel id: {id}Status: {}",
            live.message
        );
        return (StatusCode::NOT_FOUND, message).into_response();
    }
    live_url = to_absolute_stream_url(&live_url, absolute_base_from_live(&live).as_deref());
    let live_hdnea = television::select_hls_hdnea_token(&live, quality, "");
    if !live_hdnea.is_empty() {
        let hdnea_key = RenderCaches::hdnea_key(id, &live_url, quality);
        state
            .render_caches
            .set_hdnea(cache_gen, &hdnea_key, &live_hdnea);
    }

    let encrypted = state.secure.encrypt(&live_url);
    let prefix_str = prefix.as_ref().map(|p| p.0 .0.clone()).unwrap_or_default();
    let q = if quality == "auto" {
        String::new()
    } else {
        format!("&q={quality}")
    };
    Redirect::to(&format!(
        "{prefix_str}/render.m3u8?auth={encrypted}&channel_key_id={id}{q}"
    ))
    .into_response()
}

#[derive(serde::Deserialize)]
pub struct RenderQuery {
    auth: Option<String>,
    channel_key_id: Option<String>,
    q: Option<String>,
    nested: Option<bool>,
}

/// Rewrites a fetched HLS manifest so every media/key URI routes back
/// through this server, mirroring `RenderHandler`'s two regex passes (done
/// here as plain line/substring scanning — see module docs).
fn rewrite_m3u8(
    body: &str,
    base_url: &str,
    params: &str,
    channel_id: &str,
    quality: &str,
    disable_ts_handler: bool,
) -> String {
    let mut out = String::with_capacity(body.len());
    for line in body.split_inclusive('\n') {
        let (content, newline) = match line.strip_suffix('\n') {
            Some(c) => (c, "\n"),
            None => (line, ""),
        };
        let trimmed = content.trim_end_matches('\r');
        let cr = if trimmed.len() != content.len() {
            "\r"
        } else {
            ""
        };

        if let Some(rewritten) =
            rewrite_media_attr_line(trimmed, base_url, params, channel_id, quality)
        {
            out.push_str(&rewritten);
            out.push_str(cr);
            out.push_str(newline);
            continue;
        }

        if let Some(rewritten) = rewrite_key_attr_line(trimmed, params, channel_id) {
            out.push_str(&rewritten);
            out.push_str(cr);
            out.push_str(newline);
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push_str(trimmed);
            out.push_str(cr);
            out.push_str(newline);
            continue;
        }

        let full_url = resolve_media_url(trimmed, base_url, params);
        let path_only = full_url
            .split('?')
            .next()
            .unwrap_or(&full_url)
            .to_lowercase();
        let endpoint = if path_only.ends_with(".m3u8") {
            Some(("/render.m3u8", true))
        } else if path_only.ends_with(".ts") {
            Some(("/render.ts", false))
        } else if path_only.ends_with(".aac") {
            // Players pick the HLS segment container from the URI extension;
            // packed audio behind a `.ts` path is parsed as MPEG-TS and dropped.
            Some(("/render.aac", false))
        } else {
            None
        };

        match endpoint {
            Some((endpoint, is_manifest)) => {
                if !is_manifest && disable_ts_handler {
                    out.push_str(&full_url);
                } else {
                    out.push_str(&build_encrypted_link(
                        endpoint,
                        &full_url,
                        channel_id,
                        quality,
                        is_manifest,
                    ));
                }
            }
            None => out.push_str(trimmed),
        }
        out.push_str(cr);
        out.push_str(newline);
    }
    out
}

/// A line's own query-string params (parsed from a `?`-suffixed relative
/// URI) win over the manifest-level `params` passed down from the request
/// that fetched this playlist, matching how `CreateEncryptedURL` just
/// concatenates whatever `Params` it's given onto whatever `Match` it's given.
fn resolve_media_url(uri: &str, base_url: &str, params: &str) -> String {
    let lower = uri.to_lowercase();
    let mut full = if lower.starts_with("http://") || lower.starts_with("https://") {
        uri.to_string()
    } else if let Ok(base) = url::Url::parse(base_url) {
        base.join(uri)
            .map(|url| url.to_string())
            .unwrap_or_else(|_| format!("{base_url}{uri}"))
    } else {
        format!("{base_url}{uri}")
    };
    if !params.is_empty() {
        let sep = if full.contains('?') { '&' } else { '?' };
        full.push(sep);
        full.push_str(params);
    }
    full
}

fn build_encrypted_link(
    endpoint: &str,
    full_url: &str,
    channel_id: &str,
    quality: &str,
    nested: bool,
) -> String {
    // Encryption happens in the caller (needs access to AppState::secure);
    // this function is only reached through `render_replace`, which does
    // the encryption inline. Kept separate for the unit tests below, which
    // exercise URL resolution without needing a real SecureUrl.
    format!(
        "{endpoint}||{full_url}||{channel_id}||{quality}||{}",
        if nested { "1" } else { "" }
    )
}

fn rewrite_key_attr_line(line: &str, params: &str, channel_id: &str) -> Option<String> {
    let upper = line.to_uppercase();
    if !(upper.starts_with("#EXT-X-KEY") || upper.starts_with("#EXT-X-SESSION-KEY")) {
        return None;
    }
    let uri_start = line.find("URI=\"")? + 5;
    let rest = &line[uri_start..];
    let uri_end = rest.find('"')?;
    let key_url = &rest[..uri_end];
    if !(key_url.starts_with("http://") || key_url.starts_with("https://")) {
        return None;
    }
    let replacement = build_encrypted_link("/render.key", key_url, channel_id, "", false);
    let _ = params;
    Some(format!(
        "{}{}{}",
        &line[..uri_start],
        replacement,
        &line[uri_start + uri_end..]
    ))
}

fn rewrite_media_attr_line(
    line: &str,
    base_url: &str,
    params: &str,
    channel_id: &str,
    quality: &str,
) -> Option<String> {
    if !line.starts_with("#EXT-X-MEDIA:") {
        return None;
    }
    let uri_start = line.find("URI=\"")? + 5;
    let uri_end = line[uri_start..].find('"')? + uri_start;
    let url = resolve_media_url(&line[uri_start..uri_end], base_url, params);
    let replacement = build_encrypted_link("/render.m3u8", &url, channel_id, quality, true);
    Some(format!(
        "{}{}{}",
        &line[..uri_start],
        replacement,
        &line[uri_end..]
    ))
}

/// Runs `rewrite_m3u8` and then actually encrypts every
/// `endpoint||url||id||q||nested`
/// placeholder it produced (see `build_encrypted_link`).
fn render_replace(
    state: &AppState,
    body: &str,
    base_url: &str,
    params: &str,
    channel_id: &str,
    quality: &str,
) -> String {
    let placeholder = rewrite_m3u8(
        body,
        base_url,
        params,
        channel_id,
        quality,
        state.config.disable_ts_handler,
    );
    let mut out = String::with_capacity(placeholder.len());
    let mut rest = placeholder.as_str();
    while let Some(start) = rest.find("/render.") {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        // Quoted URI attributes end before the closing quote. Preserve that
        // quote and any following attributes when encrypting the placeholder.
        let line_end = tail.find(['\n', '\r', '"']).unwrap_or(tail.len());
        let placeholder_str = &tail[..line_end];
        if let Some(encoded) = encode_placeholder(state, placeholder_str) {
            out.push_str(&encoded);
            rest = &tail[line_end..];
        } else {
            out.push_str(&tail[..1]);
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

fn encode_placeholder(state: &AppState, s: &str) -> Option<String> {
    let mut parts = s.splitn(5, "||");
    let endpoint = parts.next()?;
    let url = parts.next()?;
    let channel_id = parts.next()?;
    let quality = parts.next()?;
    let nested = parts.next().unwrap_or_default() == "1";
    let encrypted = state.secure.encrypt(url);
    let mut out = format!("{endpoint}?auth={encrypted}");
    if !channel_id.is_empty() {
        out.push_str(&format!("&channel_key_id={channel_id}"));
    }
    if !quality.is_empty() {
        out.push_str(&format!("&q={quality}"));
    }
    if nested {
        out.push_str("&nested=true");
    }
    Some(out)
}

pub async fn render_m3u8_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<RenderQuery>,
) -> Response {
    let epoch = state.secure.current_epoch();
    let response = render_m3u8_inner(state.clone(), q).await;
    state.stable_since(epoch, response)
}

async fn render_m3u8_inner(state: Arc<AppState>, q: RenderQuery) -> Response {
    // Captured before any cache read: writes from before an account switch are dropped.
    let cache_gen = state.render_caches.generation();
    let (Some(auth), Some(channel_id)) = (q.auth, q.channel_key_id) else {
        return (
            StatusCode::BAD_REQUEST,
            "auth and channel_key_id are required",
        )
            .into_response();
    };
    if channel_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "channel_key_id is required").into_response();
    }
    let decoded = match state.secure.decrypt(&auth) {
        Ok(d) => d,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid auth parameter").into_response(),
    };
    let decoded = to_absolute_stream_url(&decoded, None);
    let quality = q.q.unwrap_or_default();
    let nested = q.nested.unwrap_or(false);

    let hdnea_key = RenderCaches::hdnea_key(&channel_id, &decoded, &quality);
    let cached = state.render_caches.get_hdnea(&hdnea_key);
    let url_token = extract_hdnea_from_url(&decoded);
    let (mut render_url, mut token) = match &cached {
        Some(t) => (strip_hdnea_from_url(&decoded), t.clone()),
        None => (decoded.clone(), url_token.clone().unwrap_or_default()),
    };

    let mut rejected_token = token.clone();
    let (mut body, mut status, new_hdnea) = state.tv.render(&render_url, &token).await;
    if !new_hdnea.is_empty() {
        state
            .render_caches
            .set_hdnea(cache_gen, &hdnea_key, &new_hdnea);
        token = new_hdnea.clone();
    }

    if matches!(status, 401 | 403) && !new_hdnea.is_empty() && new_hdnea != rejected_token {
        rejected_token = token.clone();
        let (b, s, h) = state.tv.render(&render_url, &token).await;
        body = b;
        status = s;
        if !h.is_empty() {
            state.render_caches.set_hdnea(cache_gen, &hdnea_key, &h);
            token = h;
        }
    }

    if matches!(status, 401 | 403 | 404) {
        if status != 404 {
            state.render_caches.clear_hdnea(&hdnea_key);
        }
        let recently_dead = status == 404 && state.render_caches.is_dead(&channel_id);
        if !recently_dead && !channel_id.is_empty() {
            if let Ok(refreshed) = refresh_channel_token(&state, &channel_id).await {
                let retry_quality = if quality.is_empty() { "auto" } else { &quality };
                let rejected = rejected_hdnea_for_refresh(status, &rejected_token);
                let fresh_token =
                    television::select_hls_hdnea_token(&refreshed, retry_quality, rejected);
                if !fresh_token.is_empty() {
                    state
                        .render_caches
                        .set_hdnea(cache_gen, &hdnea_key, &fresh_token);
                    token = fresh_token;
                }
                render_url = strip_hdnea_from_url(&decoded);
                let (b, s, h) = state.tv.render(&render_url, &token).await;
                body = b;
                status = s;
                if !h.is_empty() {
                    state.render_caches.set_hdnea(cache_gen, &hdnea_key, &h);
                    token = h;
                }

                if status == 404 && !nested {
                    let candidates = [retry_quality, "auto", "high", "medium", "low"];
                    let mut tried = std::collections::HashSet::new();
                    tried.insert(render_url.clone());
                    for cq in candidates {
                        let candidate = television::select_best_live_hls_url(&refreshed, cq);
                        let candidate = to_absolute_stream_url(
                            &candidate,
                            absolute_base_from_live(&refreshed).as_deref(),
                        );
                        if candidate.is_empty() || tried.contains(&candidate) {
                            continue;
                        }
                        tried.insert(candidate.clone());
                        render_url = candidate;
                        let (b, s, h) = state.tv.render(&render_url, &token).await;
                        body = b;
                        status = s;
                        if !h.is_empty() {
                            state.render_caches.set_hdnea(cache_gen, &hdnea_key, &h);
                            token = h;
                        }
                        if status == 200 {
                            break;
                        }
                    }
                    if status == 404 {
                        state.render_caches.mark_dead(cache_gen, &channel_id);
                    } else {
                        state.render_caches.clear_dead(&channel_id);
                    }
                } else if status == 200 {
                    state.render_caches.clear_dead(&channel_id);
                }
            }
        }
    }

    let base_string_url = render_url
        .split('?')
        .next()
        .unwrap_or(&render_url)
        .to_string();
    let base_url = strip_trailing_filename(&base_string_url);
    let mut params = render_url
        .split_once('?')
        .map(|(_, q)| q.to_string())
        .unwrap_or_default();
    params = drop_hdnea_params(&params);
    params = append_hdnea_query_param(params, &token);

    let body_str = String::from_utf8_lossy(&body);
    let rewritten = render_replace(&state, &body_str, &base_url, &params, &channel_id, &quality);

    let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    Response::builder()
        .status(status_code)
        .header(header::CACHE_CONTROL, "public, must-revalidate, max-age=3")
        .body(Body::from(rewritten))
        .unwrap()
}

fn strip_trailing_filename(base: &str) -> String {
    match base.rfind('/') {
        Some(idx) if base[idx + 1..].to_lowercase().ends_with(".m3u8") => {
            base[..idx + 1].to_string()
        }
        _ => base.to_string(),
    }
}

fn drop_hdnea_params(params: &str) -> String {
    params
        .split('&')
        .filter(|p| !p.is_empty() && !p.starts_with("hdnea=") && !p.starts_with("__hdnea__="))
        .collect::<Vec<_>>()
        .join("&")
}

fn append_hdnea_query_param(params: String, token: &str) -> String {
    if token.is_empty() {
        return params;
    }
    let encoded = urlencoding::encode(token);
    if params.is_empty() {
        format!("__hdnea__={encoded}")
    } else {
        format!("{params}&__hdnea__={encoded}")
    }
}

fn rejected_hdnea_for_refresh(status: u16, token: &str) -> &str {
    if matches!(status, 401 | 403) {
        token
    } else {
        ""
    }
}

#[derive(serde::Deserialize)]
pub struct SegmentQuery {
    auth: Option<String>,
    channel_key_id: Option<String>,
    hdnea: Option<String>,
    q: Option<String>,
}

pub async fn render_ts_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SegmentQuery>,
) -> Response {
    // Captured before any cache read: writes from before an account switch are dropped.
    let cache_gen = state.render_caches.generation();
    let Some(auth) = q.auth else {
        return (StatusCode::BAD_REQUEST, "auth is required").into_response();
    };
    let Some(channel_id) = q.channel_key_id.filter(|id| !id.is_empty()) else {
        return (StatusCode::BAD_REQUEST, "channel_key_id is required").into_response();
    };
    let mut decoded = match state.secure.decrypt(&auth) {
        Ok(d) => d,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid auth parameter").into_response(),
    };

    let quality = q.q.as_deref().filter(|q| !q.is_empty()).unwrap_or("auto");
    let hdnea_key = RenderCaches::hdnea_key(&channel_id, &decoded, quality);
    let mut token = q
        .hdnea
        .clone()
        .or_else(|| state.render_caches.get_hdnea(&hdnea_key));
    if token.is_some() {
        decoded = strip_hdnea_from_url(&decoded);
    } else {
        token = extract_hdnea_from_url(&decoded);
    }

    let resp = proxy_segment(&state, &decoded, token.as_deref()).await;
    let (mut status, mut body, mut headers) = resp;

    if matches!(status, 401 | 403) {
        state.render_caches.clear_hdnea(&hdnea_key);
        let stripped = strip_hdnea_from_url(&decoded);
        let mut fresh_token = None;
        if !channel_id.is_empty() {
            if let Ok(refreshed) = refresh_channel_token(&state, &channel_id).await {
                let rejected_token = token.as_deref().unwrap_or_default();
                let refreshed_token =
                    television::select_hls_hdnea_token(&refreshed, quality, rejected_token);
                if !refreshed_token.is_empty() {
                    state
                        .render_caches
                        .set_hdnea(cache_gen, &hdnea_key, &refreshed_token);
                    fresh_token = Some(refreshed_token);
                }
            }
        }
        let (s, b, h) = proxy_segment(&state, &stripped, fresh_token.as_deref()).await;
        status = s;
        body = b;
        headers = h;
    }

    let mut builder =
        Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY));
    if let Some(ct) = headers {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    builder.body(Body::from(body)).unwrap()
}

async fn proxy_segment(
    state: &AppState,
    url: &str,
    hdnea: Option<&str>,
) -> (u16, Vec<u8>, Option<String>) {
    let ua = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| state.extras.player_user_agent_for(h)))
        .unwrap_or(television::PLAYER_USER_AGENT);
    let mut req = state.http.get(url).header(header::USER_AGENT, ua);
    if let Some(t) = hdnea {
        req = req.header(header::COOKIE, format!("__hdnea__={t}"));
    }
    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let ct = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let body = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
            (status, body, ct)
        }
        Err(_) => (502, Vec::new(), None),
    }
}

/// `/render.key` — an AES-128 HLS key request. Mirrors `RenderKeyHandler`:
/// a extras-routed channel gets the extras app's key headers
/// (`extrasKeyHeaders`); otherwise it gets the usual JioTV ones.
pub async fn render_key_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SegmentQuery>,
) -> Response {
    let Some(auth) = q.auth else {
        return (StatusCode::BAD_REQUEST, "auth is required").into_response();
    };
    let Some(channel_id) = q.channel_key_id.clone().filter(|id| !id.is_empty()) else {
        return (StatusCode::BAD_REQUEST, "channel_key_id is required").into_response();
    };
    let decoded = match state.secure.decrypt(&auth) {
        Ok(d) => d,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid auth parameter").into_response(),
    };
    let hdnea = q.hdnea.clone().or_else(|| extract_hdnea_from_url(&decoded));
    let is_custom = state.custom_channels.contains(&channel_id);

    let mut req = state
        .http
        .get(&decoded)
        .header(header::USER_AGENT, television::PLAYER_USER_AGENT);
    if let Some(t) = &hdnea {
        req = req.header(header::COOKIE, format!("__hdnea__={t}"));
    }
    if let Some(content_id) = state
        .extras
        .route(&channel_id, state.tv.logged_in(), is_custom)
    {
        for (k, v) in state.extras.key_headers(&content_id) {
            req = req.header(k, v);
        }
    } else {
        let creds = state.tv.creds.read().unwrap().clone().unwrap_or_default();
        req = req
            .header("srno", "230203144000")
            .header("ssotoken", &creds.sso_token)
            .header("channelId", &channel_id)
            .header("accesstoken", &creds.access_token)
            .header("crmid", &creds.crm)
            .header("uniqueId", &creds.unique_id);
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
        Err(_) => (StatusCode::BAD_GATEWAY, "key upstream request failed").into_response(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn render_cache_writes_from_before_a_clear_are_dropped() {
        let caches = RenderCaches::default();
        let started = caches.generation();
        caches.set_hdnea(started, "154", "old-token");
        assert_eq!(caches.get_hdnea("154").as_deref(), Some("old-token"));

        // An account switch clears the caches; the request that captured
        // `started` is still running and tries to write afterwards.
        caches.clear();
        caches.set_hdnea(started, "154", "old-token");
        caches.mark_dead(started, "154");
        assert!(caches.get_hdnea("154").is_none());
        assert!(!caches.is_dead("154"));

        let current = caches.generation();
        caches.set_hdnea(current, "154", "new-token");
        caches.mark_dead(current, "154");
        assert_eq!(caches.get_hdnea("154").as_deref(), Some("new-token"));
        assert!(caches.is_dead("154"));
    }

    use super::*;

    #[test]
    fn hdnea_cache_key_is_scoped_to_hls_quality() {
        let high_segment_one = RenderCaches::hdnea_key(
            "154",
            "https://cdn.example/high/segment-001.ts?hdnea=token-one",
            "high",
        );
        let high_segment_two = RenderCaches::hdnea_key(
            "154",
            "https://cdn.example/high/segment-002.ts?hdnea=token-two",
            "high",
        );
        let auto_segment = RenderCaches::hdnea_key(
            "154",
            "https://cdn.example/auto/segment-001.ts?hdnea=token-three",
            "auto",
        );

        assert_eq!(high_segment_one, high_segment_two);
        assert_ne!(high_segment_one, auto_segment);
    }

    #[test]
    fn only_auth_failures_reject_the_previous_hdnea_token() {
        assert_eq!(rejected_hdnea_for_refresh(401, "old-token"), "old-token");
        assert_eq!(rejected_hdnea_for_refresh(403, "old-token"), "old-token");
        assert_eq!(rejected_hdnea_for_refresh(404, "old-token"), "");
    }

    #[test]
    fn absolute_url_is_passed_through() {
        assert_eq!(
            to_absolute_stream_url("https://a.b/c.m3u8", None),
            "https://a.b/c.m3u8"
        );
    }

    #[test]
    fn protocol_relative_url_gets_https() {
        assert_eq!(
            to_absolute_stream_url("//a.b/c.m3u8", None),
            "https://a.b/c.m3u8"
        );
    }

    #[test]
    fn relative_path_uses_fallback_base() {
        assert_eq!(
            to_absolute_stream_url("/foo/bar.m3u8", None),
            "https://jiotvapi.cdn.jio.com/foo/bar.m3u8"
        );
    }

    #[test]
    fn strips_hdnea_query_param() {
        assert_eq!(
            strip_hdnea_from_url("https://a.b/c.ts?hdnea=xyz&x=1"),
            "https://a.b/c.ts?x=1"
        );
        assert_eq!(
            strip_hdnea_from_url("https://a.b/c.ts?__hdnea__=xyz"),
            "https://a.b/c.ts"
        );
    }

    #[test]
    fn extracts_hdnea_from_query() {
        assert_eq!(
            extract_hdnea_from_url("https://a.b/c.ts?hdnea=abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(extract_hdnea_from_url("https://a.b/c.ts?x=1"), None);
    }

    #[test]
    fn strip_trailing_filename_keeps_directory() {
        assert_eq!(
            strip_trailing_filename("https://a.b/path/master.m3u8"),
            "https://a.b/path/"
        );
        assert_eq!(
            strip_trailing_filename("https://a.b/path/"),
            "https://a.b/path/"
        );
    }

    #[test]
    fn rewrites_master_playlist_variant_lines() {
        let body = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nchunk_1.m3u8?hdnea=old\n";
        let rewritten = rewrite_m3u8(body, "https://a.b/live/", "", "154", "auto", false);
        assert!(
            rewritten.contains("/render.m3u8||https://a.b/live/chunk_1.m3u8?hdnea=old||154||auto")
        );
    }

    #[test]
    fn resolves_root_relative_manifest_uri_against_origin() {
        assert_eq!(
            resolve_media_url(
                "/bpk-tv/channel/variant.m3u8",
                "https://a.b/live/channel/",
                ""
            ),
            "https://a.b/bpk-tv/channel/variant.m3u8"
        );
    }

    #[test]
    fn resolves_parent_relative_manifest_uri() {
        assert_eq!(
            resolve_media_url("../variant.m3u8", "https://a.b/live/channel/", ""),
            "https://a.b/live/variant.m3u8"
        );
    }

    #[test]
    fn rewrites_ts_segment_lines() {
        let body = "#EXTM3U\nseg1.ts\n";
        let rewritten = rewrite_m3u8(
            body,
            "https://a.b/live/",
            "__hdnea__=fresh",
            "154",
            "auto",
            false,
        );
        assert!(
            rewritten.contains("/render.ts||https://a.b/live/seg1.ts?__hdnea__=fresh||154||auto")
        );
    }

    #[test]
    fn rewrites_packed_audio_segments_to_aac_route() {
        let body = "#EXTM3U\n#EXTINF:4,\naudio_1.aac?x=1\n#EXTINF:4,\nvideo_1.ts\n";
        let rewritten = rewrite_m3u8(body, "https://a.b/live/", "", "ex_1", "auto", false);
        assert!(rewritten.contains("\n/render.aac||https://a.b/live/audio_1.aac?x=1||ex_1||auto"));
        assert!(rewritten.contains("\n/render.ts||https://a.b/live/video_1.ts||ex_1||auto"));
    }

    #[test]
    fn rewritten_segments_keep_forced_quality() {
        let body = "#EXTM3U\nseg1.ts\n";
        let rewritten = rewrite_m3u8(body, "https://a.b/live/", "", "154", "high", false);
        assert!(rewritten.contains("/render.ts||https://a.b/live/seg1.ts||154||high"));
    }

    #[test]
    fn hdnea_query_param_is_percent_encoded() {
        let params = append_hdnea_query_param(
            "foo=bar".to_string(),
            "st=100~exp=200~acl=/*&scope=live#fragment",
        );
        assert!(!params.contains("&scope=live"));
        assert!(!params.contains("#fragment"));
        assert_eq!(
            extract_hdnea_from_url(&format!("https://a.b/live.ts?{params}")).as_deref(),
            Some("st=100~exp=200~acl=/*&scope=live#fragment")
        );
    }

    #[test]
    fn ts_passthrough_when_disabled() {
        let body = "#EXTM3U\nseg1.ts\n";
        let rewritten = rewrite_m3u8(body, "https://a.b/live/", "", "154", "auto", true);
        assert_eq!(rewritten, "#EXTM3U\nhttps://a.b/live/seg1.ts\n");
    }

    #[test]
    fn rewrites_key_uri_in_ext_x_key_line() {
        let body =
            "#EXT-X-KEY:METHOD=AES-128,URI=\"https://tv.media.jio.com/key.pkey\",IV=0x1\nseg1.ts\n";
        let rewritten = rewrite_m3u8(body, "https://a.b/live/", "", "154", "auto", false);
        assert!(rewritten.starts_with("#EXT-X-KEY:METHOD=AES-128,URI=\"/render.key||https://tv.media.jio.com/key.pkey||154||||\""));
        assert!(rewritten.contains(",IV=0x1"));
    }

    #[test]
    fn full_render_replace_encrypts_placeholders() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            std::sync::Arc::new(crate::store::Store::open(dir.path().to_str().unwrap()).unwrap());
        let secure = crate::secureurl::SecureUrl::new(false);
        let state = AppState {
            config: crate::config::Config::default(),
            path_prefix: String::new(),
            access: std::sync::Arc::new(crate::access::Access::new(store.clone())),
            store,
            tv: std::sync::Arc::new(crate::television::Television::new(reqwest::Client::new())),
            secure: std::sync::Arc::new(secure),
            http: reqwest::Client::new(),
            drm_channels: Default::default(),
            custom_channels: std::sync::Arc::new(crate::custom_channels::CustomChannels::new()),
            render_caches: Default::default(),
            dash_state: Default::default(),
            epg_state: Default::default(),
            extras: Arc::new(crate::extras_state::ExtrasState::new(false, None)),
            vod_state: Default::default(),
            public_ip: Arc::new(crate::unlock::PublicIp::new(reqwest::Client::new())),
            unlock_limiter: Arc::new(crate::unlock::AttemptLimiter::default()),
            listen: Default::default(),
        };
        let body = "#EXTM3U\nseg1.ts\n";
        let out = render_replace(&state, body, "https://a.b/live/", "", "154", "auto");
        assert!(out.contains("/render.ts?auth="));
        assert!(out.contains("&channel_key_id=154"));
        assert!(!out.contains("||"));

        for tag in ["EXT-X-KEY", "EXT-X-SESSION-KEY"] {
            for suffix in ["", ",IV=0x1,KEYFORMAT=\"identity\""] {
                let body = format!(
                    "#EXTM3U\n#{tag}:METHOD=AES-128,URI=\"https://a.b/key.pkey\"{suffix}\nseg1.ts\n"
                );
                let out = render_replace(&state, &body, "https://a.b/live/", "", "154", "auto");
                let line = out.lines().nth(1).unwrap();
                let uri = line.split("URI=\"").nth(1).unwrap();
                let (uri, remainder) = uri.split_once('"').expect("key URI must close");
                assert_eq!(remainder, suffix);
                assert!(uri.starts_with("/render.key?auth="));
                let url = url::Url::parse(&format!("http://localhost{uri}")).unwrap();
                let auth = url.query_pairs().find(|(k, _)| k == "auth").unwrap().1;
                assert_eq!(state.secure.decrypt(&auth).unwrap(), "https://a.b/key.pkey");
                assert!(out.contains("/render.ts?auth="));
                assert!(!out.contains("||"));
            }
        }

        let muxed = "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"Hindi\"";
        for (uri, expected) in [
            (
                "audio.m3u8?language=hin",
                "https://a.b/live/audio.m3u8?language=hin&__hdnea__=fresh",
            ),
            ("/audio.m3u8", "https://a.b/audio.m3u8?__hdnea__=fresh"),
            (
                "https://c.d/audio.m3u8",
                "https://c.d/audio.m3u8?__hdnea__=fresh",
            ),
        ] {
            let body = format!("{muxed},URI=\"{uri}\",DEFAULT=YES\r\n{muxed}\r\n");
            let out = render_replace(
                &state,
                &body,
                "https://a.b/live/",
                "__hdnea__=fresh",
                "154",
                "auto",
            );
            let uri = out.lines().next().unwrap().split("URI=\"").nth(1).unwrap();
            let (uri, suffix) = uri.split_once('"').unwrap();
            assert_eq!(suffix, ",DEFAULT=YES");
            let url = url::Url::parse(&format!("http://localhost{uri}")).unwrap();
            let auth = url.query_pairs().find(|(k, _)| k == "auth").unwrap().1;
            assert_eq!(state.secure.decrypt(&auth).unwrap(), expected);
            assert!(uri.contains("&nested=true"));
            assert!(out.ends_with(&format!("{muxed}\r\n")));
        }
    }

    #[test]
    fn child_manifest_links_are_marked_nested() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            std::sync::Arc::new(crate::store::Store::open(dir.path().to_str().unwrap()).unwrap());
        let secure = crate::secureurl::SecureUrl::new(false);
        let state = AppState {
            config: crate::config::Config::default(),
            path_prefix: String::new(),
            access: std::sync::Arc::new(crate::access::Access::new(store.clone())),
            store,
            tv: std::sync::Arc::new(crate::television::Television::new(reqwest::Client::new())),
            secure: std::sync::Arc::new(secure),
            http: reqwest::Client::new(),
            drm_channels: Default::default(),
            custom_channels: std::sync::Arc::new(crate::custom_channels::CustomChannels::new()),
            render_caches: Default::default(),
            dash_state: Default::default(),
            epg_state: Default::default(),
            extras: Arc::new(crate::extras_state::ExtrasState::new(false, None)),
            vod_state: Default::default(),
            public_ip: Arc::new(crate::unlock::PublicIp::new(reqwest::Client::new())),
            unlock_limiter: Arc::new(crate::unlock::AttemptLimiter::default()),
            listen: Default::default(),
        };
        let body = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nchild.m3u8\n";
        let out = render_replace(&state, body, "https://a.b/live/", "", "154", "auto");
        assert!(out.contains("/render.m3u8?auth="));
        assert!(out.contains("&nested=true"));
    }
}
