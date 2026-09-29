//! JioTV login, channel list and playlist generation. Mirrors
//! `pkg/television` and the login half of `pkg/utils` in the Go version. The
//! HTTP endpoints below are copied from
//! `internal/constants/urls/urls.go` in the Go tree (not called by tests;
//! tests exercise this module against a local mock server).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::time::{Duration, Instant};

pub const JIOTV_API_DOMAIN: &str = "jiotvapi.media.jio.com";
pub const CHANNELS_API_URL: &str = "https://jiotvapi.cdn.jio.com/apis/v3.1/getMobileChannelList/get/?langId=6&os=android&devicetype=phone&usertype=JIO&version=315&langId=6";
pub const ACTIVE_PLANS_API_URL: &str = "https://jiotvapi.media.jio.com/userservice/apis/v1/plans";
pub const REFRESH_TOKEN_URL: &str =
    "https://auth.media.jio.com/tokenservice/apis/v1/refreshtoken?langId=6";
/// The SSO-token fallback-TTL refresh path is not ported (see README); only
/// the JWT-`exp`-based access-token refresh in `token_refresh.rs` is.
#[allow(dead_code)]
pub const REFRESH_SSO_TOKEN_URL: &str =
    "https://tv.media.jio.com/apis/v2.0/loginotp/refresh?langId=6";
pub const PLAYBACK_API_PATH: &str = "/playback/apis/v1.1/geturl?langId=6";
pub const LOGIN_SEND_OTP_PATH: &str = "/userservice/apis/v1/loginotp/send";
pub const LOGIN_VERIFY_OTP_PATH: &str = "/userservice/apis/v1/loginotp/verify";
const CHANNELS_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const PLAN_SUMMARY_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

pub const CATEGORY_MAP: &[(i64, &str)] = &[
    (0, "All Categories"),
    (5, "Entertainment"),
    (6, "Movies"),
    (7, "Kids"),
    (8, "Sports"),
    (9, "Lifestyle"),
    (10, "Infotainment"),
    (12, "News"),
    (13, "Music"),
    (15, "Devotional"),
    (16, "Business"),
    (17, "Educational"),
    (18, "Shopping"),
    (19, "JioDarshan"),
];

pub const LANGUAGE_MAP: &[(i64, &str)] = &[
    (0, "All Languages"),
    (1, "Hindi"),
    (2, "Marathi"),
    (3, "Punjabi"),
    (4, "Urdu"),
    (5, "Bengali"),
    (6, "English"),
    (7, "Malayalam"),
    (8, "Tamil"),
    (9, "Gujarati"),
    (10, "Odia"),
    (11, "Telugu"),
    (12, "Bhojpuri"),
    (13, "Kannada"),
    (14, "Assamese"),
    (15, "Nepali"),
    (16, "French"),
    (18, "Other"),
];

pub fn category_name(id: i64) -> &'static str {
    CATEGORY_MAP
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, v)| *v)
        .unwrap_or("")
}

pub fn language_name(id: i64) -> &'static str {
    LANGUAGE_MAP
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, v)| *v)
        .unwrap_or("")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Channel {
    #[serde(rename = "channel_id", deserialize_with = "id_from_int_or_string")]
    pub id: String,
    #[serde(rename = "channel_name")]
    pub name: String,
    #[serde(rename = "channel_url", default)]
    pub url: String,
    #[serde(rename = "key_url", default)]
    pub key_url: String,
    #[serde(rename = "logoUrl", default)]
    pub logo_url: String,
    #[serde(rename = "channelCategoryId", default)]
    pub category: i64,
    #[serde(rename = "channelLanguageId", default)]
    pub language: i64,
    #[serde(rename = "isHD", default)]
    pub is_hd: bool,
    #[serde(rename = "isCatchupAvailable", default)]
    pub is_catchup_available: bool,
    #[serde(rename = "business_type", default)]
    pub business_type: String,
    #[serde(rename = "plan_type", default)]
    pub plan_type: String,
    #[serde(
        rename = "packageIds",
        default,
        deserialize_with = "ids_from_int_or_string_vec"
    )]
    pub package_ids: Vec<String>,
    #[serde(
        rename = "playbackRightIds",
        default,
        deserialize_with = "ids_from_int_or_string_vec"
    )]
    pub playback_right_ids: Vec<String>,
    #[serde(rename = "is_premium", default)]
    pub is_premium: bool,
}

