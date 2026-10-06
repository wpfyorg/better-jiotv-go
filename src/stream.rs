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
const RENDER_AUTH_PREFIX: &str = "jiotv-hls-v1:";

#[derive(serde::Serialize, serde::Deserialize)]
struct RenderAuthPayload {
    url: String,
    scope: String,
}

// Identity travels only inside the encrypted scope, including on segment links.
#[derive(serde::Serialize, serde::Deserialize)]
struct ManifestIdentity {
    url: String,
    // None identifies a root selected by the live API.
    selector: Option<String>,
}

fn identity_scope(url: &str, selector: Option<String>) -> String {
    format!(
        "identity:{}",
        hex::encode(
            serde_json::to_vec(&ManifestIdentity {
                url: strip_hdnea_from_url(url),
                selector,
            })
            .expect("manifest identity serializes")
        )
    )
}

fn manifest_identity(scope: &str) -> Option<ManifestIdentity> {
    serde_json::from_slice(&hex::decode(scope.strip_prefix("identity:")?).ok()?).ok()
}

// Split HLS attributes without splitting commas inside quoted values.
fn rendition_selector(line: &str) -> Option<String> {
    let (tag, attrs) = line.split_once(':')?;
    let mut quoted = false;
    let mut start = 0;
    let mut fields = std::collections::BTreeMap::new();
    for (i, c) in attrs
        .char_indices()
        .chain(std::iter::once((attrs.len(), ',')))
    {
        if c == '"' {
            quoted = !quoted;
        }
        if c == ',' && !quoted {
            let (key, value) = attrs[start..i].split_once('=')?;
            if key != "URI" && fields.insert(key.trim(), value.trim()).is_some() {
                return None;
            }
            start = i + 1;
        }
    }
    if quoted {
        return None;
    }
    match tag {
        "#EXT-X-STREAM-INF" | "#EXT-X-I-FRAME-STREAM-INF" if fields.contains_key("BANDWIDTH") => {}
        "#EXT-X-MEDIA" if fields.get("TYPE") == Some(&"AUDIO") => {
            if !fields.contains_key("GROUP-ID") || !fields.contains_key("NAME") {
                return None;
            }
            fields.retain(|key, _| matches!(*key, "TYPE" | "GROUP-ID" | "NAME" | "LANGUAGE"));
        }
        _ => return None,
    }
    Some(format!("{tag}:{}", serde_json::to_string(&fields).ok()?))
}

fn rendition_entries(body: &str, base: &str) -> Vec<(String, String)> {
    let params = url::Url::parse(base)
        .ok()
        .map(|u| drop_hdnea_params(u.query().unwrap_or_default()))
        .unwrap_or_default();
    let mut pending = None;
    let mut entries = Vec::new();
    for line in body.lines().map(str::trim) {
        if line.starts_with("#EXT-X-STREAM-INF:") {
            pending = rendition_selector(line);
        } else if line.starts_with("#EXT-X-MEDIA:")
            || line.starts_with("#EXT-X-I-FRAME-STREAM-INF:")
        {
            if let (Some(selector), Some(uri)) = (
                rendition_selector(line),
                line.split_once("URI=\"")
                    .and_then(|(_, rest)| rest.split('"').next()),
            ) {
                entries.push((resolve_media_url(uri, base, &params), selector));
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            if let Some(selector) = pending.take() {
                entries.push((resolve_media_url(line, base, &params), selector));
            }
        }
    }
    entries
}

struct RecoveredManifest {
    url: String,
    token: String,
    shared_media: bool,
}

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

    fn hdnea_key(channel_id: &str, quality: &str, scope: &str) -> String {
        let quality = if quality.is_empty() { "auto" } else { quality };
        let scope = manifest_identity(scope)
            .map(|i| identity_scope(&i.url, i.selector))
            .unwrap_or_else(|| strip_hdnea_from_url(scope));
        format!("{channel_id}|hls|{quality}|{scope}")
    }

    pub fn get_hdnea(&self, key: &str) -> Option<String> {
        let expired_at = {
            let map = self.hdnea.read().unwrap();
            let (token, at) = map.get(key)?;
            if at.elapsed() <= HDNEA_CACHE_TTL {
                return Some(token.clone());
            }
            *at
        };
        let mut map = self.hdnea.write().unwrap();
        if map
            .get(key)
            .map(|(_, at)| *at == expired_at)
            .unwrap_or(false)
        {
            map.remove(key);
        }
        None
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
        map.retain(|_, (_, at)| at.elapsed() <= HDNEA_CACHE_TTL);
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

// Share only a credential verified as common to every media URI in this playlist.
fn media_credential(
    body: &str,
    child: &str,
    target: &str,
    fallback: &str,
) -> Option<(String, bool)> {
    let params = url::Url::parse(child)
        .ok()
        .map(|u| drop_hdnea_params(u.query().unwrap_or_default()))
        .unwrap_or_default();
    let mut all = std::collections::HashSet::new();
    let mut exact = std::collections::HashSet::new();
    for line in body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let candidate = resolve_media_url(line, child, &params);
        let token = extract_hdnea_from_url(&candidate).unwrap_or_else(|| fallback.to_string());
        if strip_hdnea_from_url(&candidate) == strip_hdnea_from_url(target) {
            exact.insert(token.clone());
        }
        all.insert(token);
    }
    if exact.len() == 1 {
        Some((exact.into_iter().next().unwrap(), all.len() == 1))
    } else {
        None
    }
}

fn shared_media_key(rendition_key: &str, original_token: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{rendition_key}|shared|{}",
        hex::encode(Sha256::digest(original_token.as_bytes()))
    )
}

