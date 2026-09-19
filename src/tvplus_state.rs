//! JioTV+ routing and caching layer: which channels play through TV+, the
//! playback/catalogue caches, and the learned DASH/HLS stream-type map.
//! Mirrors the `tvPlusState`/package-level functions in
//! `internal/handlers/tvplus.go`. Only active when `config.tvplus` is true
//! and a login has been completed (`tvplus login`); otherwise every
//! function here is a cheap no-op, matching the Go version's behaviour when
//! `tvPlus.client == nil`.

use crate::television::{Channel, LiveUrlOutput, Television};
use crate::tvplus::{Client, Credentials, Device, LiveChannel};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const CATALOGUE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const TOKEN_LEAD: Duration = Duration::from_secs(60 * 60);
const LIVE_TTL: Duration = Duration::from_secs(60);

use crate::keyed_locks::KeyedLocks;

#[derive(Default)]
struct PendingLogin {
    number: String,
    connections: Vec<crate::tvplus::Connection>,
    identifier: String,
}

pub struct TvPlusState {
    enabled: bool,
    client: RwLock<Option<Arc<Client>>>,
    catalogue: RwLock<Vec<LiveChannel>>,
    catalogue_fetched_at: RwLock<Option<Instant>>,
    ext_ids: RwLock<HashMap<String, String>>,
    mirrors: RwLock<HashMap<String, String>>,
    cdn_hosts: RwLock<HashSet<String>>,
    live: RwLock<HashMap<String, (LiveUrlOutput, Instant)>>,
    dash: RwLock<HashMap<String, bool>>,
    pending: Mutex<PendingLogin>,
    locks: KeyedLocks,
}

impl TvPlusState {
    pub fn new(enabled: bool) -> TvPlusState {
        TvPlusState {
            enabled,
            client: RwLock::new(None),
            catalogue: RwLock::new(Vec::new()),
            catalogue_fetched_at: RwLock::new(None),
            ext_ids: RwLock::new(HashMap::new()),
            mirrors: RwLock::new(HashMap::new()),
            cdn_hosts: RwLock::new(HashSet::new()),
            live: RwLock::new(HashMap::new()),
            dash: RwLock::new(HashMap::new()),
            pending: Mutex::new(PendingLogin::default()),
            locks: KeyedLocks::default(),
        }
    }

    /// Loads (or creates) the device and any saved login from the store.
    /// Safe to call again after login/logout, mirroring `InitTVPlus`.
    pub fn init(&self, http: &reqwest::Client, store: &crate::store::Store) {
        if !self.enabled {
            *self.client.write().unwrap() = None;
            return;
        }
        let need_new_client = self.client.read().unwrap().is_none();
        if need_new_client {
            match Device::load_or_create(store) {
                Ok(device) => *self.client.write().unwrap() = Some(Arc::new(Client::new(http.clone(), device))),
                Err(e) => {
                    tracing::warn!("JioTV+: cannot load device: {e}");
                    return;
                }
            }
        }
        let creds = Credentials::load(store);
        if let Some(client) = self.client.read().unwrap().as_ref() {
            client.set_credentials(creds.clone());
        }
        self.live.write().unwrap().clear();
        if let Some(raw) = store.get_opt(crate::tvplus::STORE_KEY_DASH) {
            if let Ok(dash) = serde_json::from_str::<HashMap<String, bool>>(&raw) {
                *self.dash.write().unwrap() = dash;
            }
        }
        if creds.is_some() {
            tracing::info!("JioTV+ login loaded");
        }
    }

    pub fn is_tvplus_channel(channel_id: &str) -> bool {
        channel_id.starts_with(crate::tvplus::ID_PREFIX)
    }

    fn client(&self) -> Option<Arc<Client>> {
        self.client.read().unwrap().clone()
    }

    /// The client, only if TV+ is enabled and logged in with a working
    /// access token — for on-demand playback, which (like live) needs a
    /// completed login. Mirrors `tvPlusClient`.
    pub fn client_for_vod(&self) -> Option<Arc<Client>> {
        let client = self.client()?;
        let cr = client.credentials()?;
        if cr.auth_token.is_empty() {
            None
        } else {
            Some(client)
        }
    }

    /// Enabled and logged in with a working access token.
    pub fn connected(&self) -> bool {
        match self.client() {
            Some(c) => c.credentials().map(|cr| !cr.auth_token.is_empty()).unwrap_or(false),
            None => false,
        }
    }

    pub fn enabled(&self) -> bool {
        self.client().is_some()
    }

