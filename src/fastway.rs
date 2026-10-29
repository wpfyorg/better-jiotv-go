//! Client for Fastway Live TV, an optional provider alongside JioTV and the
//! extras source. Off by default (`fastway` config / `JIOTV_FASTWAY`), and
//! only usable after `jiotv fastway login` or the Settings card.
//!
//! The provider is a plain JSON API: one login token, a channel catalogue
//! filtered by the account's subscribed list, a day-guide endpoint, and a
//! PallyCon token-issue endpoint. Playback itself is a DASH manifest URL from
//! the catalogue. See `src/fastway_state.rs` for routing/caching and
//! `src/dash.rs` for the license proxy.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub const ID_PREFIX: &str = "fw_";
pub const API_BASE: &str = "http://fwlivetvapp.netplus.co.in:8080";
pub const PALLYCON_LICENSE_URL: &str = "http://license.pallycon.com/ri/licenseManager.do";
/// Mirrors the native player's ExoPlayer user agent (`Util.getUserAgent`).
pub const PLAYER_USER_AGENT: &str = "UlkaExoPlayer/2.13.3 (Linux;Android 14) ExoPlayerLib/2.13.3";
/// Mirrors the PallyCon sample player's user agent used by the native SDK.
pub const LICENSE_USER_AGENT: &str =
    "ExoPlayerSample/2.13.3 (Linux;Android 14) ExoPlayerLib/2.13.3";

pub const STORE_KEY_DEVICE: &str = "fastway_device";
pub const STORE_KEY_CREDENTIALS: &str = "fastway_credentials";

const APP_NAME: &str = "Fastway Live TV";
const APP_VERSION: &str = "2.1";
const API_VERSION: &str = "34";
const DEVICE_BRAND: &str = "JioTV Go";
const OS_NAME: &str = "Android";
const OS_VERSION: &str = "14";
const SCREEN_SIZE: &str = "1920x1080";
const DEVICE_IP: &str = "127.0.0.1";
const RADIO_GENRE_ID: i64 = 52;
const FAVORITES_GENRE_ID: i64 = 666;
pub const IST_OFFSET_SECS: i64 = 5 * 3600 + 30 * 60;

/// Stable identity presented to Fastway. The box id is the account's device
/// slot, so it must survive restarts; a new one looks like a new device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub boxid: String,
    pub mac: String,
    pub model: String,
    pub os_version: String,
}

impl Device {
    pub fn new() -> Device {
        let mut boxid = [0u8; 16];
        let mut mac = [0u8; 6];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut boxid);
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut mac);
        Device {
            boxid: hex::encode(boxid),
            mac: hex::encode(mac),
            model: "AFTKA".to_string(),
            os_version: OS_VERSION.to_string(),
        }
    }

    pub fn load_or_create(store: &crate::store::Store) -> anyhow::Result<Device> {
        if let Some(json) = store.get_opt(STORE_KEY_DEVICE) {
            if let Ok(d) = serde_json::from_str::<Device>(&json) {
                if !d.boxid.is_empty() && !d.mac.is_empty() {
                    return Ok(d);
                }
            }
        }
        let d = Device::new();
        store.set(STORE_KEY_DEVICE, &serde_json::to_string(&d)?)?;
        Ok(d)
    }
}

/// The Fastway username/password are kept so an expired access token can be
/// renewed without asking the user again. The store is local and untracked.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub access_token: String,
}

impl Credentials {
    pub fn load(store: &crate::store::Store) -> Option<Credentials> {
        let json = store.get_opt(STORE_KEY_CREDENTIALS)?;
        let cr: Credentials = serde_json::from_str(&json).ok()?;
        (!cr.username.is_empty()).then_some(cr)
    }

    pub fn save(&self, store: &crate::store::Store) -> anyhow::Result<()> {
        store.set(STORE_KEY_CREDENTIALS, &serde_json::to_string(self)?)?;
        Ok(())
    }
}

pub fn delete_credentials(store: &crate::store::Store) -> anyhow::Result<()> {
    let _ = store.delete(STORE_KEY_CREDENTIALS);
    Ok(())
}

/// Maps a Fastway channel ID (`fw_<id>`) to its numeric provider ID.
pub fn content_id(channel_id: &str) -> Option<&str> {
    channel_id.strip_prefix(ID_PREFIX)
}

pub fn channel_id(id: i64) -> String {
    format!("{ID_PREFIX}{id}")
}

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub api: String,
    pub license: String,
}

