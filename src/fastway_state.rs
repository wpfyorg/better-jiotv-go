//! Fastway routing and caching layer: which channels play through Fastway,
//! the catalogue/EPG/live caches, and the PallyCon token cache.
//!
//! Fastway is a separate provider from JioTV and the extras source: its
//! channels are always `fw_<id>` and only exist while `fastway` is enabled
//! and a login has been completed. Every function here is a cheap no-op when
//! that is not the case.

use crate::fastway::{Catalogue, Client, Credentials, Device, FastwayChannel, Programme};
use crate::keyed_locks::KeyedLocks;
use crate::television::{Bitrates, Channel, LiveUrlOutput, Mpd};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const CATALOGUE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const EPG_TTL: Duration = Duration::from_secs(10 * 60);
const LIVE_TTL: Duration = Duration::from_secs(60);
const LICENSE_TTL: Duration = Duration::from_secs(5 * 60);

/// Cached PallyCon token plus the client-meta header that must accompany it.
#[derive(Debug, Clone)]
struct LicenseToken {
    token: String,
    meta: String,
}

pub struct FastwayState {
    config_enabled: bool,
    client: RwLock<Option<Arc<Client>>>,
    catalogue: RwLock<Catalogue>,
    catalogue_fetched_at: RwLock<Option<Instant>>,
    epg: RwLock<HashMap<i64, Vec<Programme>>>,
    epg_fetched_at: RwLock<Option<Instant>>,
    live: RwLock<HashMap<String, (LiveUrlOutput, Instant)>>,
    licenses: RwLock<HashMap<String, (LicenseToken, Instant)>>,
    cdn_hosts: RwLock<HashSet<String>>,
    locks: KeyedLocks,
    /// Bumped, under `commit`, whenever the account-scoped caches are cleared.
    /// A fetch captures it first and commits its result only if it is
    /// unchanged, so a request that outlived a login/logout cannot write the
    /// previous account's catalogue or stream URLs back into the cleared
    /// caches.
    generation: std::sync::atomic::AtomicU64,
    commit: Mutex<()>,
}

impl FastwayState {
    pub fn new(config_enabled: bool) -> FastwayState {
        FastwayState {
            config_enabled,
            client: RwLock::new(None),
            catalogue: RwLock::new(Catalogue::default()),
            catalogue_fetched_at: RwLock::new(None),
            epg: RwLock::new(HashMap::new()),
            epg_fetched_at: RwLock::new(None),
            live: RwLock::new(HashMap::new()),
            licenses: RwLock::new(HashMap::new()),
            cdn_hosts: RwLock::new(HashSet::new()),
            locks: KeyedLocks::default(),
            generation: std::sync::atomic::AtomicU64::new(0),
            commit: Mutex::new(()),
        }
    }

    fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Runs `commit` only if no clear happened since `generation` was read.
    /// The check and the writes share the lock `clear_catalogue_state` holds
    /// while it bumps the generation, so they cannot interleave.
    fn commit_if_current<R>(&self, generation: u64, commit: impl FnOnce() -> R) -> Option<R> {
        let _guard = self.commit.lock().unwrap();
        (self.generation() == generation).then(commit)
    }

    fn clear_catalogue_state(&self) {
        let _guard = self.commit.lock().unwrap();
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *self.catalogue.write().unwrap() = Catalogue::default();
        *self.catalogue_fetched_at.write().unwrap() = None;
        self.epg.write().unwrap().clear();
        *self.epg_fetched_at.write().unwrap() = None;
        self.live.write().unwrap().clear();
        self.licenses.write().unwrap().clear();
        self.cdn_hosts.write().unwrap().clear();
    }

    /// Loads (or creates) the device and any saved login from the store.
    /// Safe to call again after login/logout.
    pub fn init(&self, http: &reqwest::Client, store: &crate::store::Store) {
        if !self.config_enabled {
            *self.client.write().unwrap() = None;
            self.clear_catalogue_state();
            return;
        }
        let need_new_client = self.client.read().unwrap().is_none();
        if need_new_client {
            match Device::load_or_create(store) {
                Ok(device) => {
                    *self.client.write().unwrap() =
                        Some(Arc::new(Client::new(http.clone(), device)))
                }
                Err(e) => {
                    tracing::warn!("fastway: cannot load device: {e}");
                    return;
                }
            }
        }
        let creds = Credentials::load(store);
        if let Some(client) = self.client.read().unwrap().as_ref() {
            client.set_credentials(creds.clone());
        }
        self.clear_catalogue_state();
        if creds.is_some() {
            tracing::info!("fastway login loaded");
        }
    }

