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

/// `secure` is true for sessions created over the TLS listener, so the browser
/// never replays an HTTPS-authenticated session over the plain-HTTP listener.
fn session_cookie_header(value: &str, secure: bool) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        crate::access::SESSION_COOKIE,
        value,
        crate::access::SESSION_TTL.as_secs(),
        if secure { "; Secure" } else { "" },
    )
}

fn clear_cookie_header() -> String {
    format!(
        "{}=; Path=/; HttpOnly; Max-Age=0",
        crate::access::SESSION_COOKIE
    )
}

fn err(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"message": message.into()}))).into_response()
}

pub fn has_session(state: &AppState, cookie_header: Option<&str>) -> bool {
    let Some(cookies) = cookie_header else {
        return false;
    };
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
        || has_session(
            &state,
            headers.get(header::COOKIE).and_then(|v| v.to_str().ok()),
        );
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
    https: Option<axum::Extension<crate::tls::Https>>,
    Json(body): Json<PasswordBody>,
) -> Response {
    if state.access.has_password() {
        return err(StatusCode::CONFLICT, "a password is already set");
    }
    let has_prefix = prefix.map(|p| !p.0 .0.is_empty()).unwrap_or(false);
    if !has_prefix && !state.config.disable_auth {
        return err(
            StatusCode::FORBIDDEN,
            "open the setup link that contains the access key",
        );
    }
    match state.access.set_password(&body.password) {
        Ok(()) => login_response(&state, https.is_some()),
        Err(crate::access::AccessError::WeakPassword(n)) => err(
            StatusCode::BAD_REQUEST,
            format!("the password needs at least {n} characters"),
        ),
        Err(_) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot save the password",
        ),
    }
}

pub async fn auth_login(
    State(state): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    https: Option<axum::Extension<crate::tls::Https>>,
    Json(body): Json<PasswordBody>,
) -> Response {
    match state
        .access
        .login(&addr.ip().to_string(), &body.password, SystemTime::now())
    {
        Ok(true) => login_response(&state, https.is_some()),
        Ok(false) => err(StatusCode::UNAUTHORIZED, "wrong password"),
        Err(crate::access::AccessError::TooManyAttempts) => err(
            StatusCode::TOO_MANY_REQUESTS,
            "too many wrong passwords, try again later",
        ),
        Err(_) => err(StatusCode::BAD_REQUEST, "no password is set yet"),
    }
}

fn login_response(state: &AppState, secure: bool) -> Response {
    let session = match state.access.new_session(SystemTime::now()) {
        Ok(s) => s,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "cannot start a session"),
    };
    let mut resp = Json(json!({"status": true})).into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie_header(&session, secure).parse().unwrap(),
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
    https: Option<axum::Extension<crate::tls::Https>>,
    Json(body): Json<ChangePasswordBody>,
) -> Response {
    match state
        .access
        .login(&addr.ip().to_string(), &body.current, SystemTime::now())
    {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::UNAUTHORIZED, "the current password is wrong"),
        Err(crate::access::AccessError::TooManyAttempts) => {
            return err(
                StatusCode::TOO_MANY_REQUESTS,
                "too many wrong passwords, try again later",
            )
        }
        Err(_) => return err(StatusCode::UNAUTHORIZED, "the current password is wrong"),
    }
    match state.access.set_password(&body.new) {
        Ok(()) => login_response(&state, https.is_some()),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

pub async fn status(State(state): State<SharedState>) -> Response {
    let playlist = if state.config.disable_auth {
        "/playlist.m3u".to_string()
    } else {
        match state.access.playlist_path() {
            Ok(p) => p,
            Err(_) => {
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "cannot read the access key",
                )
            }
        }
    };
    let epg_path = format!("{}epg.xml.gz", playlist.trim_end_matches("playlist.m3u"));
    let tv_plan = state.tv.plan_summary().await;
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "jiotv": {"loggedIn": state.tv.logged_in()},
        "extras": {"enabled": state.extras.enabled(), "connected": state.extras.connected()},
        "catalogue": {
            "activeProduct": state.active_product().as_str(),
            "extrasEntitlementsAvailable": state.extras.entitlements_available(),
            "extrasEntitlementsApplied": state.extras.entitlements_applied(),
            "extrasEntitlementStatus": state.extras.entitlement_status().as_str(),
            "tvPlanDataAvailable": tv_plan.is_some(),
            "tvActivePlanCount": tv_plan.as_ref().map(|summary| summary.active_plan_count),
            "tvPlanProviderCount": tv_plan.as_ref().map(|summary| summary.provider_count),
            "tvLinearEntitlement": "unknown"
        },
        "playlistPath": playlist,
        "epgPath": epg_path,
        "httpPort": state.listen.get().map(|l| l.http),
        "tlsPort": state.listen.get().and_then(|l| l.tls),
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
    extras: bool,
    catchup: bool,
    #[serde(rename = "requiresSubscription")]
    requires_subscription: bool,
    playable: bool,
}

