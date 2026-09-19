//! HTTP server wiring: the access gate, the JSON API, playlist/channels,
//! logo proxy and (full build) the Svelte UI's static assets. Mirrors
//! `cmd/jiotv_go.go` / `cmd/ui.go` / `internal/access/access.go`.
//!
//! The gate strips a `/k/<key>` prefix by hand, as a plain `tower::Service`
//! wrapped *outside* the axum `Router`, rather than as `Router::layer()`
//! middleware or a `Router::nest("/k/:key", ...)`. Both of those were tried
//! and both are wrong for this: `.layer()` middleware runs after axum's own
//! route matching, so mutating the request's URI there never changes which
//! handler gets picked (see the git history for the first, discarded fix).
//! `.nest("/k/:key", ...)` *does* route to the right handler, but it does so
//! by adding `key` as an extra captured path parameter on every matched
//! route — which silently breaks any handler using a positional extractor
//! (`Path<String>`, `Path<(String, String)>`), since axum requires an exact
//! parameter count for those and now sees one more than the handler
//! declared. That shipped, undetected by tests that only ever exercised
//! `/playlist.m3u` (no path parameters), and broke every IPTV stream route
//! in production. Stripping the prefix before the request ever reaches the
//! `Router::call` that does matching avoids both problems: the router only
//! ever sees a request shaped exactly like the unkeyed one.

use crate::api::KeyPrefix;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::Service;

fn content_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/playlist.m3u", get(playlist_redirect))
        .route("/channels", get(channels_or_playlist))
        .route("/jtvimage/:file", get(jtvimage))
        .route("/jtvposter/:date/:file", get(crate::epg::poster_handler))
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
        .route("/api/ott/search", get(crate::vod::api_ott_search))
        .route("/api/ott/screen/:id", get(crate::vod::api_ott_screen))
        .route("/api/ott/show/:id", get(crate::vod::api_ott_episodes))
        .route("/api/ott/play/:id", get(crate::vod::api_ott_play))
        .route("/api/ott/license/:id", post(crate::vod::ott_license))
        .with_state(state)
}

/// On-demand playback: manifests/segments come straight from the
/// providers' own CDNs, but the playlist, the stream redirect and the
/// license proxy are ours. Kept behind the key/session gate like the live
/// routes above (`content_routes`).
fn vod_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/vod.m3u", get(crate::vod::vod_playlist_handler))
        .route("/vod/:id", get(crate::vod::vod_stream_handler))
        .route("/vod/license/:id", post(crate::vod::ott_license))
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

/// Builds the plain (un-gated) router: every route at its bare path, no
/// `/k/:key` involved at all. `GatedService` strips the prefix before a
/// request ever reaches this router's own matching.
fn build_router(state: Arc<AppState>) -> Router {
    let mut router = Router::new()
        .merge(content_routes(state.clone()))
        .merge(api_routes(state.clone()))
        .merge(vod_routes(state.clone()))
        .merge(open_stream_routes(state.clone()));

    #[cfg(feature = "full")]
    {
        router = router.fallback(crate::ui_assets::serve_ui);
    }
    #[cfg(not(feature = "full"))]
    {
        router = router.fallback(|| async { (StatusCode::NOT_FOUND, "not found") });
    }

    router.with_state(state)
}

type BoxRespFuture = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

/// The whole app as one `tower::Service`: the access gate wrapping the
/// plain router from the outside, so prefix-stripping happens before the
/// router ever sees the request.
#[derive(Clone)]
pub struct GatedService {
    router: Router,
    state: Arc<AppState>,
}

impl GatedService {
    pub fn new(state: Arc<AppState>) -> GatedService {
        GatedService {
            router: build_router(state.clone()),
            state,
        }
    }
}

impl Service<Request> for GatedService {
    type Response = Response;
    type Error = Infallible;
    type Future = BoxRespFuture;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let mut router = self.router.clone();
        let state = self.state.clone();
        Box::pin(async move { Ok(route_request(state, &mut router, req).await) })
    }
}