    pub fn is_fastway_channel(channel_id: &str) -> bool {
        channel_id.starts_with(crate::fastway::ID_PREFIX)
    }

    fn client(&self) -> Option<Arc<Client>> {
        self.client.read().unwrap().clone()
    }

    pub fn enabled(&self) -> bool {
        self.client().is_some()
    }

    pub fn connected(&self) -> bool {
        match self.client() {
            Some(c) => c
                .credentials()
                .map(|cr| !cr.access_token.is_empty())
                .unwrap_or(false),
            None => false,
        }
    }

    /// The provider's numeric ID for a `fw_` channel, if Fastway is active.
    pub fn route(&self, channel_id: &str) -> Option<String> {
        if !self.enabled() {
            return None;
        }
        crate::fastway::content_id(channel_id).map(str::to_string)
    }

    /// Stable, non-secret digest used only to bind cached EPG artifacts to the
    /// active Fastway account.
    pub fn account_fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"fastway-account-v1\0");
        if let Some(client) = self.client() {
            if let Some(creds) = client.credentials() {
                hasher.update(creds.username.as_bytes());
            } else {
                hasher.update(b"anonymous");
            }
        } else {
            hasher.update(b"disabled");
        }
        hex::encode(hasher.finalize())
    }

    /// Re-logs in once with the saved credentials after the provider rejects
    /// the access token. Fastway has no refresh endpoint; the password is kept
    /// locally for exactly this case.
    async fn reauthenticate(&self, store: &crate::store::Store) -> anyhow::Result<()> {
        let client = self
            .client()
            .ok_or_else(|| anyhow::anyhow!("fastway is not enabled"))?;
        let _guard = self.locks.lock("refresh").await;
        let cr = client
            .credentials()
            .ok_or_else(|| anyhow::anyhow!("fastway is not connected"))?;
        if cr.username.is_empty() || cr.password.is_empty() {
            anyhow::bail!("fastway: the access token expired; log in again");
        }
        let new = client.login(&cr.username, &cr.password).await?;
        new.save(store)?;
        Ok(())
    }

    /// Refreshes the catalogue if stale. Must be awaited once before any of the
    /// sync lookup methods below (`route`, `is_drm`, `contains`).
    pub async fn refresh_catalogue_if_needed(&self, store: &crate::store::Store) {
        let Some(client) = self.client() else { return };
        if client
            .credentials()
            .map(|c| c.access_token.is_empty())
            .unwrap_or(true)
        {
            return;
        }
        let generation = self.generation();
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
        let fresh = self
            .catalogue_fetched_at
            .read()
            .unwrap()
            .map(|at| at.elapsed() < CATALOGUE_TTL)
            .unwrap_or(false);
        if fresh {
            return;
        }
        let mut fetched = client.catalogue().await;
        if matches!(fetched, Err(crate::fastway::FastwayError::BadToken)) {
            if let Err(e) = self.reauthenticate(store).await {
                tracing::warn!("fastway: re-login failed: {e}");
                return;
            }
            fetched = client.catalogue().await;
        }
        match fetched {
            Ok(catalogue) => {
                self.commit_if_current(generation, || {
                    *self.catalogue.write().unwrap() = catalogue;
                    *self.catalogue_fetched_at.write().unwrap() = Some(Instant::now());
                });
            }
            Err(e) => tracing::warn!("fastway: cannot fetch channels: {e}"),
        }
    }

    /// The active Fastway catalogue exposed by this server.
    pub fn catalogue_channels(&self) -> Vec<Channel> {
        self.catalogue.read().unwrap().channels_as_tv()
    }

    pub fn contains_catalogue_channel(&self, channel_id: &str) -> bool {
        self.catalogue.read().unwrap().channel(channel_id).is_some()
    }

    pub fn is_cdn_host(&self, host: &str) -> bool {
        self.cdn_hosts.read().unwrap().contains(host)
    }

    /// Whether a Fastway channel needs a Widevine license. Fastway streams are
    /// DASH either way; clear channels just have no `encryption_url`.
    pub fn is_drm(&self, channel_id: &str) -> bool {
        self.catalogue
            .read()
            .unwrap()
            .channel(channel_id)
            .map(FastwayChannel::drm)
            .unwrap_or(false)
    }

    /// Fastway catalogue rows are always DASH manifests.
    pub fn is_dash(&self, channel_id: &str) -> bool {
        self.catalogue.read().unwrap().channel(channel_id).is_some()
    }

    fn channel_snapshot(&self, content_id: &str) -> Option<FastwayChannel> {
        self.catalogue.read().unwrap().channel(content_id).cloned()
    }

    /// Fetches (or returns cached) playback for a Fastway channel. Fastway
    /// returns the manifest URL in the catalogue itself, so this mostly maps it
    /// into the shared `LiveUrlOutput` shape.
    pub async fn live(&self, content_id: &str) -> anyhow::Result<LiveUrlOutput> {
        let channel = self
            .channel_snapshot(content_id)
            .ok_or_else(|| anyhow::anyhow!("unknown fastway channel {content_id}"))?;
        let generation = self.generation();
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

        let stream_url = channel.stream_url.trim().to_string();
        if stream_url.is_empty() {
            anyhow::bail!("fastway channel {content_id} has no stream URL");
        }
        let token_url = channel.token_issue_url(crate::fastway::API_BASE);
        // Fastway's catalogue is DASH only. Leave `result`/`bitrates` empty so
        // the HLS selector does not mistake the MPD URL for an HLS source;
        // `build_drm_mpd_output` reads the MPD fields below.
        let mut result = LiveUrlOutput {
            mpd: Mpd {
                result: stream_url.clone(),
                key: token_url.clone(),
                bitrates: Bitrates {
                    auto: stream_url.clone(),
                    ..Default::default()
                },
                ..Default::default()
            },
            is_drm: channel.drm(),
            key_url: token_url,
            ..Default::default()
        };
        crate::television::finish_live_result(&mut result);

        let stored = self.commit_if_current(generation, || {
            if let Ok(u) = url::Url::parse(&stream_url) {
                if let Some(host) = u.host_str() {
                    self.cdn_hosts.write().unwrap().insert(host.to_string());
                }
            }
            self.live
                .write()
                .unwrap()
                .insert(content_id.to_string(), (result.clone(), Instant::now()));
        });
        if stored.is_none() {
            anyhow::bail!("the active account changed while resolving {content_id}; retry");
        }
        Ok(result)
    }

    /// The PallyCon token and client-meta header for a channel, cached briefly.
    pub async fn license_token(
        &self,
        content_id: &str,
        store: &crate::store::Store,
    ) -> anyhow::Result<(String, String)> {
        let client = self
            .client()
            .ok_or_else(|| anyhow::anyhow!("fastway is not enabled"))?;
        let channel = self
            .channel_snapshot(content_id)
            .ok_or_else(|| anyhow::anyhow!("unknown fastway channel {content_id}"))?;
        let drm_content_id = channel.encryption_url.trim().to_string();
        if drm_content_id.is_empty() {
            anyhow::bail!("fastway channel {content_id} has no DRM content id");
        }
        if let Some((license, at)) = self.licenses.read().unwrap().get(content_id).cloned() {
            if at.elapsed() < LICENSE_TTL {
                return Ok((license.token, license.meta));
            }
        }
        let _guard = self.locks.lock(&format!("license_{content_id}")).await;
        if let Some((license, at)) = self.licenses.read().unwrap().get(content_id).cloned() {
            if at.elapsed() < LICENSE_TTL {
                return Ok((license.token, license.meta));
            }
        }
        let mut issued = client.token_issue(&drm_content_id).await;
        if matches!(issued, Err(crate::fastway::FastwayError::BadToken)) {
            self.reauthenticate(store).await?;
            issued = client.token_issue(&drm_content_id).await;
        }
        let token = issued?;
        let meta = client.client_meta();
        let license = LicenseToken {
            token: token.clone(),
            meta: meta.clone(),
        };
        self.licenses
            .write()
            .unwrap()
            .insert(content_id.to_string(), (license, Instant::now()));
        Ok((token, meta))
    }

    pub fn license_url(&self) -> Option<String> {
        self.client().map(|c| c.license_url())
    }

    /// Channels and their programmes for XMLTV generation. The provider's EPG
    /// endpoint takes channel numbers, not IDs, so the catalogue's channel
    /// number is the lookup key.
    pub async fn epg_source(
        &self,
        store: &crate::store::Store,
    ) -> anyhow::Result<(Vec<(String, String)>, Vec<crate::epg::XmlProgramme>)> {
        self.refresh_catalogue_if_needed(store).await;
        let catalogue = self.catalogue.read().unwrap().clone();
        let numbers: Vec<i64> = catalogue
            .channels
            .iter()
            .map(|ch| ch.channel_number)
            .filter(|n| *n > 0)
            .collect();
        let guide = self.epg_for_numbers(&numbers, store).await?;

        let mut xml_channels = Vec::new();
        let mut programmes = Vec::new();
        for ch in &catalogue.channels {
            xml_channels.push((crate::fastway::channel_id(ch.id), ch.title.clone()));
            if let Some(progs) = guide.get(&ch.channel_number) {
                let category = crate::television::category_name(ch.category);
                programmes.extend(crate::fastway::to_xmltv(ch, category, progs));
            }
        }
        Ok((xml_channels, programmes))
    }

    async fn epg_for_numbers(
        &self,
        numbers: &[i64],
        store: &crate::store::Store,
    ) -> anyhow::Result<HashMap<i64, Vec<Programme>>> {
        if numbers.is_empty() {
            return Ok(HashMap::new());
        }
        let client = self
            .client()
            .ok_or_else(|| anyhow::anyhow!("fastway is not enabled"))?;
        let generation = self.generation();
        let fresh = self
            .epg_fetched_at
            .read()
            .unwrap()
            .map(|at| at.elapsed() < EPG_TTL)
            .unwrap_or(false);
        if fresh {
            return Ok(self.epg.read().unwrap().clone());
        }
        let _guard = self.locks.lock("epg").await;
        let fresh = self
            .epg_fetched_at
            .read()
            .unwrap()
            .map(|at| at.elapsed() < EPG_TTL)
            .unwrap_or(false);
        if fresh {
            return Ok(self.epg.read().unwrap().clone());
        }
        let mut fetched = client.epg(numbers).await;
        if matches!(fetched, Err(crate::fastway::FastwayError::BadToken)) {
            self.reauthenticate(store).await?;
            fetched = client.epg(numbers).await;
        }
        let fetched = fetched?;
        self.commit_if_current(generation, || {
            *self.epg.write().unwrap() = fetched.clone();
            *self.epg_fetched_at.write().unwrap() = Some(Instant::now());
        });
        Ok(fetched)
    }

    /// One channel's guide, optionally limited to an IST day offset. Falls back
    /// to the whole response when the provider's entries do not line up with
    /// the requested day.
    pub async fn web_epg(
        &self,
        content_id: &str,
        offset: i64,
        store: &crate::store::Store,
    ) -> anyhow::Result<Vec<Programme>> {
        let channel = self
            .channel_snapshot(content_id)
            .ok_or_else(|| anyhow::anyhow!("unknown fastway channel {content_id}"))?;
        let guide = self
            .epg_for_numbers(&[channel.channel_number], store)
            .await?;
        let all = guide
            .get(&channel.channel_number)
            .cloned()
            .unwrap_or_default();
        if offset <= 0 || all.is_empty() {
            return Ok(all);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let day = (now + crate::fastway::IST_OFFSET_SECS) / 86400 + offset;
        let filtered: Vec<_> = all
            .iter()
            .filter(|p| (p.start_epoch + crate::fastway::IST_OFFSET_SECS) / 86400 == day)
            .cloned()
            .collect();
        if filtered.is_empty() {
            Ok(all)
        } else {
            Ok(filtered)
        }
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
        store: &crate::store::Store,
    ) -> anyhow::Result<()> {
        let client = self
            .client()
            .ok_or_else(|| anyhow::anyhow!("fastway is not enabled"))?;
        let creds = client.login(username, password).await?;
        creds.save(store)?;
        self.clear_catalogue_state();
        Ok(())
    }

    pub async fn logout(&self, store: &crate::store::Store) -> anyhow::Result<()> {
        if let Some(client) = self.client() {
            let _ = client.logout().await;
        }
        crate::fastway::delete_credentials(store)?;
        if let Some(client) = self.client() {
            client.set_credentials(None);
        }
        self.clear_catalogue_state();
        Ok(())
    }
}

