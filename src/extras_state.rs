//! extras routing and caching layer: which channels play through extras, the
//! playback/catalogue caches, and the learned DASH/HLS stream-type map.
//! Mirrors the `extras_state`/package-level functions in
//! `internal/handlers/extras.go`. Only active when `config.extras` is true
//! and a login has been completed (`extras login`); otherwise every
//! function here is a cheap no-op, matching the Go version's behaviour when
//! `extras.client == nil`.

use crate::extras::{Client, Credentials, Device, LiveChannel};
use crate::television::{Channel, LiveUrlOutput, Television};
use sha2::{Digest, Sha256};
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
    connections: Vec<crate::extras::Connection>,
    identifier: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntitlementStatus {
    Unknown,
    Available,
    AuthenticationRequired,
    UpstreamFailure,
}

impl EntitlementStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            EntitlementStatus::Unknown => "unknown",
            EntitlementStatus::Available => "available",
            EntitlementStatus::AuthenticationRequired => "authentication_required",
            EntitlementStatus::UpstreamFailure => "upstream_failure",
        }
    }
}

pub struct ExtrasState {
    config_enabled: bool,
    override_set: std::sync::atomic::AtomicBool,
    unlocked: std::sync::atomic::AtomicBool,
    client: RwLock<Option<Arc<Client>>>,
    catalogue: RwLock<Vec<LiveChannel>>,
    catalogue_fetched_at: RwLock<Option<Instant>>,
    subscriptions: RwLock<Option<HashMap<String, bool>>>,
    entitlement_status: RwLock<EntitlementStatus>,
    entitlements_applied: std::sync::atomic::AtomicBool,
    ext_ids: RwLock<HashMap<String, String>>,
    mirrors: RwLock<HashMap<String, String>>,
    cdn_hosts: RwLock<HashSet<String>>,
    live: RwLock<HashMap<String, (LiveUrlOutput, Instant)>>,
    dash: RwLock<HashMap<String, bool>>,
    pending: Mutex<PendingLogin>,
    locks: KeyedLocks,
    /// Bumped, under `commit`, every time the account-scoped caches are
    /// cleared. A fetch captures it first and commits its result only if it is
    /// unchanged, so a request that outlived an account switch cannot write the
    /// previous account's catalogue or stream URLs back into the cleared caches.
    generation: std::sync::atomic::AtomicU64,
    commit: Mutex<()>,
}

impl ExtrasState {
    /// `config_enabled` is the `extras` config/env switch (headless/router
    /// installs). `panel_override` is the stored UI gate: `Some(true)` forces
    /// extras on, `Some(false)` explicitly locks it, and `None` follows the
    /// config switch. This lets a headless config default to on while still
    /// making the UI's Lock button authoritative once the user presses it.
    pub fn new(config_enabled: bool, panel_override: Option<bool>) -> ExtrasState {
        ExtrasState {
            config_enabled,
            override_set: std::sync::atomic::AtomicBool::new(panel_override.is_some()),
            unlocked: std::sync::atomic::AtomicBool::new(panel_override.unwrap_or(false)),
            client: RwLock::new(None),
            catalogue: RwLock::new(Vec::new()),
            catalogue_fetched_at: RwLock::new(None),
            subscriptions: RwLock::new(None),
            entitlement_status: RwLock::new(EntitlementStatus::Unknown),
            entitlements_applied: std::sync::atomic::AtomicBool::new(false),
            ext_ids: RwLock::new(HashMap::new()),
            mirrors: RwLock::new(HashMap::new()),
            cdn_hosts: RwLock::new(HashSet::new()),
            live: RwLock::new(HashMap::new()),
            dash: RwLock::new(HashMap::new()),
            pending: Mutex::new(PendingLogin::default()),
            locks: KeyedLocks::default(),
            generation: std::sync::atomic::AtomicU64::new(0),
            commit: Mutex::new(()),
        }
    }

    /// Whether extras is switched on. An explicit panel lock/unlock overrides
    /// the config/env default; without an override the config decides.
    fn gate_enabled(&self) -> bool {
        if self.override_set.load(std::sync::atomic::Ordering::Relaxed) {
            self.unlocked.load(std::sync::atomic::Ordering::Relaxed)
        } else {
            self.config_enabled
        }
    }