pub async fn channels(State(state): State<SharedState>) -> Response {
    let all = match state.effective_channels().await {
        Ok(l) => l,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let mut out: Vec<ApiChannel> = all
        .iter()
        .map(|ch| {
            let logo = if ch.logo_url.starts_with("http://") || ch.logo_url.starts_with("https://")
            {
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
                extras: ch.id.starts_with(crate::extras::ID_PREFIX),
                catchup: ch.is_catchup_available,
                requires_subscription: ch.requires_subscription(),
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
pub async fn jiotv_send_otp(
    State(state): State<SharedState>,
    Json(body): Json<JioTvOtpBody>,
) -> Response {
    if body.number.is_empty() {
        return err(StatusCode::BAD_REQUEST, "Mobile Number is required");
    }
    match crate::login::LoginClient::new(state.http.clone())
        .send_otp(&body.number)
        .await
    {
        Ok(sent) => Json(json!({"status": sent})).into_response(),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not send the OTP: {e}"),
        ),
    }
}

/// `POST /login/verifyOTP`: completes the JioTV login and loads it. Mirrors
/// `LoginVerifyOTPHandler`.
pub async fn jiotv_verify_otp(
    State(state): State<SharedState>,
    Json(body): Json<JioTvOtpBody>,
) -> Response {
    if body.number.is_empty() || body.otp.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "Mobile Number and OTP are required",
        );
    }
    let creds = match crate::login::LoginClient::new(state.http.clone())
        .verify_otp(&body.number, &body.otp, &state.tv.device_id)
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => {
            return Json(json!({"status": "failed", "message": "Invalid OTP"})).into_response()
        }
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not verify the OTP: {e}"),
            )
        }
    };
    if crate::login::save(&state.store, &creds, crate::login::TOUCH_ALL).is_err() {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "cannot save the login");
    }
    state.invalidate_context();
    state.tv.set_credentials(creds);
    state.extras.invalidate_account_context();
    // A request that entered after the first rotation still used the old
    // credentials; rotate again now that the new ones are installed.
    state.invalidate_context();
    crate::epg::trigger_regeneration(&state);
    Json(json!({"status": "success"})).into_response()
}

pub async fn jiotv_logout(State(state): State<SharedState>) -> Response {
    if state.config.disable_logout {
        return err(StatusCode::FORBIDDEN, "logout is disabled");
    }
    let _ = crate::login::clear(&state.store);
    state.invalidate_context();
    state.tv.clear_credentials();
    state.extras.invalidate_account_context();
    state.invalidate_context();
    crate::epg::trigger_regeneration(&state);
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
pub struct ExtrasSendOtpBody {
    number: String,
    connection: Option<usize>,
}

/// `POST /api/extras/login/sendOTP`. The first call (number only) returns
/// the fibre connections on the number when there's a choice; a second call
/// with `connection` sends the OTP for that connection. Mirrors
/// `ExtrasSendOTPHandler`.
pub async fn extras_send_otp(
    State(state): State<SharedState>,
    Json(body): Json<ExtrasSendOtpBody>,
) -> Response {
    if !state.extras.enabled() {
        return err(StatusCode::BAD_REQUEST, "extras is not enabled");
    }
    match state.extras.send_otp(&body.number, body.connection).await {
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
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not send the OTP: {e}"),
        ),
    }
}