#[cfg(test)]
impl FastwayState {
    pub fn set_endpoints_for_test(&self, endpoints: crate::fastway::Endpoints) {
        self.client()
            .expect("test fastway client must be initialized")
            .set_endpoints(endpoints);
    }

    pub fn prime_for_test(&self, channels: Vec<FastwayChannel>) {
        let subscribed: HashSet<i64> = channels.iter().map(|ch| ch.id).collect();
        let catalogue = Catalogue::build(channels, &subscribed, &HashMap::new(), &HashMap::new());
        *self.catalogue.write().unwrap() = catalogue;
        *self.catalogue_fetched_at.write().unwrap() = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_state_reports_not_connected() {
        let s = FastwayState::new(false);
        assert!(!s.enabled());
        assert!(!s.connected());
        assert_eq!(s.route("fw_1"), None);
        assert!(!s.is_dash("fw_1"));
        assert!(!s.is_drm("fw_1"));
    }

    #[test]
    fn enabled_but_logged_out_reports_not_connected() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let s = FastwayState::new(true);
        s.init(&reqwest::Client::new(), &store);
        assert!(s.enabled());
        assert!(!s.connected());
        assert_eq!(s.route("fw_1"), Some("1".to_string()));
    }

    #[test]
    fn commits_apply_only_in_the_generation_they_started_in() {
        let s = FastwayState::new(false);
        let started = s.generation();
        assert_eq!(s.commit_if_current(started, || 7), Some(7));
        s.clear_catalogue_state();
        assert_ne!(s.generation(), started);
        assert_eq!(
            s.commit_if_current(started, || unreachable!("stale fetch must not commit")),
            None
        );
    }