/// A `MakeService` that inserts a real `ConnectInfo<SocketAddr>` per
/// connection, without going through `Router::into_make_service_with_
/// connect_info` (not usable here since `GatedService` isn't a `Router`;
/// its constructor is private to axum). Mirrors that helper's own logic,
/// which is public in spirit — `axum::extract::connect_info::Connected` and
/// `axum::serve::IncomingStream::remote_addr` are both public API.
#[derive(Clone)]
pub struct WithConnectInfo<S> {
    inner: S,
}

impl<S> WithConnectInfo<S> {
    pub fn new(inner: S) -> WithConnectInfo<S> {
        WithConnectInfo { inner }
    }
}

impl<'a, S> Service<axum::serve::IncomingStream<'a>> for WithConnectInfo<S>
where
    S: Clone + Send + 'static,
{
    type Response = ConnectInfoService<S>;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Self::Response, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, target: axum::serve::IncomingStream<'a>) -> Self::Future {
        let addr = target.remote_addr();
        std::future::ready(Ok(ConnectInfoService { inner: self.inner.clone(), addr }))
    }
}

#[derive(Clone)]
pub struct ConnectInfoService<S> {
    inner: S,
    addr: std::net::SocketAddr,
}

impl<S> Service<Request> for ConnectInfoService<S>
where
    S: Service<Request, Response = Response, Error = Infallible>,
{
    type Response = Response;
    type Error = Infallible;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request) -> Self::Future {
        req.extensions_mut().insert(axum::extract::ConnectInfo(self.addr));
        self.inner.call(req)
    }
}

/// The access gate: `/k/<key>/...` needs a matching key, some paths are
/// always open, and everything else (the admin UI's own API surface) needs a
/// valid session cookie once a password is set. See
/// `internal/access/access.go` in the Go tree; `crate::access::OPEN_PATHS`
/// mirrors its `openPaths` list.
async fn route_request(state: Arc<AppState>, router: &mut Router, mut req: Request) -> Response {
    let path = req.uri().path().to_string();

    if let Some(rest) = path.strip_prefix(crate::access::KEY_PREFIX) {
        let (given, tail) = rest.split_once('/').unwrap_or((rest, ""));
        let want = match state.access.key() {
            Ok(k) => k,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "access key unavailable").into_response(),
        };
        use subtle::ConstantTimeEq;
        if given.as_bytes().ct_eq(want.as_bytes()).unwrap_u8() != 1 {
            return (StatusCode::UNAUTHORIZED, "invalid access key").into_response();
        }

        // Strip the /k/<key> prefix from the URI *before* handing the
        // request to the router, so its own matching (and every handler's
        // Path extractor) sees exactly the same shape it would for an
        // unkeyed request.
        let new_path = format!("/{tail}");
        let mut parts = req.uri().clone().into_parts();
        let path_and_query = match req.uri().query() {
            Some(q) => format!("{new_path}?{q}"),
            None => new_path,
        };
        parts.path_and_query = Some(path_and_query.parse().expect("valid path+query"));
        *req.uri_mut() = axum::http::Uri::from_parts(parts).expect("valid uri");
        req.extensions_mut()
            .insert(KeyPrefix(format!("{}{given}", crate::access::KEY_PREFIX)));

        return call_router(router, req).await;
    }

    if crate::access::is_open(&path) || state.config.disable_auth {
        return call_router(router, req).await;
    }

    let cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if crate::api::has_session(&state, cookie.as_deref()) {
        return call_router(router, req).await;
    }

    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

