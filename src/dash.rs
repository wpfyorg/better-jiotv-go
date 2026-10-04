//! DASH/Widevine proxying: `/live/mpd/:id`, `/live/key/:id`, `/render.mpd`,
//! `/render.dash/*`, `/drm` and `/dashtime`. Mirrors `LiveManifestMpdHandler`
//! / `LiveManifestKeyHandler` / `MpdHandler` / `DashHandler` /
//! `DRMKeyHandler` / `DASHTimeHandler` in `internal/handlers/drm.go`, with
//! the same reduced fidelity noted in `stream.rs` (no singleflight, and here
//! also: no extras-specific CDN user-agent switching, and Set-Cookie from the
//! CDN is not forwarded to the client — the embedded `/hdnea/<enc>` segment
//! in the rewritten `BaseURL` carries the token instead, which is enough for
//! same-process segment proxying but not for a client using cookies to talk
//! to the CDN directly).

use crate::state::AppState;
use crate::television::{self, LiveUrlOutput};
use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DRM_MPD_CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
pub struct DrmMpdOutput {
    // Computed for parity with the Go struct (`buildDrmMpdOutput`) and kept
    // in case a future in-app DRM player page needs them directly; nothing
    // reads them today since /live/mpd/:id redirects straight to play_url.
    #[allow(dead_code)]
    pub is_drm: bool,
    pub play_url: String,
    pub license_url: String,
    #[allow(dead_code)]
    pub tv_url_host: String,
    #[allow(dead_code)]
    pub tv_url_path: String,
}

#[derive(Default)]
pub struct DashState {
    drm_mpd_cache: RwLock<std::collections::HashMap<String, (DrmMpdOutput, Instant)>>,
    live_cache: RwLock<std::collections::HashMap<String, (LiveUrlOutput, Instant, u64)>>,
    /// Coalesces concurrent live-cache misses for the same channel.
    live_locks: crate::keyed_locks::KeyedLocks,
    cdn_clock: Mutex<Option<(SystemTime, Instant)>>,
}

impl DashState {
    fn get_cached(&self, key: &str) -> Option<DrmMpdOutput> {
        let map = self.drm_mpd_cache.read().unwrap();
        let (out, at) = map.get(key)?;
        if at.elapsed() > DRM_MPD_CACHE_TTL {
            return None;
        }
        Some(out.clone())
    }

    fn set_cached(&self, key: &str, out: DrmMpdOutput) {
        self.drm_mpd_cache
            .write()
            .unwrap()
            .insert(key.to_string(), (out, Instant::now()));
    }

    /// A cached entry is only served in the context epoch it was fetched in,
    /// so a hit racing an account switch (epoch rotated, cache not yet
    /// cleared) is a miss rather than the previous account's data.
    fn get_live(&self, channel_id: &str, epoch: u64) -> Option<LiveUrlOutput> {
        let map = self.live_cache.read().unwrap();
        let (out, at, entry_epoch) = map.get(channel_id)?;
        (*entry_epoch == epoch && at.elapsed() <= DRM_MPD_CACHE_TTL).then(|| out.clone())
    }

    /// Stores `out`, fetched under `epoch`, only if `current_epoch()` still
    /// equals it. The comparison runs under the cache's write lock, which
    /// `clear` also takes after the epoch rotates, so a fetch started before an
    /// account switch cannot repopulate the cleared cache. Returns whether the
    /// entry was stored.
    fn set_live(
        &self,
        channel_id: &str,
        out: LiveUrlOutput,
        epoch: u64,
        current_epoch: impl FnOnce() -> u64,
    ) -> bool {
        let mut map = self.live_cache.write().unwrap();
        if current_epoch() != epoch {
            return false;
        }
        map.insert(channel_id.to_string(), (out, Instant::now(), epoch));
        true
    }

    fn record_publish_time(&self, t: SystemTime) {
        *self.cdn_clock.lock().unwrap() = Some((t, Instant::now()));
    }

    /// The CDN's extrapolated current time, if an MPD fetch has observed one.
    pub fn cdn_now(&self) -> Option<SystemTime> {
        let guard = self.cdn_clock.lock().unwrap();
        let (t, at) = (*guard)?;
        Some(t + at.elapsed())
    }

    pub fn clear(&self) {
        self.drm_mpd_cache.write().unwrap().clear();
        self.live_cache.write().unwrap().clear();
        *self.cdn_clock.lock().unwrap() = None;
    }
}