impl Channel {
    /// True for channels the playback API refuses without a separate
    /// subscription, derived from `business_type` (the API's own
    /// `is_premium` flag mispredicts about one in five channels; see the Go
    /// comment on `television.Channel.RequiresSubscription`).
    pub fn requires_subscription(&self) -> bool {
        self.business_type.trim().eq_ignore_ascii_case("premium")
    }
}

fn id_from_int_or_string<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IdOrStr {
        Int(i64),
        Str(String),
    }
    Ok(match IdOrStr::deserialize(d)? {
        IdOrStr::Int(i) => i.to_string(),
        IdOrStr::Str(s) => s,
    })
}

fn ids_from_int_or_string_vec<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IdOrStr {
        Int(i64),
        Str(String),
    }
    Ok(Vec::<IdOrStr>::deserialize(d)?
        .into_iter()
        .map(|v| match v {
            IdOrStr::Int(i) => i.to_string(),
            IdOrStr::Str(s) => s,
        })
        .collect())
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChannelsResponse {
    // Present on the wire (and kept for parity with the Go struct); callers
    // only ever use `result`.
    #[allow(dead_code)]
    #[serde(default)]
    pub code: i64,
    #[allow(dead_code)]
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub result: Vec<Channel>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TvPlanSummary {
    pub active_plan_count: usize,
    pub provider_count: usize,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ActivePlansResponse {
    #[serde(rename = "PackageInfo", default)]
    package_info: Vec<ActiveSubscriptionPlan>,
    #[serde(default)]
    result: ActivePlansResult,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ActiveSubscriptionPlan {
    #[serde(default)]
    isactive: Option<bool>,
    #[serde(rename = "packageDetail", default)]
    package_detail: ActiveSubscriptionPack,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ActiveSubscriptionPack {
    #[serde(default)]
    providers: Vec<PlanProvider>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ActivePlansResult {
    #[serde(default)]
    plans: Vec<ActiveSubscriptionPack>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PlanProvider {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    provider: String,
    #[serde(rename = "providerId", default)]
    provider_id: String,
    #[serde(rename = "providerName", default)]
    provider_name: String,
}

impl ActivePlansResponse {
    fn summary(&self) -> TvPlanSummary {
        let any_explicitly_active = self
            .package_info
            .iter()
            .any(|plan| plan.isactive == Some(true));
        let selected: Vec<&ActiveSubscriptionPlan> = self
            .package_info
            .iter()
            .filter(|plan| !any_explicitly_active || plan.isactive == Some(true))
            .collect();

        let mut providers = HashSet::new();
        if !selected.is_empty() {
            for plan in &selected {
                for provider in &plan.package_detail.providers {
                    if let Some(key) = provider.key() {
                        providers.insert(key);
                    }
                }
            }
            return TvPlanSummary {
                active_plan_count: selected.len(),
                provider_count: providers.len(),
            };
        }

        for plan in &self.result.plans {
            for provider in &plan.providers {
                if let Some(key) = provider.key() {
                    providers.insert(key);
                }
            }
        }
        TvPlanSummary {
            active_plan_count: self.result.plans.len(),
            provider_count: providers.len(),
        }
    }
}

impl PlanProvider {
    fn key(&self) -> Option<String> {
        [
            &self.id,
            &self.provider_id,
            &self.provider,
            &self.name,
            &self.provider_name,
        ]
        .into_iter()
        .find(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_ascii_lowercase())
    }
}

/// Holds JioTV credentials in memory. Persisted to the store by the caller
/// (see `crate::login`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub sso_token: String,
    pub crm: String,
    pub unique_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

/// Bitrates for one stream family (HLS `bitrates`, or DASH `mpd.bitrates`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Bitrates {
    #[serde(default)]
    pub auto: String,
    #[serde(default)]
    pub high: String,
    #[serde(default)]
    pub low: String,
    #[serde(default)]
    pub medium: String,
}

/// The DASH half of a playback response. Live channels nest URLs under
/// `bitrates`; premium/SVOD content returns a single `auto` URL at this
/// level instead (see `resolved_bitrates`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Mpd {
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub bitrates: Bitrates,
    #[serde(default)]
    pub auto: String,
    #[serde(default)]
    pub high: String,
    #[serde(default)]
    pub low: String,
    #[serde(default)]
    pub medium: String,
}

impl Mpd {
    pub fn resolved_bitrates(&self) -> Bitrates {
        let mut b = self.bitrates.clone();
        if b.auto.is_empty() {
            b.auto = self.auto.clone();
        }
        if b.high.is_empty() {
            b.high = self.high.clone();
        }
        if b.medium.is_empty() {
            b.medium = self.medium.clone();
        }
        if b.low.is_empty() {
            b.low = self.low.clone();
        }
        b
    }
}

/// The JioTV playback API response (`/playback/apis/v1.1/geturl`), mirroring
/// `LiveURLOutput` in the Go tree.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LiveUrlOutput {
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub message: String,
    // Present on the wire, kept for parity with the Go struct; not read.
    #[allow(dead_code)]
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub bitrates: Bitrates,
    #[serde(default)]
    pub mpd: Mpd,
    #[allow(dead_code)]
    #[serde(default, rename = "m3u8")]
    pub m3u8: Bitrates,
    #[serde(default, rename = "isDRM")]
    pub is_drm: bool,
    #[serde(default, rename = "keyUrl")]
    pub key_url: String,
    #[serde(default, rename = "algoName")]
    pub algo_name: String,
    /// Not part of the wire format: filled in after parsing from whichever
    /// stream URL carried a `hdnea=` query parameter (see `Television::live`).
    #[serde(skip)]
    pub hdnea: String,
}

impl LiveUrlOutput {
    /// The DRM license URL: live channels carry it in `mpd.key`, premium
    /// provider content returns a top-level `keyUrl`.
    pub fn resolved_license_url(&self) -> &str {
        if !self.key_url.trim().is_empty() {
            self.key_url.trim()
        } else {
            self.mpd.key.trim()
        }
    }

    /// Kept for parity with the Go method; `AppState::is_drm_channel` (the
    /// static DRM-ID list plus extras's learned map) is what actually decides
    /// this in the current routing.
    #[allow(dead_code)]
    pub fn has_drm_stream(&self) -> bool {
        !self.mpd.resolved_bitrates().auto.is_empty() && !self.resolved_license_url().is_empty()
    }
}

/// Picks a bitrate by name, mirroring `internalUtils.SelectQuality`.
pub fn select_quality<'a>(
    quality: &str,
    auto: &'a str,
    high: &'a str,
    medium: &'a str,
    low: &'a str,
) -> &'a str {
    match quality {
        "high" | "h" => high,
        "medium" | "med" | "m" => medium,
        "low" | "l" => low,
        _ => auto,
    }
}

fn extract_hdnea_from_url(u: &str) -> Option<String> {
    let idx = u.find("hdnea=")?;
    let rest = &u[idx + "hdnea=".len()..];
    let end = rest.find('&').unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

fn append_hdnea(u: &str, hdnea: &str) -> String {
    if u.is_empty() || u.contains("hdnea=") {
        return u.to_string();
    }
    let sep = if u.contains('?') { '&' } else { '?' };
    format!("{u}{sep}hdnea={hdnea}")
}

pub struct Television {
    pub creds: RwLock<Option<Credentials>>,
    pub client: reqwest::Client,
    pub device_id: String,
    channels_cache: RwLock<Option<(ChannelsResponse, Instant)>>,
    plan_summary_cache: RwLock<Option<(TvPlanSummary, Instant)>>,
    /// Serialises token refreshes so concurrent requests share one.
    pub refresh_lock: tokio::sync::Mutex<()>,
}

impl Television {
    #[cfg(test)]
    pub fn new(client: reqwest::Client) -> Television {
        Television::with_device_id(client, String::new())
    }

    pub fn with_device_id(client: reqwest::Client, device_id: String) -> Television {
        Television {
            creds: RwLock::new(None),
            client,
            device_id,
            channels_cache: RwLock::new(None),
            plan_summary_cache: RwLock::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn logged_in(&self) -> bool {
        self.creds.read().unwrap().is_some()
    }

    pub fn set_credentials(&self, c: Credentials) {
        *self.creds.write().unwrap() = Some(c);
        *self.channels_cache.write().unwrap() = None;
        *self.plan_summary_cache.write().unwrap() = None;
    }

    pub fn clear_credentials(&self) {
        *self.creds.write().unwrap() = None;
        *self.channels_cache.write().unwrap() = None;
        *self.plan_summary_cache.write().unwrap() = None;
    }

    /// Stable, non-secret digest used to bind account-sensitive caches to
    /// the active TV account without persisting raw account identifiers.
    pub fn account_fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"tv-account-v1\0");
        match self.creds.read().unwrap().as_ref() {
            Some(creds) => {
                hasher.update(creds.crm.as_bytes());
                hasher.update(b"\0");
                hasher.update(creds.unique_id.as_bytes());
            }
            None => hasher.update(b"anonymous"),
        }
        hex::encode(hasher.finalize())
    }

    fn auth_headers(&self) -> HashMap<String, String> {
        let mut h = HashMap::new();
        if let Some(c) = self.creds.read().unwrap().as_ref() {
            h.insert("ssoToken".into(), c.sso_token.clone());
            h.insert("crmid".into(), c.crm.clone());
            h.insert("uniqueId".into(), c.unique_id.clone());
            h.insert("accesstoken".into(), c.access_token.clone());
        }
        h
    }

    /// Fetches the full channel list. Works with or without a login (the
    /// list itself needs no auth in the Go version either).
    pub async fn channels(&self) -> anyhow::Result<ChannelsResponse> {
        if let Some((channels, fetched_at)) = self.channels_cache.read().unwrap().as_ref() {
            if fetched_at.elapsed() < CHANNELS_CACHE_TTL {
                return Ok(channels.clone());
            }
        }
        let channels = self.channels_from(CHANNELS_API_URL).await?;
        *self.channels_cache.write().unwrap() = Some((channels.clone(), Instant::now()));
        Ok(channels)
    }

    #[cfg(test)]
    pub fn set_channels_for_test(&self, channels: Vec<Channel>) {
        *self.channels_cache.write().unwrap() = Some((
            ChannelsResponse {
                code: 200,
                message: "test".into(),
                result: channels,
            },
            Instant::now(),
        ));
    }

    pub async fn channels_from(&self, url: &str) -> anyhow::Result<ChannelsResponse> {
        let mut req = self.client.get(url);
        for (k, v) in self.auth_headers() {
            req = req.header(k, v);
        }
        let resp = req.send().await?.error_for_status()?;
        Ok(resp.json::<ChannelsResponse>().await?)
    }

    /// Returns a sanitized summary of the account plans API. This proves
    /// whether account-level plan data exists, but intentionally does not
    /// claim that its provider/plan IDs authorize any particular linear TV
    /// channel: no verified mapping to Channel::package_ids exists yet.
    pub async fn plan_summary(&self) -> Option<TvPlanSummary> {
        if !self.logged_in() {
            return None;
        }
        if let Some((summary, fetched_at)) = self.plan_summary_cache.read().unwrap().as_ref() {
            if fetched_at.elapsed() < PLAN_SUMMARY_CACHE_TTL {
                return Some(summary.clone());
            }
        }
        let summary = self.plan_summary_from(ACTIVE_PLANS_API_URL).await.ok()?;
        *self.plan_summary_cache.write().unwrap() = Some((summary.clone(), Instant::now()));
        Some(summary)
    }

    async fn plan_summary_from(&self, url: &str) -> anyhow::Result<TvPlanSummary> {
        let creds = self
            .creds
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("not logged in"))?;
        let mut req = self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, "okhttp/4.12.0")
            .header(reqwest::header::ACCEPT, "application/json")
            .header("devicetype", "phone")
            .header("os", "android")
            .header("versionCode", "422")
            .header("Connection", "close");
        if !creds.access_token.is_empty() {
            req = req.header("accesstoken", &creds.access_token);
        }
        if !creds.unique_id.is_empty() {
            req = req.header("uniqueId", &creds.unique_id);
        }
        let response = req
            .send()
            .await?
            .error_for_status()?
            .json::<ActivePlansResponse>()
            .await?;
        Ok(response.summary())
    }

    /// The form headers `Television::New` builds in the Go version, sent on
    /// every `Live`/`GetCatchupURL` call.
    fn playback_headers(&self) -> Vec<(&'static str, String)> {
        let creds = self.creds.read().unwrap();
        let (crm, unique_id, access_token) = creds
            .as_ref()
            .map(|c| (c.crm.clone(), c.unique_id.clone(), c.access_token.clone()))
            .unwrap_or_default();
        vec![
            ("appkey", "NzNiMDhlYzQyNjJm".to_string()),
            ("crmid", crm.clone()),
            ("userId", crm.clone()),
            ("deviceId", self.device_id.clone()),
            ("devicetype", "phone".to_string()),
            ("isott", "false".to_string()),
            ("languageId", "6".to_string()),
            ("lbcookie", "1".to_string()),
            ("os", "android".to_string()),
            ("osVersion", "13".to_string()),
            ("subscriberId", crm),
            ("uniqueId", unique_id),
            ("usergroup", "tvYR7NSNn7rymo3F".to_string()),
            ("versionCode", "422".to_string()),
            ("accessToken", access_token),
        ]
    }

    fn access_token(&self) -> String {
        self.creds
            .read()
            .unwrap()
            .as_ref()
            .map(|c| c.access_token.clone())
            .unwrap_or_default()
    }

    /// Requests a playback URL for a live channel (`POST
    /// /playback/apis/v1.1/geturl`), mirroring `Television.Live` in the Go
    /// tree: same form fields, and the same after-the-fact `hdnea=`
    /// extraction/propagation across every URL field in the response (the
    /// API does not set it via `Set-Cookie` on this call).
    pub async fn live(&self, channel_id: &str) -> anyhow::Result<LiveUrlOutput> {
        self.live_at(
            &format!("https://{JIOTV_API_DOMAIN}{PLAYBACK_API_PATH}"),
            channel_id,
        )
        .await
    }

    pub async fn live_at(&self, url: &str, channel_id: &str) -> anyhow::Result<LiveUrlOutput> {
        let form = [
            ("channel_id", channel_id.to_string()),
            ("stream_type", "Seek".to_string()),
            ("begin", chrono_like_now("%Y%m%dT%H%M%S")),
            ("srno", chrono_like_now("%Y%m%d")),
        ];
        let mut req = self
            .client
            .post(url)
            .header("accessToken", self.access_token());
        for (k, v) in self.playback_headers() {
            req = req.header(k, v);
        }
        let resp = req.form(&form).send().await?.error_for_status()?;
        let mut result: LiveUrlOutput = resp.json().await?;
        finish_live_result(&mut result);
        Ok(result)
    }

    /// Requests a catchup playback URL, mirroring `Television.GetCatchupURL`.
    pub async fn catchup_url(
        &self,
        channel_id: &str,
        srno: &str,
        start: &str,
        end: &str,
    ) -> anyhow::Result<LiveUrlOutput> {
        self.catchup_url_at(
            &format!("https://{JIOTV_API_DOMAIN}{PLAYBACK_API_PATH}"),
            channel_id,
            srno,
            start,
            end,
        )
        .await
    }

    pub async fn catchup_url_at(
        &self,
        url: &str,
        channel_id: &str,
        srno: &str,
        start: &str,
        end: &str,
    ) -> anyhow::Result<LiveUrlOutput> {
        let form = [
            ("stream_type", "Catchup".to_string()),
            ("channel_id", channel_id.to_string()),
            ("programId", srno.to_string()),
            ("showtime", "000000".to_string()),
            ("srno", srno.to_string()),
            ("begin", start.to_string()),
            ("end", end.to_string()),
        ];
        let mut req = self
            .client
            .post(url)
            .header("accessToken", self.access_token());
        for (k, v) in self.playback_headers() {
            req = req.header(k, v);
        }
        let resp = req.form(&form).send().await?.error_for_status()?;
        let mut result: LiveUrlOutput = resp.json().await?;
        finish_live_result(&mut result);
        Ok(result)
    }

    /// GETs a stream URL (an m3u8/mpd manifest), sending the `__hdnea__`
    /// cookie when one is known, and returns the body, status, and any fresh
    /// `__hdnea__` the upstream set via `Set-Cookie` — mirroring
    /// `Television.Render`.
    pub async fn render(&self, stream_url: &str, hdnea_token: &str) -> (Vec<u8>, u16, String) {
        let mut req = self
            .client
            .get(stream_url)
            .header("User-Agent", PLAYER_USER_AGENT);
        let token = if !hdnea_token.is_empty() {
            Some(hdnea_token.to_string())
        } else {
            extract_hdnea_from_url(stream_url)
        };
        if let Some(t) = &token {
            req = req.header(header::COOKIE, format!("__hdnea__={t}"));
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(_) => return (Vec::new(), 502, String::new()),
        };
        let status = resp.status().as_u16();
        let new_hdnea = resp
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|sc| {
                sc.split(';')
                    .map(str::trim)
                    .find_map(|part| part.strip_prefix("__hdnea__="))
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let body = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
        (body, status, new_hdnea)
    }
}

pub const PLAYER_USER_AGENT: &str = "plaYtv/7.1.8 (Linux;Android 8.1.0) ExoPlayerLib/2.11.7";

use axum::http::header;

fn chrono_like_now(fmt: &str) -> String {
    // Avoids pulling in the `chrono` crate for two UTC timestamp formats.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let (y, m, d, hh, mm, ss) = civil_from_unix(now.as_secs());
    match fmt {
        "%Y%m%d" => format!("{y:04}{m:02}{d:02}"),
        _ => format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}"),
    }
}