async fn render_with_cookie_retry(
    state: &AppState,
    url: &str,
    token: &str,
) -> (Vec<u8>, u16, String) {
    let (body, status, cookie) = state.tv.render(url, token).await;
    if matches!(status, 401 | 403) && !cookie.is_empty() && cookie != token {
        let (body, status, next_cookie) =
            state.tv.render(&strip_hdnea_from_url(url), &cookie).await;
        return (
            body,
            status,
            if next_cookie.is_empty() {
                cookie
            } else {
                next_cookie
            },
        );
    }
    (body, status, cookie)
}

/// Recover from the refreshed rendition itself, never reuse a master token
/// for a child merely because both were requested with auto quality.
async fn refresh_rendition(
    state: &AppState,
    live: &LiveUrlOutput,
    quality: &str,
    scope: &str,
    rejected: &str,
    media_url: Option<&str>,
) -> Option<RecoveredManifest> {
    let root = television::select_best_live_hls_url(live, quality);
    if root.is_empty() {
        return None;
    }
    let root = to_absolute_stream_url(&root, absolute_base_from_live(live).as_deref());
    let identity = manifest_identity(scope);
    let original_url = identity.as_ref().map(|i| i.url.as_str()).unwrap_or(scope);
    // Legacy links have no authenticated manifest scope.
    if media_url.is_none()
        && (scope.starts_with("legacy-")
            || identity.as_ref().is_some_and(|i| i.selector.is_none())
            || (identity.is_none() && strip_hdnea_from_url(&root) == original_url))
    {
        return Some(RecoveredManifest {
            shared_media: false,
            token: television::select_hls_hdnea_token(live, quality, rejected),
            url: root,
        });
    }
    let root_token = extract_hdnea_from_url(&root).unwrap_or_default();
    let (mut body, mut status, mut root_cookie) = state.tv.render(&root, &root_token).await;
    if matches!(status, 401 | 403) && !root_cookie.is_empty() && root_cookie != root_token {
        let (next_body, next_status, next_cookie) = state
            .tv
            .render(&strip_hdnea_from_url(&root), &root_cookie)
            .await;
        body = next_body;
        status = next_status;
        if !next_cookie.is_empty() {
            root_cookie = next_cookie;
        }
    }
    if status != 200 {
        return None;
    }
    let child = if identity.as_ref().is_some_and(|i| i.selector.is_none())
        || (identity.is_none() && strip_hdnea_from_url(&root) == original_url)
    {
        Some(root.clone())
    } else {
        matching_rendition_url(&String::from_utf8_lossy(&body), &root, scope)
    };
    let child = child?;
    // Tokenless child URIs inherit master credentials in the original rewrite.
    // Verify that credential against this child before returning it for media.
    let child_is_root = child == root;
    let child_token = if child_is_root && !root_cookie.is_empty() {
        root_cookie.clone()
    } else {
        extract_hdnea_from_url(&child).unwrap_or_else(|| {
            if root_cookie.is_empty() {
                root_token.clone()
            } else {
                root_cookie.clone()
            }
        })
    };
    let child_request = if child_token.is_empty() {
        child.clone()
    } else {
        strip_hdnea_from_url(&child)
    };
    let (mut body, mut status, mut cookie) = if child_is_root {
        (body, status, root_cookie)
    } else {
        state.tv.render(&child_request, &child_token).await
    };
    if matches!(status, 401 | 403) && !cookie.is_empty() && cookie != child_token {
        let (next_body, next_status, next_cookie) = state
            .tv
            .render(&strip_hdnea_from_url(&child_request), &cookie)
            .await;
        body = next_body;
        status = next_status;
        if !next_cookie.is_empty() {
            cookie = next_cookie;
        }
    }
    if status != 200 {
        return None;
    }
    if let Some(media_url) = media_url {
        let fallback = if cookie.is_empty() {
            &child_token
        } else {
            &cookie
        };
        let (token, shared_media) =
            media_credential(&String::from_utf8_lossy(&body), &child, media_url, fallback)?;
        return Some(RecoveredManifest {
            url: child,
            token,
            shared_media,
        });
    }
    if !cookie.is_empty() {
        return Some(RecoveredManifest {
            shared_media: false,
            url: child,
            token: cookie,
        });
    }
    if !child_token.is_empty() {
        return Some(RecoveredManifest {
            shared_media: false,
            url: child,
            token: child_token,
        });
    }
    // Media-only credentials stay on their own rewritten URIs.
    Some(RecoveredManifest {
        shared_media: false,
        url: child,
        token: String::new(),
    })
}

#[cfg(test)]
async fn refresh_rendition_token(
    state: &AppState,
    live: &LiveUrlOutput,
    quality: &str,
    scope: &str,
    rejected: &str,
    media_url: Option<&str>,
) -> String {
    refresh_rendition(state, live, quality, scope, rejected, media_url)
        .await
        .map(|r| r.token)
        .unwrap_or_default()
}