/// Splits a CDN manifest URL into its host and the directory its segments
/// live in (the URL with its filename dropped, trailing slash kept).
/// Mirrors Go's `buildDrmMpdOutput`/`MpdHandler`:
/// `parsed := url.Parse(tvURL); dir := strings.Join(strings.Split(parsed.Path, "/")[:n-1], "/") + "/"`.
///
/// Operates only on `Url::path()`, never on the raw URL string, so a query
/// string containing its own `/` characters (a real shape for JioTV/extras's
/// `hdnea` token, e.g. `...~acl=/*~...`) can never leak into the split: the
/// `url` crate has already separated path from query at the first
/// unescaped `?` by the time `.path()` returns anything, regardless of what
/// the query itself contains. The one thing worth guarding explicitly is a
/// path that ends in `/` (or has `//` from an upstream quirk) producing a
/// trailing empty split segment that would otherwise get treated as the
/// "filename" to drop, silently keeping the real last directory component
/// and reproducing exactly the one-extra-path-segment symptom this was
/// written to fix.
fn cdn_host_and_dir(url_str: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(url_str).ok()?;
    let host = parsed.host_str()?.to_string();
    let mut path = parsed.path();
    while path.len() > 1 && path.ends_with('/') {
        path = &path[..path.len() - 1];
    }
    let dir = match path.rfind('/') {
        Some(idx) => path[..=idx].to_string(),
        None => "/".to_string(),
    };
    Some((host, dir))
}

/// The full playback response for the in-app player, cached for the same
/// short window as the DASH output so reloads and concurrent viewers do not
/// each hit the playback API. Returns the context epoch the response belongs
/// to; callers that encrypt URLs from it must confirm the epoch is unchanged
/// afterwards, or the previous account's data would be re-encrypted under the
/// new epoch.
pub(crate) async fn get_live_cached(
    state: &AppState,
    channel_id: &str,
) -> anyhow::Result<(LiveUrlOutput, u64)> {
    let epoch = state.secure.current_epoch();
    if let Some(cached) = state.dash_state.get_live(channel_id, epoch) {
        return Ok((cached, epoch));
    }
    let _guard = state.dash_state.live_locks.lock(channel_id).await;
    // Retry once if an account/product switch lands mid-fetch.
    for _ in 0..2 {
        let epoch = state.secure.current_epoch();
        // Another request may have filled the cache while this one waited.
        if let Some(cached) = state.dash_state.get_live(channel_id, epoch) {
            return Ok((cached, epoch));
        }
        let live = crate::stream::fetch_live(state, channel_id).await?;
        if state
            .dash_state
            .set_live(channel_id, live.clone(), epoch, || {
                state.secure.current_epoch()
            })
        {
            return Ok((live, epoch));
        }
    }
    anyhow::bail!("the active account changed while resolving channel {channel_id}; retry")
}

pub(crate) async fn get_drm_mpd(
    state: &AppState,
    channel_id: &str,
    quality: &str,
) -> anyhow::Result<DrmMpdOutput> {
    let cache_key = format!("{channel_id}_{quality}");
    if let Some(cached) = state.dash_state.get_cached(&cache_key) {
        return Ok(cached);
    }
    let live = crate::stream::fetch_live(state, channel_id).await?;
    let out = build_drm_mpd_output(state, &live, channel_id, quality)?;
    state.dash_state.set_cached(&cache_key, out.clone());
    Ok(out)
}

pub(crate) fn build_drm_mpd_output(
    state: &AppState,
    live: &LiveUrlOutput,
    channel_id: &str,
    quality: &str,
) -> anyhow::Result<DrmMpdOutput> {
    let bitrates = live.mpd.resolved_bitrates();
    let mut tv_url = television::select_quality(
        quality,
        &bitrates.auto,
        &bitrates.high,
        &bitrates.medium,
        &bitrates.low,
    )
    .to_string();
    if tv_url.is_empty() {
        tv_url = [
            &bitrates.high,
            &bitrates.auto,
            &bitrates.medium,
            &bitrates.low,
        ]
        .into_iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default();
    }
    if tv_url.is_empty() {
        tv_url = live.mpd.result.clone();
    }
    if tv_url.is_empty() {
        return Ok(DrmMpdOutput {
            is_drm: live.is_drm,
            ..Default::default()
        });
    }

    let channel_enc_url = state.secure.encrypt(&tv_url);
    let license_url = if !live.resolved_license_url().is_empty() {
        let enc_key = state.secure.encrypt(live.resolved_license_url());
        format!("/drm?auth={enc_key}&channel_id={channel_id}&channel={channel_enc_url}")
    } else {
        String::new()
    };

    if live.algo_name == "timesplay" {
        return Ok(DrmMpdOutput {
            is_drm: live.is_drm,
            play_url: tv_url,
            license_url,
            ..Default::default()
        });
    }

    let (host, dir_path) =
        cdn_host_and_dir(&tv_url).ok_or_else(|| anyhow::anyhow!("invalid upstream URL"))?;
    let tv_url_path = state.secure.encrypt_deterministic(&dir_path);
    let tv_url_host = state.secure.encrypt_deterministic(&host);

    Ok(DrmMpdOutput {
        is_drm: live.is_drm,
        play_url: format!("/render.mpd?auth={channel_enc_url}&channel_id={channel_id}&q={quality}"),
        license_url,
        tv_url_host,
        tv_url_path,
    })
}

#[derive(serde::Deserialize)]
pub struct QualityQuery {
    q: Option<String>,
}

