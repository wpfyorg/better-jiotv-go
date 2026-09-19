//! DASH/Widevine proxying: `/live/mpd/:id`, `/live/key/:id`, `/render.mpd`,
//! `/render.dash/*`, `/drm` and `/dashtime`. Mirrors `LiveManifestMpdHandler`
//! / `LiveManifestKeyHandler` / `MpdHandler` / `DashHandler` /
//! `DRMKeyHandler` / `DASHTimeHandler` in `internal/handlers/drm.go`, with
//! the same reduced fidelity noted in `stream.rs` (no singleflight, and here
//! also: no TV+-specific CDN user-agent switching, and Set-Cookie from the
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
    pub is_drm: bool,
    pub play_url: String,
    pub license_url: String,
    pub tv_url_host: String,
    pub tv_url_path: String,
}

#[derive(Default)]
pub struct DashState {
    drm_mpd_cache: RwLock<std::collections::HashMap<String, (DrmMpdOutput, Instant)>>,
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
        self.drm_mpd_cache.write().unwrap().insert(key.to_string(), (out, Instant::now()));
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
}

async fn get_drm_mpd(state: &AppState, channel_id: &str, quality: &str) -> anyhow::Result<DrmMpdOutput> {
    let cache_key = format!("{channel_id}_{quality}");
    if let Some(cached) = state.dash_state.get_cached(&cache_key) {
        return Ok(cached);
    }
    let live = state.tv.live(channel_id).await?;
    let out = build_drm_mpd_output(state, &live, channel_id, quality)?;
    state.dash_state.set_cached(&cache_key, out.clone());
    Ok(out)
}