    #[tokio::test]
    async fn live_maps_the_catalogue_row_to_the_shared_playback_shape() {
        let s = FastwayState::new(true);
        let mut ch = FastwayChannel {
            id: 7,
            channel_number: 554,
            title: "ANANDPUR LIVE".into(),
            stream_url: "http://cdn.example:8081/live/index.mpd".into(),
            encryption_url: "CID_7".into(),
            channel_mode: "live".into(),
            ..Default::default()
        };
        s.prime_for_test(vec![ch.clone()]);
        let live = s.live("7").await.unwrap();
        assert_eq!(live.mpd.bitrates.auto, ch.stream_url);
        assert_eq!(
            live.key_url,
            format!(
                "{}/apiv2/pallycon/TokenIssueV3?contentId=CID_7",
                crate::fastway::API_BASE
            )
        );
        assert!(live.is_drm);
        assert!(s.is_dash("fw_7"));
        assert!(s.is_drm("fw_7"));
        assert!(s.is_cdn_host("cdn.example"));

        ch.encryption_url.clear();
        s.prime_for_test(vec![ch]);
        assert!(!s.is_drm("fw_7"));
    }

    #[test]
    fn clear_channels_still_route_as_dash() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let s = FastwayState::new(true);
        s.init(&reqwest::Client::new(), &store);
        s.prime_for_test(vec![FastwayChannel {
            id: 8,
            channel_number: 555,
            title: "CLEAR".into(),
            stream_url: "http://cdn.example/clear.mpd".into(),
            channel_mode: "live".into(),
            ..Default::default()
        }]);
        assert!(s.is_dash("fw_8"));
        assert!(!s.is_drm("fw_8"));
        assert_eq!(s.route("fw_8"), Some("8".to_string()));
    }

    #[test]
    fn account_fingerprint_changes_with_the_login() {
        let s = FastwayState::new(true);
        let before = s.account_fingerprint();
        assert!(!before.is_empty());
    }
}