async fn call_router(router: &mut Router, req: Request) -> Response {
    match router.call(req).await {
        Ok(resp) => resp,
        Err(never) => match never {},
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{access::Access, config::Config, secureurl::SecureUrl, store::Store, television::Television};
    use tower::ServiceExt;

    /// A client that can never reach the real internet: every one of Jio's
    /// hostnames used anywhere in this crate resolves to a closed local
    /// port, so any handler that gets far enough to attempt an upstream
    /// call fails instantly with a connection error instead of making a
    /// live request. Used to prove a handler's `Path`/`Query` extraction
    /// succeeded (it ran at all) without the test ever touching the real
    /// JioTV/JioTV+ APIs, per the project's rule against live calls in tests.
    fn blackholed_client() -> reqwest::Client {
        let sink: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        let hosts = [
            "jiotvapi.media.jio.com",
            "jiotvapi.cdn.jio.com",
            "jiotv.data.cdn.jio.com",
            "jiotv.catchup.cdn.jio.com",
            "auth.media.jio.com",
            "tv.media.jio.com",
            "content-jiotvplus.media.jio.com",
            "api-jiotvplus.media.jio.com",
            "jiotvapi.media.jio.com",
        ];
        let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_millis(500));
        for h in hosts {
            builder = builder.resolve(h, sink);
        }
        builder.build().unwrap()
    }

    fn test_state() -> Arc<AppState> {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().to_str().unwrap()).unwrap());
        std::mem::forget(dir);
        let http = blackholed_client();
        Arc::new(AppState {
            config: Config::default(),
            path_prefix: String::new(),
            access: Arc::new(Access::new(store.clone())),
            store,
            tv: Arc::new(Television::with_device_id(http.clone(), "test-device".to_string())),
            secure: Arc::new(SecureUrl::new(false)),
            http,
            drm_channels: Default::default(),
            custom_channels: Arc::new(crate::custom_channels::CustomChannels::new()),
            render_caches: Default::default(),
            dash_state: Default::default(),
            tvplus: Arc::new(crate::tvplus_state::TvPlusState::new(false)),
            vod_state: Default::default(),
        })
    }

    async fn send(state: Arc<AppState>, uri: &str) -> Response {
        let mut svc = GatedService::new(state);
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        svc.ready().await.unwrap();
        svc.call(req).await.unwrap()
    }

    async fn body_text(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        String::from_utf8_lossy(&bytes).to_string()
    }

    /// Never again: a request rewritten to strip `/k/<key>` must reach the
    /// exact same handler, with the exact same extracted path parameters,
    /// as the equivalent unkeyed request — this is the bug the lead's live
    /// check found (`nest("/k/:key", ...)` added an extra captured
    /// parameter, breaking every `Path<String>`/`Path<(String,String)>`
    /// handler under the prefix with a 500 "Wrong number of path arguments").
    const PATH_EXTRACTION_FAILURE: &str = "Wrong number of path arguments";

    async fn assert_reaches_handler(state: Arc<AppState>, key: &str, session_cookie: Option<&str>, path: &str) {
        // Keyed form.
        let resp = send(state.clone(), &format!("/k/{key}{path}")).await;
        let status = resp.status();
        let body = body_text(resp).await;
        assert!(
            !body.contains(PATH_EXTRACTION_FAILURE),
            "keyed {path} hit the path-extraction bug: {body}"
        );
        assert_ne!(status, StatusCode::NOT_FOUND, "keyed {path} didn't match any route");

        // Unkeyed form, with an admin session standing in for the browser UI.
        let mut svc = GatedService::new(state.clone());
        let mut req = Request::builder().uri(path).body(Body::empty()).unwrap();
        if let Some(c) = session_cookie {
            req.headers_mut().insert(header::COOKIE, c.parse().unwrap());
        }
        svc.ready().await.unwrap();
        let resp = svc.call(req).await.unwrap();
        let status = resp.status();
        let body = body_text(resp).await;
        assert!(
            !body.contains(PATH_EXTRACTION_FAILURE),
            "unkeyed {path} hit the path-extraction bug: {body}"
        );
        assert_ne!(status, StatusCode::NOT_FOUND, "unkeyed {path} didn't match any route");
    }

    #[tokio::test]
    async fn keyed_content_route_rejects_wrong_key_before_calling_out() {
        let state = test_state();
        let resp = send(state, "/k/0000000000000000000000000000000000/channels").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn keyed_and_bare_playlist_routes_both_resolve_past_the_gate() {
        let state = test_state();
        let key = state.access.key().unwrap();

        let resp = send(state.clone(), &format!("/k/{key}/playlist.m3u")).await;
        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);

        // The bare path with no key and no session is rejected by the gate.
        let resp = send(state, "/playlist.m3u").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn every_parameterised_iptv_route_survives_the_key_prefix() {
        let state = test_state();
        let key = state.access.key().unwrap();
        state.access.set_password("hunter22hunter").unwrap();
        let session = state.access.new_session(std::time::SystemTime::now()).unwrap();
        let cookie = format!("{}={session}", crate::access::SESSION_COOKIE);

        // Custom channels short-circuit before any network call, so they
        // exercise real extraction+handler logic with zero upstream I/O.
        let custom_json = format!(
            "{{\"channels\":[{{\"id\":\"custom1\",\"name\":\"Custom\",\"url\":\"https://example.com/x.m3u8\"}}]}}"
        );
        let dir = tempfile::tempdir().unwrap();
        let custom_path = dir.path().join("custom.json");
        std::fs::write(&custom_path, custom_json).unwrap();
        state.custom_channels.load(custom_path.to_str().unwrap()).unwrap();

        let paths = [
            "/live/custom1",
            "/live/high/custom1",
            "/live/mpd/custom1",
            // Not custom-channel-shortcut routes: these run far enough to
            // attempt a real upstream call, which the blackholed client
            // turns into a fast connection error rather than a live request.
            "/live/key/999999",
            "/catchup/stream/999999?start=1700000000000&end=1700000100000",
            "/epg/999999/0",
            "/jtvimage/does-not-exist.png",
            "/jtvposter/2024-01-01/does-not-exist.png",
            // TV+ is off in this test state, so these resolve at
            // require_client()'s check, never touching the network either.
            "/vod/999999",
            "/api/ott/screen/1",
            "/api/ott/show/999999",
        ];
        for path in paths {
            assert_reaches_handler(state.clone(), &key, Some(&cookie), path).await;
        }
    }

    #[tokio::test]
    async fn render_dash_survives_the_key_prefix_against_a_local_mock() {
        use wiremock::matchers::path as wm_path;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;
        Mock::given(wm_path("/seg/init.mp4"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
            .mount(&mock)
            .await;
        let mock_addr = mock.uri().trim_start_matches("http://").to_string();

        let state = test_state();
        let key = state.access.key().unwrap();
        let enc_host = state.secure.encrypt_deterministic(&mock_addr);
        let enc_path = state.secure.encrypt_deterministic("/seg/");
        let path = format!("/render.dash/host/{enc_host}/path/{enc_path}/init.mp4");

        // /render.dash is an open path (no key needed), but it's still a
        // parameterised route, so confirm it also works fine reached
        // through the /k/<key> prefix, matching how a real client always
        // reaches it via a URL this server itself generated (from /render.mpd).
        // The handler always proxies over https, and wiremock only serves
        // plain http, so this can't reach 200 end to end here; a 502 (a
        // failed *upstream connection*, from inside the handler) still
        // proves the host/path decryption and extraction both ran, which is
        // what this test is actually checking.
        let resp = send(state, &format!("/k/{key}{path}")).await;
        let status = resp.status();
        let body = body_text(resp).await;
        assert!(!body.contains(PATH_EXTRACTION_FAILURE));
        assert_eq!(status, StatusCode::BAD_GATEWAY, "expected a failed https connection to the plain-http mock: {body}");
    }

    #[tokio::test]
    async fn ott_play_route_reaches_its_handler_through_the_key_prefix() {
        let state = test_state();
        let key = state.access.key().unwrap();
        // TV+ is off in the test state, so this can't get past "connect
        // JioTV+ in Settings" — but that response, not a routing 404 or the
        // path-extraction bug, is exactly what proves /api/ott/play/:id was
        // reached through the /k/<key> prefix.
        let resp = send(state, &format!("/k/{key}/api/ott/play/999")).await;
        let status = resp.status();
        let body = body_text(resp).await;
        assert!(!body.contains(PATH_EXTRACTION_FAILURE));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