/// `/live/mpd/:channelID` — the IPTV-facing MPD route: redirects straight to
/// the manifest, no player page (there is no Go-template/HTML player in this
/// rewrite; see the README's UI parity gap).
pub async fn live_mpd_handler(
    axum::extract::Path(channel_id): axum::extract::Path<String>,
    Query(q): Query<QualityQuery>,
    State(state): State<Arc<AppState>>,
    prefix: Option<axum::Extension<crate::api::KeyPrefix>>,
) -> Response {
    let quality = q.q.unwrap_or_else(|| "auto".to_string());

    if !state.channel_allowed(&channel_id).await {
        return (
            StatusCode::NOT_FOUND,
            format!("Channel {channel_id} is not available for the active account"),
        )
            .into_response();
    }

    if let Some(ch) = state.custom_channels.get(&channel_id) {
        return Redirect::to(&ch.url).into_response();
    }

    // get_drm_mpd -> fetch_live already refreshes the right credentials
    // (JioTV's or extras's, depending on how the channel routes).
    let drm = get_drm_mpd(&state, &channel_id, &quality).await;
    match drm {
        Ok(out) if !out.play_url.is_empty() => Redirect::to(&out.play_url).into_response(),
        _ => {
            // No DRM/DASH stream available; fall back to this server's own
            // HLS route rather than the Go version's HTML fallback player.
            let prefix_str = prefix.as_ref().map(|p| p.0 .0.clone()).unwrap_or_default();
            let hls_path = format!(
                "{prefix_str}{}",
                crate::extras_state::ExtrasState::live_hls_path(&channel_id, &quality)
            );
            Redirect::to(&hls_path).into_response()
        }
    }
}

/// `/live/key/:channelID` — proxies the Widevine license the same way
/// `/drm` does, after resolving it via `getDrmMpd`.
pub async fn live_key_handler(
    axum::extract::Path(channel_id): axum::extract::Path<String>,
    Query(q): Query<QualityQuery>,
    State(state): State<Arc<AppState>>,
    method: Method,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    let quality = q.q.unwrap_or_else(|| "auto".to_string());
    if !state.channel_allowed(&channel_id).await {
        return (
            StatusCode::NOT_FOUND,
            format!("Channel {channel_id} is not available for the active account"),
        )
            .into_response();
    }
    let drm = match get_drm_mpd(&state, &channel_id, &quality).await {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if drm.license_url.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            format!("No License URL found for channel {channel_id}"),
        )
            .into_response();
    }
    let Some((_, query)) = drm.license_url.split_once('?') else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "malformed license URL").into_response();
    };
    let params: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
    drm_license_impl(state, params, method, headers, body).await
}

/// `/drm?auth=...&channel=...&channel_id=...` license proxy.
pub async fn drm_license_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    method: Method,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    drm_license_impl(state, params, method, headers, body).await
}

async fn drm_license_impl(
    state: Arc<AppState>,
    params: std::collections::HashMap<String, String>,
    method: Method,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    let Some(auth) = params.get("auth") else {
        return (StatusCode::BAD_REQUEST, "auth is required").into_response();
    };
    let Some(channel_id) = params
        .get("channel_id")
        .filter(|id| !id.is_empty())
        .cloned()
    else {
        return (StatusCode::BAD_REQUEST, "channel_id is required").into_response();
    };
    if !state.channel_allowed(&channel_id).await {
        return (
            StatusCode::NOT_FOUND,
            "channel is not available for the active account",
        )
            .into_response();
    }
    let decoded_url = match state.secure.decrypt(auth) {
        Ok(u) => u,
        Err(_) => return (StatusCode::FORBIDDEN, "invalid auth parameter").into_response(),
    };

    // A extras channel's license is authorised by the token already in the
    // license URL; send the headers the extras app sends and skip the
    // cookie-harvesting dance below entirely, mirroring `DRMKeyHandler`'s
    // `extrasLicenseHeaders` branch.
    let is_custom = state.custom_channels.contains(&channel_id);
    if let Some(content_id) = state
        .extras
        .route(&channel_id, state.tv.logged_in(), is_custom)
    {
        let mut req = state
            .http
            .request(method, &decoded_url)
            .header(header::USER_AGENT, crate::extras::PLAYER_USER_AGENT)
            .header(header::CONTENT_TYPE, "application/octet-stream");
        for (k, v) in state.extras.license_headers(&content_id, "") {
            req = req.header(k, v);
        }
        return send_license_request(req, body).await;
    }

    // A HEAD request to the (still-encrypted-in-the-URL) channel manifest
    // harvests any auth cookie the CDN wants forwarded onto the license
    // request, mirroring DRMKeyHandler's cookie relay.
    let mut cookie_header = None;
    if let Some(channel_enc) = params.get("channel") {
        if let Ok(channel_url) = state.secure.decrypt(channel_enc) {
            if let Ok(resp) = state.http.head(&channel_url).send().await {
                let cookies: Vec<String> = resp
                    .headers()
                    .get_all(header::SET_COOKIE)
                    .iter()
                    .filter_map(|v| v.to_str().ok())
                    .filter_map(|sc| sc.split(';').next().map(str::trim).map(str::to_string))
                    .filter(|c| c.contains('='))
                    .collect();
                if !cookies.is_empty() {
                    cookie_header = Some(cookies.join("; "));
                }
            }
        }
    }

    let creds = state.tv.creds.read().unwrap().clone().unwrap_or_default();
    let mut req = state
        .http
        .request(method, &decoded_url)
        .header("accesstoken", &creds.access_token)
        .header("Connection", "keep-alive")
        .header("os", "android")
        .header("appName", "RJIL_JioTV")
        .header("subscriberId", &creds.crm)
        .header(header::USER_AGENT, television::PLAYER_USER_AGENT)
        .header("ssotoken", &creds.sso_token)
        .header("x-platform", "android")
        .header("srno", generate_date_time())
        .header("crmid", &creds.crm)
        .header("channelid", &channel_id)
        .header("uniqueId", &creds.unique_id)
        .header("versionCode", "422")
        .header("usergroup", "tvYR7NSNn7rymo3F")
        .header("devicetype", "phone")
        .header("Accept-Encoding", "gzip, deflate")
        .header("osVersion", "13")
        .header("deviceId", &state.tv.device_id)
        .header(header::CONTENT_TYPE, "application/octet-stream");
    if let Some(c) = cookie_header {
        req = req.header(header::COOKIE, c);
    }
    // The player's own Accept/Origin headers aren't forwarded (mirrors the
    // Go handler explicitly deleting them before proxying).
    let _ = &headers;

    send_license_request(req, body).await
}