fn build_drm_mpd_output(state: &AppState, live: &LiveUrlOutput, channel_id: &str, quality: &str) -> anyhow::Result<DrmMpdOutput> {
    let bitrates = live.mpd.resolved_bitrates();
    let mut tv_url = television::select_quality(quality, &bitrates.auto, &bitrates.high, &bitrates.medium, &bitrates.low).to_string();
    if tv_url.is_empty() {
        tv_url = [&bitrates.high, &bitrates.auto, &bitrates.medium, &bitrates.low]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_default();
    }
    if tv_url.is_empty() {
        tv_url = live.mpd.result.clone();
    }
    if tv_url.is_empty() {
        return Ok(DrmMpdOutput { is_drm: live.is_drm, ..Default::default() });
    }

    let channel_enc_url = state.secure.encrypt(&tv_url);
    let license_url = if !live.resolved_license_url().is_empty() {
        let enc_key = state.secure.encrypt(live.resolved_license_url());
        format!("/drm?auth={enc_key}&channel_id={channel_id}&channel={channel_enc_url}")
    } else {
        String::new()
    };

    if live.algo_name == "timesplay" {
        return Ok(DrmMpdOutput { is_drm: live.is_drm, play_url: tv_url, license_url, ..Default::default() });
    }

    let parsed = url::Url::parse(&tv_url)?;
    let mut segments: Vec<&str> = parsed.path().split('/').collect();
    segments.pop();
    let dir_path = format!("{}/", segments.join("/"));
    let tv_url_path = state.secure.encrypt_deterministic(&dir_path);
    let tv_url_host = state.secure.encrypt_deterministic(parsed.host_str().unwrap_or(""));

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

    if let Some(ch) = state.custom_channels.get(&channel_id) {
        return Redirect::to(&ch.url).into_response();
    }

    crate::token_refresh::ensure_fresh(&state).await;

    let drm = get_drm_mpd(&state, &channel_id, &quality).await;
    match drm {
        Ok(out) if !out.play_url.is_empty() => Redirect::to(&out.play_url).into_response(),
        _ => {
            // No DRM/DASH stream available; fall back to this server's own
            // HLS route rather than the Go version's HTML fallback player.
            let prefix_str = prefix.as_ref().map(|p| p.0 .0.clone()).unwrap_or_default();
            let hls_path = if quality == "auto" {
                format!("{prefix_str}/live/{channel_id}.m3u8")
            } else {
                format!("{prefix_str}/live/{quality}/{channel_id}.m3u8")
            };
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
    let drm = match get_drm_mpd(&state, &channel_id, &quality).await {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if drm.license_url.is_empty() {
        return (StatusCode::NOT_FOUND, format!("No License URL found for channel {channel_id}")).into_response();
    }
    let Some((_, query)) = drm.license_url.split_once('?') else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "malformed license URL").into_response();
    };
    let params: std::collections::HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
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
    let channel_id = params.get("channel_id").cloned().unwrap_or_default();
    let decoded_url = match state.secure.decrypt(auth) {
        Ok(u) => u,
        Err(_) => return (StatusCode::FORBIDDEN, "invalid auth parameter").into_response(),
    };

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
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(body);
    if let Some(c) = cookie_header {
        req = req.header(header::COOKIE, c);
    }
    // The player's own Accept/Origin headers aren't forwarded (mirrors the
    // Go handler explicitly deleting them before proxying).
    let _ = &headers;

    match req.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
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
    format!("{:02}{:02}{:02}{:02}{:02}{:03}", y % 100, m, d, hh, mm, millis)
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
pub async fn render_mpd_handler(State(state): State<Arc<AppState>>, Query(q): Query<RenderMpdQuery>) -> Response {
    let mut decrypted = match state.secure.decrypt(&q.auth) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid auth parameter").into_response(),
    };
    let quality = q.q.clone().unwrap_or_default();

    if let Some(channel_id) = &q.channel_id {
        if !channel_id.is_empty() {
            if let Ok(live) = state.tv.live(channel_id).await {
                let fresh = television::select_best_live_mpd_url(&live, &quality);
                if !fresh.is_empty() {
                    decrypted = fresh;
                }
            }
        }
    }

    let parsed = match url::Url::parse(&decrypted) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid upstream URL").into_response(),
    };
    let proxy_host = parsed.host_str().unwrap_or("").to_string();
    let mut segs: Vec<&str> = parsed.path().split('/').collect();
    segs.pop();
    let base_path = format!("{}/", segs.join("/"));
    let enc_host = state.secure.encrypt_deterministic(&proxy_host);
    let enc_path = state.secure.encrypt_deterministic(&base_path);

    let cached_hdnea = q.hdnea.clone();
    let mut dash_base = format!("/render.dash/host/{enc_host}/path/{enc_path}");
    if let Some(h) = &cached_hdnea {
        if !h.is_empty() {
            let enc = state.secure.encrypt_deterministic(&format!("__hdnea__={h}"));
            dash_base = format!("/render.dash/host/{enc_host}/path/{enc_path}/hdnea/{enc}");
        }
    }

    let (status, body, set_cookies) = proxy_mpd(&state, &decrypted).await;
    let mut status = status;
    let mut body = body;
    if matches!(status, 401 | 403) {
        let stripped = crate::stream::strip_hdnea_from_url(&decrypted);
        let (s, b, _) = proxy_mpd(&state, &stripped).await;
        status = s;
        body = b;
    }

    let upstream_hdnea = set_cookies
        .iter()
        .find_map(|sc| sc.split(';').map(str::trim).find_map(|p| p.strip_prefix("__hdnea__=")));
    if let Some(h) = upstream_hdnea {
        let enc = state.secure.encrypt_deterministic(&format!("__hdnea__={h}"));
        dash_base = format!("/render.dash/host/{enc_host}/path/{enc_path}/hdnea/{enc}");
    }

    let mut body_str = String::from_utf8_lossy(&body).to_string();

    if let Some(pt) = extract_publish_time(&body_str) {
        state.dash_state.record_publish_time(pt);
    }

    if !body_str.contains("<UTCTiming") {
        if let Some(idx) = body_str.find("<MPD").and_then(|i| body_str[i..].find('>').map(|j| i + j + 1)) {
            let utc_timing = "<UTCTiming schemeIdUri=\"urn:mpeg:dash:utc:http-xsdate:2014\" value=\"/dashtime\"/>";
            body_str.insert_str(idx, utc_timing);
        }
    }

    body_str = rewrite_base_url(&body_str, &format!("{dash_base}/dash/"));

    let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    Response::builder()
        .status(status_code)
        .header(header::CONTENT_TYPE, "application/dash+xml")
        .body(Body::from(body_str))
        .unwrap()
}

