//! HTTP server wiring: the access gate, the JSON API, playlist/channels,
//! logo proxy and (full build) the Svelte UI's static assets. Mirrors
//! `cmd/jiotv_go.go` / `cmd/ui.go` / `internal/access/access.go`.

use crate::api::KeyPrefix;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;

/// Routes an IPTV player reaches either as `/k/<key>/<path>` or, for an
/// already-authenticated admin session, directly at `<path>` (mirrors the Go
/// gate: `SessionCheck` accepts any non-open path, not just these). Kept
/// separate from `api_routes` so it can be `.nest()`-ed under `/k/:key`:
/// axum resolves `nest()` prefixes as part of its own route matching, which
/// (unlike mutating the URI from inside a `middleware::from_fn` layer, which
/// runs after matching and cannot influence it) reliably routes
/// `/k/<key>/channels` to the same handler as `/channels`.
fn content_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/playlist.m3u", get(playlist_redirect))
        .route("/channels", get(channels_or_playlist))
        .route("/jtvimage/:file", get(jtvimage))
        .route("/live/:id", get(crate::stream::live_handler))
        .route("/live/:quality/:id", get(crate::stream::live_quality_handler))
        .route("/live/mpd/:channelId", get(crate::dash::live_mpd_handler))
        .route("/live/key/:channelId", axum::routing::any(crate::dash::live_key_handler))
        .route("/catchup/stream/:id", get(crate::catchup::catchup_stream_handler))
        .route("/epg.xml.gz", get(crate::epg::epg_handler))
        .route("/epg/:channelId/:offset", get(crate::epg::web_epg_handler))
        .with_state(state)
}

fn api_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/auth/state", get(crate::api::auth_state))
        .route("/api/auth/setup", post(crate::api::auth_setup))
        .route("/api/auth/login", post(crate::api::auth_login))
        .route("/api/auth/logout", post(crate::api::auth_logout))
        .route("/api/status", get(crate::api::status))
        .route("/api/channels", get(crate::api::channels))
        .route("/api/account/password", post(crate::api::account_password))
        .route("/api/key/rotate", post(crate::api::rotate_key))
        .route("/api/jiotv/logout", post(crate::api::jiotv_logout))
        .route("/api/tvplus/login/sendOTP", post(crate::api::tvplus_send_otp))
        .route("/api/tvplus/login/verifyOTP", post(crate::api::tvplus_verify_otp))
        .route("/api/tvplus/logout", post(crate::api::tvplus_logout))
        .route("/api/ott/play/:id", get(crate::api::ott_not_implemented))
        .with_state(state)
}

/// Stream-proxy routes that never need the access key: their parameters are
/// AES-encrypted with a key generated fresh on every start (see
/// `secureurl`), so they cannot be forged and are only ever handed out
/// through a gated route. Mirrors the Go gate's `openPaths` for
/// `/render.*`, `/drm` and `/dashtime`.
fn open_stream_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/render.m3u8", get(crate::stream::render_m3u8_handler))
        .route("/render.ts", get(crate::stream::render_ts_handler))
        .route("/render.key", get(crate::stream::render_key_handler))
        .route("/render.mpd", get(crate::dash::render_mpd_handler))
        .route("/render.dash/*rest", get(crate::dash::render_dash_handler))
        .route("/drm", axum::routing::any(crate::dash::drm_license_handler))
        .route("/dashtime", get(crate::dash::dashtime_handler))
        .with_state(state)
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let mut router = Router::new()
        .nest("/k/:key", content_routes(state.clone()))
        .merge(content_routes(state.clone()))
        .merge(api_routes(state.clone()))
        .merge(open_stream_routes(state.clone()))
        .merge(open_routes(state.clone()));

    #[cfg(feature = "full")]
    {
        router = router.fallback(crate::ui_assets::serve_ui);
    }
    #[cfg(not(feature = "full"))]
    {
        router = router.fallback(|| async { (StatusCode::NOT_FOUND, "not found") });
    }

    router
        .layer(middleware::from_fn_with_state(state.clone(), gate))
        .with_state(state)
}

async fn playlist_redirect(Query(q): Query<HashMap<String, String>>) -> Redirect {
    let qs = q
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let target = if qs.is_empty() {
        "channels?type=m3u".to_string()
    } else {
        format!("channels?type=m3u&{qs}")
    };
    Redirect::permanent(&target)
}