async fn send_license_request(req: reqwest::RequestBuilder, body: Bytes) -> Response {
    match req.body(body).send().await {
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
        Err(_) => (StatusCode::BAD_GATEWAY, "license upstream request failed").into_response(),
    }
}

fn generate_date_time() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let millis = now.subsec_millis();
    let (y, m, d, hh, mm, _ss) = crate::television::civil_from_unix(now.as_secs());
    format!(
        "{:02}{:02}{:02}{:02}{:02}{:03}",
        y % 100,
        m,
        d,
        hh,
        mm,
        millis
    )
}

#[derive(serde::Deserialize)]
pub struct RenderMpdQuery {
    auth: String,
    channel_id: Option<String>,
    q: Option<String>,
    hdnea: Option<String>,
}

/// `/render.mpd` — fetches the MPD, rewrites `BaseURL` to route segments
/// through `/render.dash/...`, injects a `UTCTiming` element pointing at
/// `/dashtime`, and records the CDN's own clock from `publishTime`.
pub async fn render_mpd_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<RenderMpdQuery>,
) -> Response {
    let Some(channel_id) = q.channel_id.as_deref().filter(|id| !id.is_empty()) else {
        return (StatusCode::BAD_REQUEST, "channel_id is required").into_response();
    };
    if !state.channel_allowed(channel_id).await {
        return (
            StatusCode::NOT_FOUND,
            "channel is not available for the active account",
        )
            .into_response();
    }
    let mut decrypted = match state.secure.decrypt(&q.auth) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid auth parameter").into_response(),
    };
    let quality = q.q.clone().unwrap_or_default();

    if let Ok(live) = crate::stream::fetch_live(&state, channel_id).await {
        let fresh = television::select_best_live_mpd_url(&live, &quality);
        if !fresh.is_empty() {
            decrypted = fresh;
        }
    }

    let Some((proxy_host, base_path)) = cdn_host_and_dir(&decrypted) else {
        return (StatusCode::BAD_REQUEST, "invalid upstream URL").into_response();
    };
    let enc_host = state.secure.encrypt_deterministic(&proxy_host);
    let enc_path = state.secure.encrypt_deterministic(&base_path);

    let encoded_channel = urlencoding::encode(channel_id);
    let cached_hdnea = q.hdnea.clone();
    let mut dash_base =
        format!("/render.dash/channel/{encoded_channel}/host/{enc_host}/path/{enc_path}");
    if let Some(h) = &cached_hdnea {
        if !h.is_empty() {
            let enc = state
                .secure
                .encrypt_deterministic(&format!("__hdnea__={h}"));
            dash_base = format!("/render.dash/channel/{encoded_channel}/host/{enc_host}/path/{enc_path}/hdnea/{enc}");
        }
    }

    let (status, body, set_cookies) = proxy_mpd(&state, &decrypted).await;
    let mut status = status;
    let mut body = body;
    let mut set_cookies = set_cookies;
    if matches!(status, 401 | 403) {
        let stripped = crate::stream::strip_hdnea_from_url(&decrypted);
        let (s, b, c) = proxy_mpd(&state, &stripped).await;
        status = s;
        body = b;
        set_cookies = c;
    }

    let upstream_hdnea = set_cookies.iter().find_map(|sc| {
        sc.split(';')
            .map(str::trim)
            .find_map(|p| p.strip_prefix("__hdnea__="))
    });
    if let Some(h) = upstream_hdnea {
        let enc = state
            .secure
            .encrypt_deterministic(&format!("__hdnea__={h}"));
        dash_base = format!(
            "/render.dash/channel/{encoded_channel}/host/{enc_host}/path/{enc_path}/hdnea/{enc}"
        );
    }

    let mut body_str = String::from_utf8_lossy(&body).to_string();

    if let Some(pt) = extract_publish_time(&body_str) {
        state.dash_state.record_publish_time(pt);
    }

    if !body_str.contains("<UTCTiming") {
        if let Some(idx) = body_str
            .find("<MPD")
            .and_then(|i| body_str[i..].find('>').map(|j| i + j + 1))
        {
            let utc_timing = "<UTCTiming schemeIdUri=\"urn:mpeg:dash:utc:http-xsdate:2014\" value=\"/dashtime\"/>";
            body_str.insert_str(idx, utc_timing);
        }
    }

    body_str = rewrite_base_url(&body_str, &dash_base);

    let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder()
        .status(status_code)
        .header(header::CONTENT_TYPE, "application/dash+xml");
    // Forward the CDN's own cookies to the client (Shaka and other
    // same-origin players use them for the segment requests that follow),
    // with Domain stripped and Path rewritten to this server's own
    // /render.dash prefix — mirrors MpdHandler's Set-Cookie rewriting.
    for raw in &set_cookies {
        let domain_needle = format!("Domain={proxy_host};");
        let rewritten = raw
            .replace(&domain_needle, "")
            .replacen("path=/", "path=/render.dash", 1);
        builder = builder.header(header::SET_COOKIE, rewritten);
    }
    builder.body(Body::from(body_str)).unwrap()
}