impl Default for Endpoints {
    fn default() -> Endpoints {
        Endpoints {
            api: API_BASE.to_string(),
            license: PALLYCON_LICENSE_URL.to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FastwayError {
    #[error("fastway: not logged in")]
    NotLoggedIn,
    #[error("fastway: {0} returned {1}: {2}")]
    Api(String, i64, String),
    #[error("fastway: the access token was rejected")]
    BadToken,
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(bound(deserialize = "T: serde::Deserialize<'de>"))]
pub struct Envelope<T> {
    #[serde(default)]
    pub status_code: i64,
    #[serde(default)]
    #[allow(dead_code)]
    pub error_code: i64,
    #[serde(default)]
    pub error_description: String,
    #[serde(default)]
    pub extra_data: String,
    #[serde(default)]
    pub response_object: Vec<T>,
}

impl<T> Envelope<T> {
    fn message(&self) -> String {
        let description = self.error_description.trim();
        let extra = self.extra_data.trim();
        match (description, extra) {
            ("", "") => format!("status {}", self.status_code),
            (description, "") => description.to_string(),
            ("", extra) => extra.to_string(),
            (description, extra) if description == extra => description.to_string(),
            (description, extra) => format!("{description}: {extra}"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LoginResponse {
    #[serde(default)]
    pub access_token: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
pub struct Genre {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub icon: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Subgenre {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
}

/// One catalogue row. `category`, `language` and `is_hd` are filled in by
/// `Catalogue::build`; the wire format does not carry them directly.
#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
pub struct FastwayChannel {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub genre_id: i64,
    #[serde(default)]
    pub channel_number: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub icon_url: String,
    #[serde(default)]
    pub subgenre_id: Option<i64>,
    #[serde(default)]
    pub catchup_mode: i64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub stream_source_id: i64,
    #[serde(default)]
    pub stream_url: String,
    #[serde(default)]
    pub channel_mode: String,
    #[serde(default)]
    pub stream_format: String,
    #[serde(default)]
    pub drm_platform: String,
    #[serde(default)]
    pub token: i64,
    #[serde(default)]
    pub token_url: String,
    #[serde(default)]
    pub encryption: i64,
    #[serde(default)]
    pub encryption_url: String,
    #[serde(default)]
    pub is_octoshape: i64,
    #[serde(default)]
    pub favorite_channel: String,
    #[serde(skip)]
    pub category: i64,
    #[serde(skip)]
    pub language: i64,
    #[serde(skip)]
    pub is_hd: bool,
}

impl FastwayChannel {
    pub fn drm(&self) -> bool {
        !self.encryption_url.trim().is_empty()
    }

    pub fn token_issue_url(&self, api_base: &str) -> String {
        if !self.drm() {
            return String::new();
        }
        format!(
            "{api_base}/apiv2/pallycon/TokenIssueV3?contentId={}",
            urlencoding::encode(self.encryption_url.trim())
        )
    }

    pub fn to_channel(&self) -> crate::television::Channel {
        crate::television::Channel {
            id: channel_id(self.id),
            name: self.title.clone(),
            logo_url: self.icon_url.clone(),
            category: self.category,
            language: self.language,
            is_hd: self.is_hd,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct Catalogue {
    pub channels: Vec<FastwayChannel>,
    pub by_id: HashMap<i64, FastwayChannel>,
    pub genres: HashMap<i64, String>,
    pub subgenres: HashMap<i64, String>,
}

impl Catalogue {
    /// Keeps only subscribed, live, playable rows; drops radio and the
    /// pseudo "Favorites" genre. The subscribed list is authoritative: the
    /// full `channels/list` response includes rows the account cannot play.
    pub fn build(
        list: Vec<FastwayChannel>,
        subscribed: &HashSet<i64>,
        genres: &HashMap<i64, String>,
        subgenres: &HashMap<i64, String>,
    ) -> Catalogue {
        let mut by_id = HashMap::new();
        for mut ch in list {
            if !subscribed.contains(&ch.id) || ch.stream_url.trim().is_empty() {
                continue;
            }
            if !ch.channel_mode.is_empty() && !ch.channel_mode.eq_ignore_ascii_case("live") {
                continue;
            }
            let genre_name = genres
                .get(&ch.genre_id)
                .map(String::as_str)
                .unwrap_or_default();
            if ch.genre_id == RADIO_GENRE_ID
                || ch.genre_id == FAVORITES_GENRE_ID
                || genre_name.eq_ignore_ascii_case("RADIO")
                || genre_name.eq_ignore_ascii_case("Favorites")
            {
                continue;
            }
            ch.category = category_for_genre(genre_name);
            let subgenre_name = ch
                .subgenre_id
                .and_then(|id| subgenres.get(&id))
                .map(String::as_str)
                .unwrap_or_default();
            ch.language = language_for_subgenre(subgenre_name);
            ch.is_hd = is_hd_title(&ch.title);
            by_id.insert(ch.id, ch);
        }
        let mut channels: Vec<_> = by_id.into_values().collect();
        channels.sort_by(|a, b| {
            a.channel_number
                .cmp(&b.channel_number)
                .then_with(|| a.title.cmp(&b.title))
        });
        let by_id = channels.iter().cloned().map(|ch| (ch.id, ch)).collect();
        Catalogue {
            channels,
            by_id,
            genres: genres.clone(),
            subgenres: subgenres.clone(),
        }
    }

    pub fn channel(&self, id: &str) -> Option<&FastwayChannel> {
        let id: i64 = content_id(id).unwrap_or(id).parse().ok()?;
        self.by_id.get(&id)
    }

    pub fn channels_as_tv(&self) -> Vec<crate::television::Channel> {
        self.channels
            .iter()
            .map(FastwayChannel::to_channel)
            .collect()
    }
}

fn is_hd_title(title: &str) -> bool {
    title
        .split_whitespace()
        .any(|word| word.eq_ignore_ascii_case("HD"))
}

pub fn category_for_genre(name: &str) -> i64 {
    match name.trim().to_ascii_uppercase().as_str() {
        "SPORTS" => 8,
        "ENTERTAINMENT" => 5,
        "MOVIES" => 6,
        "NEWS" => 12,
        "MUSIC" => 13,
        "INFOTAINMENT" => 10,
        "KIDS" => 7,
        "DEVOTIONAL" => 15,
        "REGIONAL" => 20,
        _ => 0,
    }
}

pub fn language_for_subgenre(name: &str) -> i64 {
    match name.trim().to_ascii_uppercase().as_str() {
        "PUNJABI" => 3,
        "HINDI" => 1,
        "ENGLISH" => 6,
        "ALL CHANNELS" | "" => 0,
        _ => 18,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Programme {
    pub title: String,
    pub description: String,
    pub start_epoch: i64,
    pub end_epoch: i64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
struct EpgEntry {
    #[serde(default)]
    id: serde_json::Value,
    #[serde(default)]
    number: serde_json::Value,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    duration: serde_json::Value,
    #[serde(default, alias = "programStart")]
    programstart: String,
    #[serde(default, alias = "programEnd")]
    programend: String,
}

fn value_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

pub struct Client {
    http: reqwest::Client,
    pub device: Device,
    endpoints: RwLock<Endpoints>,
    creds: RwLock<Option<Credentials>>,
}

impl Client {
    pub fn new(http: reqwest::Client, device: Device) -> Client {
        Client {
            http,
            device,
            endpoints: RwLock::new(Endpoints::default()),
            creds: RwLock::new(None),
        }
    }

    #[cfg(test)]
    pub fn set_endpoints(&self, e: Endpoints) {
        *self.endpoints.write().unwrap() = e;
    }

    fn endpoints(&self) -> Endpoints {
        self.endpoints.read().unwrap().clone()
    }

    /// The PallyCon license endpoint. The provider-issued token already names
    /// the site, so no separate site key is needed.
    pub fn license_url(&self) -> String {
        self.endpoints().license
    }

    pub fn credentials(&self) -> Option<Credentials> {
        self.creds.read().unwrap().clone()
    }

    pub fn set_credentials(&self, cr: Option<Credentials>) {
        *self.creds.write().unwrap() = cr;
    }

    fn auth_header(&self) -> Result<String, FastwayError> {
        let cr = self.credentials().ok_or(FastwayError::NotLoggedIn)?;
        if cr.access_token.is_empty() {
            return Err(FastwayError::NotLoggedIn);
        }
        Ok(format!("auth={}", cr.access_token))
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<Credentials, FastwayError> {
        #[derive(Serialize)]
        struct Body<'a> {
            appid: i64,
            ntype: &'a str,
            device_timezone: &'a str,
            hdmi: &'a str,
            language: &'a str,
            devicebrand: &'a str,
            os: &'a str,
            firmwareversion: &'a str,
            app_name: &'a str,
            api_version: &'a str,
            appversion: &'a str,
            device_ip: &'a str,
            macaddress: &'a str,
            googleappid: &'a str,
            screensize: &'a str,
            auth: String,
        }
        let username = username.trim();
        if username.is_empty() || password.is_empty() {
            return Err(FastwayError::Api(
                "login".to_string(),
                400,
                "username and password are required".to_string(),
            ));
        }
        let auth = format!(
            "username={username};password={password};boxid={};appid=1;timestamp={}",
            self.device.boxid,
            now_millis()
        );
        let body = Body {
            appid: 1,
            ntype: "1",
            device_timezone: "6",
            hdmi: "false",
            language: "eng",
            devicebrand: DEVICE_BRAND,
            os: OS_NAME,
            firmwareversion: "unknown",
            app_name: APP_NAME,
            api_version: API_VERSION,
            appversion: APP_VERSION,
            device_ip: DEVICE_IP,
            macaddress: &self.device.mac,
            googleappid: "",
            screensize: SCREEN_SIZE,
            auth,
        };
        let e = self.endpoints();
        let resp = self
            .http
            .post(format!("{}/apiv2/credentials/loginMini", e.api))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;
        let env: Envelope<LoginResponse> = read_envelope(resp, "login").await?;
        let token = env
            .response_object
            .into_iter()
            .next()
            .map(|r| r.access_token)
            .unwrap_or_default();
        if token.is_empty() {
            return Err(FastwayError::Api(
                "login".to_string(),
                200,
                "login returned no access token".to_string(),
            ));
        }
        let cr = Credentials {
            username: username.to_string(),
            password: password.to_string(),
            access_token: token,
        };
        self.set_credentials(Some(cr.clone()));
        Ok(cr)
    }

    pub async fn logout(&self) -> Result<(), FastwayError> {
        let auth = match self.auth_header() {
            Ok(a) => a,
            Err(_) => return Ok(()),
        };
        let e = self.endpoints();
        let resp = self
            .http
            .post(format!("{}/apiv2/credentials/logoutLite", e.api))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("auth", auth)
            .send()
            .await?;
        let _: Envelope<serde_json::Value> = read_envelope(resp, "logout").await?;
        Ok(())
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        name: &str,
    ) -> Result<Envelope<T>, FastwayError> {
        let auth = self.auth_header()?;
        let e = self.endpoints();
        let resp = self
            .http
            .get(format!("{}{path}", e.api))
            .header("auth", auth)
            .send()
            .await?;
        read_envelope(resp, name).await
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        name: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<Envelope<T>, FastwayError> {
        let auth = self.auth_header()?;
        let e = self.endpoints();
        let mut req = self
            .http
            .post(format!("{}{path}", e.api))
            .header("auth", auth);
        if let Some(body) = body {
            req = req
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(body);
        }
        let resp = req.send().await?;
        read_envelope(resp, name).await
    }

    pub async fn catalogue(&self) -> Result<Catalogue, FastwayError> {
        let genres: HashMap<i64, String> = self
            .get::<Genre>("/apiv2/channels/genre", "genres")
            .await?
            .response_object
            .into_iter()
            .map(|g| (g.id, g.name))
            .collect();
        let subgenres: HashMap<i64, String> = self
            .get::<Subgenre>("/apiv2/channels/subgenre", "subgenres")
            .await?
            .response_object
            .into_iter()
            .map(|s| (s.id, s.name))
            .collect();
        let list = self
            .post::<FastwayChannel>("/apiv2/channels/list", "channels", None)
            .await?
            .response_object;
        let subscribed: HashSet<i64> = self
            .get::<FastwayChannel>("/apiv2/channels/sublist?platform=2", "subscribed channels")
            .await?
            .response_object
            .into_iter()
            .map(|ch| ch.id)
            .collect();
        Ok(Catalogue::build(list, &subscribed, &genres, &subgenres))
    }

    pub async fn epg(&self, numbers: &[i64]) -> Result<HashMap<i64, Vec<Programme>>, FastwayError> {
        let mut out: HashMap<i64, Vec<Programme>> = HashMap::new();
        for chunk in numbers.chunks(100) {
            let body = serde_json::json!({
                "number": chunk,
                "device_timezone": 5.5,
                "timeshift": 2
            });
            let env = self
                .post::<EpgEntry>("/apiv2/channels/epg/", "epg", Some(&body))
                .await?;
            for entry in env.response_object {
                let Some(number) = value_i64(&entry.number) else {
                    continue;
                };
                let (Some(start), Some(end)) = (
                    parse_epoch(&entry.programstart),
                    parse_epoch(&entry.programend),
                ) else {
                    continue;
                };
                if end <= start || entry.title.trim().is_empty() {
                    continue;
                }
                let list = out.entry(number).or_default();
                if list
                    .iter()
                    .any(|p| p.start_epoch == start && p.end_epoch == end && p.title == entry.title)
                {
                    continue;
                }
                list.push(Programme {
                    title: entry.title,
                    description: entry.description,
                    start_epoch: start,
                    end_epoch: end,
                });
            }
        }
        for list in out.values_mut() {
            list.sort_by_key(|p| p.start_epoch);
        }
        Ok(out)
    }

    pub async fn token_issue(&self, content_id: &str) -> Result<String, FastwayError> {
        #[derive(Debug, Clone, Deserialize, Default)]
        struct TokenResponse {
            #[serde(rename = "base64Token", default)]
            base64_token: String,
        }
        let path = format!(
            "/apiv2/pallycon/TokenIssueV3?contentId={}",
            urlencoding::encode(content_id)
        );
        let env = self.get::<TokenResponse>(&path, "pallycon token").await?;
        let token = env
            .response_object
            .into_iter()
            .next()
            .map(|t| t.base64_token)
            .unwrap_or_default();
        if token.is_empty() {
            return Err(FastwayError::Api(
                "pallycon token".to_string(),
                200,
                "token issue returned no token".to_string(),
            ));
        }
        Ok(token)
    }

    pub fn client_meta(&self) -> String {
        use base64::Engine;
        let meta = serde_json::json!({
            "device_info": {
                "device_id": "",
                "device_model": self.device.model,
                "os_version": format!("{}({API_VERSION})", self.device.os_version),
            }
        });
        base64::engine::general_purpose::STANDARD.encode(meta.to_string())
    }
}

async fn read_envelope<T: DeserializeOwned>(
    resp: reqwest::Response,
    name: &str,
) -> Result<Envelope<T>, FastwayError> {
    let status = resp.status();
    if !status.is_success() {
        return Err(FastwayError::Api(
            name.to_string(),
            status.as_u16() as i64,
            status
                .canonical_reason()
                .unwrap_or("HTTP error")
                .to_string(),
        ));
    }
    let env: Envelope<T> = resp.json().await?;
    if env.status_code == 888 || env.error_description.eq_ignore_ascii_case("BAD_TOKEN") {
        return Err(FastwayError::BadToken);
    }
    if env.status_code != 0 && !(200..=202).contains(&env.status_code) {
        return Err(FastwayError::Api(
            name.to_string(),
            env.status_code,
            env.message(),
        ));
    }
    Ok(env)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Parses the EPG's `programstart`/`programend`. Values without a zone are
/// IST (the provider's local time); ISO values with `Z` or a numeric offset
/// keep that offset.
pub fn parse_epoch(value: &str) -> Option<i64> {
    let s = value.trim();
    if s.is_empty() {
        return None;
    }
    let s = s.replacen(' ', "T", 1);
    let (date, rest) = s.split_once('T')?;
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let m: i64 = dp.next()?.parse().ok()?;
    let d: i64 = dp.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }

    let (time, offset) = if let Some(stripped) = rest.strip_suffix('Z') {
        (stripped, 0)
    } else if let Some((time, off)) = rest.rsplit_once('+') {
        (time, parse_offset(off, 1)?)
    } else if let Some((time, off)) = rest.rsplit_once('-') {
        (time, parse_offset(off, -1)?)
    } else {
        (rest, IST_OFFSET_SECS)
    };
    let time = time.split('.').next()?;
    let mut tp = time.split(':');
    let hh: i64 = tp.next()?.parse().ok()?;
    let mm: i64 = tp.next()?.parse().ok()?;
    let ss: i64 = tp.next().unwrap_or("0").parse().ok()?;
    if hh > 23 || mm > 59 || ss > 60 {
        return None;
    }

    Some(days_from_civil(y, m, d) * 86400 + hh * 3600 + mm * 60 + ss - offset)
}

fn parse_offset(value: &str, sign: i64) -> Option<i64> {
    let mut parts = value.split(':');
    let hh: i64 = parts.next()?.parse().ok()?;
    let mm: i64 = parts.next().unwrap_or("0").parse().ok()?;
    Some(sign * (hh * 3600 + mm * 60))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn to_xmltv(
    channel: &FastwayChannel,
    category: &str,
    progs: &[Programme],
) -> Vec<crate::epg::XmlProgramme> {
    progs
        .iter()
        .map(|p| crate::epg::XmlProgramme {
            channel: channel_id(channel.id),
            start: crate::epg::format_xmltv_time(p.start_epoch.saturating_mul(1000)),
            stop: crate::epg::format_xmltv_time(p.end_epoch.saturating_mul(1000)),
            title: p.title.clone(),
            desc: p.description.clone(),
            category: category.to_string(),
            icon: String::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn channel(id: i64, number: i64, title: &str, genre: i64) -> FastwayChannel {
        FastwayChannel {
            id,
            channel_number: number,
            title: title.to_string(),
            genre_id: genre,
            subgenre_id: Some(20),
            channel_mode: "live".to_string(),
            stream_url: format!("http://cdn.example/{id}/index.mpd"),
            encryption: 1,
            encryption_url: format!("CID_{id}"),
            drm_platform: "pallycon".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn id_prefix_round_trips() {
        assert_eq!(channel_id(1568), "fw_1568");
        assert_eq!(content_id("fw_1568"), Some("1568"));
        assert_eq!(content_id("1568"), None);
    }

    #[test]
    fn genre_and_language_mapping_matches_shared_categories() {
        assert_eq!(category_for_genre("SPORTS"), 8);
        assert_eq!(category_for_genre("ENTERTAINMENT"), 5);
        assert_eq!(category_for_genre("MOVIES"), 6);
        assert_eq!(category_for_genre("NEWS"), 12);
        assert_eq!(category_for_genre("MUSIC"), 13);
        assert_eq!(category_for_genre("INFOTAINMENT"), 10);
        assert_eq!(category_for_genre("KIDS"), 7);
        assert_eq!(category_for_genre("DEVOTIONAL"), 15);
        assert_eq!(category_for_genre("REGIONAL"), 20);
        assert_eq!(category_for_genre("RADIO"), 0);
        assert_eq!(language_for_subgenre("PUNJABI"), 3);
        assert_eq!(language_for_subgenre("HINDI"), 1);
        assert_eq!(language_for_subgenre("ENGLISH"), 6);
        assert_eq!(language_for_subgenre("ALL CHANNELS"), 0);
        assert_eq!(language_for_subgenre("MARATHI"), 18);
    }

    #[test]
    fn catalogue_keeps_only_subscribed_live_non_radio_rows() {
        let mut genres = HashMap::new();
        genres.insert(50, "DEVOTIONAL".to_string());
        genres.insert(52, "RADIO".to_string());
        genres.insert(666, "Favorites".to_string());
        let mut subgenres = HashMap::new();
        subgenres.insert(20, "PUNJABI".to_string());
        subgenres.insert(21, "HINDI".to_string());

        let subscribed: HashSet<i64> = [1, 2, 3].into_iter().collect();
        let mut off_air = channel(3, 3, "Off Air", 50);
        off_air.channel_mode = "vod".to_string();
        let list = vec![
            channel(1, 554, "ANANDPUR LIVE", 50),
            channel(2, 555, "RADIO ONE", 52),
            off_air,
            channel(4, 556, "UNSUBSCRIBED", 50),
            channel(5, 557, "FAVORITE", 666),
        ];
        let cat = Catalogue::build(list, &subscribed, &genres, &subgenres);
        assert_eq!(cat.channels.len(), 1);
        let ch = &cat.channels[0];
        assert_eq!(ch.id, 1);
        assert_eq!(ch.category, 15);
        assert_eq!(ch.language, 3);
        assert_eq!(cat.by_id.get(&1).unwrap().title, "ANANDPUR LIVE");
    }

    #[test]
    fn catalogue_marks_hd_by_title_word() {
        let mut genres = HashMap::new();
        genres.insert(50, "DEVOTIONAL".to_string());
        let subscribed: HashSet<i64> = [1, 2].into_iter().collect();
        let list = vec![
            channel(1, 1, "ANANDPUR LIVE HD", 50),
            channel(2, 2, "SHAHEEDAN LIVE", 50),
        ];
        let cat = Catalogue::build(list, &subscribed, &genres, &HashMap::new());
        assert!(cat.by_id.get(&1).unwrap().is_hd);
        assert!(!cat.by_id.get(&2).unwrap().is_hd);
    }

    #[test]
    fn parses_ist_timestamps_with_and_without_a_zone() {
        let expected = days_from_civil(2026, 10, 28) * 86400 + 15 * 3600 - IST_OFFSET_SECS;
        assert_eq!(parse_epoch("2026-10-28 15:00:00"), Some(expected));
        assert_eq!(parse_epoch("2026-10-28T15:00:00"), Some(expected));
        assert_eq!(parse_epoch("2026-10-28T15:00:00+05:30"), Some(expected));
        assert_eq!(parse_epoch("2026-10-28T09:30:00Z"), Some(expected));
        assert_eq!(parse_epoch(""), None);
        assert_eq!(parse_epoch("not a date"), None);
    }

    #[tokio::test]
    async fn epg_uses_the_fixed_timezone_body_and_skips_bad_rows() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/apiv2/channels/epg/"))
            .and(header("auth", "auth=redacted-token"))
            .and(body_string_contains("\"device_timezone\":5.5"))
            .and(body_string_contains("\"timeshift\":2"))
            .and(body_string_contains("\"number\":[554,555]"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status_code": 200,
                "response_object": [
                    {"number": 554, "title": "Late", "description": "d",
                     "programstart": "2026-10-28 15:00:00",
                     "programend": "2026-10-28 15:30:00"},
                    {"number": 554, "title": "Late", "description": "d",
                     "programstart": "2026-10-28 15:00:00",
                     "programend": "2026-10-28 15:30:00"},
                    {"number": 554, "title": "Early",
                     "programstart": "2026-10-28T14:00:00+05:30",
                     "programend": "2026-10-28T14:30:00+05:30"},
                    {"number": "555", "title": "Zoned",
                     "programstart": "2026-10-28T09:30:00Z",
                     "programend": "2026-10-28T10:00:00Z"},
                    {"number": 555, "title": "",
                     "programstart": "2026-10-28T10:00:00Z",
                     "programend": "2026-10-28T11:00:00Z"},
                    {"number": 555, "title": "Backwards",
                     "programstart": "2026-10-28T12:00:00Z",
                     "programend": "2026-10-28T11:00:00Z"},
                    {"number": 555, "title": "Broken",
                     "programstart": "not a date",
                     "programend": "2026-10-28T11:00:00Z"}
                ]
            })))
            .mount(&server)
            .await;

        let client = Client::new(reqwest::Client::new(), Device::new());
        client.set_endpoints(Endpoints {
            api: server.uri(),
            license: server.uri(),
        });
        client.set_credentials(Some(Credentials {
            username: "redacted-user".into(),
            password: "redacted-pass".into(),
            access_token: "redacted-token".into(),
        }));

        let guide = client.epg(&[554, 555]).await.unwrap();
        let late = &guide.get(&554).unwrap();
        assert_eq!(late.len(), 2);
        assert_eq!(late[0].title, "Early");
        assert_eq!(late[1].title, "Late");
        assert_eq!(
            late[1].start_epoch,
            days_from_civil(2026, 10, 28) * 86400 + 15 * 3600 - IST_OFFSET_SECS
        );
        let zoned = &guide.get(&555).unwrap();
        assert_eq!(zoned.len(), 1);
        assert_eq!(zoned[0].title, "Zoned");
        assert_eq!(
            zoned[0].start_epoch,
            days_from_civil(2026, 10, 28) * 86400 + 9 * 3600 + 30 * 60
        );
    }

    #[test]
    fn token_issue_url_is_built_only_for_encrypted_channels() {
        let ch = channel(1, 1, "A", 50);
        assert_eq!(
            ch.token_issue_url(API_BASE),
            format!("{API_BASE}/apiv2/pallycon/TokenIssueV3?contentId=CID_1")
        );
        let mut clear = ch;
        clear.encryption_url.clear();
        assert!(clear.token_issue_url(API_BASE).is_empty());
        assert!(!clear.drm());
    }

    #[test]
    fn client_meta_is_base64_device_json() {
        use base64::Engine;
        let client = Client::new(reqwest::Client::new(), Device::new());
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(client.client_meta())
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(v["device_info"]["device_id"], "");
        assert_eq!(v["device_info"]["device_model"], "AFTKA");
        assert_eq!(v["device_info"]["os_version"], "14(34)");
    }

    #[test]
    fn device_round_trips_through_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let d1 = Device::load_or_create(&store).unwrap();
        let d2 = Device::load_or_create(&store).unwrap();
        assert_eq!(d1.boxid, d2.boxid);
        assert_eq!(d1.mac, d2.mac);
    }

    #[test]
    fn credentials_round_trip_through_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let cr = Credentials {
            username: "redacted-user".into(),
            password: "redacted-pass".into(),
            access_token: "redacted-token".into(),
        };
        cr.save(&store).unwrap();
        assert_eq!(Credentials::load(&store).unwrap().username, "redacted-user");
        delete_credentials(&store).unwrap();
        assert!(Credentials::load(&store).is_none());
    }

    #[tokio::test]
    async fn login_catalogue_and_token_flow_against_mock_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/apiv2/credentials/loginMini"))
            .and(body_string_contains("username=redacted-user"))
            .and(body_string_contains("password=redacted-pass"))
            .and(body_string_contains("appid=1;timestamp="))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status_code": 200,
                "error_code": 1,
                "error_description": "OK",
                "response_object": [{"access_token": "redacted-token"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/apiv2/channels/genre"))
            .and(header("auth", "auth=redacted-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status_code": 200,
                "response_object": [
                    {"id": 50, "name": "DEVOTIONAL", "order": 8},
                    {"id": 52, "name": "RADIO", "order": 11}
                ]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/apiv2/channels/subgenre"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status_code": 200,
                "response_object": [{"id": 20, "name": "PUNJABI"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/apiv2/channels/list"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status_code": 200,
                "response_object": [
                    {"id": 1, "genre_id": 50, "channel_number": 554, "title": "A",
                     "stream_url": "http://cdn/a.mpd", "channel_mode": "live",
                     "subgenre_id": 20, "encryption_url": "CID_A"},
                    {"id": 2, "genre_id": 52, "channel_number": 555, "title": "R",
                     "stream_url": "http://cdn/r.mpd", "channel_mode": "live"}
                ]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/apiv2/channels/sublist"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status_code": 200,
                "response_object": [{"id": 1}, {"id": 2}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/apiv2/pallycon/TokenIssueV3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "error_code": 1,
                "response_object": [{"base64Token": "redacted-pallycon"}]
            })))
            .mount(&server)
            .await;

        let client = Client::new(reqwest::Client::new(), Device::new());
        client.set_endpoints(Endpoints {
            api: server.uri(),
            license: server.uri(),
        });
        let cr = client
            .login("redacted-user", "redacted-pass")
            .await
            .unwrap();
        assert_eq!(cr.access_token, "redacted-token");

        let cat = client.catalogue().await.unwrap();
        assert_eq!(cat.channels.len(), 1);
        assert_eq!(cat.channels[0].title, "A");

        assert_eq!(
            client.token_issue("CID_A").await.unwrap(),
            "redacted-pallycon"
        );
    }

    #[tokio::test]
    async fn bad_token_is_distinct_from_other_api_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/apiv2/channels/genre"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status_code": 888,
                "error_description": "BAD_TOKEN",
                "extra_data": "Token could not be decrypted",
                "response_object": []
            })))
            .mount(&server)
            .await;
        let client = Client::new(reqwest::Client::new(), Device::new());
        client.set_endpoints(Endpoints {
            api: server.uri(),
            license: server.uri(),
        });
        client.set_credentials(Some(Credentials {
            username: "u".into(),
            password: "p".into(),
            access_token: "expired".into(),
        }));
        assert!(matches!(
            client.catalogue().await,
            Err(FastwayError::BadToken)
        ));
    }

    #[tokio::test]
    async fn login_error_reports_the_provider_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/apiv2/credentials/loginMini"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status_code": 704,
                "error_description": "WRONG_PASSWORD",
                "extra_data": "Username and password do not match",
                "response_object": []
            })))
            .mount(&server)
            .await;
        let client = Client::new(reqwest::Client::new(), Device::new());
        client.set_endpoints(Endpoints {
            api: server.uri(),
            license: server.uri(),
        });
        let err = client.login("u", "p").await.unwrap_err();
        assert!(err.to_string().contains("WRONG_PASSWORD"));
        assert!(err.to_string().contains("do not match"));
    }
}
