use crate::access::Access;
use crate::config::Config;
use crate::custom_channels::CustomChannels;
use crate::dash::DashState;
use crate::secureurl::SecureUrl;
use crate::store::Store;
use crate::stream::RenderCaches;
use crate::television::Television;
use crate::tvplus_state::TvPlusState;
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
    /// it at runtime yet (TV+'s "learned tvplus_dash" persistence is not
    /// implemented — see the final report).
    pub drm_channels: std::sync::RwLock<std::collections::HashSet<String>>,
    pub custom_channels: Arc<CustomChannels>,
    pub render_caches: RenderCaches,
    pub dash_state: DashState,
    pub tvplus: Arc<TvPlusState>,
    pub vod_state: crate::vod::VodState,
}

impl AppState {
    /// Mirrors `isDRMChannel`: DRM state comes from the TV+ learned map when
    /// the channel routes through TV+, else the static JioTV DRM list.
    /// Callers on a hot path that needs a fresh mirrors lookup should await
    /// `tvplus.refresh_catalogue_if_needed` first (playlist/channels
    /// handlers already do).
    pub fn is_drm_channel(&self, id: &str) -> bool {
        if !self.config.drm {
            return false;
        }
        match self.tvplus.route(id, self.tv.logged_in(), self.custom_channels.contains(id)) {
            Some(content_id) => {
                let is_tvp_id = TvPlusState::is_tvplus_channel(id);
                self.tvplus
                    .is_drm(&content_id, is_tvp_id)
                    .unwrap_or_else(|| crate::drm_channels::is_drm_channel(id) || self.drm_channels.read().unwrap().contains(id))
            }
            None => crate::drm_channels::is_drm_channel(id) || self.drm_channels.read().unwrap().contains(id),
        }
    }

    /// Mirrors `channelPlayable`: false only for a JioTV channel that needs
    /// a JioTV login when there is none and TV+ (standing in for it) does
    /// not carry it.
    pub fn is_playable(&self, id: &str) -> bool {
        let is_custom = self.custom_channels.contains(id);
        if self.tv.logged_in() || !self.tvplus.connected() || is_custom {
            return true;
        }
        self.tvplus.route(id, self.tv.logged_in(), is_custom).is_some()
    }
}