    /// Refreshes the catalogue/mirrors cache if stale. Cheap no-op when TV+
    /// isn't connected. Must be awaited once before any of the sync lookup
    /// methods below (`route`, `is_drm_channel`, `channel_playable`) so they
    /// see up-to-date mirrors — mirrors the Go version calling
    /// `tvPlusCatalogue()` synchronously inline.
    pub async fn refresh_catalogue_if_needed(&self, tv: &Television) {
        let Some(client) = self.client() else { return };
        if client.credentials().map(|c| c.auth_token.is_empty()).unwrap_or(true) {
            return;
        }
        let fresh = self
            .catalogue_fetched_at
            .read()
            .unwrap()
            .map(|at| at.elapsed() < CATALOGUE_TTL)
            .unwrap_or(false);
        if fresh {
            return;
        }
        let _guard = self.locks.lock("catalogue").await;
        // Re-check inside the lock: another caller may have just refreshed it.
        let fresh = self
            .catalogue_fetched_at
            .read()
            .unwrap()
            .map(|at| at.elapsed() < CATALOGUE_TTL)
            .unwrap_or(false);
        if fresh {
            return;
        }
        match client.channels().await {
            Ok(fetched) => {
                let mirrors = match tv.channels().await {
                    Ok(jiotv) => Some(crate::tvplus::mirrors(&fetched, &jiotv.result)),
                    Err(e) => {
                        tracing::warn!("JioTV+: cannot fetch JioTV channels: {e}");
                        None
                    }
                };
                let mut ext_ids = self.ext_ids.write().unwrap();
                for ch in &fetched {
                    if !ch.ext_id.is_empty() {
                        ext_ids.insert(ch.content_id.clone(), ch.ext_id.clone());
                    }
                }
                drop(ext_ids);
                if let Some(m) = mirrors {
                    *self.mirrors.write().unwrap() = m;
                }
                *self.catalogue.write().unwrap() = fetched;
                *self.catalogue_fetched_at.write().unwrap() = Some(Instant::now());
            }
            Err(e) => tracing::warn!("JioTV+: cannot fetch channels: {e}"),
        }
    }

    pub fn catalogue(&self) -> Vec<LiveChannel> {
        self.catalogue.read().unwrap().clone()
    }

    /// The TV+ channels JioTV doesn't already carry (`tvplus.Exclusive`).
    pub fn exclusive_channels(&self, jiotv: &[Channel]) -> Vec<Channel> {
        crate::tvplus::exclusive(&self.catalogue(), jiotv)
    }

    /// Resolves a channel ID to a TV+ content ID: always for `tvp_` IDs;
    /// for a plain JioTV ID, only when there's no JioTV login, TV+ is
    /// connected, and TV+ mirrors that channel. Requires
    /// `refresh_catalogue_if_needed` to have been awaited first for the
    /// mirrors lookup to be current. Mirrors `tvPlusRoute`.
    pub fn route(&self, channel_id: &str, jiotv_logged_in: bool, is_custom_channel: bool) -> Option<String> {
        if let Some(id) = crate::tvplus::content_id::content_id(channel_id) {
            return Some(id.to_string());
        }
        if is_custom_channel || jiotv_logged_in || !self.connected() {
            return None;
        }
        self.mirrors.read().unwrap().get(channel_id).cloned()
    }

    pub fn is_cdn_host(&self, host: &str) -> bool {
        self.cdn_hosts.read().unwrap().contains(host)
    }

