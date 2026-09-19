//! The JSON API behind the web UI. Mirrors `internal/handlers/api.go`.
//! Routes under `/api/auth/` are open; the rest need an admin session
//! (checked by the gate middleware in `server.rs`), except in the slim
//! build, which does not compile this module in at all.

use crate::state::AppState;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

pub type SharedState = Arc<AppState>;

fn session_cookie_header(_state: &AppState, value: &str) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        crate::access::SESSION_COOKIE,
        value,
        crate::access::SESSION_TTL.as_secs(),
    )
}

fn clear_cookie_header() -> String {
    format!("{}=; Path=/; HttpOnly; Max-Age=0", crate::access::SESSION_COOKIE)
}

fn err(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"message": message.into()}))).into_response()
}

pub fn has_session(state: &AppState, cookie_header: Option<&str>) -> bool {
    let Some(cookies) = cookie_header else { return false };
    for part in cookies.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(&format!("{}=", crate::access::SESSION_COOKIE)) {
            return state.access.valid_session(v, SystemTime::now());
        }
    }
    false
}

/// The `/k/<key>` prefix the gate middleware stripped from this request, if
/// any. Inserted as a request extension so downstream handlers (here, and
/// URL-building for playlists) can see it without re-parsing the path.
#[derive(Clone, Default)]
pub struct KeyPrefix(pub String);

pub async fn auth_state(
    State(state): State<SharedState>,
    prefix: Option<axum::Extension<KeyPrefix>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let authed = state.config.disable_auth
        || has_session(&state, headers.get(header::COOKIE).and_then(|v| v.to_str().ok()));
    let key_presented = prefix.map(|p| !p.0 .0.is_empty()).unwrap_or(false);
    Json(json!({
        "passwordSet": state.access.has_password(),
        "authenticated": authed,
        "keyPresented": key_presented,
    }))
}

#[derive(Deserialize)]
pub struct PasswordBody {
    password: String,
}

pub async fn auth_setup(
    State(state): State<SharedState>,
    prefix: Option<axum::Extension<KeyPrefix>>,
    Json(body): Json<PasswordBody>,
) -> Response {
    if state.access.has_password() {
        return err(StatusCode::CONFLICT, "a password is already set");
    }
    let has_prefix = prefix.map(|p| !p.0 .0.is_empty()).unwrap_or(false);
    if !has_prefix && !state.config.disable_auth {
        return err(StatusCode::FORBIDDEN, "open the setup link that contains the access key");
    }
    match state.access.set_password(&body.password) {
        Ok(()) => login_response(&state),
        Err(crate::access::AccessError::WeakPassword(n)) => {
            err(StatusCode::BAD_REQUEST, format!("the password needs at least {n} characters"))
        }
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "cannot save the password"),
    }
}

pub async fn auth_login(
    State(state): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<PasswordBody>,
) -> Response {
    match state.access.login(&addr.ip().to_string(), &body.password, SystemTime::now()) {
        Ok(true) => login_response(&state),
        Ok(false) => err(StatusCode::UNAUTHORIZED, "wrong password"),
        Err(crate::access::AccessError::TooManyAttempts) => {
            err(StatusCode::TOO_MANY_REQUESTS, "too many wrong passwords, try again later")
        }
        Err(_) => err(StatusCode::BAD_REQUEST, "no password is set yet"),
    }
}

fn login_response(state: &AppState) -> Response {
    let session = match state.access.new_session(SystemTime::now()) {
        Ok(s) => s,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "cannot start a session"),
    };
    let mut resp = Json(json!({"status": true})).into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie_header(state, &session).parse().unwrap(),
    );
    resp
}

pub async fn auth_logout() -> Response {
    let mut resp = Json(json!({"status": true})).into_response();
    resp.headers_mut()
        .insert(header::SET_COOKIE, clear_cookie_header().parse().unwrap());
    resp
}

#[derive(Deserialize)]
pub struct ChangePasswordBody {
    current: String,
    new: String,
}

pub async fn account_password(
    State(state): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<ChangePasswordBody>,
) -> Response {
    match state.access.login(&addr.ip().to_string(), &body.current, SystemTime::now()) {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::UNAUTHORIZED, "the current password is wrong"),
        Err(crate::access::AccessError::TooManyAttempts) => {
            return err(StatusCode::TOO_MANY_REQUESTS, "too many wrong passwords, try again later")
        }
        Err(_) => return err(StatusCode::UNAUTHORIZED, "the current password is wrong"),
    }
    match state.access.set_password(&body.new) {
        Ok(()) => login_response(&state),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

pub async fn status(State(state): State<SharedState>) -> Response {
    let playlist = if state.config.disable_auth {
        "/playlist.m3u".to_string()
    } else {
        match state.access.playlist_path() {
            Ok(p) => p,
            Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "cannot read the access key"),
        }
    };
    let epg_path = format!("{}epg.xml.gz", playlist.trim_end_matches("playlist.m3u"));
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "jiotv": {"loggedIn": state.tv.logged_in()},
        "tvplus": {"enabled": state.tvplus.enabled(), "connected": state.tvplus.connected()},
        "playlistPath": playlist,
        "epgPath": epg_path,
        "epg": state.config.epg,
        "drm": state.config.drm,
        "logoutDisabled": state.config.disable_logout,
    }))
    .into_response()
}