/// Splits a Unix timestamp into UTC (year, month, day, hour, minute, second)
/// using a tiny hand-rolled civil calendar (Howard Hinnant's
/// days_from_civil algorithm), just enough to reproduce a few of Go's
/// `time.Now().UTC().Format(...)` call sites without a date/time dependency.
pub fn civil_from_unix(secs: u64) -> (i64, i64, i64, i64, i64, i64) {
    let days = (secs / 86400) as i64;
    let rem = (secs % 86400) as i64;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, hh, mm, ss)
}

fn finish_live_result(result: &mut LiveUrlOutput) {
    let hdnea = extract_hdnea_from_url(&result.bitrates.auto)
        .or_else(|| extract_hdnea_from_url(&result.mpd.result))
        .unwrap_or_default();
    result.hdnea = hdnea.clone();
    if !hdnea.is_empty() {
        result.bitrates.auto = append_hdnea(&result.bitrates.auto, &hdnea);
        result.bitrates.high = append_hdnea(&result.bitrates.high, &hdnea);
        result.bitrates.medium = append_hdnea(&result.bitrates.medium, &hdnea);
        result.bitrates.low = append_hdnea(&result.bitrates.low, &hdnea);
        result.result = append_hdnea(&result.result, &hdnea);
        if !result.mpd.result.is_empty() {
            result.mpd.result = append_hdnea(&result.mpd.result, &hdnea);
        }
        if !result.mpd.key.is_empty() {
            result.mpd.key = append_hdnea(&result.mpd.key, &hdnea);
        }
    }
}

