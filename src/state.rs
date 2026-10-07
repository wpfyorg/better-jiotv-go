use crate::access::Access;
use crate::config::Config;
use crate::custom_channels::CustomChannels;
use crate::dash::DashState;
use crate::extras_state::ExtrasState;
use crate::secureurl::SecureUrl;
use crate::store::Store;
use crate::stream::RenderCaches;
use crate::television::Television;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveProduct {
    Tv,
    Extras,
}

impl ActiveProduct {
    pub fn as_str(self) -> &'static str {
        match self {
            ActiveProduct::Tv => "tv",
            ActiveProduct::Extras => "extras",
        }
    }
}

pub struct AppState {
    pub config: Config,
    /// The resolved data directory (`config.path_prefix`, or `~/.jiotv_go`; `/etc/jiotv` on OpenWrt),
    /// always ending in `/`. Used for `epg.xml.gz` and similar data files.
    pub path_prefix: String,
    pub access: Arc<Access>,
    pub store: Arc<Store>,
    pub tv: Arc<Television>,
    pub secure: Arc<SecureUrl>,
    pub http: reqwest::Client,
    /// Extra channels this server has been told (via `drm_channels.rs`'s
    /// static list plus any learned entries) need a DASH/mpd route rather
    /// than HLS. Currently just the static Go-sourced list; nothing adds to
    /// it at runtime yet (extras's "learned extras_stream_kinds" persistence is not
    /// implemented — see the final report).
    pub drm_channels: std::sync::RwLock<std::collections::HashSet<String>>,
    pub custom_channels: Arc<CustomChannels>,
    pub render_caches: RenderCaches,
    pub dash_state: DashState,
    pub epg_state: crate::epg::EpgState,
    pub extras: Arc<ExtrasState>,
    pub vod_state: crate::vod::VodState,
    /// The server's cached public IPv4, used to compute today's extras
    /// unlock code. See `unlock.rs`.
    pub public_ip: Arc<crate::unlock::PublicIp>,
    pub unlock_limiter: Arc<crate::unlock::AttemptLimiter>,
    /// The ports `serve` listens on, recorded once the listeners are chosen so
    /// the UI can offer the plain-HTTP playlist origin from an HTTPS page.
    pub listen: std::sync::OnceLock<ListenPorts>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenPorts {
    pub http: u16,
    pub tls: Option<u16>,
}

impl AppState {
    /// Returns `response` only if no account or product transition rotated the
    /// context epoch since `epoch` was captured at the start of the request.
    /// A handler that mints encrypted URLs from account-scoped data must be
    /// wrapped in this: if the epoch moved meanwhile, the response could carry
    /// the previous context's data under URLs encrypted for the new one.
    pub fn stable_since(
        &self,
        epoch: u64,
        response: axum::response::Response,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if self.secure.current_epoch() == epoch {
            response
        } else {
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "The active account changed while this request was running; retry",
            )
                .into_response()
        }
    }

    /// Process-local account/mode epoch shared by all encrypted playback
    /// artifacts. Rotating it makes previously issued manifest, segment,
    /// key and license URLs unusable immediately.
    pub fn context_epoch(&self) -> u64 {
        self.secure.current_epoch()
    }

    /// Invalidates every account-sensitive process cache at one serialized
    /// context boundary. EPG publishers use the same guard before committing,
    /// so a generation started in the old epoch cannot publish after this
    /// returns.
    pub fn invalidate_context(&self) {
        let _guard = self.epg_state.context_guard();
        self.epg_state.invalidate();
        self.secure.rotate();
        self.render_caches.clear();
        self.dash_state.clear();
        self.vod_state.clear();
    }

    /// Product selection follows the authenticated session that can actually
    /// supply the alternate catalogue. Locking or logging out of extras
    /// immediately falls back to the TV catalogue.
    pub fn active_product(&self) -> ActiveProduct {
        if self.extras.connected() {
            ActiveProduct::Extras
        } else {
            ActiveProduct::Tv
        }
    }