/// Points segment requests at `/render.dash`, like Go's MpdHandler: every
/// existing `<BaseURL>` becomes `<dash_base>/dash/`; without one, a
/// `<BaseURL><dash_base>/</BaseURL>` goes after every `<Period ...>` tag.
fn rewrite_base_url(body: &str, dash_base: &str) -> String {
    if body.contains("<BaseURL>") {
        let mut out = String::with_capacity(body.len());
        let mut rest = body;
        while let Some(start) = rest.find("<BaseURL>") {
            let Some(end_rel) = rest[start..].find("</BaseURL>") else {
                break;
            };
            out.push_str(&rest[..start]);
            out.push_str(&format!("<BaseURL>{dash_base}/dash/</BaseURL>"));
            rest = &rest[start + end_rel + "</BaseURL>".len()..];
        }
        out.push_str(rest);
        return out;
    }
    let mut out = String::with_capacity(body.len() + 128);
    let mut rest = body;
    while let Some(start) = rest.find("<Period") {
        // Skip tags that only start with "<Period", such as <PeriodX>.
        let after = rest[start + "<Period".len()..].chars().next();
        let Some(tag_end_rel) = rest[start..].find('>') else {
            break;
        };
        let insert_at = start + tag_end_rel + 1;
        out.push_str(&rest[..insert_at]);
        if matches!(after, Some(c) if c.is_whitespace() || c == '>' || c == '/') {
            out.push_str(&format!("\n<BaseURL>{dash_base}/</BaseURL>"));
        }
        rest = &rest[insert_at..];
    }
    out.push_str(rest);
    out
}

fn extract_publish_time(body: &str) -> Option<SystemTime> {
    let idx = body.find("publishTime=\"")? + "publishTime=\"".len();
    let end = body[idx..].find('"')?;
    let value = &body[idx..idx + end];
    parse_rfc3339(value)
}

/// A minimal RFC3339 parser (`2024-01-02T03:04:05.678Z` or with a numeric
/// offset), just enough for MPD `publishTime` values, avoiding a datetime
/// dependency.
pub(crate) fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    let (date, rest) = s.split_once('T')?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    let rest = rest.trim_end_matches('Z');
    let (time_part, offset) = rest
        .split_once('+')
        .map(|(a, b)| (a, Some(("+", b))))
        .unwrap_or_else(|| {
            // A "-" offset only appears after the seconds field, so look past
            // index 5 (HH:MM) to avoid the date's own dashes not being present here.
            if let Some(idx) = rest.rfind('-') {
                if idx > 5 {
                    return (&rest[..idx], Some(("-", &rest[idx + 1..])));
                }
            }
            (rest, None)
        });
    let mut tparts = time_part.split(':');
    let hh: i64 = tparts.next()?.parse().ok()?;
    let mm: i64 = tparts.next()?.parse().ok()?;
    let ss_frac = tparts.next()?;
    let ss: i64 = ss_frac.split('.').next()?.parse().ok()?;

    let days = days_from_civil(y, m, d);
    let mut total_secs = days * 86400 + hh * 3600 + mm * 60 + ss;
    if let Some((sign, off)) = offset {
        let mut op = off.split(':');
        let oh: i64 = op.next()?.parse().ok()?;
        let om: i64 = op.next().unwrap_or("0").parse().unwrap_or(0);
        let off_secs = oh * 3600 + om * 60;
        total_secs -= if sign == "+" { off_secs } else { -off_secs };
    }
    if total_secs < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(total_secs as u64))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The extras CDN refuses DASH manifests/segments unless the User-Agent starts
/// with `JioTV.Plus/` (see `pkg/extras`'s `PlayerUserAgent` doc comment);
/// `ExtrasState::player_user_agent_for` tracks which hosts need it.
fn player_user_agent_for(state: &AppState, url: &str) -> &'static str {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| state.extras.player_user_agent_for(h)))
        .unwrap_or(television::PLAYER_USER_AGENT)
}