/// Picks the best available HLS URL for a quality, mirroring
/// `selectBestLiveHLSURL`: the requested quality first, then any other HLS
/// bitrate, then `result`/`mpd.result` if either looks like an `.m3u8` URL.
pub fn select_best_live_hls_url(live: &LiveUrlOutput, quality: &str) -> String {
    let b = &live.bitrates;
    let selected = select_quality(quality, &b.auto, &b.high, &b.medium, &b.low);
    if !selected.is_empty() {
        return selected.to_string();
    }
    for candidate in [&b.high, &b.auto, &b.medium, &b.low] {
        if !candidate.is_empty() {
            return candidate.clone();
        }
    }
    if live.result.to_lowercase().contains(".m3u8") {
        return live.result.clone();
    }
    if live.mpd.result.to_lowercase().contains(".m3u8") {
        return live.mpd.result.clone();
    }
    String::new()
}

/// Reports whether a playback response has a DASH stream. Mirrors `hasDASH`
/// in the Go tree (used for extras channels that fall back between HLS/DASH).
pub fn has_dash(r: &LiveUrlOutput) -> bool {
    let b = r.mpd.resolved_bitrates();
    !b.auto.is_empty()
        || !b.high.is_empty()
        || !b.medium.is_empty()
        || !b.low.is_empty()
        || !r.mpd.result.is_empty()
}