#[derive(Deserialize)]
pub struct ExtrasVerifyOtpBody {
    number: String,
    otp: String,
}

/// `POST /api/extras/login/verifyOTP`. Mirrors `ExtrasVerifyOTPHandler`.
pub async fn extras_verify_otp(
    State(state): State<SharedState>,
    Json(body): Json<ExtrasVerifyOtpBody>,
) -> Response {
    if !state.extras.enabled() {
        return err(StatusCode::BAD_REQUEST, "extras is not enabled");
    }
    let before = state.extras.credentials_marker();
    let result = state
        .extras
        .verify_otp(&body.number, &body.otp, &state.store)
        .await;
    // A wrong or mistyped OTP leaves the active account untouched, so it must
    // not rotate the context and cut off existing viewers. Rotate whenever the
    // installed credentials actually changed, which includes an OTP that
    // verified but failed its token exchange: that already swapped the account.
    // This also discards anything resolved while the exchange was in flight (the
    // Extras and VOD caches are guarded by a generation, and URL-minting
    // handlers re-check the epoch).
    if state.extras.credentials_marker() != before {
        state.invalidate_context();
        crate::epg::trigger_regeneration(&state);
    }
    match result {
        Ok(ok) => Json(json!({"status": ok})).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

/// `POST /api/extras/logout`. Mirrors `ExtrasLogoutHandler` (the device is
/// kept so a later login reuses the same device slot).
pub async fn extras_logout(State(state): State<SharedState>) -> Response {
    if state.config.disable_logout {
        return err(StatusCode::FORBIDDEN, "logout is disabled");
    }
    state.invalidate_context();
    match state.extras.logout(&state.store) {
        Ok(()) => {
            state.invalidate_context();
            crate::epg::trigger_regeneration(&state);
            Json(json!({"status": true})).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct UnlockBody {
    code: String,
}

fn extras_status(state: &AppState) -> serde_json::Value {
    json!({"enabled": state.extras.enabled(), "connected": state.extras.connected()})
}

/// `POST /api/extras/unlock`. Only reached when the channel search box's
/// own shape test decided the query looked like an unlock code (see
/// `unlock.rs`), so this never sees an ordinary search. Rate-limited per IP
/// like admin login; a wrong code gets the same generic message regardless
/// of which part was wrong.
pub async fn extras_unlock(
    State(state): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<UnlockBody>,
) -> Response {
    if !crate::unlock::looks_like_code(&body.code) {
        return err(StatusCode::BAD_REQUEST, "wrong code");
    }
    let ip = addr.ip().to_string();
    let now = SystemTime::now();
    if !state.unlock_limiter.allowed(&ip, now) {
        return err(
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts, try again later",
        );
    }
    let Some(public_ip) = state.public_ip.get().await else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "the server's public IPv4 address isn't available right now, so no unlock code will work",
        );
    };
    if !crate::unlock::code_matches(public_ip, now, &body.code) {
        state.unlock_limiter.record_failure(&ip, now);
        return err(StatusCode::UNAUTHORIZED, "wrong code");
    }
    state.unlock_limiter.record_success(&ip);
    if let Err(e) = state.store.set(crate::unlock::STORE_KEY_UNLOCKED, "true") {
        tracing::warn!("extras unlock: cannot save: {e}");
    }
    state.invalidate_context();
    state.extras.set_unlocked(true, &state.http, &state.store);
    state.invalidate_context();
    crate::epg::trigger_regeneration(&state);
    Json(json!({"status": true, "extras": extras_status(&state)})).into_response()
}

/// `POST /api/extras/lock` — the Settings page's "Lock" button. The stored
/// false value explicitly overrides the `extras` config/env default, so a
/// router configured with `JIOTV_EXTRAS=true` can still be locked from UI.
pub async fn extras_lock(State(state): State<SharedState>) -> Response {
    if let Err(e) = state.store.set(crate::unlock::STORE_KEY_UNLOCKED, "false") {
        tracing::warn!("extras lock: cannot save: {e}");
    }
    state.invalidate_context();
    state.extras.set_unlocked(false, &state.http, &state.store);
    state.invalidate_context();
    crate::epg::trigger_regeneration(&state);
    Json(json!({"status": true, "extras": extras_status(&state)})).into_response()
}

/// `GET /api/live/play/:id?q=` — resolves a live channel to what the
/// in-app player needs, the same shape `/api/ott/play/:id` gives plus an
/// optional HLS alternative, so `Watch.svelte` can use
/// the same Shaka/hls.js logic instead of the old Go-template `/mpd/:id`
/// iframe. DASH remains the preferred source, matching the TV+ app. When the
/// provider also returned HLS, expose that exact source so the UI can retry it
/// after a DASH player failure. DASH-only channels therefore never fall into a
/// fabricated `/live/...m3u8` request.
pub async fn live_play(
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    State(state): State<SharedState>,
) -> Response {
    let quality = q.get("q").cloned().unwrap_or_else(|| "auto".to_string());

    if !state.channel_allowed(&id).await {
        return err(
            StatusCode::NOT_FOUND,
            format!("Channel {id} is not available for the active account"),
        );
    }

    if let Some(ch) = state.custom_channels.get(&id) {
        return Json(json!({"dash": false, "url": ch.url, "license": null})).into_response();
    }

    // The response URLs are encrypted under the current context epoch, so the
    // data they carry must belong to that epoch from lookup until the last URL
    // is generated. If an account switch lands in between, resolve again.
    for _ in 0..2 {
        let (live, epoch) = match crate::dash::get_live_cached(&state, &id).await {
            Ok(l) => l,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        let response = live_play_response_from_live(&state, &live, &id, &quality);
        if state.secure.current_epoch() == epoch {
            return response;
        }
    }
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        "The active account changed while resolving the stream; retry",
    )
}

fn live_play_response_from_live(
    state: &AppState,
    live: &crate::television::LiveUrlOutput,
    id: &str,
    quality: &str,
) -> Response {
    let hls_quality = crate::stream::hls_quality_for_channel(id, quality);
    let live_url = crate::television::select_best_live_hls_url(live, hls_quality);
    let hls = if live_url.is_empty() {
        serde_json::Value::Null
    } else {
        let abs = crate::stream::to_absolute_stream_url(
            &live_url,
            crate::stream::absolute_base_from_live(live).as_deref(),
        );
        let encrypted = state.secure.encrypt(&abs);
        // Carry a forced quality so a 404 recovery retries it before `auto`.
        let q = if hls_quality == "auto" {
            String::new()
        } else {
            format!("&q={hls_quality}")
        };
        serde_json::Value::String(format!(
            "/render.m3u8?auth={encrypted}&channel_key_id={id}{q}"
        ))
    };

    if let Ok(out) = crate::dash::build_drm_mpd_output(state, live, id, quality) {
        if !out.play_url.is_empty() {
            let license = if out.license_url.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(out.license_url)
            };
            return Json(json!({
                "dash": true,
                "url": out.play_url,
                "license": license,
                "hls": hls
            }))
            .into_response();
        }
    }

    if live_url.is_empty() {
        return err(
            StatusCode::NOT_FOUND,
            format!("No stream found for channel id: {id}"),
        );
    }
    let url = hls.as_str().unwrap_or_default();
    Json(json!({"dash": false, "url": url, "license": null, "hls": null})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{access::Access, secureurl::SecureUrl, store::Store, television::Television};
    use http_body_util::BodyExt;

    use crate::config::Config;

    fn state() -> Arc<AppState> {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().to_str().unwrap()).unwrap());
        std::mem::forget(dir);
        let extras = Arc::new(crate::extras_state::ExtrasState::new(false, None));
        state_with(store, extras)
    }

    fn state_with(
        store: Arc<Store>,
        extras: Arc<crate::extras_state::ExtrasState>,
    ) -> Arc<AppState> {
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
            epg_state: Default::default(),
            extras,
            vod_state: Default::default(),
            public_ip: Arc::new(crate::unlock::PublicIp::new(reqwest::Client::new())),
            unlock_limiter: Arc::new(crate::unlock::AttemptLimiter::default()),
            listen: Default::default(),
        })
    }

    /// Extras state with a mock auth service. `exchange_ok` decides whether the
    /// token exchange after a verified OTP succeeds; `verify_status` is the
    /// status of the OTP verification itself.
    async fn extras_state_with_mock_auth(
        verify_status: u16,
        exchange_status: u16,
    ) -> (Arc<AppState>, wiremock::MockServer, wiremock::MockServer) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let auth = MockServer::start().await;
        let user_service = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/apis/v3.2/stbotplogin/sendotp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "message": "ok", "identifier": "redacted-identifier", "fttxIds": []
            })))
            .mount(&auth)
            .await;
        Mock::given(method("POST"))
            .and(path("/apis/v3.2/stbotplogin/verifyotp"))
            .respond_with(if verify_status == 200 {
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ssoToken": "redacted-sso",
                    "sessionAttributes": {"user": {"subscriberId": "redacted-sub", "unique": "redacted-uniq"}}
                }))
            } else {
                ResponseTemplate::new(verify_status)
            })
            .mount(&auth)
            .await;
        Mock::given(method("POST"))
            .and(path("/loginotp/exchangetoken"))
            .respond_with(if exchange_status == 200 {
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "authToken": "redacted-at", "refreshToken": "redacted-rt", "userId": "redacted-uid"
                }))
            } else {
                ResponseTemplate::new(exchange_status)
            })
            .mount(&user_service)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().to_str().unwrap()).unwrap());
        std::mem::forget(dir);
        let extras = Arc::new(crate::extras_state::ExtrasState::new(true, None));
        extras.init(&reqwest::Client::new(), &store);
        extras.set_endpoints_for_test(crate::extras::Endpoints {
            auth: auth.uri(),
            user_service: user_service.uri(),
            ..Default::default()
        });
        let s = state_with(store, extras);
        s.extras.send_otp("9876543210", None).await.unwrap();
        (s, auth, user_service)
    }

    async fn verify(s: &Arc<AppState>) -> Response {
        extras_verify_otp(
            State(s.clone()),
            Json(ExtrasVerifyOtpBody {
                number: "9876543210".into(),
                otp: "123456".into(),
            }),
        )
        .await
    }

    #[tokio::test]
    async fn otp_that_verifies_but_fails_its_exchange_still_rotates_the_context() {
        // The verification installed the new account's SSO credentials before
        // the exchange failed, so the previous account's artifacts are stale.
        let (s, _auth, _user) = extras_state_with_mock_auth(200, 500).await;
        let before = s.secure.current_epoch();
        let resp = verify(&s).await;
        let json = response_json(resp).await;
        assert_eq!(json["status"], false);
        assert!(s.secure.current_epoch() > before, "context was not rotated");
        assert!(!s.extras.connected(), "no auth token after a failed exchange");
    }

    #[tokio::test]
    async fn rejected_otp_leaves_the_context_alone() {
        let (s, _auth, _user) = extras_state_with_mock_auth(401, 200).await;
        let before = s.secure.current_epoch();
        let resp = verify(&s).await;
        assert_eq!(response_json(resp).await["status"], false);
        assert_eq!(s.secure.current_epoch(), before);
    }

    #[tokio::test]
    async fn successful_otp_rotates_the_context() {
        let (s, _auth, _user) = extras_state_with_mock_auth(200, 200).await;
        let before = s.secure.current_epoch();
        let resp = verify(&s).await;
        assert_eq!(response_json(resp).await["status"], true);
        assert!(s.secure.current_epoch() > before);
        assert!(s.extras.connected());
    }

    #[tokio::test]
    async fn failed_extras_otp_does_not_rotate_the_context() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().to_str().unwrap()).unwrap());
        let extras = Arc::new(crate::extras_state::ExtrasState::new(true, None));
        extras.init(&reqwest::Client::new(), &store);
        let s = state_with(store, extras);
        assert!(s.extras.enabled());
        let before = s.secure.current_epoch();

        // No OTP was sent first, so verification fails without changing the
        // active account; existing viewers' URLs must keep working.
        let resp = extras_verify_otp(
            State(s.clone()),
            Json(ExtrasVerifyOtpBody {
                number: "9876543210".into(),
                otp: "123456".into(),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(s.secure.current_epoch(), before);
    }

    #[tokio::test]
    async fn setup_requires_key_prefix_when_auth_enabled() {
        let s = state();
        let resp = auth_setup(
            State(s.clone()),
            None,
            None,
            Json(PasswordBody {
                password: "longenough".into(),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn setup_succeeds_with_key_prefix_extension() {
        let s = state();
        let prefix = Some(axum::Extension(KeyPrefix("/k/abc/".to_string())));
        let resp = auth_setup(
            State(s.clone()),
            prefix,
            None,
            Json(PasswordBody {
                password: "longenough".into(),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(s.access.has_password());
        let cookie = resp.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(
            !cookie.contains("Secure"),
            "plain-HTTP setup keeps a non-Secure cookie"
        );
    }

    #[tokio::test]
    async fn setup_over_tls_issues_a_secure_session_cookie() {
        let s = state();
        let prefix = Some(axum::Extension(KeyPrefix("/k/abc/".to_string())));
        let https = Some(axum::Extension(crate::tls::Https));
        let resp = auth_setup(
            State(s.clone()),
            prefix,
            https,
            Json(PasswordBody {
                password: "longenough".into(),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let cookie = resp.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.ends_with("; Secure"), "{cookie}");
    }

    #[tokio::test]
    async fn status_does_not_claim_unknown_entitlements_were_applied() {
        let response = status(State(state())).await;
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["catalogue"]["tvLinearEntitlement"], "unknown");
        assert_eq!(json["catalogue"]["extrasEntitlementStatus"], "unknown");
        assert_eq!(json["catalogue"]["extrasEntitlementsAvailable"], false);
        assert_eq!(json["catalogue"]["extrasEntitlementsApplied"], false);
    }

    #[tokio::test]
    async fn status_reports_the_listen_ports_once_serve_has_chosen_them() {
        let s = state();
        let json = response_json(status(State(s.clone())).await).await;
        assert!(json["httpPort"].is_null());
        assert!(json["tlsPort"].is_null());

        s.listen
            .set(crate::state::ListenPorts {
                http: 5001,
                tls: Some(5443),
            })
            .unwrap();
        let json = response_json(status(State(s)).await).await;
        assert_eq!(json["httpPort"], 5001);
        assert_eq!(json["tlsPort"], 5443);
    }

    #[tokio::test]
    async fn channels_marks_premium_business_type_as_subscription_required() {
        let s = state();
        s.tv.set_channels_for_test(vec![
            crate::television::Channel {
                id: "154".into(),
                name: "Premium".into(),
                business_type: "premium".into(),
                ..Default::default()
            },
            crate::television::Channel {
                id: "1148".into(),
                name: "Free".into(),
                business_type: "free".into(),
                ..Default::default()
            },
        ]);

        let response = channels(State(s)).await;
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let channels = json["channels"].as_array().unwrap();

        let premium = channels.iter().find(|row| row["id"] == "154").unwrap();
        let free = channels.iter().find(|row| row["id"] == "1148").unwrap();
        assert_eq!(premium["requiresSubscription"], true);
        assert_eq!(free["requiresSubscription"], false);
    }

    async fn response_json(response: Response) -> serde_json::Value {
        let body = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn live_play_response_keeps_mpd_only_source_on_dash() {
        let s = state();
        let live = crate::television::LiveUrlOutput {
            mpd: crate::television::Mpd {
                auto: "https://media.example/live/manifest.mpd".into(),
                key: "https://license.example/widevine".into(),
                ..Default::default()
            },
            is_drm: true,
            ..Default::default()
        };

        let json = response_json(live_play_response_from_live(&s, &live, "ex_mpd", "auto")).await;
        assert_eq!(json["dash"], true);
        assert!(json["url"].as_str().unwrap().starts_with("/render.mpd?"));
        assert!(json["license"].as_str().unwrap().starts_with("/drm?"));
        assert!(json["hls"].is_null());
    }

    #[tokio::test]
    async fn live_play_response_exposes_provider_hls_as_dash_alternative() {
        let s = state();
        let live = crate::television::LiveUrlOutput {
            bitrates: crate::television::Bitrates {
                auto: "https://media.example/live/master.m3u8".into(),
                ..Default::default()
            },
            mpd: crate::television::Mpd {
                auto: "https://media.example/live/manifest.mpd".into(),
                key: "https://license.example/widevine".into(),
                ..Default::default()
            },
            is_drm: true,
            ..Default::default()
        };

        let json = response_json(live_play_response_from_live(&s, &live, "ex_both", "auto")).await;
        assert_eq!(json["dash"], true);
        assert!(json["url"].as_str().unwrap().starts_with("/render.mpd?"));
        assert!(json["hls"].as_str().unwrap().starts_with("/render.m3u8?"));
    }

    #[tokio::test]
    async fn live_play_response_marks_primary_hls_without_alternative() {
        let s = state();
        let live = crate::television::LiveUrlOutput {
            bitrates: crate::television::Bitrates {
                auto: "https://media.example/live/master.m3u8".into(),
                ..Default::default()
            },
            ..Default::default()
        };

        let json = response_json(live_play_response_from_live(&s, &live, "ex_hls", "auto")).await;
        assert_eq!(json["dash"], false);
        assert!(json["url"].as_str().unwrap().starts_with("/render.m3u8?"));
        assert!(json["license"].is_null());
        assert!(json["hls"].is_null());
    }

    fn two_quality_hls() -> crate::television::LiveUrlOutput {
        crate::television::LiveUrlOutput {
            bitrates: crate::television::Bitrates {
                auto: "https://media.example/live/auto.m3u8".into(),
                high: "https://media.example/live/high.m3u8".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn decrypted_hls(s: &AppState, url: &str) -> (String, String) {
        let query = url.split_once('?').unwrap().1;
        let param = |name: &str| {
            query
                .split('&')
                .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
                .unwrap_or_default()
                .to_string()
        };
        (s.secure.decrypt(&param("auth")).unwrap(), param("q"))
    }

    #[tokio::test]
    async fn live_play_response_carries_forced_quality_into_the_hls_url() {
        let s = state();
        let json = response_json(live_play_response_from_live(
            &s,
            &two_quality_hls(),
            "ex_hls",
            "high",
        ))
        .await;
        let (source, q) = decrypted_hls(&s, json["url"].as_str().unwrap());
        assert!(source.ends_with("/high.m3u8"));
        assert_eq!(q, "high");
    }

    #[tokio::test]
    async fn live_play_response_keeps_audio_only_channels_on_auto() {
        let s = state();
        for id in ["1349", "1322"] {
            let json = response_json(live_play_response_from_live(
                &s,
                &two_quality_hls(),
                id,
                "high",
            ))
            .await;
            let (source, q) = decrypted_hls(&s, json["url"].as_str().unwrap());
            assert!(source.ends_with("/auto.m3u8"), "channel {id}");
            assert_eq!(q, "", "channel {id}");
        }
    }

    #[tokio::test]
    async fn account_transitions_rotate_again_after_the_new_state_is_installed() {
        let s = state();
        let before = s.secure.current_epoch();
        jiotv_logout(State(s.clone())).await;
        assert_eq!(s.secure.current_epoch(), before + 2, "logout");

        let before = s.secure.current_epoch();
        extras_lock(State(s.clone())).await;
        assert_eq!(s.secure.current_epoch(), before + 2, "extras lock");
    }

    #[test]
    fn session_cookie_is_secure_only_for_tls_logins() {
        assert!(session_cookie_header("abc", true).ends_with("; Secure"));
        assert!(!session_cookie_header("abc", false).contains("Secure"));
        assert!(session_cookie_header("abc", true).contains("HttpOnly; SameSite=Strict"));
    }
}