#[derive(serde::Serialize)]
struct ApiChannel {
    id: String,
    name: String,
    logo: String,
    category: String,
    language: String,
    hd: bool,
    tvplus: bool,
    catchup: bool,
    playable: bool,
}

pub async fn channels(State(state): State<SharedState>) -> Response {
    let list = match state.tv.channels().await {
        Ok(l) => l,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    state.tvplus.refresh_catalogue_if_needed(&state.tv).await;
    let mut all = list.result;
    all.extend(state.tvplus.exclusive_channels(&all));
    all.extend(state.custom_channels.all());

    let mut out: Vec<ApiChannel> = all
        .iter()
        .map(|ch| {
            let logo = if ch.logo_url.starts_with("http://") || ch.logo_url.starts_with("https://") {
                ch.logo_url.clone()
            } else {
                format!("/jtvimage/{}", ch.logo_url)
            };
            ApiChannel {
                id: ch.id.clone(),
                name: ch.name.clone(),
                logo,
                category: crate::television::category_name(ch.category).to_string(),
                language: crate::television::language_name(ch.language).to_string(),
                hd: ch.is_hd,
                tvplus: ch.id.starts_with(crate::tvplus::ID_PREFIX),
                catchup: ch.is_catchup_available,
                playable: state.is_playable(&ch.id),
            }
        })
        .collect();
    out.sort_by_key(|c| !c.playable);
    Json(json!({"channels": out})).into_response()
}

#[derive(Deserialize)]
pub struct JioTvOtpBody {
    number: String,
    #[serde(default)]
    otp: String,
}

/// `POST /login/sendOTP`: sends a JioTV login OTP. Mirrors
/// `LoginSendOTPHandler`.
pub async fn jiotv_send_otp(State(state): State<SharedState>, Json(body): Json<JioTvOtpBody>) -> Response {
    if body.number.is_empty() {
        return err(StatusCode::BAD_REQUEST, "Mobile Number is required");
    }
    match crate::login::LoginClient::new(state.http.clone()).send_otp(&body.number).await {
        Ok(sent) => Json(json!({"status": sent})).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("Could not send the OTP: {e}")),
    }
}

/// `POST /login/verifyOTP`: completes the JioTV login and loads it. Mirrors
/// `LoginVerifyOTPHandler`.
pub async fn jiotv_verify_otp(State(state): State<SharedState>, Json(body): Json<JioTvOtpBody>) -> Response {
    if body.number.is_empty() || body.otp.is_empty() {
        return err(StatusCode::BAD_REQUEST, "Mobile Number and OTP are required");
    }
    let creds = match crate::login::LoginClient::new(state.http.clone()).verify_otp(&body.number, &body.otp, &state.tv.device_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return Json(json!({"status": "failed", "message": "Invalid OTP"})).into_response(),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("Could not verify the OTP: {e}")),
    };
    if crate::login::save(&state.store, &creds, crate::login::TOUCH_ALL).is_err() {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "cannot save the login");
    }
    state.tv.set_credentials(creds);
    Json(json!({"status": "success"})).into_response()
}

pub async fn jiotv_logout(State(state): State<SharedState>) -> Response {
    if state.config.disable_logout {
        return err(StatusCode::FORBIDDEN, "logout is disabled");
    }
    let _ = crate::login::clear(&state.store);
    state.tv.clear_credentials();
    Json(json!({"status": true})).into_response()
}

pub async fn rotate_key(State(state): State<SharedState>) -> Response {
    if state.config.disable_auth {
        return err(StatusCode::BAD_REQUEST, "auth is disabled");
    }
    if state.access.rotate().is_err() {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "cannot rotate the key");
    }
    status(State(state)).await
}


#[derive(Deserialize)]
pub struct TvPlusSendOtpBody {
    number: String,
    connection: Option<usize>,
}

/// `POST /api/tvplus/login/sendOTP`. The first call (number only) returns
/// the fibre connections on the number when there's a choice; a second call
/// with `connection` sends the OTP for that connection. Mirrors
/// `TVPlusSendOTPHandler`.
pub async fn tvplus_send_otp(State(state): State<SharedState>, Json(body): Json<TvPlusSendOtpBody>) -> Response {
    if !state.tvplus.enabled() {
        return err(StatusCode::BAD_REQUEST, "JioTV+ is not enabled");
    }
    match state.tvplus.send_otp(&body.number, body.connection).await {
        Ok(outcome) => {
            let connections: Vec<_> = outcome
                .connections
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let tail = if c.identifier.len() > 4 { &c.identifier[c.identifier.len() - 4..] } else { &c.identifier };
                    json!({"index": i, "name": c.name, "product": c.product_name, "lineEndsWith": tail})
                })
                .collect();
            Json(json!({"status": outcome.sent, "connections": connections})).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("Could not send the OTP: {e}")),
    }
}