async fn proxy_mpd(state: &AppState, url: &str) -> (u16, Vec<u8>, Vec<String>) {
    let ua = player_user_agent_for(state, url);
    match state
        .http
        .get(url)
        .header(header::USER_AGENT, ua)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let cookies: Vec<String> = resp
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .map(str::to_string)
                .collect();
            let body = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
            (status, body, cookies)
        }
        Err(_) => (502, Vec::new(), Vec::new()),
    }
}

/// `/render.dash/channel/<id>/host/<enc>/path/<enc>[/hdnea/<enc>]/<segment-path>` —
/// proxies one DASH segment or the manifest's own directory listing to the
/// real CDN. The whole remainder of the path is taken as a single wildcard
/// and parsed by hand (mirrors `DashHandler`'s own manual parsing, which
/// exists for the same reason: the segment path's own depth is unbounded).
/// Parses `/render.dash/channel/<id>/host/<enc>/path/<enc>[/hdnea/<enc>]/<segment-path>`
/// into `(channel_id, enc_host, enc_path, enc_hdnea, segment_path)`, where
/// `segment_path` starts with `/`. A pure function (no decryption, no I/O)
/// so it's directly testable against the exact path shapes a real client
/// resolves a relative `SegmentTemplate` reference into.
fn split_dash_path(path: &str) -> Option<(String, &str, &str, Option<&str>, String)> {
    let rest = path.strip_prefix("/render.dash/channel/")?;
    let (encoded_channel, rest) = rest.split_once("/host/")?;
    let channel_id = urlencoding::decode(encoded_channel).ok()?.into_owned();
    let (enc_host, remainder) = rest.split_once("/path/")?;
    if let Some((before, after)) = remainder.split_once("/hdnea/") {
        let (enc_hdnea, seg) = after.split_once('/').unwrap_or((after, ""));
        Some((
            channel_id,
            enc_host,
            before,
            Some(enc_hdnea),
            format!("/{seg}"),
        ))
    } else {
        let (p, seg) = remainder.split_once('/').unwrap_or((remainder, ""));
        Some((channel_id, enc_host, p, None, format!("/{seg}")))
    }
}

/// Builds the final upstream CDN URL from a decrypted host/directory pair,
/// the segment's own relative path, and the query the client's resolved
/// segment request carried (its own `?m=...` cache-buster, say) — kept
/// pure and separate from `cdn_host_and_dir` (which goes the other way,
/// from an upstream manifest URL down to host+dir) so both directions of
/// this round trip are independently testable.
fn build_dash_proxy_url(host: &str, base_path: &str, segment_path: &str, query: &str) -> String {
    let base_path = base_path.trim_end_matches('/');
    let mut url = format!("https://{host}{base_path}{segment_path}");
    if !query.is_empty() {
        url.push('?');
        url.push_str(query);
    }
    url
}

pub async fn render_dash_handler(
    State(state): State<Arc<AppState>>,
    uri: axum::http::Uri,
) -> Response {
    let (channel_id, enc_host, enc_path, enc_hdnea, segment_path) =
        match split_dash_path(uri.path()) {
            Some(v) => v,
            None => return (StatusCode::BAD_REQUEST, "malformed dash path").into_response(),
        };
    if !state.channel_allowed(&channel_id).await {
        return (
            StatusCode::NOT_FOUND,
            "channel is not available for the active account",
        )
            .into_response();
    }

    let hdnea_token = enc_hdnea.and_then(|enc| {
        state
            .secure
            .decrypt(enc)
            .ok()
            .and_then(|s| s.strip_prefix("__hdnea__=").map(str::to_string))
    });

    let host = match state.secure.decrypt(enc_host) {
        Ok(h) => h,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid host parameter").into_response(),
    };
    let base_path = match state.secure.decrypt(enc_path) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid path parameter").into_response(),
    };

    let proxy_url =
        build_dash_proxy_url(&host, &base_path, &segment_path, uri.query().unwrap_or(""));

    let (status, body, ct) = proxy_dash_segment(&state, &proxy_url, hdnea_token.as_deref()).await;
    let mut status = status;
    let mut body = body;
    let mut ct = ct;
    if matches!(status, 401 | 403) {
        let (s, b, c) = proxy_dash_segment(&state, &proxy_url, None).await;
        status = s;
        body = b;
        ct = c;
    }

    let mut builder =
        Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY));
    if let Some(ct) = ct {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    builder.body(Body::from(body)).unwrap()
}