fn rewrite_base_url(body: &str, new_base: &str) -> String {
    if let Some(start) = body.find("<BaseURL>") {
        if let Some(end_rel) = body[start..].find("</BaseURL>") {
            let end = start + end_rel + "</BaseURL>".len();
            return format!("{}{}{}", &body[..start], format!("<BaseURL>{new_base}</BaseURL>"), &body[end..]);
        }
    }
    if let Some(period_start) = body.find("<Period") {
        if let Some(tag_end_rel) = body[period_start..].find('>') {
            let insert_at = period_start + tag_end_rel + 1;
            return format!("{}\n<BaseURL>{}</BaseURL>{}", &body[..insert_at], new_base, &body[insert_at..]);
        }
    }
    body.to_string()
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
fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    let (date, rest) = s.split_once('T')?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    let rest = rest.trim_end_matches('Z');
    let (time_part, offset) = rest.split_once('+').map(|(a, b)| (a, Some(("+", b)))).unwrap_or_else(|| {
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

async fn proxy_mpd(state: &AppState, url: &str) -> (u16, Vec<u8>, Vec<String>) {
    match state.http.get(url).header(header::USER_AGENT, television::PLAYER_USER_AGENT).send().await {
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

/// `/render.dash/host/<enc>/path/<enc>[/hdnea/<enc>]/<segment-path>` —
/// proxies one DASH segment or the manifest's own directory listing to the
/// real CDN. The whole remainder of the path is taken as a single wildcard
/// and parsed by hand (mirrors `DashHandler`'s own manual parsing, which
/// exists for the same reason: the segment path's own depth is unbounded).
pub async fn render_dash_handler(State(state): State<Arc<AppState>>, uri: axum::http::Uri) -> Response {
    let path = uri.path();
    let query = uri.query().unwrap_or("");
    let Some(rest) = path.strip_prefix("/render.dash/host/") else {
        return (StatusCode::BAD_REQUEST, "malformed dash path").into_response();
    };
    let Some((enc_host, remainder)) = rest.split_once("/path/") else {
        return (StatusCode::BAD_REQUEST, "malformed dash path").into_response();
    };

    let (enc_path, hdnea_token, segment_path) = if let Some((before, after)) = remainder.split_once("/hdnea/") {
        let (enc_hdnea, seg) = after.split_once('/').unwrap_or((after, ""));
        let hdnea = state
            .secure
            .decrypt(enc_hdnea)
            .ok()
            .and_then(|s| s.strip_prefix("__hdnea__=").map(str::to_string));
        (before.to_string(), hdnea, format!("/{seg}"))
    } else {
        let (p, seg) = remainder.split_once('/').unwrap_or((remainder, ""));
        (p.to_string(), None, format!("/{seg}"))
    };

    let host = match state.secure.decrypt(enc_host) {
        Ok(h) => h,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid host parameter").into_response(),
    };
    let base_path = match state.secure.decrypt(&enc_path) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid path parameter").into_response(),
    };
    let base_path = base_path.trim_end_matches('/');

    let mut proxy_url = format!("https://{host}{base_path}{segment_path}");
    if !query.is_empty() {
        proxy_url.push('?');
        proxy_url.push_str(query);
    }

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

    let mut builder = Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY));
    if let Some(ct) = ct {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    builder.body(Body::from(body)).unwrap()
}

async fn proxy_dash_segment(state: &AppState, url: &str, hdnea: Option<&str>) -> (u16, Vec<u8>, Option<String>) {
    let mut req = state.http.get(url).header(header::USER_AGENT, television::PLAYER_USER_AGENT);
    if let Some(t) = hdnea {
        req = req.header(header::COOKIE, format!("__hdnea__={t}"));
    }
    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let ct = resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
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
        format!("{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT", WD[weekday as usize], d, MO[m as usize], y, hh, mm, ss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let out = rewrite_base_url(body, "/render.dash/host/x/path/y/dash/");
        assert_eq!(out, "<Period><BaseURL>/render.dash/host/x/path/y/dash/</BaseURL></Period>");
    }

    #[test]
    fn inserts_base_url_when_missing() {
        let body = "<Period id=\"0\"><AdaptationSet/></Period>";
        let out = rewrite_base_url(body, "/render.dash/host/x/path/y/");
        assert!(out.contains("<Period id=\"0\">\n<BaseURL>/render.dash/host/x/path/y/</BaseURL>"));
    }
}