#[derive(Deserialize)]
pub struct TvPlusVerifyOtpBody {
    number: String,
    otp: String,
}

/// `POST /api/tvplus/login/verifyOTP`. Mirrors `TVPlusVerifyOTPHandler`.
pub async fn tvplus_verify_otp(State(state): State<SharedState>, Json(body): Json<TvPlusVerifyOtpBody>) -> Response {
    if !state.tvplus.enabled() {
        return err(StatusCode::BAD_REQUEST, "JioTV+ is not enabled");
    }
    match state.tvplus.verify_otp(&body.number, &body.otp, &state.store).await {
        Ok(ok) => Json(json!({"status": ok})).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

/// `POST /api/tvplus/logout`. Mirrors `TVPlusLogoutHandler` (the device is
/// kept so a later login reuses the same device slot).
pub async fn tvplus_logout(State(state): State<SharedState>) -> Response {
    if state.config.disable_logout {
        return err(StatusCode::FORBIDDEN, "logout is disabled");
    }
    match state.tvplus.logout(&state.store) {
        Ok(()) => Json(json!({"status": true})).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `GET /api/live/play/:id?q=` — resolves a live channel to what the
/// in-app player needs, the same shape `/api/ott/play/:id` gives
/// `VodPlayer.svelte` (`{dash, url, license}`), so `Watch.svelte` can use
/// the same Shaka/hls.js logic instead of the old Go-template `/mpd/:id`
/// iframe. Tries DASH first via the same `get_drm_mpd` the IPTV
/// `/live/mpd/:id` route uses, falling back to HLS exactly like
/// `LiveHandler` does when there's no DASH stream.
pub async fn live_play(Path(id): Path<String>, Query(q): Query<HashMap<String, String>>, State(state): State<SharedState>) -> Response {
    let quality = q.get("q").cloned().unwrap_or_else(|| "auto".to_string());

    if let Some(ch) = state.custom_channels.get(&id) {
        return Json(json!({"dash": false, "url": ch.url, "license": null})).into_response();
    }

    if let Ok(out) = crate::dash::get_drm_mpd(&state, &id, &quality).await {
        if !out.play_url.is_empty() {
            let license = if out.license_url.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(out.license_url)
            };
            return Json(json!({"dash": true, "url": out.play_url, "license": license})).into_response();
        }
    }

    let live = match crate::stream::fetch_live(&state, &id).await {
        Ok(l) => l,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let live_url = crate::television::select_best_live_hls_url(&live, &quality);
    if live_url.is_empty() {
        return err(StatusCode::NOT_FOUND, format!("No stream found for channel id: {id}"));
    }
    let abs = crate::stream::to_absolute_stream_url(&live_url, crate::stream::absolute_base_from_live(&live).as_deref());
    let encrypted = state.secure.encrypt(&abs);
    let url = format!("/render.m3u8?auth={encrypted}&channel_key_id={id}");
    Json(json!({"dash": false, "url": url, "license": null})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{access::Access, secureurl::SecureUrl, store::Store, television::Television};

    use crate::config::Config;

    fn state() -> Arc<AppState> {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().to_str().unwrap()).unwrap());
        std::mem::forget(dir);
        Arc::new(AppState {
            config: Config::default(),
            path_prefix: String::new(),
            access: Arc::new(Access::new(store.clone())),
            store,
            tv: Arc::new(Television::new(reqwest::Client::new())),
            secure: Arc::new(SecureUrl::new(false)),
            http: reqwest::Client::new(),
            drm_channels: Default::default(),
            custom_channels: Arc::new(crate::custom_channels::CustomChannels::new()),
            render_caches: Default::default(),
            dash_state: Default::default(),
            tvplus: Arc::new(crate::tvplus_state::TvPlusState::new(false)),
            vod_state: Default::default(),
        })
    }

    #[tokio::test]
    async fn setup_requires_key_prefix_when_auth_enabled() {
        let s = state();
        let resp = auth_setup(State(s.clone()), None, Json(PasswordBody { password: "longenough".into() })).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn setup_succeeds_with_key_prefix_extension() {
        let s = state();
        let prefix = Some(axum::Extension(KeyPrefix("/k/abc/".to_string())));
        let resp = auth_setup(State(s.clone()), prefix, Json(PasswordBody { password: "longenough".into() })).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(s.access.has_password());
    }
}
