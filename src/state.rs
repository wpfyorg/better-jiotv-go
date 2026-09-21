use crate::access::Access;
use crate::config::Config;
use crate::custom_channels::CustomChannels;
use crate::dash::DashState;
use crate::secureurl::SecureUrl;
use crate::store::Store;
use crate::stream::RenderCaches;
use crate::television::Television;
use crate::extras_state::ExtrasState;
use std::sync::Arc;

pub struct AppState {
    pub config: Config,
    /// The resolved data directory (`config.path_prefix`, or `~/.jiotv_go`),
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
    pub extras: Arc<ExtrasState>,
    pub vod_state: crate::vod::VodState,
    /// The server's cached public IPv4, used to compute today's extras
    /// unlock code. See `unlock.rs`.
    pub public_ip: Arc<crate::unlock::PublicIp>,
    pub unlock_limiter: Arc<crate::unlock::AttemptLimiter>,
}

impl AppState {
    /// Mirrors `isDRMChannel`: DRM state comes from the extras learned map when
    /// the channel routes through extras, else the static JioTV DRM list.
    /// Callers on a hot path that needs a fresh mirrors lookup should await
    /// `extras.refresh_catalogue_if_needed` first (playlist/channels
    /// handlers already do).
    pub fn is_drm_channel(&self, id: &str) -> bool {
        if !self.config.drm {
            return false;
        }
        match self.extras.route(id, self.tv.logged_in(), self.custom_channels.contains(id)) {
            Some(content_id) => {
                let is_ex_id = ExtrasState::is_extras_channel(id);
                self.extras
                    .is_drm(&content_id, is_ex_id)
                    .unwrap_or_else(|| crate::drm_channels::is_drm_channel(id) || self.drm_channels.read().unwrap().contains(id))
            }
            None => crate::drm_channels::is_drm_channel(id) || self.drm_channels.read().unwrap().contains(id),
        }
    }

    /// Mirrors `channelPlayable`: false only for a JioTV channel that needs
    /// a JioTV login when there is none and extras (standing in for it) does
    /// not carry it.
    pub fn is_playable(&self, id: &str) -> bool {
        let is_custom = self.custom_channels.contains(id);
        if self.tv.logged_in() || !self.extras.connected() || is_custom {
            return true;
        }
        self.extras.route(id, self.tv.logged_in(), is_custom).is_some()
    }
}