async fn channels_or_playlist(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
    prefix: Option<axum::Extension<KeyPrefix>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let mut list = match state.tv.channels().await {
        Ok(l) => l,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    state.tvplus.refresh_catalogue_if_needed(&state.tv).await;
    list.result.extend(state.tvplus.exclusive_channels(&list.result));

    if q.get("type").map(String::as_str) != Some("m3u") {
        return axum::Json(list.result).into_response();
    }

    let host_url = base_url(&state, &prefix, &headers);
    let empty = String::new();
    let opts = crate::television::PlaylistOptions {
        host_url: &host_url,
        quality: q.get("q").unwrap_or(&empty),
        split_category: q.get("c").unwrap_or(&empty),
        languages: q.get("l").unwrap_or(&empty),
        skip_genres: q.get("sg").unwrap_or(&empty),
        sub_filter: q.get("sub").unwrap_or(&empty),
    };
    let m3u = crate::television::generate_m3u_playlist(
        &list.result,
        &opts,
        |id| state.is_drm_channel(id),
        |id| state.is_playable(id),
    );

    Response::builder()
        .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .header(header::CONTENT_DISPOSITION, "attachment; filename=jiotv_playlist.m3u")
        .body(Body::from(m3u))
        .unwrap()
}

fn base_url(state: &AppState, prefix: &Option<axum::Extension<KeyPrefix>>, headers: &axum::http::HeaderMap) -> String {
    // TLS termination is handled by the `--tls` flag on `serve`, not by
    // config; this always renders http:// because the gate and playlist
    // links are meant to be followed from the same connection they came in
    // on (a reverse proxy or `--tunnel` in front changes the effective
    // scheme, which isn't visible here without trusting X-Forwarded-Proto).
    let _ = state;
    let scheme = "http";
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let p = prefix.as_ref().map(|e| e.0 .0.clone()).unwrap_or_default();
    format!("{scheme}://{host}{p}")
}

#[derive(serde::Deserialize)]
struct FileParam {
    file: String,
}

async fn jtvimage(axum::extract::Path(p): axum::extract::Path<FileParam>, State(state): State<Arc<AppState>>) -> Response {
    let url = format!("https://jiotv.catchup.cdn.jio.com/dare_images/images/{}", p.file);
    proxy_get(&state.http, &url).await
}

fn open_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/jtvposter/:date/:file", get(crate::epg::poster_handler))
        .with_state(state)
}

async fn proxy_get(client: &reqwest::Client, url: &str) -> Response {
    match client.get(url).send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .cloned()
                .unwrap_or_else(|| header::HeaderValue::from_static("application/octet-stream"));
            match resp.bytes().await {
                Ok(bytes) => Response::builder()
                    .status(status)
                    .header(header::CONTENT_TYPE, content_type)
                    .body(Body::from(bytes))
                    .unwrap(),
                Err(_) => (StatusCode::BAD_GATEWAY, "upstream read error").into_response(),
            }
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "upstream request failed").into_response(),
    }
}

/// The access gate: `/k/<key>/...` needs a matching key, some paths are
/// always open, and everything else (the admin UI's own API surface) needs a
/// valid session cookie once a password is set. See
/// `internal/access/access.go` in the Go tree; `crate::access::OPEN_PATHS`
/// mirrors its `openPaths` list.
async fn gate(State(state): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();

    if let Some(rest) = path.strip_prefix(crate::access::KEY_PREFIX) {
        let (given, _tail) = rest.split_once('/').unwrap_or((rest, ""));
        let want = match state.access.key() {
            Ok(k) => k,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "access key unavailable").into_response(),
        };
        use subtle::ConstantTimeEq;
        if given.as_bytes().ct_eq(want.as_bytes()).unwrap_u8() != 1 {
            return (StatusCode::UNAUTHORIZED, "invalid access key").into_response();
        }
        // Actual routing to the un-prefixed handler is done by axum's own
        // `nest("/k/:key", ...)` matching in `build_router`; this layer only
        // needs to authorize the request and record the prefix for
        // handlers that build URLs (playlist generation, auth state).
        req.extensions_mut()
            .insert(KeyPrefix(format!("{}{given}", crate::access::KEY_PREFIX)));
        return next.run(req).await;
    }

    if crate::access::is_open(&path) {
        return next.run(req).await;
    }

    if state.config.disable_auth {
        return next.run(req).await;
    }

    let cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if crate::api::has_session(&state, cookie.as_deref()) {
        return next.run(req).await;
    }

    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{access::Access, config::Config, secureurl::SecureUrl, store::Store, television::Television};
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
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
        })
    }

    /// Regression test for a real bug found while smoke-testing: axum's
    /// `Router::layer()` middleware runs *after* route matching, so a
    /// `/k/<key>/...` request rewritten by mutating `req.uri()` inside
    /// `middleware::from_fn` never actually re-routes — the router had
    /// already decided (and failed to find) a match beforehand. Routing the
    /// prefix through `nest("/k/:key", content_routes(...))` instead (see
    /// `build_router`) makes axum's own matcher do the work.  This checks
    /// the key-gated path without touching the network: an invalid key must
    /// still be rejected before the handler (which would call the real
    /// JioTV API) ever runs.
    #[tokio::test]
    async fn keyed_content_route_rejects_wrong_key_before_calling_out() {
        let state = test_state();
        let router = build_router(state);
        let req = Request::builder()
            .uri("/k/0000000000000000000000000000000000/channels")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn keyed_and_bare_playlist_routes_both_resolve_past_the_gate() {
        let state = test_state();
        let key = state.access.key().unwrap();
        let router = build_router(state.clone());

        // The right key reaches the handler (a redirect, not a 404/401).
        let req = Request::builder()
            .uri(format!("/k/{key}/playlist.m3u"))
            .body(Body::empty())
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);

        // The bare path with no key and no session is rejected by the gate.
        let req = Request::builder().uri("/playlist.m3u").body(Body::empty()).unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
