use crate::access::Access;
use crate::config::Config;
use crate::custom_channels::CustomChannels;
use crate::dash::DashState;
use crate::secureurl::SecureUrl;
use crate::store::Store;
use crate::stream::RenderCaches;
use crate::television::Television;
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
}

impl AppState {
    pub fn is_drm_channel(&self, id: &str) -> bool {
        crate::drm_channels::is_drm_channel(id) || self.drm_channels.read().unwrap().contains(id)
    }

    pub fn is_playable(&self, id: &str) -> bool {
        // Without TV+ wired in, a channel is playable when a JioTV login is
        // present, or when it's a locally-defined custom channel (TV+
        // mirrors are a documented gap; see the final report).
        self.tv.logged_in() || self.custom_channels.contains(id)
    }
}