async fn proxy_dash_segment(
    state: &AppState,
    url: &str,
    hdnea: Option<&str>,
) -> (u16, Vec<u8>, Option<String>) {
    let ua = player_user_agent_for(state, url);
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

/// `/dashtime` — a UTCTiming clock source; the CDN's own extrapolated clock
/// once one has been observed, else the machine clock.
pub async fn dashtime_handler(State(state): State<Arc<AppState>>) -> Response {
    let now = state.dash_state.cdn_now().unwrap_or_else(SystemTime::now);
    let secs = now.duration_since(UNIX_EPOCH).unwrap();
    let (y, m, d, hh, mm, ss) = crate::television::civil_from_unix(secs.as_secs());
    let millis = secs.subsec_millis();
    let body = format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{millis:03}Z");
    Response::builder()
        .header(header::DATE, httpdate::fmt_http_date(now))
        .body(Body::from(body))
        .unwrap()
}

mod httpdate {
    use std::time::SystemTime;

    /// A minimal RFC 7231 `Date` header formatter (no chrono dependency).
    pub fn fmt_http_date(t: SystemTime) -> String {
        let secs = t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let (y, m, d, hh, mm, ss) = crate::television::civil_from_unix(secs);
        let days_since_epoch = (secs / 86400) as i64;
        // 1970-01-01 was a Thursday (weekday index 4 in a Mon=0 scheme).
        let weekday = ((days_since_epoch % 7) + 4).rem_euclid(7);
        const WD: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        const MO: [&str; 13] = [
            "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        format!(
            "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
            WD[weekday as usize], d, MO[m as usize], y, hh, mm, ss
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_cache_serves_recent_entries_and_clears_with_the_context() {
        let state = DashState::default();
        assert!(state.get_live("154", 1).is_none());
        assert!(state.set_live(
            "154",
            LiveUrlOutput {
                hdnea: "token".into(),
                ..Default::default()
            },
            1,
            || 1,
        ));
        assert_eq!(state.get_live("154", 1).unwrap().hdnea, "token");
        assert!(state.get_live("155", 1).is_none());
        state.clear();
        assert!(state.get_live("154", 1).is_none());
    }

    #[test]
    fn live_cache_discards_fetches_that_outlived_their_context() {
        let state = DashState::default();
        assert!(!state.set_live("154", LiveUrlOutput::default(), 1, || 2));
        assert!(state.get_live("154", 1).is_none());
        assert!(state.get_live("154", 2).is_none());
    }

    #[test]
    fn live_cache_hit_in_a_newer_epoch_is_a_miss() {
        let state = DashState::default();
        assert!(state.set_live("154", LiveUrlOutput::default(), 1, || 1));
        // The epoch rotated but `clear` has not run yet.
        assert!(state.get_live("154", 2).is_none());
    }

    #[tokio::test]
    async fn concurrent_live_cache_misses_share_one_fetch() {
        let state = Arc::new(DashState::default());
        let fetches = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let viewer = |state: Arc<DashState>, fetches: Arc<std::sync::atomic::AtomicU32>| async move {
            if state.get_live("154", 1).is_some() {
                return;
            }
            let _guard = state.live_locks.lock("154").await;
            if state.get_live("154", 1).is_some() {
                return;
            }
            fetches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(30)).await;
            state.set_live("154", LiveUrlOutput::default(), 1, || 1);
        };
        let tasks: Vec<_> = (0..5)
            .map(|_| tokio::spawn(viewer(state.clone(), fetches.clone())))
            .collect();
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(fetches.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn parses_rfc3339_publish_time() {
        let t = parse_rfc3339("2024-01-02T03:04:05.678Z").unwrap();
        let secs = t.duration_since(UNIX_EPOCH).unwrap().as_secs();
        // 2024-01-02T03:04:05Z
        assert_eq!(secs, 1704164645);
    }

    #[test]
    fn extracts_publish_time_attr() {
        let body = r#"<MPD publishTime="2024-01-02T03:04:05.000Z" other="x">"#;
        assert!(extract_publish_time(body).is_some());
    }

    #[test]
    fn rewrites_existing_base_url() {
        let body = "<Period><BaseURL>https://old/</BaseURL></Period>";
        let out = rewrite_base_url(body, "/render.dash/channel/ex_1/host/x/path/y");
        assert_eq!(
            out,
            "<Period><BaseURL>/render.dash/channel/ex_1/host/x/path/y/dash/</BaseURL></Period>"
        );
    }

    #[test]
    fn inserts_base_url_when_missing() {
        let body = "<Period id=\"0\"><AdaptationSet/></Period>";
        let out = rewrite_base_url(body, "/render.dash/channel/ex_1/host/x/path/y");
        assert!(out.contains(
            "<Period id=\"0\">\n<BaseURL>/render.dash/channel/ex_1/host/x/path/y/</BaseURL>"
        ));
        assert!(!out.contains("/dash/"));
    }

    #[test]
    fn rewrites_every_period_and_base_url() {
        let two = "<Period id=\"a\"><AdaptationSet/></Period><Period id=\"b\"/>";
        assert_eq!(
            rewrite_base_url(two, "/r")
                .matches("<BaseURL>/r/</BaseURL>")
                .count(),
            2
        );
        let bases = "<BaseURL>a/</BaseURL><Period><BaseURL>b/</BaseURL></Period>";
        assert_eq!(
            rewrite_base_url(bases, "/r")
                .matches("<BaseURL>/r/dash/</BaseURL>")
                .count(),
            2
        );
    }

    /// Regression test for a live-verified bug: channel 151's extras-mirrored
    /// MPD carries an `hdnea` token whose value contains a literal `/`
    /// (Akamai-style ACLs look like `exp=...~acl=/*~data=...~hmac=...`).
    /// The directory must come out exactly as many segments deep as the
    /// path alone implies, regardless of what's in the query.
    #[test]
    fn host_and_dir_ignore_slashes_inside_the_query() {
        let url = "https://cdn.example.com/bpk-tv/MoviesNow_BTS/WDVLive/index.mpd\
                   ?hdnea=exp=1830000000~acl=/*~data=hdntl~hmac=deadbeef";
        let (host, dir) = cdn_host_and_dir(url).unwrap();
        assert_eq!(host, "cdn.example.com");
        assert_eq!(dir, "/bpk-tv/MoviesNow_BTS/WDVLive/");
        assert_eq!(
            dir.matches('/').count(),
            4,
            "expected exactly 4 slashes: leading + 3 directories"
        );
    }

    #[test]
    fn host_and_dir_are_not_thrown_off_by_a_trailing_slash() {
        // A defensive case beyond what's been seen live: a manifest URL
        // ending in `/` (naming a directory, not a file) must not leave the
        // real last directory component in the "filename" slot.
        let (_, dir) = cdn_host_and_dir("https://cdn.example.com/a/b/c/index.mpd/").unwrap();
        assert_eq!(dir, "/a/b/c/");
    }

    #[test]
    fn host_and_dir_plain_case_matches_go() {
        let (host, dir) =
            cdn_host_and_dir("https://cdn.example.com/bpk-tv/Name_BTS/WDVLive/index.mpd").unwrap();
        assert_eq!(host, "cdn.example.com");
        assert_eq!(dir, "/bpk-tv/Name_BTS/WDVLive/");
    }

    /// End to end (as pure functions, no network): a relative
    /// `SegmentTemplate` reference that carries its own `?m=` query, as
    /// resolved by a real client against a BaseURL this server derived from
    /// an upstream MPD URL whose own query contains `/` (Akamai-style ACLs:
    /// `...~acl=/*~...`) — the exact scenario behind the channel 151
    /// "Rust's BaseURL has one more path segment than Go's" bug. Checks the
    /// full round trip: `cdn_host_and_dir` (manifest URL -> host + dir) then
    /// `split_dash_path` + `build_dash_proxy_url` (rewritten request path ->
    /// upstream URL) must land back on exactly the original directory, with
    /// the segment's query preserved and nothing extra.
    #[test]
    fn dash_round_trip_survives_a_query_with_slashes_and_a_segment_with_its_own_query() {
        let secure = crate::secureurl::SecureUrl::new(false);
        let upstream = "https://cdn.example.com/bpk-tv/MoviesNow_BTS/WDVLive/index.mpd\
                         ?hdnea=exp=1830000000~acl=/*~hmac=deadbeef";

        let (host, dir_path) = cdn_host_and_dir(upstream).unwrap();
        assert_eq!(host, "cdn.example.com");
        assert_eq!(dir_path, "/bpk-tv/MoviesNow_BTS/WDVLive/");
        let enc_host = secure.encrypt_deterministic(&host);
        let enc_path = secure.encrypt_deterministic(&dir_path);

        // Shaka resolves the relative SegmentTemplate reference
        // "index_video_7_0_init.mp4?m=1773052885" against our rewritten
        // BaseURL ("/render.dash/channel/<id>/host/<enc>/path/<enc>/dash/") into this
        // request.
        let request_path = format!("/render.dash/channel/ex_151/host/{enc_host}/path/{enc_path}/dash/index_video_7_0_init.mp4");
        let (channel_id, got_enc_host, got_enc_path, got_hdnea, segment_path) =
            split_dash_path(&request_path).unwrap();
        assert_eq!(channel_id, "ex_151");
        assert_eq!(got_enc_host, enc_host);
        assert_eq!(got_enc_path, enc_path);
        assert!(got_hdnea.is_none());
        assert_eq!(segment_path, "/dash/index_video_7_0_init.mp4");

        let decrypted_host = secure.decrypt(got_enc_host).unwrap();
        let decrypted_dir = secure.decrypt(got_enc_path).unwrap();
        assert_eq!(decrypted_host, "cdn.example.com");
        assert_eq!(decrypted_dir, "/bpk-tv/MoviesNow_BTS/WDVLive/");

        let proxy_url = build_dash_proxy_url(
            &decrypted_host,
            &decrypted_dir,
            &segment_path,
            "m=1773052885",
        );
        assert_eq!(
            proxy_url,
            "https://cdn.example.com/bpk-tv/MoviesNow_BTS/WDVLive/dash/index_video_7_0_init.mp4?m=1773052885"
        );
    }

    #[test]
    fn split_dash_path_extracts_the_hdnea_segment_when_present() {
        let (channel, host, path, hdnea, seg) =
            split_dash_path("/render.dash/channel/ex_42/host/H/path/P/hdnea/HD/seg.m4s").unwrap();
        assert_eq!(channel, "ex_42");
        assert_eq!(
            (host, path, hdnea, seg.as_str()),
            ("H", "P", Some("HD"), "/seg.m4s")
        );
    }

    #[test]
    fn split_dash_path_rejects_a_malformed_prefix() {
        assert!(split_dash_path("/not-render-dash/x").is_none());
        assert!(split_dash_path("/render.dash/host/H/path/P/seg.m4s").is_none());
    }
}