    pub fn player_user_agent_for(&self, host: &str) -> &'static str {
        if self.is_cdn_host(host) {
            crate::tvplus::PLAYER_USER_AGENT
        } else {
            crate::television::PLAYER_USER_AGENT
        }
    }

    /// Fetches (or returns cached) stream URLs for a TV+ content ID, with a
    /// 60s cache and per-content-ID de-duplication. Mirrors `tvPlusLive`.
    pub async fn live(&self, content_id: &str, store: &crate::store::Store) -> anyhow::Result<LiveUrlOutput> {
        let client = self.client().ok_or_else(|| anyhow::anyhow!("JioTV+ is not enabled"))?;
        if let Some((result, at)) = self.live.read().unwrap().get(content_id).cloned() {
            if at.elapsed() < LIVE_TTL {
                return Ok(result);
            }
        }
        let _guard = self.locks.lock(&format!("live_{content_id}")).await;
        if let Some((result, at)) = self.live.read().unwrap().get(content_id).cloned() {
            if at.elapsed() < LIVE_TTL {
                return Ok(result);
            }
        }

        if let Err(e) = self.ensure_token(false, store).await {
            tracing::warn!("JioTV+: token refresh failed: {e}");
        }

        let resp = match client.playback(content_id).await {
            Ok(r) => r,
            Err(crate::tvplus::TvPlusError::NotSubscribed) => {
                anyhow::bail!("channel {content_id} is not in your JioTV+ plan")
            }
            Err(e) => return Err(e.into()),
        };
        let result = resp.to_live_url_output();

        if !resp.data.ext_id.is_empty() {
            self.ext_ids.write().unwrap().insert(content_id.to_string(), resp.data.ext_id.clone());
        }
        for stream in [&result.mpd.auto, &result.result] {
            if let Ok(u) = url::Url::parse(stream) {
                if let Some(host) = u.host_str() {
                    self.cdn_hosts.write().unwrap().insert(host.to_string());
                }
            }
        }
        self.live.write().unwrap().insert(content_id.to_string(), (result.clone(), Instant::now()));

        let has_dash = has_dash(&result);
        let mut dash_map = self.dash.write().unwrap();
        let changed = dash_map.get(content_id).map(|had| *had != has_dash).unwrap_or(true);
        if changed {
            dash_map.insert(content_id.to_string(), has_dash);
            let json = serde_json::to_string(&*dash_map)?;
            drop(dash_map);
            if let Err(e) = store.set(crate::tvplus::STORE_KEY_DASH, &json) {
                tracing::warn!("JioTV+: cannot save stream types: {e}");
            }
        }

        Ok(result)
    }

    /// Refreshes the access token if it's within `TOKEN_LEAD` of expiry (or
    /// always, if `force`). Concurrent callers share one refresh. Mirrors
    /// `ensureTVPlusToken`.
    pub async fn ensure_token(&self, force: bool, store: &crate::store::Store) -> anyhow::Result<()> {
        let client = self.client().ok_or_else(|| anyhow::anyhow!("JioTV+ is not enabled"))?;
        let cr = client.credentials().ok_or_else(|| anyhow::anyhow!("JioTV+ is not connected"))?;
        if !force && !cr.needs_refresh(std::time::SystemTime::now(), TOKEN_LEAD) {
            return Ok(());
        }
        let _guard = self.locks.lock("refresh").await;
        // Re-check: another caller may have just refreshed it.
        let cr = client.credentials().ok_or_else(|| anyhow::anyhow!("JioTV+ is not connected"))?;
        if !force && !cr.needs_refresh(std::time::SystemTime::now(), TOKEN_LEAD) {
            return Ok(());
        }
        client.refresh().await?;
        if let Some(cr) = client.credentials() {
            cr.save(store)?;
        }
        Ok(())
    }

    pub fn key_headers(&self, content_id: &str) -> Vec<(String, String)> {
        let Some(client) = self.client() else { return Vec::new() };
        let ext_id = self.ext_ids.read().unwrap().get(content_id).cloned().unwrap_or_default();
        client.key_headers(&ext_id)
    }

    pub fn license_headers(&self, content_id: &str, playback_token: &str) -> Vec<(String, String)> {
        let Some(client) = self.client() else { return Vec::new() };
        client.license_headers(content_id, playback_token)
    }

    /// Mirrors `isDRMChannel`'s TV+ branch: a channel played through TV+ is
    /// DRM (DASH) when its last playback had a DASH stream; before it's been
    /// played, a `tvp_` channel is assumed DASH.
    pub fn is_drm(&self, content_id: &str, is_tvp_id: bool) -> Option<bool> {
        if let Some(had) = self.dash.read().unwrap().get(content_id) {
            return Some(*had);
        }
        if is_tvp_id {
            Some(true)
        } else {
            None
        }
    }

    /// `/live/:channelID`'s HLS path, used as the fallback redirect for a
    /// TV+ channel that turns out to have no DASH stream. Mirrors
    /// `liveHLSPath`.
    pub fn live_hls_path(channel_id: &str, quality: &str) -> String {
        if quality.is_empty() || quality == "auto" {
            format!("/live/{channel_id}.m3u8")
        } else {
            format!("/live/{quality}/{channel_id}.m3u8")
        }
    }

    // ---- OTP login flow (mirrors TVPlusSendOTPHandler/TVPlusVerifyOTPHandler) ----

    pub async fn send_otp(&self, number: &str, connection_index: Option<usize>) -> anyhow::Result<SendOtpOutcome> {
        let client = self.client().ok_or_else(|| anyhow::anyhow!("JioTV+ is not enabled"))?;
        let number = number.trim().strip_prefix("+91").unwrap_or(number.trim()).to_string();

        if let Some(i) = connection_index {
            let (pending_number, conns) = {
                let p = self.pending.lock().unwrap();
                (p.number.clone(), p.connections.clone())
            };
            if pending_number != number || i >= conns.len() {
                anyhow::bail!("Unknown connection, start again");
            }
            let resp = client.send_otp(&number, &conns[i].identifier).await?;
            self.pending.lock().unwrap().identifier = resp.identifier.clone();
            return Ok(SendOtpOutcome { sent: !resp.identifier.is_empty(), connections: Vec::new() });
        }

        let resp = client.send_otp(&number, "").await?;
        let conns = resp.connections();
        {
            let mut p = self.pending.lock().unwrap();
            p.number = number;
            p.connections = conns.clone();
            p.identifier = resp.identifier.clone();
        }
        Ok(SendOtpOutcome { sent: !resp.identifier.is_empty(), connections: conns })
    }

    pub async fn verify_otp(&self, number: &str, otp: &str, store: &crate::store::Store) -> anyhow::Result<bool> {
        let client = self.client().ok_or_else(|| anyhow::anyhow!("JioTV+ is not enabled"))?;
        let number = number.trim().strip_prefix("+91").unwrap_or(number.trim()).to_string();
        let identifier = {
            let p = self.pending.lock().unwrap();
            if p.number != number || p.identifier.is_empty() {
                anyhow::bail!("Send the OTP first");
            }
            p.identifier.clone()
        };
        let result = client.verify_otp(&number, &identifier, otp).await;
        // Save even on a failed exchange: the SSO token is valid and the
        // exchange can be retried without another OTP.
        if let Some(cr) = client.credentials() {
            let _ = cr.save(store);
        }
        match result {
            Ok(_) => {
                *self.pending.lock().unwrap() = PendingLogin::default();
                *self.catalogue_fetched_at.write().unwrap() = None;
                self.init(&reqwest::Client::new(), store);
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    /// TV+ channels and their next two days' programmes, for
    /// `epg::generate_xml`'s `extra_sources`/`extra_programmes`. Mirrors
    /// `TVPlusEPGSource`.
    pub async fn epg_source(&self, tv: &Television) -> anyhow::Result<(Vec<(String, String)>, Vec<crate::epg::XmlProgramme>)> {
        let client = self.client().ok_or_else(|| anyhow::anyhow!("JioTV+ is not enabled"))?;
        self.refresh_catalogue_if_needed(tv).await;
        let jiotv = tv.channels().await?;
        let channels = self.exclusive_channels(&jiotv.result);
        let ids: Vec<String> = channels
            .iter()
            .filter_map(|ch| crate::tvplus::content_id::content_id(&ch.id).map(str::to_string))
            .collect();
        let guide = client.epg(&ids, &[0, 1]).await?;

        let mut xml_channels = Vec::new();
        let mut programmes = Vec::new();
        for ch in &channels {
            let Some(id) = crate::tvplus::content_id::content_id(&ch.id) else { continue };
            xml_channels.push((ch.id.clone(), ch.name.clone()));
            if let Some(progs) = guide.get(id) {
                let category = crate::television::category_name(ch.category);
                programmes.extend(crate::tvplus::to_xmltv(id, category, progs));
            }
        }
        Ok((xml_channels, programmes))
    }

    pub fn logout(&self, store: &crate::store::Store) -> anyhow::Result<()> {
        crate::tvplus::delete_credentials(store)?;
        if let Some(client) = self.client() {
            client.set_credentials(None);
        }
        Ok(())
    }
}

pub struct SendOtpOutcome {
    pub sent: bool,
    pub connections: Vec<crate::tvplus::Connection>,
}

use crate::television::has_dash;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_state_reports_not_connected() {
        let s = TvPlusState::new(false);
        assert!(!s.connected());
        assert!(!s.enabled());
        assert_eq!(s.route("154", false, false), None);
        // A tvp_ id always maps to its content id even when disabled...
        assert_eq!(s.route("tvp_1", false, false), Some("1".to_string()));
    }

    #[test]
    fn route_prefers_jiotv_login_over_tvplus() {
        let s = TvPlusState::new(true);
        // Not connected (no client set up), so a plain JioTV id never routes.
        assert_eq!(s.route("154", true, false), None);
        assert_eq!(s.route("154", false, false), None);
    }

    #[test]
    fn live_hls_path_matches_quality() {
        assert_eq!(TvPlusState::live_hls_path("tvp_1", "auto"), "/live/tvp_1.m3u8");
        assert_eq!(TvPlusState::live_hls_path("tvp_1", "high"), "/live/high/tvp_1.m3u8");
    }

    #[test]
    fn is_drm_defaults_to_true_for_unplayed_tvp_channel() {
        let s = TvPlusState::new(true);
        assert_eq!(s.is_drm("5", true), Some(true));
        assert_eq!(s.is_drm("5", false), None);
    }
}