/// Mirrors `selectBestLiveMPDURL`.
pub fn select_best_live_mpd_url(live: &LiveUrlOutput, quality: &str) -> String {
    let b = live.mpd.resolved_bitrates();
    let selected = select_quality(quality, &b.auto, &b.high, &b.medium, &b.low);
    if !selected.is_empty() {
        return selected.to_string();
    }
    for candidate in [&b.high, &b.auto, &b.medium, &b.low] {
        if !candidate.is_empty() {
            return candidate.clone();
        }
    }
    live.mpd.result.clone()
}

/// Generates an M3U playlist, mirroring `GenerateM3UPlaylist` in
/// `internal/handlers/handlers.go`: same `#EXTINF` attributes, same
/// `/live/...` and `/live/mpd/...` URL shapes, same KODIPROP block for DRM
/// channels.
pub struct PlaylistOptions<'a> {
    pub host_url: &'a str,
    pub quality: &'a str,
    pub split_category: &'a str,
    pub languages: &'a str,
    pub skip_genres: &'a str,
    pub sub_filter: &'a str,
}

pub fn generate_m3u_playlist(
    channels: &[Channel],
    opts: &PlaylistOptions,
    is_drm_channel: impl Fn(&str) -> bool,
    is_playable: impl Fn(&str) -> bool,
) -> String {
    let mut out = String::new();
    out.push_str("#EXTM3U x-tvg-url=\"");
    out.push_str(opts.host_url);
    out.push_str("/epg.xml.gz\"\n");
    let logo_url = format!("{}/jtvimage", opts.host_url);

    let langs: Vec<&str> = opts
        .languages
        .split(',')
        .filter(|s| !s.is_empty())
        .collect();
    let skip_genres: Vec<&str> = opts
        .skip_genres
        .split(',')
        .filter(|s| !s.is_empty())
        .collect();

    for ch in channels {
        if !is_playable(&ch.id) {
            continue;
        }
        if !opts.languages.is_empty() && !langs.contains(&language_name(ch.language)) {
            continue;
        }
        if !opts.skip_genres.is_empty() && skip_genres.contains(&category_name(ch.category)) {
            continue;
        }
        match opts.sub_filter {
            "hide" if ch.requires_subscription() => continue,
            "only" if !ch.requires_subscription() => continue,
            _ => {}
        }

        let (channel_url, kodi_props) = if is_drm_channel(&ch.id) {
            let mut url = format!("{}/live/mpd/{}", opts.host_url, ch.id);
            if !opts.quality.is_empty() {
                url.push_str("?q=");
                url.push_str(opts.quality);
            }
            let mut props = format!(
                "#KODIPROP:inputstream=inputstream.adaptive\n#KODIPROP:inputstream.adaptive.manifest_type=mpd\n#KODIPROP:inputstream.adaptive.license_type=com.widevine.alpha\n#KODIPROP:inputstream.adaptive.license_key={}/live/key/{}",
                opts.host_url, ch.id
            );
            if !opts.quality.is_empty() {
                props.push_str("?q=");
                props.push_str(opts.quality);
            }
            props.push('\n');
            (url, props)
        } else {
            let url = if !opts.quality.is_empty() {
                format!("{}/live/{}/{}.m3u8", opts.host_url, opts.quality, ch.id)
            } else {
                format!("{}/live/{}.m3u8", opts.host_url, ch.id)
            };
            (url, String::new())
        };

        let channel_logo_url =
            if ch.logo_url.starts_with("http://") || ch.logo_url.starts_with("https://") {
                ch.logo_url.clone()
            } else {
                format!("{}/{}", logo_url, ch.logo_url)
            };

        let group_title = match opts.split_category {
            "split" => format!(
                "{} - {}",
                category_name(ch.category),
                language_name(ch.language)
            ),
            "language" => language_name(ch.language).to_string(),
            _ => category_name(ch.category).to_string(),
        };

        out.push_str(&format!(
            "#EXTINF:-1 tvg-id=\"{}\" tvg-name=\"{}\" tvg-logo=\"{}\" tvg-language=\"{}\" tvg-type=\"{}\" group-title=\"{}\", {}\n{}{}\n",
            ch.id, ch.name, channel_logo_url, language_name(ch.language), category_name(ch.category),
            group_title, ch.name, kodi_props, channel_url
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: &str, name: &str, cat: i64, lang: i64) -> Channel {
        Channel {
            id: id.to_string(),
            name: name.to_string(),
            category: cat,
            language: lang,
            logo_url: "logo.png".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn parses_int_channel_id() {
        let json = r#"{"channel_id": 154, "channel_name": "Test", "logoUrl": "x.png"}"#;
        let ch: Channel = serde_json::from_str(json).unwrap();
        assert_eq!(ch.id, "154");
    }

    #[test]
    fn parses_upstream_entitlement_metadata() {
        let json = r#"{
            "channel_id":154,
            "channel_name":"Sony SAB",
            "business_type":"premium",
            "plan_type":"premium",
            "packageIds":["1",6,"7",23],
            "playbackRightIds":["1",4],
            "is_premium":true
        }"#;
        let ch: Channel = serde_json::from_str(json).unwrap();
        assert_eq!(ch.plan_type, "premium");
        assert_eq!(ch.package_ids, vec!["1", "6", "7", "23"]);
        assert_eq!(ch.playback_right_ids, vec!["1", "4"]);
        assert!(ch.is_premium);
    }

    #[tokio::test]
    async fn active_plans_summary_uses_authenticated_account_endpoint_shape() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/plans"))
            .and(header("accesstoken", "redacted-access"))
            .and(header("uniqueId", "redacted-unique"))
            .and(header("devicetype", "phone"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "PackageInfo": [
                    {
                        "isactive": false,
                        "packageDetail": {"providers": [{"id": "inactive-provider"}]}
                    },
                    {
                        "isactive": true,
                        "packageDetail": {
                            "providers": [
                                {"id": "provider-a", "name": "Provider A"},
                                {"providerId": "provider-b", "providerName": "Provider B"}
                            ]
                        }
                    }
                ]
            })))
            .mount(&server)
            .await;

        let tv = Television::new(reqwest::Client::new());
        tv.set_credentials(Credentials {
            access_token: "redacted-access".into(),
            unique_id: "redacted-unique".into(),
            ..Default::default()
        });
        let summary = tv
            .plan_summary_from(&format!("{}/plans", server.uri()))
            .await
            .unwrap();
        assert_eq!(summary.active_plan_count, 1);
        assert_eq!(summary.provider_count, 2);
    }

    #[test]
    fn premium_business_type_requires_subscription() {
        let mut ch = channel("1", "A", 5, 1);
        ch.business_type = "PREMIUM".into();
        assert!(ch.requires_subscription());
        ch.business_type = "free".into();
        assert!(!ch.requires_subscription());
    }

    #[test]
    fn playlist_contains_extinf_and_live_url() {
        let channels = vec![channel("154", "Star Plus", 5, 1)];
        let opts = PlaylistOptions {
            host_url: "http://localhost:5001",
            quality: "",
            split_category: "",
            languages: "",
            skip_genres: "",
            sub_filter: "",
        };
        let m3u = generate_m3u_playlist(&channels, &opts, |_| false, |_| true);
        assert!(m3u.starts_with("#EXTM3U"));
        assert!(m3u.contains("tvg-id=\"154\""));
        assert!(m3u.contains("http://localhost:5001/live/154.m3u8"));
    }

    #[test]
    fn drm_channel_gets_mpd_and_kodiprops() {
        let channels = vec![channel("999", "DRM Ch", 5, 1)];
        let opts = PlaylistOptions {
            host_url: "http://h",
            quality: "",
            split_category: "",
            languages: "",
            skip_genres: "",
            sub_filter: "",
        };
        let m3u = generate_m3u_playlist(&channels, &opts, |_| true, |_| true);
        assert!(m3u.contains("http://h/live/mpd/999"));
        assert!(m3u.contains("KODIPROP:inputstream.adaptive.license_key=http://h/live/key/999"));
    }

    #[test]
    fn language_filter_excludes_other_languages() {
        let channels = vec![
            channel("1", "Hindi Ch", 5, 1),
            channel("2", "English Ch", 5, 6),
        ];
        let opts = PlaylistOptions {
            host_url: "http://h",
            quality: "",
            split_category: "",
            languages: "Hindi",
            skip_genres: "",
            sub_filter: "",
        };
        let m3u = generate_m3u_playlist(&channels, &opts, |_| false, |_| true);
        assert!(m3u.contains("Hindi Ch"));
        assert!(!m3u.contains("English Ch"));
    }

    #[test]
    fn sub_filter_hides_premium() {
        let mut premium = channel("1", "Premium Ch", 5, 1);
        premium.business_type = "premium".into();
        let channels = vec![premium, channel("2", "Free Ch", 5, 1)];
        let opts = PlaylistOptions {
            host_url: "http://h",
            quality: "",
            split_category: "",
            languages: "",
            skip_genres: "",
            sub_filter: "hide",
        };
        let m3u = generate_m3u_playlist(&channels, &opts, |_| false, |_| true);
        assert!(!m3u.contains("Premium Ch"));
        assert!(m3u.contains("Free Ch"));
    }

    #[test]
    fn not_playable_channels_are_skipped() {
        let channels = vec![channel("1", "A", 5, 1)];
        let opts = PlaylistOptions {
            host_url: "http://h",
            quality: "",
            split_category: "",
            languages: "",
            skip_genres: "",
            sub_filter: "",
        };
        let m3u = generate_m3u_playlist(&channels, &opts, |_| false, |_| false);
        assert_eq!(m3u.trim(), "#EXTM3U x-tvg-url=\"http://h/epg.xml.gz\"");
    }
}