    /// Returns only channels from the active upstream product. Custom
    /// channels are deliberately excluded here because they are local user
    /// additions rather than part of either provider catalogue.
    pub async fn effective_upstream_channels(
        &self,
    ) -> anyhow::Result<Vec<crate::television::Channel>> {
        match self.active_product() {
            ActiveProduct::Extras => {
                self.extras.refresh_catalogue_if_needed(&self.tv).await;
                Ok(self.extras.catalogue_channels())
            }
            ActiveProduct::Tv => Ok(self.tv.channels().await?.result),
        }
    }

    /// The one catalogue exposed to UI, playlists and channel lookups.
    /// Local custom channels remain available regardless of provider mode.
    pub async fn effective_channels(&self) -> anyhow::Result<Vec<crate::television::Channel>> {
        let mut channels = self.effective_upstream_channels().await?;
        channels.extend(self.custom_channels.all());
        Ok(channels)
    }

    pub async fn channel_allowed(&self, id: &str) -> bool {
        if self.custom_channels.contains(id) {
            return true;
        }
        match self.active_product() {
            ActiveProduct::Extras => {
                self.extras.refresh_catalogue_if_needed(&self.tv).await;
                self.extras.contains_catalogue_channel(id)
            }
            ActiveProduct::Tv => self
                .tv
                .channels()
                .await
                .map(|list| list.result.iter().any(|ch| ch.id == id))
                .unwrap_or(false),
        }
    }

    /// Mirrors `isDRMChannel`: DRM state comes from the extras learned map when
    /// the channel routes through extras, else the static JioTV DRM list.
    /// Callers on a hot path that needs a fresh mirrors lookup should await
    /// `extras.refresh_catalogue_if_needed` first (playlist/channels
    /// handlers already do).
    pub fn is_drm_channel(&self, id: &str) -> bool {
        if !self.config.drm {
            return false;
        }
        match self
            .extras
            .route(id, self.tv.logged_in(), self.custom_channels.contains(id))
        {
            Some(content_id) => {
                let is_ex_id = ExtrasState::is_extras_channel(id);
                self.extras
                    .is_drm(&content_id, is_ex_id)
                    .unwrap_or_else(|| {
                        crate::drm_channels::is_drm_channel(id)
                            || self.drm_channels.read().unwrap().contains(id)
                    })
            }
            None => {
                crate::drm_channels::is_drm_channel(id)
                    || self.drm_channels.read().unwrap().contains(id)
            }
        }
    }

    /// Whether a channel from the already-filtered effective catalogue can
    /// be started with the active account.
    pub fn is_playable(&self, id: &str) -> bool {
        if self.custom_channels.contains(id) {
            return true;
        }
        match self.active_product() {
            ActiveProduct::Extras => self.extras.connected(),
            ActiveProduct::Tv => self.tv.logged_in(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

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
            custom_channels: Arc::new(CustomChannels::new()),
            render_caches: Default::default(),
            dash_state: Default::default(),
            epg_state: Default::default(),
            extras: Arc::new(ExtrasState::new(false, None)),
            vod_state: Default::default(),
            public_ip: Arc::new(crate::unlock::PublicIp::new(reqwest::Client::new())),
            unlock_limiter: Arc::new(crate::unlock::AttemptLimiter::default()),
            listen: Default::default(),
        })
    }

    #[test]
    fn stable_since_keeps_a_response_from_an_unchanged_context() {
        let s = state();
        let epoch = s.secure.current_epoch();
        let resp = s.stable_since(epoch, axum::http::StatusCode::OK.into_response());
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[test]
    fn stable_since_discards_a_response_that_straddled_a_context_switch() {
        let s = state();
        let epoch = s.secure.current_epoch();
        s.invalidate_context();
        let resp = s.stable_since(epoch, axum::http::StatusCode::OK.into_response());
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }
}
