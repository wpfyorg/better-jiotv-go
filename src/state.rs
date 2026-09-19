use crate::access::Access;
use crate::config::Config;
use crate::secureurl::SecureUrl;
use crate::store::Store;
use crate::television::Television;
use std::sync::Arc;

pub struct AppState {
    pub config: Config,
    pub access: Arc<Access>,
    pub store: Arc<Store>,
    pub tv: Arc<Television>,
    pub secure: Arc<SecureUrl>,
    pub http: reqwest::Client,
    /// DRM channels this server has learned (or been told) need a DASH/mpd
    /// route rather than HLS. The Go version ships a fuller
    /// `drm_channels.go` list derived from probing; this is intentionally a
    /// runtime-only, empty-by-default set here (see the parity gaps in the
    /// final report) rather than a hardcoded incomplete list.
    pub drm_channels: std::sync::RwLock<std::collections::HashSet<String>>,
}

impl AppState {
    pub fn is_drm_channel(&self, id: &str) -> bool {
        self.drm_channels.read().unwrap().contains(id)
    }

    pub fn is_playable(&self, _id: &str) -> bool {
        // Without TV+ wired in, a channel is playable exactly when a JioTV
        // login is present (custom channels and TV+ mirrors are a
        // documented gap; see the final report).
        self.tv.logged_in()
    }
}