fn matching_rendition_url(body: &str, master_url: &str, scope: &str) -> Option<String> {
    if let Some(identity) = manifest_identity(scope) {
        let entries = rendition_entries(body, master_url);
        let exact: Vec<_> = entries
            .iter()
            .filter(|(u, _)| strip_hdnea_from_url(u) == identity.url)
            .collect();
        if exact.len() == 1 {
            return Some(exact[0].0.clone());
        }
        if exact.len() > 1 {
            return None;
        }
        let selector = identity.selector?;
        let matches: Vec<_> = entries
            .into_iter()
            .filter(|(_, s)| *s == selector)
            .collect();
        return if matches.len() == 1 {
            Some(matches[0].0.clone())
        } else {
            None
        };
    }
    let base = url::Url::parse(master_url).ok()?;
    let params = drop_hdnea_params(base.query().unwrap_or_default());
    for line in body.lines() {
        let line = line.trim();
        let uri = if line.starts_with("#EXT-X-MEDIA:")
            || line.starts_with("#EXT-X-I-FRAME-STREAM-INF:")
        {
            match line
                .split_once("URI=\"")
                .and_then(|(_, rest)| rest.split('"').next())
            {
                Some(uri) => uri,
                None => continue,
            }
        } else if line.is_empty() || line.starts_with('#') {
            continue;
        } else {
            line
        };
        let candidate = resolve_media_url(uri, master_url, &params);
        if strip_hdnea_from_url(&candidate) == scope {
            return Some(candidate);
        }
    }
    None
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
        let scope = identity_scope(&live_url, None);
        let hdnea_key = RenderCaches::hdnea_key(id, quality, &scope);
        state
            .render_caches
            .set_hdnea(cache_gen, &hdnea_key, &live_hdnea);
    }

    let encrypted = encrypt_render_auth(state, &live_url, &identity_scope(&live_url, None));
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
    scope: &str,
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
                    let link_scope = if is_manifest {
                        strip_hdnea_from_url(&full_url)
                    } else {
                        scope.to_string()
                    };
                    out.push_str(&build_encrypted_link(
                        endpoint,
                        &full_url,
                        channel_id,
                        quality,
                        is_manifest,
                        &link_scope,
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
    scope: &str,
) -> String {
    // Encryption happens in the caller (needs access to AppState::secure);
    // this function is only reached through `render_replace`, which does
    // the encryption inline. Kept separate for the unit tests below, which
    // exercise URL resolution without needing a real SecureUrl.
    format!(
        "{endpoint}||{full_url}||{channel_id}||{quality}||{}||{scope}",
        if nested { "1" } else { "" },
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
    let replacement = build_encrypted_link("/render.key", key_url, channel_id, "", false, "");
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
    if !line.starts_with("#EXT-X-MEDIA:") && !line.starts_with("#EXT-X-I-FRAME-STREAM-INF:") {
        return None;
    }
    let uri_start = line.find("URI=\"")? + 5;
    let uri_end = line[uri_start..].find('"')? + uri_start;
    let url = resolve_media_url(&line[uri_start..uri_end], base_url, params);
    let scope = strip_hdnea_from_url(&url);
    let replacement = build_encrypted_link("/render.m3u8", &url, channel_id, quality, true, &scope);
    Some(format!(
        "{}{}{}",
        &line[..uri_start],
        replacement,
        &line[uri_end..]
    ))
}

/// Runs `rewrite_m3u8` and then actually encrypts every
/// `endpoint||url||id||q||nested||scope`
/// placeholder it produced (see `build_encrypted_link`).
fn render_replace(
    state: &AppState,
    body: &str,
    base_url: &str,
    params: &str,
    channel_id: &str,
    quality: &str,
    scope: &str,
) -> String {
    let placeholder = rewrite_m3u8(
        body,
        base_url,
        params,
        channel_id,
        quality,
        scope,
        state.config.disable_ts_handler,
    );
    let entries = rendition_entries(
        body,
        &resolve_media_url(base_url, base_url, &drop_hdnea_params(params)),
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
        let mut parts: Vec<_> = placeholder_str.splitn(6, "||").map(str::to_owned).collect();
        if parts.len() == 6 && parts[4] == "1" {
            let candidate = resolve_media_url(&parts[1], base_url, "");
            let matches: Vec<_> = entries
                .iter()
                .filter(|(url, _)| strip_hdnea_from_url(url) == strip_hdnea_from_url(&candidate))
                .collect();
            if matches.len() == 1
                && entries
                    .iter()
                    .filter(|(_, selector)| selector == &matches[0].1)
                    .count()
                    == 1
            {
                parts[5] = identity_scope(&parts[1], Some(matches[0].1.clone()));
            }
        }
        if let Some(encoded) = encode_placeholder(state, &parts.join("||")) {
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
    let mut parts = s.splitn(6, "||");
    let endpoint = parts.next()?;
    let url = parts.next()?;
    let channel_id = parts.next()?;
    let quality = parts.next()?;
    let nested = parts.next().unwrap_or_default() == "1";
    let scope = parts.next().unwrap_or_default();
    let encrypted = encrypt_render_auth(state, url, scope);
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

fn encrypt_render_auth(state: &AppState, url: &str, scope: &str) -> String {
    if scope.is_empty() {
        return state.secure.encrypt(url);
    }
    let payload = RenderAuthPayload {
        url: url.to_string(),
        scope: scope.to_string(),
    };
    let serialized = serde_json::to_string(&payload).expect("render auth payload serializes");
    state
        .secure
        .encrypt(&format!("{RENDER_AUTH_PREFIX}{serialized}"))
}

fn decrypt_render_auth(state: &AppState, auth: &str) -> Option<(String, Option<String>)> {
    let decoded = state.secure.decrypt(auth).ok()?;
    let Some(serialized) = decoded.strip_prefix(RENDER_AUTH_PREFIX) else {
        return Some((decoded, None));
    };
    let payload: RenderAuthPayload = serde_json::from_str(serialized).ok()?;
    Some((payload.url, Some(payload.scope)))
}

fn valid_hls_quality(quality: &str) -> bool {
    quality.is_empty()
        || matches!(
            quality,
            "auto" | "high" | "h" | "medium" | "med" | "m" | "low" | "l"
        )
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
    let (decoded, authenticated_scope) = match decrypt_render_auth(&state, &auth) {
        Some(decoded) => decoded,
        None => return (StatusCode::BAD_REQUEST, "invalid auth parameter").into_response(),
    };
    let decoded = to_absolute_stream_url(&decoded, None);
    let mut quality = q.q.unwrap_or_default();
    if !valid_hls_quality(&quality) {
        return (StatusCode::BAD_REQUEST, "invalid quality parameter").into_response();
    }
    let nested = q.nested.unwrap_or(false);
    let mut scope = authenticated_scope
        .filter(|scope| !scope.is_empty())
        .unwrap_or_else(|| strip_hdnea_from_url(&decoded));

    let mut hdnea_key = RenderCaches::hdnea_key(&channel_id, &quality, &scope);
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
        render_url = strip_hdnea_from_url(&render_url);
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
                let retry_quality = if quality.is_empty() {
                    "auto".to_string()
                } else {
                    quality.clone()
                };
                let recovered = refresh_rendition(
                    &state,
                    &refreshed,
                    &retry_quality,
                    &scope,
                    if status == 404 { "" } else { &rejected_token },
                    None,
                )
                .await;
                if let Some(recovered) = recovered {
                    render_url = strip_hdnea_from_url(&recovered.url);
                    scope = if let Some(identity) = manifest_identity(&scope) {
                        identity_scope(&render_url, identity.selector)
                    } else {
                        render_url.clone()
                    };
                    hdnea_key = RenderCaches::hdnea_key(&channel_id, &quality, &scope);
                    token = recovered.token;
                    state.render_caches.set_hdnea(cache_gen, &hdnea_key, &token);
                    let (b, s, h) = render_with_cookie_retry(&state, &render_url, &token).await;
                    body = b;
                    status = s;
                    if !h.is_empty() {
                        state.render_caches.set_hdnea(cache_gen, &hdnea_key, &h);
                        token = h;
                    }
                }

                if status == 404
                    && !nested
                    && manifest_identity(&scope).is_none_or(|i| i.selector.is_none())
                {
                    let candidates = [retry_quality.as_str(), "auto", "high", "medium", "low"];
                    let mut tried = std::collections::HashSet::new();
                    tried.insert((render_url.clone(), token.clone()));
                    for cq in candidates {
                        let (candidate, candidate_token) = hls_fallback_candidate(&refreshed, cq);
                        if candidate.is_empty()
                            || !tried.insert((candidate.clone(), candidate_token.clone()))
                        {
                            continue;
                        }
                        scope = identity_scope(&candidate, None);
                        hdnea_key = RenderCaches::hdnea_key(&channel_id, cq, &scope);
                        render_url = candidate;
                        token = candidate_token;
                        if !token.is_empty() {
                            state.render_caches.set_hdnea(cache_gen, &hdnea_key, &token);
                        }
                        let (b, s, h) = state.tv.render(&render_url, &token).await;
                        body = b;
                        status = s;
                        if matches!(status, 401 | 403) && !h.is_empty() && h != token {
                            let (retry_body, retry_status, retry_cookie) = state
                                .tv
                                .render(&strip_hdnea_from_url(&render_url), &h)
                                .await;
                            body = retry_body;
                            status = retry_status;
                            token = if retry_cookie.is_empty() {
                                h
                            } else {
                                retry_cookie
                            };
                            state.render_caches.set_hdnea(cache_gen, &hdnea_key, &token);
                        } else if !h.is_empty() {
                            state.render_caches.set_hdnea(cache_gen, &hdnea_key, &h);
                            token = h;
                        }
                        if status == 200 {
                            quality = cq.to_string();
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
    let rewritten = render_replace(
        &state,
        &body_str,
        &base_url,
        &params,
        &channel_id,
        &quality,
        &scope,
    );

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

fn hls_fallback_candidate(live: &LiveUrlOutput, quality: &str) -> (String, String) {
    let candidate = television::select_best_live_hls_url(live, quality);
    if candidate.is_empty() {
        return (String::new(), String::new());
    }
    let candidate = to_absolute_stream_url(&candidate, absolute_base_from_live(live).as_deref());
    let token = extract_hdnea_from_url(&candidate).unwrap_or_default();
    (strip_hdnea_from_url(&candidate), token)
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
    let (mut decoded, authenticated_scope) = match decrypt_render_auth(&state, &auth) {
        Some(decoded) => decoded,
        None => return (StatusCode::BAD_REQUEST, "invalid auth parameter").into_response(),
    };

    let requested_quality = q.q.as_deref().unwrap_or_default();
    if !valid_hls_quality(requested_quality) {
        return (StatusCode::BAD_REQUEST, "invalid quality parameter").into_response();
    }
    let quality = if requested_quality.is_empty() {
        "auto"
    } else {
        requested_quality
    };
    let scope = authenticated_scope
        .filter(|scope| !scope.is_empty())
        .unwrap_or_else(|| {
            if decoded.to_lowercase().contains("catchup") {
                "legacy-catchup".to_string()
            } else {
                format!("legacy-{quality}")
            }
        });
    let rendition_key = RenderCaches::hdnea_key(&channel_id, quality, &scope);
    let original_token = extract_hdnea_from_url(&decoded).unwrap_or_default();
    let shared_key = shared_media_key(&rendition_key, &original_token);
    let hdnea_key = format!(
        "{}|segment|{}",
        rendition_key,
        strip_hdnea_from_url(&decoded)
    );
    let mut token = state
        .render_caches
        .get_hdnea(&hdnea_key)
        .or_else(|| state.render_caches.get_hdnea(&shared_key))
        .or_else(|| q.hdnea.clone());
    if token.is_some() {
        decoded = strip_hdnea_from_url(&decoded);
    } else {
        token = extract_hdnea_from_url(&decoded);
    }

    let resp = proxy_segment(&state, &decoded, token.as_deref()).await;
    let (mut status, mut body, mut headers) = resp;

    if matches!(status, 401 | 403) {
        state.render_caches.clear_hdnea(&hdnea_key);
        state.render_caches.clear_hdnea(&shared_key);
        let stripped = strip_hdnea_from_url(&decoded);
        let mut fresh_token = None;
        if !channel_id.is_empty() {
            if let Ok(refreshed) = refresh_channel_token(&state, &channel_id).await {
                let recovered = refresh_rendition(
                    &state,
                    &refreshed,
                    quality,
                    &scope,
                    token.as_deref().unwrap_or_default(),
                    Some(&decoded),
                )
                .await;
                if let Some(recovered) = recovered {
                    let refreshed_token = recovered.token;
                    let cache_key = if recovered.shared_media {
                        &shared_key
                    } else {
                        &hdnea_key
                    };
                    state
                        .render_caches
                        .set_hdnea(cache_gen, cache_key, &refreshed_token);
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
    if let Some(t) = hdnea.filter(|t| !t.is_empty()) {
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
    let (decoded, _) = match decrypt_render_auth(&state, &auth) {
        Some(decoded) => decoded,
        None => return (StatusCode::BAD_REQUEST, "invalid auth parameter").into_response(),
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
    fn hdnea_cache_key_is_scoped_to_hls_rendition() {
        let high_one = RenderCaches::hdnea_key(
            "154",
            "high",
            "https://cdn.example/high/rendition.m3u8?hdnea=token-one",
        );
        let high_two = RenderCaches::hdnea_key(
            "154",
            "high",
            "https://cdn.example/high/rendition.m3u8?__hdnea__=token-two",
        );
        let auto_same_scope =
            RenderCaches::hdnea_key("154", "auto", "https://cdn.example/high/rendition.m3u8");
        let auto_other_rendition =
            RenderCaches::hdnea_key("154", "auto", "https://cdn.example/low/rendition.m3u8");

        assert_eq!(high_one, high_two);
        assert_ne!(high_one, auto_same_scope);
        assert_ne!(auto_same_scope, auto_other_rendition);
    }

    #[test]
    fn expired_hdnea_entries_are_removed() {
        let caches = RenderCaches::default();
        let key = RenderCaches::hdnea_key("154", "auto", "scope");
        caches.hdnea.write().unwrap().insert(
            key.clone(),
            (
                "expired".into(),
                Instant::now() - HDNEA_CACHE_TTL - Duration::from_secs(1),
            ),
        );

        caches.set_hdnea(caches.generation(), "new-scope", "fresh");
        assert!(!caches.hdnea.read().unwrap().contains_key(&key));
        assert!(caches.get_hdnea(&key).is_none());
        assert!(!caches.hdnea.read().unwrap().contains_key(&key));
    }

    #[test]
    fn hls_quality_is_bounded_to_known_values() {
        for quality in ["", "auto", "high", "h", "medium", "med", "m", "low", "l"] {
            assert!(valid_hls_quality(quality), "quality: {quality}");
        }
        for quality in ["ultra", "auto-1", "HIGH", "../../cache"] {
            assert!(!valid_hls_quality(quality), "quality: {quality}");
        }
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
        let rewritten = rewrite_m3u8(
            body,
            "https://a.b/live/",
            "",
            "154",
            "auto",
            "https://a.b/live/master.m3u8",
            false,
        );
        assert!(
            rewritten.contains("/render.m3u8||https://a.b/live/chunk_1.m3u8?hdnea=old||154||auto")
        );
        assert!(rewritten.contains("||1||https://a.b/live/chunk_1.m3u8"));
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
            "https://a.b/live/rendition.m3u8",
            false,
        );
        assert!(
            rewritten.contains("/render.ts||https://a.b/live/seg1.ts?__hdnea__=fresh||154||auto")
        );
        assert!(rewritten.contains("||||https://a.b/live/rendition.m3u8"));
    }

    #[test]
    fn rewrites_packed_audio_segments_to_aac_route() {
        let body = "#EXTM3U\n#EXTINF:4,\naudio_1.aac?x=1\n#EXTINF:4,\nvideo_1.ts\n";
        let rewritten = rewrite_m3u8(
            body,
            "https://a.b/live/",
            "",
            "ex_1",
            "auto",
            "https://a.b/live/rendition.m3u8",
            false,
        );
        assert!(rewritten.contains("\n/render.aac||https://a.b/live/audio_1.aac?x=1||ex_1||auto"));
        assert!(rewritten.contains("\n/render.ts||https://a.b/live/video_1.ts||ex_1||auto"));
    }

    #[test]
    fn rewritten_segments_keep_forced_quality() {
        let body = "#EXTM3U\nseg1.ts\n";
        let rewritten = rewrite_m3u8(
            body,
            "https://a.b/live/",
            "",
            "154",
            "high",
            "https://a.b/live/high.m3u8",
            false,
        );
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
        let rewritten = rewrite_m3u8(
            body,
            "https://a.b/live/",
            "",
            "154",
            "auto",
            "https://a.b/live/rendition.m3u8",
            true,
        );
        assert_eq!(rewritten, "#EXTM3U\nhttps://a.b/live/seg1.ts\n");
    }

    #[test]
    fn rewrites_key_uri_in_ext_x_key_line() {
        let body =
            "#EXT-X-KEY:METHOD=AES-128,URI=\"https://tv.media.jio.com/key.pkey\",IV=0x1\nseg1.ts\n";
        let rewritten = rewrite_m3u8(
            body,
            "https://a.b/live/",
            "",
            "154",
            "auto",
            "https://a.b/live/rendition.m3u8",
            false,
        );
        assert!(rewritten.starts_with(
            "#EXT-X-KEY:METHOD=AES-128,URI=\"/render.key||https://tv.media.jio.com/key.pkey||154||"
        ));
        assert!(rewritten.contains(",IV=0x1"));
    }

    #[tokio::test]
    async fn full_render_replace_encrypts_placeholders() {
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
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tokenless.ts"))
            .and(|request: &wiremock::Request| !request.headers.contains_key("cookie"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&upstream)
            .await;
        let tokenless_segment = format!("{}/tokenless.ts", upstream.uri());
        assert_eq!(proxy_segment(&state, &tokenless_segment, None).await.0, 200);
        assert_eq!(
            proxy_segment(&state, &tokenless_segment, Some("")).await.0,
            200
        );
        Mock::given(method("GET"))
            .and(path("/credential.ts"))
            .and(header("cookie", "__hdnea__=valid"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&upstream)
            .await;
        assert_eq!(
            proxy_segment(
                &state,
                &format!("{}/credential.ts", upstream.uri()),
                Some("valid")
            )
            .await
            .0,
            200
        );

        Mock::given(method("GET"))
            .and(path("/direct.m3u8"))
            .and(header("cookie", "__hdnea__=direct-old"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Set-Cookie", "__hdnea__=direct-new; Path=/")
                    .set_body_string("#EXTM3U\nseg.ts\n"),
            )
            .expect(1)
            .mount(&upstream)
            .await;
        let direct = LiveUrlOutput {
            bitrates: television::Bitrates {
                auto: format!("{}/direct.m3u8?hdnea=direct-old", upstream.uri()),
                ..Default::default()
            },
            ..Default::default()
        };
        let recovered = refresh_rendition(
            &state,
            &direct,
            "auto",
            &identity_scope(&direct.bitrates.auto, None),
            "expired",
            Some(&format!("{}/seg.ts", upstream.uri())),
        )
        .await
        .unwrap();
        assert_eq!(recovered.token, "direct-new");
        assert!(recovered.shared_media);

        Mock::given(method("GET"))
            .and(path("/tokenless-master.m3u8"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\ntokenless-child.m3u8\n",
                ),
            )
            .expect(1)
            .mount(&upstream)
            .await;
        Mock::given(method("GET"))
            .and(path("/tokenless-child.m3u8"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("#EXTM3U\nseg.ts?hdnea=media-only\n"),
            )
            .expect(1)
            .mount(&upstream)
            .await;
        let tokenless = LiveUrlOutput {
            bitrates: television::Bitrates {
                auto: format!("{}/tokenless-master.m3u8", upstream.uri()),
                ..Default::default()
            },
            ..Default::default()
        };
        let recovered = refresh_rendition(
            &state,
            &tokenless,
            "auto",
            &format!("{}/tokenless-child.m3u8", upstream.uri()),
            "expired",
            None,
        )
        .await
        .unwrap();
        assert!(recovered.token.is_empty());

        Mock::given(method("GET"))
            .and(path("/rotated/root.m3u8"))
            .and(header("cookie", "__hdnea__=root-fresh"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("Set-Cookie", "__hdnea__=root-rotated; Path=/"),
            )
            .expect(2)
            .mount(&upstream)
            .await;
        Mock::given(method("GET"))
            .and(path("/rotated/root.m3u8"))
            .and(header("cookie", "__hdnea__=root-rotated"))
            .respond_with(ResponseTemplate::new(200).set_body_string("#EXTM3U\nnew.ts\n"))
            .expect(2)
            .mount(&upstream)
            .await;
        // Verify both rejected statuses recover using the new manifest URL.
        for rejected_status in [403, 404] {
            let old_path = format!("/old-{rejected_status}/root.m3u8");
            Mock::given(method("GET"))
                .and(path(&old_path))
                .respond_with(ResponseTemplate::new(rejected_status))
                .expect(1)
                .mount(&upstream)
                .await;
            let old_url = format!("{}{old_path}", upstream.uri());
            assert_eq!(
                state.tv.render(&old_url, "expired").await.1,
                rejected_status
            );
            let live = LiveUrlOutput {
                bitrates: television::Bitrates {
                    auto: format!("{}/rotated/root.m3u8?hdnea=root-fresh", upstream.uri()),
                    ..Default::default()
                },
                ..Default::default()
            };
            let recovered = refresh_rendition(
                &state,
                &live,
                "auto",
                &identity_scope(&old_url, None),
                "expired",
                None,
            )
            .await
            .unwrap();
            assert_eq!(recovered.url, live.bitrates.auto);
            assert_eq!(recovered.token, "root-fresh");
            let (_, status, cookie) = render_with_cookie_retry(
                &state,
                &strip_hdnea_from_url(&recovered.url),
                &recovered.token,
            )
            .await;
            assert_eq!(cookie, "root-rotated");
            assert_eq!(status, 200);
        }
        Mock::given(method("GET")).and(path("/rotated/master.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=123\nnew/child.m3u8?hdnea=child-fresh&session=new\n"))
            .expect(2).mount(&upstream).await;
        Mock::given(method("GET"))
            .and(path("/rotated/new/child.m3u8"))
            .and(header("cookie", "__hdnea__=child-fresh"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("#EXTM3U\nsegment.ts?hdnea=segment-fresh\n"),
            )
            .expect(2)
            .mount(&upstream)
            .await;
        let rotated_live = LiveUrlOutput {
            bitrates: television::Bitrates {
                auto: format!("{}/rotated/master.m3u8", upstream.uri()),
                ..Default::default()
            },
            ..Default::default()
        };
        let identity = identity_scope(
            "https://old.example/old/child.m3u8?session=old",
            rendition_selector("#EXT-X-STREAM-INF:BANDWIDTH=123"),
        );
        let recovered =
            refresh_rendition(&state, &rotated_live, "auto", &identity, "expired", None)
                .await
                .unwrap();
        assert_eq!(
            recovered.url,
            format!(
                "{}/rotated/new/child.m3u8?hdnea=child-fresh&session=new",
                upstream.uri()
            )
        );
        assert_eq!(recovered.token, "child-fresh");
        let fresh_scope = identity_scope(
            &recovered.url,
            rendition_selector("#EXT-X-STREAM-INF:BANDWIDTH=123"),
        );
        let rewritten = render_replace(
            &state,
            "#EXTM3U\nsegment.ts\n",
            &recovered.url,
            "",
            "154",
            "auto",
            &fresh_scope,
        );
        let link = url::Url::parse(&format!(
            "http://localhost{}",
            rewritten.lines().nth(1).unwrap()
        ))
        .unwrap();
        let auth = link.query_pairs().find(|(key, _)| key == "auth").unwrap().1;
        let (url, scope) = decrypt_render_auth(&state, &auth).unwrap();
        assert_eq!(url, format!("{}/rotated/new/segment.ts", upstream.uri()));
        assert_eq!(scope.unwrap(), fresh_scope);
        assert!(refresh_rendition(
            &state,
            &rotated_live,
            "auto",
            &identity,
            "expired",
            Some("https://old.example/old/segment.ts")
        )
        .await
        .is_none());

        Mock::given(method("GET"))
            .and(path("/master.m3u8"))
            .and(header("cookie", "__hdnea__=master-token"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("Set-Cookie", "__hdnea__=rotated-master; Path=/"),
            )
            .expect(3)
            .mount(&upstream)
            .await;
        Mock::given(method("GET")).and(path("/master.m3u8"))
            .and(header("cookie", "__hdnea__=rotated-master"))
            .respond_with(ResponseTemplate::new(200).insert_header("Set-Cookie", "__hdnea__=rotated-master; Path=/").set_body_string(
                "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"muxed\"\n#EXT-X-STREAM-INF:BANDWIDTH=1\nlow.m3u8?hdnea=low-token\n#EXT-X-STREAM-INF:BANDWIDTH=2\nhigh.m3u8?hdnea=child-token\nshared.m3u8\n"
            )).expect(3).mount(&upstream).await;
        Mock::given(method("GET"))
            .and(path("/high.m3u8"))
            .and(header("cookie", "__hdnea__=child-token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("#EXTM3U\nseg.ts?hdnea=segment-token\n"),
            )
            .expect(2)
            .mount(&upstream)
            .await;
        Mock::given(method("GET"))
            .and(path("/shared.m3u8"))
            .and(header("cookie", "__hdnea__=rotated-master"))
            .respond_with(ResponseTemplate::new(200).set_body_string("#EXTM3U\nseg.ts\n"))
            .expect(1)
            .mount(&upstream)
            .await;
        let live = LiveUrlOutput {
            bitrates: television::Bitrates {
                auto: format!("{}/master.m3u8?hdnea=master-token", upstream.uri()),
                ..Default::default()
            },
            ..Default::default()
        };
        let child_scope = format!("{}/high.m3u8", upstream.uri());
        assert_eq!(
            refresh_rendition_token(&state, &live, "auto", &child_scope, "", None).await,
            "child-token"
        );

        assert_eq!(
            refresh_rendition_token(
                &state,
                &live,
                "auto",
                &format!("{}/shared.m3u8", upstream.uri()),
                "",
                None,
            )
            .await,
            "rotated-master"
        );

        assert_eq!(
            refresh_rendition_token(
                &state,
                &live,
                "auto",
                &child_scope,
                "expired",
                Some(&format!("{}/seg.ts", upstream.uri()))
            )
            .await,
            "segment-token"
        );

        let conflicting = LiveUrlOutput {
            bitrates: television::Bitrates {
                auto: "https://cdn.example/root.m3u8?hdnea=rejected".into(),
                high: "https://cdn.example/high.m3u8?hdnea=rotated".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            refresh_rendition_token(
                &state,
                &conflicting,
                "auto",
                "https://cdn.example/root.m3u8",
                "rejected",
                None,
            )
            .await,
            "rotated"
        );

        assert_eq!(
            refresh_rendition_token(
                &state,
                &conflicting,
                "auto",
                &identity_scope("https://cdn.example/root.m3u8", None),
                "rejected",
                None
            )
            .await,
            "rotated"
        );

        let body = "#EXTM3U\nseg1.ts\n";
        let out = render_replace(
            &state,
            body,
            "https://a.b/live/",
            "",
            "154",
            "auto",
            "https://a.b/live/rendition.m3u8",
        );
        assert!(out.contains("/render.ts?auth="));
        assert!(out.contains("&channel_key_id=154"));
        assert!(!out.contains("&scope="));
        assert!(!out.contains("https://a.b/live/rendition.m3u8"));
        assert!(!out.contains("||"));

        let segment_uri = out.lines().nth(1).unwrap();
        let url = url::Url::parse(&format!("http://localhost{segment_uri}")).unwrap();
        let auth = url.query_pairs().find(|(k, _)| k == "auth").unwrap().1;
        let (decoded, scope) = decrypt_render_auth(&state, &auth).unwrap();
        assert_eq!(decoded, "https://a.b/live/seg1.ts");
        assert_eq!(scope.as_deref(), Some("https://a.b/live/rendition.m3u8"));

        for tag in ["EXT-X-KEY", "EXT-X-SESSION-KEY"] {
            for suffix in ["", ",IV=0x1,KEYFORMAT=\"identity\""] {
                let body = format!(
                    "#EXTM3U\n#{tag}:METHOD=AES-128,URI=\"https://a.b/key.pkey\"{suffix}\nseg1.ts\n"
                );
                let out = render_replace(
                    &state,
                    &body,
                    "https://a.b/live/",
                    "",
                    "154",
                    "auto",
                    "https://a.b/live/rendition.m3u8",
                );
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
                "https://a.b/live/master.m3u8",
            );
            let uri = out.lines().next().unwrap().split("URI=\"").nth(1).unwrap();
            let (uri, suffix) = uri.split_once('"').unwrap();
            assert_eq!(suffix, ",DEFAULT=YES");
            let url = url::Url::parse(&format!("http://localhost{uri}")).unwrap();
            let auth = url.query_pairs().find(|(k, _)| k == "auth").unwrap().1;
            let (decoded, scope) = decrypt_render_auth(&state, &auth).unwrap();
            assert_eq!(decoded, expected);
            assert_eq!(
                manifest_identity(scope.as_deref().unwrap()).unwrap().url,
                strip_hdnea_from_url(expected)
            );
            assert!(uri.contains("&nested=true"));
            assert!(!uri.contains("&scope="));
            assert!(out.ends_with(&format!("{muxed}\r\n")));
        }
    }

    #[test]
    fn rotated_rendition_requires_unique_original_selector() {
        let old = "https://old.example/a/master.m3u8?session=old";
        let fresh = "https://new.example/b/master.m3u8?session=new";
        for tag in [
            "#EXT-X-STREAM-INF:BANDWIDTH=800000,CODECS=\"avc1,mp4a\"",
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"Hindi\",LANGUAGE=\"hin\",URI=\"child.m3u8\"",
            "#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=800000,URI=\"child.m3u8\"",
        ] {
            let body = if tag.starts_with("#EXT-X-STREAM-INF:") {
                format!("{tag}\nchild.m3u8\n")
            } else { format!("{tag}\n") };
            let entries = rendition_entries(&body, old);
            let scope = identity_scope(&entries[0].0, Some(entries[0].1.clone()));
            assert_eq!(matching_rendition_url(&body, fresh, &scope).unwrap(), "https://new.example/b/child.m3u8?session=new");
            assert!(matching_rendition_url(&format!("{body}{body}"), fresh, &scope).is_none());
            assert!(matching_rendition_url("#EXTM3U\nother.m3u8\n", fresh, &scope).is_none());
            assert!(matching_rendition_url(&body, fresh, &strip_hdnea_from_url(&entries[0].0)).is_none());
            assert_ne!(RenderCaches::hdnea_key("1", "auto", &scope), RenderCaches::hdnea_key("1", "auto", &identity_scope(&entries[0].0, None)));
        }
    }

    #[test]
    fn media_credentials_share_only_when_common() {
        let child = "https://cdn.example/child.m3u8";
        let a = "https://cdn.example/a.ts";
        let shared = media_credential("#EXTM3U\na.ts\nb.ts\n", child, a, "rotated").unwrap();
        assert_eq!(shared, ("rotated".into(), true));
        let caches = RenderCaches::default();
        let key = shared_media_key("rendition", "expired");
        caches.set_hdnea(caches.generation(), &key, &shared.0);
        assert_eq!(
            caches
                .get_hdnea(&shared_media_key("rendition", "expired"))
                .as_deref(),
            Some("rotated")
        );
        assert!(caches
            .get_hdnea(&shared_media_key("rendition", "different"))
            .is_none());
        assert_eq!(
            media_credential("a.ts?hdnea=A\nb.ts?hdnea=B\n", child, a, "cookie"),
            Some(("A".into(), false))
        );
        assert_eq!(
            media_credential("a.ts?hdnea=A\nb.ts\n", child, a, "cookie"),
            Some(("A".into(), false))
        );
        assert!(media_credential("b.ts\n", child, a, "cookie").is_none());
    }

    #[test]
    fn fallback_candidates_pair_each_rendition_with_its_own_token() {
        let live = LiveUrlOutput {
            bitrates: crate::television::Bitrates {
                high: "https://cdn.example/high.m3u8?hdnea=high-token".into(),
                low: "https://cdn.example/low.m3u8?hdnea=low-token".into(),
                ..Default::default()
            },
            ..Default::default()
        };

        let (high_url, high_token) = hls_fallback_candidate(&live, "high");
        let (low_url, low_token) = hls_fallback_candidate(&live, "low");

        assert_eq!(high_url, "https://cdn.example/high.m3u8");
        assert_eq!(high_token, "high-token");
        assert_eq!(low_url, "https://cdn.example/low.m3u8");
        assert_eq!(low_token, "low-token");
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
        let out = render_replace(
            &state,
            body,
            "https://a.b/live/",
            "",
            "154",
            "auto",
            "https://a.b/live/master.m3u8",
        );
        assert!(out.contains("/render.m3u8?auth="));
        assert!(out.contains("&nested=true"));
        let child_uri = out.lines().nth(2).unwrap();
        let child_url = url::Url::parse(&format!("http://localhost{child_uri}")).unwrap();
        assert!(child_url.query_pairs().all(|(k, _)| k != "scope"));
        let auth = child_url
            .query_pairs()
            .find(|(k, _)| k == "auth")
            .unwrap()
            .1;
        let (decoded, scope) = decrypt_render_auth(&state, &auth).unwrap();
        assert_eq!(decoded, "https://a.b/live/child.m3u8");
        assert_eq!(
            manifest_identity(scope.as_deref().unwrap()).unwrap().url,
            "https://a.b/live/child.m3u8"
        );
    }
}