    /// Flips the panel-unlock gate and reloads, so the feature turns on (or
    /// off, via the Settings "Lock" button) without restarting the server.
    /// Does not touch the `extras` config/env switch; it overrides it.
    pub fn set_unlocked(&self, v: bool, http: &reqwest::Client, store: &crate::store::Store) {
        self.unlocked.store(v, std::sync::atomic::Ordering::Relaxed);
        self.override_set
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.init(http, store);
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

    fn clear_live(&self) {
        let _guard = self.commit.lock().unwrap();
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.live.write().unwrap().clear();
    }

    fn clear_catalogue_state(&self) {
        let _guard = self.commit.lock().unwrap();
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.catalogue.write().unwrap().clear();
        *self.catalogue_fetched_at.write().unwrap() = None;
        *self.subscriptions.write().unwrap() = None;
        *self.entitlement_status.write().unwrap() = EntitlementStatus::Unknown;
        self.entitlements_applied
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.ext_ids.write().unwrap().clear();
        self.mirrors.write().unwrap().clear();
        self.cdn_hosts.write().unwrap().clear();
        self.live.write().unwrap().clear();
        *self.pending.lock().unwrap() = PendingLogin::default();
    }

    pub fn invalidate_account_context(&self) {
        self.clear_catalogue_state();
    }

    /// Identifies the credentials currently installed on the client, so a
    /// caller can tell whether an operation replaced them, even partway (an OTP
    /// that verifies but fails its token exchange has already swapped the
    /// account while leaving it without an auth token).
    pub fn credentials_marker(&self) -> Option<(String, String, String)> {
        self.client()?
            .credentials()
            .map(|c| (c.number, c.sso_token, c.auth_token))
    }

    /// Loads (or creates) the device and any saved login from the store.
    /// Safe to call again after login/logout, mirroring `InitExtras`.
    pub fn init(&self, http: &reqwest::Client, store: &crate::store::Store) {
        if !self.gate_enabled() {
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
                    tracing::warn!("extras: cannot load device: {e}");
                    return;
                }
            }
        }
        let creds = Credentials::load(store);
        if let Some(client) = self.client.read().unwrap().as_ref() {
            client.set_credentials(creds.clone());
        }
        self.clear_live();
        // `extras_stream_kinds` is the current store key; `tvplus_dash` is
        // read as a fallback so an existing store's learned map survives
        // the rename, and gets migrated forward on the next save below.
        let raw = store
            .get_opt(crate::extras::STORE_KEY_DASH)
            .or_else(|| store.get_opt(crate::extras::STORE_KEY_DASH_OLD));
        if let Some(raw) = raw {
            if let Ok(dash) = serde_json::from_str::<HashMap<String, bool>>(&raw) {
                *self.dash.write().unwrap() = dash;
            }
        }
        if creds.is_some() {
            tracing::info!("extras login loaded");
        }
    }

    pub fn is_extras_channel(channel_id: &str) -> bool {
        channel_id.starts_with(crate::extras::ID_PREFIX)
    }

    fn client(&self) -> Option<Arc<Client>> {
        self.client.read().unwrap().clone()
    }

    /// The client, only if extras is enabled and logged in with a working
    /// access token — for on-demand playback, which (like live) needs a
    /// completed login. Mirrors `extrasClient`.
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
            Some(c) => c
                .credentials()
                .map(|cr| !cr.auth_token.is_empty())
                .unwrap_or(false),
            None => false,
        }
    }

    pub fn enabled(&self) -> bool {
        self.client().is_some()
    }

    /// Refreshes the catalogue/mirrors cache if stale. Cheap no-op when extras
    /// isn't connected. Must be awaited once before any of the sync lookup
    /// methods below (`route`, `is_drm_channel`, `channel_playable`) so they
    /// see up-to-date mirrors — mirrors the Go version calling
    /// `extrasCatalogue()` synchronously inline.
    pub async fn refresh_catalogue_if_needed(&self, tv: &Television) {
        let Some(client) = self.client() else { return };
        if client
            .credentials()
            .map(|c| c.auth_token.is_empty())
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
        let (subscriptions, entitlement_status) = match client.subscriptions().await {
            Ok(Some(map)) => (Some(map), EntitlementStatus::Available),
            Ok(None) => (None, EntitlementStatus::Unknown),
            Err(crate::extras::ExtrasError::NotLoggedIn) => {
                (None, EntitlementStatus::AuthenticationRequired)
            }
            Err(e) => {
                tracing::warn!("extras: cannot fetch active subscriptions: {e}");
                (None, EntitlementStatus::UpstreamFailure)
            }
        };
        let stored = self.commit_if_current(generation, || {
            *self.subscriptions.write().unwrap() = subscriptions.clone();
            *self.entitlement_status.write().unwrap() = entitlement_status;
            self.entitlements_applied
                .store(false, std::sync::atomic::Ordering::Relaxed);
        });
        if stored.is_none() {
            return;
        }
        match client.channels().await {
            Ok(mut fetched) => {
                let mut applied = false;
                if let Some(map) = subscriptions.as_ref() {
                    fetched.retain(|ch| ch.allowed_by_subscriptions(map));
                    applied = true;
                }
                let mirrors = match tv.channels().await {
                    Ok(jiotv) => Some(crate::extras::mirrors(&fetched, &jiotv.result)),
                    Err(e) => {
                        tracing::warn!("extras: cannot fetch JioTV channels: {e}");
                        None
                    }
                };
                self.commit_if_current(generation, || {
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
                    // Committed with the catalogue it describes, so an obsolete
                    // refresh cannot mark a cleared state as entitlement-filtered.
                    self.entitlements_applied
                        .store(applied, std::sync::atomic::Ordering::Relaxed);
                });
            }
            Err(e) => tracing::warn!("extras: cannot fetch channels: {e}"),
        }
    }

    /// The account-aware TV+ catalogue exposed by this server. Test/deeplink
    /// rows are omitted because they are not playable through our live routes.
    pub fn catalogue_channels(&self) -> Vec<Channel> {
        self.catalogue
            .read()
            .unwrap()
            .iter()
            .filter(|ch| !ch.is_test_channel() && ch.playback_type != "deeplink")
            .map(LiveChannel::to_channel)
            .collect()
    }

    pub fn contains_catalogue_channel(&self, channel_id: &str) -> bool {
        let Some(content_id) = crate::extras::content_id::content_id(channel_id) else {
            return false;
        };
        self.catalogue.read().unwrap().iter().any(|ch| {
            ch.content_id == content_id && !ch.is_test_channel() && ch.playback_type != "deeplink"
        })
    }

    pub fn entitlements_available(&self) -> bool {
        self.subscriptions.read().unwrap().is_some()
    }

    pub fn entitlements_applied(&self) -> bool {
        self.entitlements_applied
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn entitlement_status(&self) -> EntitlementStatus {
        if self.enabled() && !self.connected() {
            return EntitlementStatus::AuthenticationRequired;
        }
        *self.entitlement_status.read().unwrap()
    }

    /// Stable, non-secret digest used only to bind cached artifacts to the
    /// active extras account. Raw subscriber/user identifiers never leave
    /// this method.
    pub fn account_fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"extras-account-v1\0");
        if let Some(client) = self.client() {
            if let Some(creds) = client.credentials() {
                hasher.update(creds.subscriber_id.as_bytes());
                hasher.update(b"\0");
                hasher.update(creds.user_id.as_bytes());
                hasher.update(b"\0");
                hasher.update(creds.unique.as_bytes());
            } else {
                hasher.update(b"anonymous");
            }
        } else {
            hasher.update(b"disabled");
        }
        hex::encode(hasher.finalize())
    }

    /// Stable digest of whether subscription data exists and, when it does,
    /// the provider decisions that were actually retrieved. This prevents an
    /// XMLTV cache produced while filtering was unknown from being mistaken
    /// for one produced after verified filtering.
    pub fn entitlement_fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"extras-entitlements-v1\0");
        hasher.update(self.entitlement_status().as_str().as_bytes());
        hasher.update(b"\0");
        let applied: &[u8] = if self.entitlements_applied() {
            b"applied"
        } else {
            b"not-applied"
        };
        hasher.update(applied);
        let mut entries: Vec<_> = self
            .subscriptions
            .read()
            .unwrap()
            .as_ref()
            .map(|map| map.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (provider, allowed) in entries {
            hasher.update(b"\0");
            hasher.update(provider.as_bytes());
            hasher.update(if allowed { b"=1" } else { b"=0" });
        }
        hex::encode(hasher.finalize())
    }

    /// Resolves a channel ID to a extras content ID: always for `ex_` IDs;
    /// for a plain JioTV ID, only when there's no JioTV login, extras is
    /// connected, and extras mirrors that channel. Requires
    /// `refresh_catalogue_if_needed` to have been awaited first for the
    /// mirrors lookup to be current. Mirrors `extrasRoute`.
    pub fn route(
        &self,
        channel_id: &str,
        jiotv_logged_in: bool,
        is_custom_channel: bool,
    ) -> Option<String> {
        if let Some(id) = crate::extras::content_id::content_id(channel_id) {
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
            crate::extras::PLAYER_USER_AGENT
        } else {
            crate::television::PLAYER_USER_AGENT
        }
    }

    /// Fetches (or returns cached) stream URLs for a extras content ID, with a
    /// 60s cache and per-content-ID de-duplication. Mirrors `extrasLive`.
    pub async fn live(
        &self,
        content_id: &str,
        store: &crate::store::Store,
    ) -> anyhow::Result<LiveUrlOutput> {
        let client = self
            .client()
            .ok_or_else(|| anyhow::anyhow!("extras is not enabled"))?;
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

        if let Err(e) = self.ensure_token(false, store).await {
            tracing::warn!("extras: token refresh failed: {e}");
        }

        let resp = match client.playback(content_id).await {
            Ok(r) => r,
            Err(crate::extras::ExtrasError::NotSubscribed) => {
                anyhow::bail!("channel {content_id} is not in your extras plan")
            }
            Err(e) => return Err(e.into()),
        };
        let result = resp.to_live_url_output();

        // An account switch during the fetch leaves `result` belonging to the
        // previous account: do not cache it and do not hand it to the caller.
        let stored = self.commit_if_current(generation, || {
            if !resp.data.ext_id.is_empty() {
                self.ext_ids
                    .write()
                    .unwrap()
                    .insert(content_id.to_string(), resp.data.ext_id.clone());
            }
            for stream in [&result.mpd.auto, &result.result] {
                if let Ok(u) = url::Url::parse(stream) {
                    if let Some(host) = u.host_str() {
                        self.cdn_hosts.write().unwrap().insert(host.to_string());
                    }
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

        let has_dash = has_dash(&result);
        let mut dash_map = self.dash.write().unwrap();
        let changed = dash_map
            .get(content_id)
            .map(|had| *had != has_dash)
            .unwrap_or(true);
        if changed {
            dash_map.insert(content_id.to_string(), has_dash);
            let json = serde_json::to_string(&*dash_map)?;
            drop(dash_map);
            if let Err(e) = store.set(crate::extras::STORE_KEY_DASH, &json) {
                tracing::warn!("extras: cannot save stream types: {e}");
            }
        }

        Ok(result)
    }

    /// Refreshes the access token if it's within `TOKEN_LEAD` of expiry (or
    /// always, if `force`). Concurrent callers share one refresh. Mirrors
    /// `ensureExtrasToken`.
    pub async fn ensure_token(
        &self,
        force: bool,
        store: &crate::store::Store,
    ) -> anyhow::Result<()> {
        let client = self
            .client()
            .ok_or_else(|| anyhow::anyhow!("extras is not enabled"))?;
        let cr = client
            .credentials()
            .ok_or_else(|| anyhow::anyhow!("extras is not connected"))?;
        if !force && !cr.needs_refresh(std::time::SystemTime::now(), TOKEN_LEAD) {
            return Ok(());
        }
        let _guard = self.locks.lock("refresh").await;
        // Re-check: another caller may have just refreshed it.
        let cr = client
            .credentials()
            .ok_or_else(|| anyhow::anyhow!("extras is not connected"))?;
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
        let Some(client) = self.client() else {
            return Vec::new();
        };
        let ext_id = self
            .ext_ids
            .read()
            .unwrap()
            .get(content_id)
            .cloned()
            .unwrap_or_default();
        client.key_headers(&ext_id)
    }

    pub fn license_headers(&self, content_id: &str, playback_token: &str) -> Vec<(String, String)> {
        let Some(client) = self.client() else {
            return Vec::new();
        };
        client.license_headers(content_id, playback_token)
    }

    /// Mirrors `isDRMChannel`'s extras branch: a channel played through extras is
    /// DRM (DASH) when its last playback had a DASH stream; before it's been
    /// played, a `ex_` channel is assumed DASH.
    pub fn is_drm(&self, content_id: &str, is_ex_id: bool) -> Option<bool> {
        if let Some(had) = self.dash.read().unwrap().get(content_id) {
            return Some(*had);
        }
        if is_ex_id {
            Some(true)
        } else {
            None
        }
    }

    /// `/live/:channelID`'s HLS path, used as the fallback redirect for a
    /// extras channel that turns out to have no DASH stream. Mirrors
    /// `liveHLSPath`.
    pub fn live_hls_path(channel_id: &str, quality: &str) -> String {
        if quality.is_empty() || quality == "auto" {
            format!("/live/{channel_id}.m3u8")
        } else {
            format!("/live/{quality}/{channel_id}.m3u8")
        }
    }

    // ---- OTP login flow (mirrors ExtrasSendOTPHandler/ExtrasVerifyOTPHandler) ----

    pub async fn send_otp(
        &self,
        number: &str,
        connection_index: Option<usize>,
    ) -> anyhow::Result<SendOtpOutcome> {
        let client = self
            .client()
            .ok_or_else(|| anyhow::anyhow!("extras is not enabled"))?;
        let number = number
            .trim()
            .strip_prefix("+91")
            .unwrap_or(number.trim())
            .to_string();

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
            return Ok(SendOtpOutcome {
                sent: !resp.identifier.is_empty(),
                connections: Vec::new(),
            });
        }

        let resp = client.send_otp(&number, "").await?;
        let conns = resp.connections();
        {
            let mut p = self.pending.lock().unwrap();
            p.number = number;
            p.connections = conns.clone();
            p.identifier = resp.identifier.clone();
        }
        Ok(SendOtpOutcome {
            sent: !resp.identifier.is_empty(),
            connections: conns,
        })
    }

    /// `on_installed` is forwarded to the client and runs as soon as the
    /// verified credentials are installed (see `Client::verify_otp`).
    pub async fn verify_otp(
        &self,
        number: &str,
        otp: &str,
        store: &crate::store::Store,
        on_installed: impl FnOnce(),
    ) -> anyhow::Result<bool> {
        let client = self
            .client()
            .ok_or_else(|| anyhow::anyhow!("extras is not enabled"))?;
        let number = number
            .trim()
            .strip_prefix("+91")
            .unwrap_or(number.trim())
            .to_string();
        let identifier = {
            let p = self.pending.lock().unwrap();
            if p.number != number || p.identifier.is_empty() {
                anyhow::bail!("Send the OTP first");
            }
            p.identifier.clone()
        };
        let before = self.credentials_marker();
        let result = client
            .verify_otp(&number, &identifier, otp, on_installed)
            .await;
        // Save even on a failed exchange: the SSO token is valid and the
        // exchange can be retried without another OTP.
        if let Some(cr) = client.credentials() {
            let _ = cr.save(store);
        }
        match result {
            Ok(_) => {
                // A different TV+ connection/account can have a different
                // catalogue and subscription map. Drop every account-scoped
                // cache before reloading the newly persisted credentials so
                // a failed refresh cannot fall back to the previous account.
                self.clear_catalogue_state();
                self.init(&reqwest::Client::new(), store);
                Ok(true)
            }
            Err(_) => {
                // The OTP can verify and the token exchange still fail, which
                // has already replaced the account's credentials; the previous
                // account's caches must not outlive that.
                if self.credentials_marker() != before {
                    self.clear_catalogue_state();
                }
                Ok(false)
            }
        }
    }

    /// extras channels and their next two days' programmes, for
    /// `epg::generate_xml`'s `extra_sources`/`extra_programmes`. Mirrors
    /// `ExtrasEPGSource`.
    pub async fn epg_source(
        &self,
        tv: &Television,
    ) -> anyhow::Result<(Vec<(String, String)>, Vec<crate::epg::XmlProgramme>)> {
        let client = self
            .client()
            .ok_or_else(|| anyhow::anyhow!("extras is not enabled"))?;
        self.refresh_catalogue_if_needed(tv).await;
        let channels = self.catalogue_channels();
        let ids: Vec<String> = channels
            .iter()
            .filter_map(|ch| crate::extras::content_id::content_id(&ch.id).map(str::to_string))
            .collect();
        let guide = client.epg(&ids, &[0, 1]).await?;

        let mut xml_channels = Vec::new();
        let mut programmes = Vec::new();
        for ch in &channels {
            let Some(id) = crate::extras::content_id::content_id(&ch.id) else {
                continue;
            };
            xml_channels.push((ch.id.clone(), ch.name.clone()));
            if let Some(progs) = guide.get(id) {
                let category = crate::television::category_name(ch.category);
                programmes.extend(crate::extras::to_xmltv(id, category, progs));
            }
        }
        Ok((xml_channels, programmes))
    }

    pub fn logout(&self, store: &crate::store::Store) -> anyhow::Result<()> {
        crate::extras::delete_credentials(store)?;
        if let Some(client) = self.client() {
            client.set_credentials(None);
        }
        self.clear_catalogue_state();
        Ok(())
    }
}

#[cfg(test)]
impl ExtrasState {
    pub fn set_endpoints_for_test(&self, endpoints: crate::extras::Endpoints) {
        self.client()
            .expect("test extras client must be initialized")
            .set_endpoints(endpoints);
    }

    pub fn prime_for_test(
        &self,
        credentials: Credentials,
        channels: Vec<LiveChannel>,
        subscriptions: Option<HashMap<String, bool>>,
    ) {
        let client = self
            .client()
            .expect("test extras client must be initialized");
        client.set_credentials(Some(credentials));
        *self.catalogue.write().unwrap() = channels;
        *self.catalogue_fetched_at.write().unwrap() = Some(Instant::now());
        *self.subscriptions.write().unwrap() = subscriptions.clone();
        *self.entitlement_status.write().unwrap() = if subscriptions.is_some() {
            EntitlementStatus::Available
        } else {
            EntitlementStatus::Unknown
        };
        self.entitlements_applied.store(
            subscriptions.is_some(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

pub struct SendOtpOutcome {
    pub sent: bool,
    pub connections: Vec<crate::extras::Connection>,
}

use crate::television::has_dash;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn obsolete_catalogue_refresh_cannot_mark_cleared_state_as_filtered() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user/v2/subscription"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 200,
                "data": {"subscriptions": {"JioCinema-Premium": true}}
            })))
            .mount(&server)
            .await;
        // The channel fetch is slow enough for an account switch to land mid-flight.
        Mock::given(method("GET"))
            .and(path("/metadata/v2/livechannels"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": {}}))
                    .set_delay(std::time::Duration::from_millis(400)),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let s = Arc::new(ExtrasState::new(true, None));
        s.init(&reqwest::Client::new(), &store);
        s.set_endpoints_for_test(crate::extras::Endpoints {
            content: server.uri(),
            user_api: server.uri(),
            ..Default::default()
        });
        s.client().unwrap().set_credentials(Some(Credentials {
            sso_token: "redacted-sso".into(),
            subscriber_id: "redacted-sub".into(),
            user_id: "redacted-user".into(),
            auth_token: "redacted-access".into(),
            ..Default::default()
        }));
        let tv = Arc::new(Television::new(reqwest::Client::new()));
        tv.set_channels_for_test(Vec::new());

        let refresh = {
            let (s, tv) = (s.clone(), tv.clone());
            tokio::spawn(async move { s.refresh_catalogue_if_needed(&tv).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        s.invalidate_account_context();
        refresh.await.unwrap();

        assert!(!s.entitlements_applied(), "obsolete refresh set the flag");
        assert!(s.catalogue_channels().is_empty());
        assert_eq!(s.entitlement_status(), EntitlementStatus::Unknown);
    }

    #[test]
    fn commits_apply_only_in_the_generation_they_started_in() {
        let s = ExtrasState::new(false, None);
        let started = s.generation();
        assert_eq!(s.commit_if_current(started, || 7), Some(7));
        // An account switch clears the caches and bumps the generation.
        s.invalidate_account_context();
        assert_ne!(s.generation(), started);
        assert_eq!(
            s.commit_if_current(started, || unreachable!("stale fetch must not commit")),
            None
        );
        assert_eq!(s.commit_if_current(s.generation(), || 1), Some(1));
    }

    #[test]
    fn clearing_the_live_cache_also_invalidates_in_flight_fetches() {
        let s = ExtrasState::new(false, None);
        let started = s.generation();
        s.clear_live();
        assert_eq!(s.commit_if_current(started, || ()), None);
    }

    #[test]
    fn disabled_state_reports_not_connected() {
        let s = ExtrasState::new(false, None);
        assert!(!s.connected());
        assert!(!s.enabled());
        assert_eq!(s.entitlement_status(), EntitlementStatus::Unknown);
        assert!(!s.entitlements_available());
        assert!(!s.entitlements_applied());
        assert_eq!(s.route("154", false, false), None);
        // A ex_ id always maps to its content id even when disabled...
        assert_eq!(s.route("ex_1", false, false), Some("1".to_string()));
    }

    #[test]
    fn enabled_but_logged_out_reports_authentication_required() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let s = ExtrasState::new(true, None);
        s.init(&reqwest::Client::new(), &store);
        assert!(s.enabled());
        assert!(!s.connected());
        assert_eq!(
            s.entitlement_status(),
            EntitlementStatus::AuthenticationRequired
        );
        assert!(!s.entitlements_available());
        assert!(!s.entitlements_applied());
    }

    #[test]
    fn entitlement_diagnostic_labels_are_explicit() {
        assert_eq!(EntitlementStatus::Unknown.as_str(), "unknown");
        assert_eq!(EntitlementStatus::Available.as_str(), "available");
        assert_eq!(
            EntitlementStatus::AuthenticationRequired.as_str(),
            "authentication_required"
        );
        assert_eq!(
            EntitlementStatus::UpstreamFailure.as_str(),
            "upstream_failure"
        );
    }

    #[test]
    fn set_unlocked_turns_extras_on_without_the_config_flag() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let s = ExtrasState::new(false, None);
        s.init(&reqwest::Client::new(), &store);
        assert!(!s.enabled());
        s.set_unlocked(true, &reqwest::Client::new(), &store);
        assert!(s.enabled());
        s.set_unlocked(false, &reqwest::Client::new(), &store);
        assert!(!s.enabled());
    }

    #[test]
    fn explicit_lock_overrides_enabled_config() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let http = reqwest::Client::new();
        let s = ExtrasState::new(true, None);
        s.init(&http, &store);
        assert!(s.enabled());
        s.set_unlocked(false, &http, &store);
        assert!(!s.enabled());

        let restored = ExtrasState::new(true, Some(false));
        restored.init(&http, &store);
        assert!(!restored.enabled());
    }

    #[test]
    fn route_prefers_jiotv_login_over_extras() {
        let s = ExtrasState::new(true, None);
        // Not connected (no client set up), so a plain JioTV id never routes.
        assert_eq!(s.route("154", true, false), None);
        assert_eq!(s.route("154", false, false), None);
    }

    #[test]
    fn stream_kind_map_migrates_from_old_store_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let mut old_map = HashMap::new();
        old_map.insert("1".to_string(), true);
        store
            .set(
                crate::extras::STORE_KEY_DASH_OLD,
                &serde_json::to_string(&old_map).unwrap(),
            )
            .unwrap();

        let s = ExtrasState::new(true, None);
        s.init(&reqwest::Client::new(), &store);
        assert_eq!(s.dash.read().unwrap().get("1"), Some(&true));
    }

    #[test]
    fn stream_kind_map_prefers_new_store_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let mut old_map = HashMap::new();
        old_map.insert("1".to_string(), true);
        store
            .set(
                crate::extras::STORE_KEY_DASH_OLD,
                &serde_json::to_string(&old_map).unwrap(),
            )
            .unwrap();
        let mut new_map = HashMap::new();
        new_map.insert("1".to_string(), false);
        store
            .set(
                crate::extras::STORE_KEY_DASH,
                &serde_json::to_string(&new_map).unwrap(),
            )
            .unwrap();

        let s = ExtrasState::new(true, None);
        s.init(&reqwest::Client::new(), &store);
        assert_eq!(s.dash.read().unwrap().get("1"), Some(&false));
    }

    #[test]
    fn live_hls_path_matches_quality() {
        assert_eq!(
            ExtrasState::live_hls_path("ex_1", "auto"),
            "/live/ex_1.m3u8"
        );
        assert_eq!(
            ExtrasState::live_hls_path("ex_1", "high"),
            "/live/high/ex_1.m3u8"
        );
    }

    #[test]
    fn is_drm_defaults_to_true_for_unplayed_ex_channel() {
        let s = ExtrasState::new(true, None);
        assert_eq!(s.is_drm("5", true), Some(true));
        assert_eq!(s.is_drm("5", false), None);
    }
}
