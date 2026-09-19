//! JioTV+ (JioFiber/AirFiber set-top box) API client. Mirrors `pkg/tvplus`
//! in the Go tree: device identity, OTP login, token exchange/refresh,
//! catalogue, playback, EPG. Behind the `tvplus` config flag, off by
//! default (see `src/tvplus_state.rs` for the routing/caching layer that
//! actually wires this into the server).

use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const ID_PREFIX: &str = "tvp_";
pub const PLAYER_USER_AGENT: &str = "JioTV.Plus/6.0.8 (Linux;Android 12) AndroidXMedia3/1.4.1";

const API_SIGNATURE: &str = "37ca682625d7";
const FEATURE_CODE: &str = "ce1eb674jdkc";
const APP_NAME: &str = "RJIL_JioTVPlus";
const PLATFORM: &str = "androidtv";
const VERSION_CODE: &str = "6008";
const CLIENT_USER_AGENT: &str = "ktor-client";

const LOGIN_API_KEY: &str = "l7xx61fae40fe3af4c93b02792ae12422a82";
const LOGIN_APP_KEY: &str = "NzNiMDhlYzQyNjJm";
const LOGIN_USER_GROUP: &str = "tvYR7NSNn7rymo3F";
const LOGIN_SESSION_ID: &str = "fa06b053-5b38-4c5b-b9f0-6459827b";

/// Store key names, kept byte-identical to the Go tree so an existing
/// `store_v4.toml` (device + saved login) keeps working.
pub const STORE_KEY_DEVICE: &str = "tvplus_device";
pub const STORE_KEY_CREDENTIALS: &str = "tvplus_credentials";
pub const STORE_KEY_DASH: &str = "tvplus_dash";

/// The device identity presented to JioTV+. JSON field names match the Go
/// struct's tags exactly (store compatibility).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    #[serde(rename = "androidId")]
    pub android_id: String,
    pub model: String,
    pub manufacturer: String,
    #[serde(rename = "osVersion")]
    pub os_version: String,
}

impl Device {
    pub fn new() -> Device {
        let mut bytes = [0u8; 8];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
        Device {
            android_id: hex::encode(bytes),
            model: "AFTKA".to_string(),
            manufacturer: "Amazon".to_string(),
            os_version: "9".to_string(),
        }
    }

    pub fn load_or_create(store: &crate::store::Store) -> anyhow::Result<Device> {
        if let Some(json) = store.get_opt(STORE_KEY_DEVICE) {
            if let Ok(d) = serde_json::from_str::<Device>(&json) {
                if !d.android_id.is_empty() {
                    return Ok(d);
                }
            }
        }
        let d = Device::new();
        store.set(STORE_KEY_DEVICE, &serde_json::to_string(&d)?)?;
        Ok(d)
    }
}

/// Tokens from a completed login. JSON field names match the Go struct's
/// tags exactly (store compatibility).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub number: String,
    #[serde(rename = "ssoToken", default)]
    pub sso_token: String,
    #[serde(rename = "subscriberId", default)]
    pub subscriber_id: String,
    #[serde(default)]
    pub unique: String,
    #[serde(rename = "userId", default)]
    pub user_id: String,
    #[serde(rename = "authToken", default)]
    pub auth_token: String,
    #[serde(rename = "refreshToken", default)]
    pub refresh_token: String,
}

impl Credentials {
    pub fn auth_token_expiry(&self) -> Option<SystemTime> {
        let parts: Vec<&str> = self.auth_token.split('.').collect();
        if parts.len() < 2 {
            return None;
        }
        let mut payload = parts[1].to_string();
        while payload.len() % 4 != 0 {
            payload.push('=');
        }
        use base64::Engine;
        let decoded = base64::engine::general_purpose::URL_SAFE.decode(&payload).ok()?;
        let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
        let exp = claims.get("exp")?.as_u64()?;
        Some(UNIX_EPOCH + Duration::from_secs(exp))
    }

    pub fn needs_refresh(&self, now: SystemTime, margin: Duration) -> bool {
        match self.auth_token_expiry() {
            Some(exp) => now + margin >= exp,
            None => true,
        }
    }

    pub fn load(store: &crate::store::Store) -> Option<Credentials> {
        let json = store.get_opt(STORE_KEY_CREDENTIALS)?;
        let cr: Credentials = serde_json::from_str(&json).ok()?;
        if cr.sso_token.is_empty() {
            None
        } else {
            Some(cr)
        }
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

/// Maps a TV+ channel ID (`tvp_<contentId>`) to its content ID, or `None` for
/// a plain JioTV ID.
pub fn content_id(channel_id: &str) -> Option<&str> {
    channel_id.strip_prefix(ID_PREFIX)
}

pub fn channel_id(content_id: &str) -> String {
    format!("{ID_PREFIX}{content_id}")
}

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub content: String,
    pub user_api: String,
    pub auth: String,
    pub send_otp: String,
    pub verify_otp: String,
    pub user_service: String,
    pub token: String,
}

impl Default for Endpoints {
    fn default() -> Endpoints {
        Endpoints {
            content: "https://content-jiotvplus.media.jio.com".to_string(),
            user_api: "https://api-jiotvplus.media.jio.com".to_string(),
            auth: "https://tv.media.jio.com".to_string(),
            send_otp: "apis/v3.2/stbotplogin/sendotp".to_string(),
            verify_otp: "apis/v3.2/stbotplogin/verifyotp".to_string(),
            user_service: "https://jiotvapi.media.jio.com/userservice/apis/v1".to_string(),
            token: "https://auth.media.jio.com/tokenservice/apis/v1.1".to_string(),
        }
    }
}

fn common_headers() -> Vec<(&'static str, String)> {
    vec![
        ("x-apisignatures", API_SIGNATURE.to_string()),
        ("x-feature-code", FEATURE_CODE.to_string()),
        ("x-platform", PLATFORM.to_string()),
    ]
}

#[derive(Debug, thiserror::Error)]
pub enum TvPlusError {
    #[error("tvplus: mobile number must have 10 digits")]
    BadNumber,
    #[error("tvplus: not logged in")]
    NotLoggedIn,
    #[error("tvplus: not subscribed")]
    NotSubscribed,
    #[error("tvplus: {0} returned HTTP {1}")]
    Api(String, u16),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn normalize_number(number: &str) -> Result<String, TvPlusError> {
    let n = number.trim().strip_prefix("+91").unwrap_or(number.trim());
    if n.len() != 10 || !n.chars().all(|c| c.is_ascii_digit()) {
        return Err(TvPlusError::BadNumber);
    }
    Ok(n.to_string())
}

#[derive(Debug, Deserialize)]
pub struct FttxProduct {
    #[serde(rename = "productName", default)]
    pub product_name: String,
    #[serde(default)]
    pub identifier: Vec<FttxIdentifier>,
}

#[derive(Debug, Deserialize)]
pub struct FttxIdentifier {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct FttxId {
    #[serde(rename = "firstName", default)]
    pub first_name: String,
    #[serde(default)]
    pub products: Vec<FttxProduct>,
}

#[derive(Debug, Deserialize, Default)]
pub struct SendOtpResponse {
    #[serde(default)]
    pub identifier: String,
    #[serde(rename = "fttxIds", default)]
    pub fttx_ids: Vec<FttxId>,
}

#[derive(Debug, Clone)]
pub struct Connection {
    pub name: String,
    pub identifier: String,
    pub product_name: String,
}

impl SendOtpResponse {
    /// Flattens the fttx list into selectable MSISDN/FLN connections, the
    /// same identifiers the app offers (AirFiber-only R4GID lines excluded).
    pub fn connections(&self) -> Vec<Connection> {
        let mut out = Vec::new();
        for f in &self.fttx_ids {
            for p in &f.products {
                for id in &p.identifier {
                    let name = id.name.to_uppercase();
                    if (name == "MSISDN" || name == "FLN") && !id.value.is_empty() {
                        out.push(Connection {
                            name: f.first_name.clone(),
                            identifier: id.value.clone(),
                            product_name: p.product_name.clone(),
                        });
                    }
                }
            }
        }
        out
    }
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

    /// Overrides the base URLs, for tests against a mock server.
    #[cfg(test)]
    pub fn set_endpoints(&self, e: Endpoints) {
        *self.endpoints.write().unwrap() = e;
    }

    pub fn credentials(&self) -> Option<Credentials> {
        self.creds.read().unwrap().clone()
    }

    pub fn set_credentials(&self, cr: Option<Credentials>) {
        *self.creds.write().unwrap() = cr;
    }

    fn endpoints(&self) -> Endpoints {
        self.endpoints.read().unwrap().clone()
    }

    fn login_headers(&self) -> Vec<(String, String)> {
        vec![
            ("app-name".into(), APP_NAME.into()),
            ("x-api-key".into(), LOGIN_API_KEY.into()),
            ("x-platform".into(), PLATFORM.into()),
            ("appkey".into(), LOGIN_APP_KEY.into()),
            ("devicetype".into(), "phone".into()),
            ("os".into(), "android".into()),
            ("deviceId".into(), self.device.android_id.clone()),
            ("uniqueId".into(), self.device.android_id.clone()),
            ("osVersion".into(), self.device.os_version.clone()),
            ("dm".into(), self.device.model.clone()),
            ("usergroup".into(), LOGIN_USER_GROUP.into()),
            ("languageId".into(), "6".into()),
            ("userId".into(), "".into()),
            ("sid".into(), LOGIN_SESSION_ID.into()),
            ("crmid".into(), "".into()),
            ("isott".into(), "false".into()),
            ("channel_id".into(), "-1".into()),
            ("langid".into(), "".into()),
            ("camid".into(), "1".into()),
            ("m-rating".into(), "100".into()),
            ("ssotoken".into(), "".into()),
            ("subscriberId".into(), "".into()),
            ("lbcookie".into(), "1".into()),
            ("versionCode".into(), VERSION_CODE.into()),
        ]
    }

    pub async fn send_otp(&self, number: &str, identifier_id: &str) -> Result<SendOtpResponse, TvPlusError> {
        let n = normalize_number(number)?;
        let e = self.endpoints();
        let mut req = self.http.post(format!("{}/{}", e.auth, e.send_otp)).header(reqwest::header::USER_AGENT, CLIENT_USER_AGENT);
        for (k, v) in self.login_headers() {
            req = req.header(&k, v);
        }
        req = req.header("number", n).header("identifierid", identifier_id);
        let resp = req.send().await?;
        check_status(&resp, "sendotp")?;
        Ok(resp.json().await?)
    }

    pub async fn verify_otp(&self, number: &str, identifier: &str, otp: &str) -> Result<Credentials, TvPlusError> {
        let n = normalize_number(number)?;
        #[derive(Serialize)]
        struct PlatformInfo {
            name: String,
        }
        #[derive(Serialize)]
        struct Info {
            #[serde(rename = "androidId")]
            android_id: String,
            platform: PlatformInfo,
            #[serde(rename = "type")]
            kind: String,
        }
        #[derive(Serialize)]
        struct DeviceInfo {
            #[serde(rename = "consumptionDeviceName")]
            consumption_device_name: String,
            info: Info,
        }
        #[derive(Serialize)]
        struct Body {
            #[serde(rename = "deviceInfo")]
            device_info: DeviceInfo,
            identifier: String,
            otp: String,
            #[serde(rename = "rememberUser")]
            remember_user: String,
            #[serde(rename = "upgradeAuth")]
            upgrade_auth: String,
        }
        let body = Body {
            device_info: DeviceInfo {
                consumption_device_name: self.device.model.clone(),
                info: Info {
                    android_id: self.device.android_id.clone(),
                    platform: PlatformInfo { name: self.device.model.clone() },
                    kind: "android".to_string(),
                },
            },
            identifier: identifier.to_string(),
            otp: otp.trim().to_string(),
            remember_user: "T".to_string(),
            upgrade_auth: "Y".to_string(),
        };

        #[derive(Deserialize)]
        struct SessionUser {
            #[serde(rename = "subscriberId", default)]
            subscriber_id: String,
            #[serde(default)]
            unique: String,
        }
        #[derive(Deserialize)]
        struct SessionAttributes {
            user: SessionUser,
        }
        #[derive(Deserialize)]
        struct VerifyOtpResponse {
            #[serde(rename = "ssoToken", default)]
            sso_token: String,
            #[serde(rename = "sessionAttributes")]
            session_attributes: SessionAttributes,
        }

        let e = self.endpoints();
        let mut req = self.http.post(format!("{}/{}", e.auth, e.verify_otp)).header(reqwest::header::USER_AGENT, CLIENT_USER_AGENT);
        for (k, v) in self.login_headers() {
            req = req.header(&k, v);
        }
        let resp = req.json(&body).send().await?;
        check_status(&resp, "verifyotp")?;
        let v: VerifyOtpResponse = resp.json().await?;
        if v.sso_token.is_empty() {
            return Err(TvPlusError::Api("verifyotp returned no ssoToken".to_string(), 200));
        }
        let mut cr = Credentials {
            number: n,
            sso_token: v.sso_token,
            subscriber_id: v.session_attributes.user.subscriber_id,
            unique: v.session_attributes.user.unique,
            ..Default::default()
        };
        self.set_credentials(Some(cr.clone()));
        match self.exchange_token().await {
            Ok(()) => {
                cr = self.credentials().unwrap();
            }
            Err(e) => {
                // The SSO token is still valid even if the exchange failed;
                // the caller saves it so the OTP isn't wasted.
                return Err(e);
            }
        }
        Ok(cr)
    }

    pub async fn exchange_token(&self) -> Result<(), TvPlusError> {
        let mut cr = self.credentials().ok_or(TvPlusError::NotLoggedIn)?;
        if cr.sso_token.is_empty() {
            return Err(TvPlusError::NotLoggedIn);
        }
        use base64::Engine;
        let number_b64 = base64::engine::general_purpose::STANDARD.encode(format!("+91{}", cr.number));
        #[derive(Serialize)]
        struct Body {
            number: String,
        }
        #[derive(Deserialize, Default)]
        struct Resp {
            #[serde(rename = "authToken", default)]
            auth_token: String,
            #[serde(rename = "refreshToken", default)]
            refresh_token: String,
            #[serde(rename = "userId", default)]
            user_id: String,
            #[serde(rename = "subscriberId", default)]
            subscriber_id: String,
        }
        let e = self.endpoints();
        let resp = self
            .http
            .post(format!("{}/loginotp/exchangetoken", e.user_service))
            .header("ssotoken", &cr.sso_token)
            .header("appname", APP_NAME)
            .header("deviceid", &self.device.android_id)
            .header("devicetype", "tv")
            .header("os", "android")
            .header("subscriberid", &cr.subscriber_id)
            .header("persistentRefreshToken", "true")
            .header("x-platform", PLATFORM)
            .json(&Body { number: number_b64 })
            .send()
            .await?;
        check_status(&resp, "exchangetoken")?;
        let x: Resp = resp.json().await?;
        if x.auth_token.is_empty() {
            return Err(TvPlusError::Api("exchangetoken returned no authToken".to_string(), 200));
        }
        cr.auth_token = x.auth_token;
        cr.refresh_token = x.refresh_token;
        cr.user_id = x.user_id;
        if !x.subscriber_id.is_empty() {
            cr.subscriber_id = x.subscriber_id;
        }
        self.set_credentials(Some(cr));
        Ok(())
    }

    pub async fn refresh(&self) -> Result<(), TvPlusError> {
        let mut cr = self.credentials().ok_or(TvPlusError::NotLoggedIn)?;
        if cr.refresh_token.is_empty() {
            return Err(TvPlusError::NotLoggedIn);
        }
        #[derive(Serialize)]
        struct Body {
            #[serde(rename = "refreshToken")]
            refresh_token: String,
            #[serde(rename = "appName")]
            app_name: String,
            #[serde(rename = "deviceId")]
            device_id: String,
        }
        #[derive(Deserialize, Default)]
        struct Resp {
            #[serde(rename = "authToken", default)]
            auth_token: String,
            #[serde(rename = "refreshToken", default)]
            refresh_token: String,
        }
        let e = self.endpoints();
        let resp = self
            .http
            .post(format!("{}/refreshtoken", e.token))
            .header("accesstoken", &cr.auth_token)
            .header("x-platform", PLATFORM)
            .header("os", "android")
            .header("devicetype", "tv")
            .json(&Body {
                refresh_token: cr.refresh_token.clone(),
                app_name: APP_NAME.to_string(),
                device_id: self.device.android_id.clone(),
            })
            .send()
            .await?;
        check_status(&resp, "refreshtoken")?;
        let out: Resp = resp.json().await?;
        if out.auth_token.is_empty() {
            return Err(TvPlusError::Api("refreshtoken returned no authToken".to_string(), 200));
        }
        cr.auth_token = out.auth_token;
        if !out.refresh_token.is_empty() {
            cr.refresh_token = out.refresh_token;
        }
        self.set_credentials(Some(cr));
        Ok(())
    }

    pub async fn channels(&self) -> Result<Vec<LiveChannel>, TvPlusError> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            data: std::collections::HashMap<String, LiveChannel>,
        }
        let e = self.endpoints();
        let mut req = self.http.get(format!("{}/metadata/v2/livechannels", e.content));
        for (k, v) in common_headers() {
            req = req.header(k, v);
        }
        req = req.header("x-page", "LiveTv");
        let resp = req.send().await?;
        check_status(&resp, "livechannels")?;
        let r: Resp = resp.json().await?;
        let mut out: Vec<LiveChannel> = r
            .data
            .into_iter()
            .map(|(id, mut ch)| {
                if ch.content_id.is_empty() {
                    ch.content_id = id;
                }
                ch
            })
            .collect();
        out.sort_by(|a, b| a.channel_number.cmp(&b.channel_number).then_with(|| a.content_id.cmp(&b.content_id)));
        Ok(out)
    }

    pub async fn playback(&self, content_id: &str) -> Result<PlaybackResponse, TvPlusError> {
        let cr = self.credentials().ok_or(TvPlusError::NotLoggedIn)?;
        if cr.auth_token.is_empty() {
            return Err(TvPlusError::NotLoggedIn);
        }
        let cid = content_id::content_id(content_id).unwrap_or(content_id);
        let e = self.endpoints();
        let mut req = self.http.post(format!("{}/playback/v2/{}", e.user_api, urlencoding::encode(cid)));
        for (k, v) in common_headers() {
            req = req.header(k, v);
        }
        req = req
            .header("x-page", "Player")
            .header("rmn", &cr.number)
            .header("deviceId", &self.device.android_id)
            .header("ssotoken", &cr.sso_token)
            .header("uniqueid", &cr.user_id)
            .header("subId", &cr.subscriber_id)
            .header("x-accesstoken", &cr.auth_token);

        #[derive(Serialize)]
        struct Body {
            #[serde(rename = "bitrateProfile")]
            bitrate_profile: String,
            model: String,
            manufacturer: String,
            #[serde(rename = "osVersion")]
            os_version: String,
            #[serde(rename = "serialNo")]
            serial_no: String,
            #[serde(rename = "is4kSupport")]
            is_4k_support: bool,
            #[serde(rename = "hevcSupport")]
            hevc_support: bool,
        }
        let body = Body {
            bitrate_profile: "xhdpi".to_string(),
            model: self.device.model.clone(),
            manufacturer: self.device.manufacturer.clone(),
            os_version: self.device.os_version.clone(),
            serial_no: self.device.android_id.clone(),
            is_4k_support: true,
            hevc_support: true,
        };
        let resp = req.json(&body).send().await?;
        if resp.status().as_u16() == 401 {
            return Err(TvPlusError::NotSubscribed);
        }
        check_status(&resp, "playback")?;
        let r: PlaybackResponse = resp.json().await?;
        if r.code == 401 {
            return Err(TvPlusError::NotSubscribed);
        }
        Ok(r)
    }

    pub async fn epg(&self, content_ids: &[String], offsets: &[i64]) -> Result<std::collections::HashMap<String, Vec<Programme>>, TvPlusError> {
        let mut out = std::collections::HashMap::new();
        if content_ids.is_empty() {
            return Ok(out);
        }
        let e = self.endpoints();
        let offs = offsets.iter().map(i64::to_string).collect::<Vec<_>>().join(", ");
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            data: std::collections::HashMap<String, Vec<Programme>>,
        }
        for chunk in content_ids.chunks(100) {
            let ids_json = serde_json::to_string(chunk)?;
            let mut req = self.http.get(format!("{}/metadata/v2/livechannels/epg", e.content)).query(&[
                ("contentIds", ids_json),
                ("offsets", format!("[{offs}]")),
            ]);
            for (k, v) in common_headers() {
                req = req.header(k, v);
            }
            req = req.header("x-page", "Player");
            let resp = req.send().await?;
            check_status(&resp, "epg")?;
            let r: Resp = resp.json().await?;
            for (id, mut progs) in r.data {
                out.entry(id).or_insert_with(Vec::new).append(&mut progs);
            }
        }
        Ok(out)
    }

    pub fn key_headers(&self, ext_id: &str) -> Vec<(String, String)> {
        let cr = match self.credentials() {
            Some(c) => c,
            None => return Vec::new(),
        };
        vec![
            ("ssotoken".into(), cr.sso_token),
            ("accesstoken".into(), cr.auth_token),
            ("srno".into(), "230203144000".into()),
            ("channelId".into(), ext_id.to_string()),
            ("subscriberid".into(), cr.subscriber_id.clone()),
            ("crmid".into(), cr.subscriber_id),
            ("uniqueId".into(), cr.unique),
            ("deviceId".into(), self.device.android_id.clone()),
            ("appkey".into(), LOGIN_APP_KEY.into()),
            ("usergroup".into(), LOGIN_USER_GROUP.into()),
            ("os".into(), "android".into()),
            ("devicetype".into(), "phone".into()),
            ("versionCode".into(), "422".into()),
        ]
    }

    pub fn license_headers(&self, content_id: &str, playback_token: &str) -> Vec<(String, String)> {
        let cr = match self.credentials() {
            Some(c) => c,
            None => return Vec::new(),
        };
        vec![
            ("os".into(), "android".into()),
            ("playbackToken".into(), playback_token.to_string()),
            ("srno".into(), "230203144000".into()),
            ("usergroup".into(), "474537347347373".into()),
            ("deviceid".into(), self.device.android_id.clone()),
            ("channelid".into(), content_id.to_string()),
            ("versionCode".into(), VERSION_CODE.into()),
            ("devicetype".into(), "tv".into()),
            ("uniqueid".into(), cr.user_id),
            ("ssotoken".into(), cr.sso_token),
            ("subscriberid".into(), cr.subscriber_id.clone()),
            ("crmid".into(), cr.subscriber_id),
        ]
    }
}

fn check_status(resp: &reqwest::Response, name: &str) -> Result<(), TvPlusError> {
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(TvPlusError::Api(name.to_string(), resp.status().as_u16()))
    }
}

pub mod content_id {
    pub fn content_id(channel_id: &str) -> Option<&str> {
        super::content_id(channel_id)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LiveChannel {
    #[serde(rename = "contentId", default)]
    pub content_id: String,
    #[serde(rename = "extId", default)]
    pub ext_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub provider: String,
    #[serde(rename = "playbackType", default)]
    pub playback_type: String,
    #[serde(rename = "channelNumber", default)]
    pub channel_number: i64,
    #[serde(default)]
    pub thumbnail: String,
    #[serde(rename = "logoUrl", default)]
    pub logo_url: String,
}

impl LiveChannel {
    pub fn is_test_channel(&self) -> bool {
        self.name.to_lowercase().contains("test")
    }

    pub fn to_channel(&self) -> crate::television::Channel {
        let logo = if !self.logo_url.is_empty() { self.logo_url.clone() } else { self.thumbnail.clone() };
        crate::television::Channel {
            id: channel_id(&self.content_id),
            name: self.name.clone(),
            logo_url: logo,
            category: category_id(&self.genres),
            language: language_id(&self.language),
            is_hd: self.quality.eq_ignore_ascii_case("hd"),
            ..Default::default()
        }
    }
}

fn category_id(genres: &[String]) -> i64 {
    for g in genres {
        if let Some(id) = lookup(crate::television::CATEGORY_MAP, g) {
            return id;
        }
    }
    0
}

fn language_id(lang: &str) -> i64 {
    lookup(crate::television::LANGUAGE_MAP, lang).unwrap_or(18)
}

fn lookup(map: &[(i64, &str)], name: &str) -> Option<i64> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    map.iter().find(|(id, v)| *id != 0 && v.eq_ignore_ascii_case(name)).map(|(id, _)| *id)
}

/// Lowercases and strips everything but letters/digits, reading "&" as
/// "and" — used to match a TV+ channel to a JioTV one by name when there's
/// no `extId`.
pub fn normalize_name(s: &str) -> String {
    s.to_lowercase()
        .replace('&', "and")
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

/// The TV+ channels JioTV doesn't carry (no matching `extId` or normalized
/// name), mapped onto JioTV Go's channel type. Mirrors `tvplus.Exclusive`.
pub fn exclusive(tvplus: &[LiveChannel], jiotv: &[crate::television::Channel]) -> Vec<crate::television::Channel> {
    let ids: std::collections::HashSet<&str> = jiotv.iter().map(|c| c.id.as_str()).collect();
    let names: std::collections::HashSet<String> = jiotv.iter().map(|c| normalize_name(&c.name)).collect();
    tvplus
        .iter()
        .filter(|ch| !ch.is_test_channel() && ch.playback_type != "deeplink")
        .filter(|ch| ch.ext_id.is_empty() || !ids.contains(ch.ext_id.as_str()))
        .filter(|ch| !names.contains(&normalize_name(&ch.name)))
        .map(|ch| ch.to_channel())
        .collect()
}

/// Maps each JioTV channel ID that TV+ also carries to its TV+ content ID.
/// Mirrors `tvplus.Mirrors`.
pub fn mirrors(tvplus: &[LiveChannel], jiotv: &[crate::television::Channel]) -> std::collections::HashMap<String, String> {
    let mut by_ext_id = std::collections::HashMap::new();
    let mut by_name = std::collections::HashMap::new();
    for ch in tvplus {
        if ch.is_test_channel() || ch.playback_type == "deeplink" {
            continue;
        }
        if !ch.ext_id.is_empty() {
            by_ext_id.insert(ch.ext_id.clone(), ch.content_id.clone());
        }
        let name = normalize_name(&ch.name);
        if !name.is_empty() {
            by_name.entry(name).or_insert_with(|| ch.content_id.clone());
        }
    }
    let mut out = std::collections::HashMap::new();
    for ch in jiotv {
        if let Some(id) = by_ext_id.get(&ch.id) {
            out.insert(ch.id.clone(), id.clone());
        } else if let Some(id) = by_name.get(&normalize_name(&ch.name)) {
            out.insert(ch.id.clone(), id.clone());
        }
    }
    out
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Streams {
    #[serde(default)]
    pub auto: String,
    #[serde(default)]
    pub high: String,
    #[serde(default)]
    pub medium: String,
    #[serde(default)]
    pub low: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PlaybackMpd {
    #[serde(default)]
    pub auto: String,
    #[serde(default)]
    pub high: String,
    #[serde(default)]
    pub medium: String,
    #[serde(default)]
    pub low: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PlaybackData {
    #[serde(rename = "contentId", default)]
    pub content_id: String,
    #[serde(rename = "extID", default)]
    pub ext_id: String,
    #[serde(default)]
    pub m3u8: Streams,
    #[serde(default)]
    pub mpd: PlaybackMpd,
    #[serde(rename = "keyURL", default)]
    pub key_url: String,
    #[serde(rename = "algoName", default)]
    pub algo_name: String,
    #[serde(rename = "playbackToken", default)]
    pub playback_token: String,
    #[serde(default)]
    pub algo: i64,
    #[serde(rename = "nl", default)]
    pub nl: String,
    #[serde(rename = "playbackUrl", default)]
    pub playback_url: String,
    #[serde(default)]
    pub provider: String,
    #[serde(rename = "totalDuration", default)]
    pub total_duration: i64,
    #[serde(default)]
    pub name: String,
}

impl PlaybackData {
    /// Picks the stream to play: DASH when there is one, else HLS, else the
    /// raw `playbackUrl` (MX Player). Mirrors `PlaybackData.VODStream`.
    pub fn vod_stream(&self) -> (String, bool) {
        if !self.mpd.auto.is_empty() {
            return (self.mpd.auto.clone(), true);
        }
        if !self.m3u8.auto.is_empty() {
            return (self.m3u8.auto.clone(), false);
        }
        let url = self.playback_url.trim().to_string();
        let is_dash = url.contains(".mpd");
        (url, is_dash)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PlaybackResponse {
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub data: PlaybackData,
}

impl PlaybackResponse {
    pub fn to_live_url_output(&self) -> crate::television::LiveUrlOutput {
        let d = &self.data;
        let hls = crate::television::Bitrates {
            auto: d.m3u8.auto.clone(),
            high: d.m3u8.high.clone(),
            medium: d.m3u8.medium.clone(),
            low: d.m3u8.low.clone(),
        };
        crate::television::LiveUrlOutput {
            code: self.code,
            message: self.message.clone(),
            result: d.m3u8.auto.clone(),
            bitrates: hls.clone(),
            m3u8: hls,
            mpd: crate::television::Mpd {
                auto: d.mpd.auto.clone(),
                high: d.mpd.high.clone(),
                medium: d.mpd.medium.clone(),
                low: d.mpd.low.clone(),
                key: d.key_url.clone(),
                ..Default::default()
            },
            is_drm: !d.key_url.is_empty() && !d.mpd.auto.is_empty(),
            key_url: d.key_url.clone(),
            algo_name: d.algo_name.clone(),
            hdnea: hdnea_from(&d.m3u8.auto),
        }
    }
}

fn hdnea_from(stream: &str) -> String {
    url::Url::parse(stream)
        .ok()
        .and_then(|u| u.query_pairs().find(|(k, _)| k == "__hdnea__").map(|(_, v)| v.into_owned()))
        .unwrap_or_default()
}

// ---- On-demand (VOD): JioCinema, ZEE5, MX Player only ----

/// Providers JioTV Go plays on demand, keyed by the catalogue's `provider`
/// field. Every other provider (Prime Video, JioHotstar, SonyLIV, ...) only
/// opens a partner app and is never shown.
pub fn vod_provider_name(provider: &str) -> Option<&'static str> {
    match provider {
        "JioCinema" => Some("JioCinema"),
        "MXPlayer" => Some("MX Player"),
        "Zee5" => Some("ZEE5"),
        _ => None,
    }
}

pub const ALGO_JIO_VOD: i64 = 4;
pub const ALGO_ZEE5: i64 = 6;
pub const ALGO_MX: i64 = 14;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct VodItem {
    #[serde(rename = "contentId", default)]
    pub content_id: String,
    #[serde(rename = "contentType", default)]
    pub content_type: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "showName", default)]
    pub show_name: String,
    #[serde(default)]
    pub thumbnail: String,
    #[serde(default)]
    pub provider: String,
    #[serde(rename = "playbackType", default)]
    pub playback_type: String,
    #[serde(default)]
    pub season: i64,
    #[serde(rename = "episodeNo", default)]
    pub episode_no: i64,
    #[serde(rename = "totalDuration", default)]
    pub total_duration: i64,
}

impl VodItem {
    /// Mirrors `VODItem.Playable`.
    pub fn playable(&self) -> bool {
        if vod_provider_name(&self.provider).is_none() || self.playback_type != "playback" {
            return false;
        }
        matches!(self.content_type.as_str(), "Movie" | "Show" | "Episode" | "Video")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Rail {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub items: Vec<VodItem>,
}

#[derive(Deserialize, Default)]
struct RailsMetadata {
    #[serde(rename = "totalPages", default)]
    #[allow(dead_code)]
    total_pages: i64,
}

#[derive(Deserialize, Default)]
struct RailsResponse {
    #[serde(default)]
    data: Vec<Rail>,
    #[serde(rename = "_metadata", default)]
    #[allow(dead_code)]
    metadata: RailsMetadata,
}

fn content_headers(page: &str) -> Vec<(&'static str, String)> {
    let mut h = common_headers();
    h.push(("x-page", page.to_string()));
    h.push(("x-livetv", "no".to_string()));
    h
}

/// Drops items JioTV Go cannot play, duplicates within a rail, and empty
/// rails. Mirrors `keepPlayable`.
fn keep_playable(rails: Vec<Rail>) -> Vec<Rail> {
    rails
        .into_iter()
        .filter_map(|r| {
            let mut seen = std::collections::HashSet::new();
            let items: Vec<VodItem> = r
                .items
                .into_iter()
                .filter(|it| it.playable() && seen.insert(it.content_id.clone()))
                .collect();
            if items.is_empty() {
                None
            } else {
                Some(Rail { title: r.title, items })
            }
        })
        .collect()
}

impl Client {
    pub async fn search(&self, query: &str) -> Result<Vec<Rail>, TvPlusError> {
        let e = self.endpoints();
        let mut req = self.http.get(format!("{}/search/v1/search", e.content)).query(&[("q", query), ("isKids", "false")]);
        for (k, v) in content_headers("Search") {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        check_status(&resp, "search")?;
        let r: RailsResponse = resp.json().await?;
        Ok(keep_playable(r.data))
    }

    /// One page (five rails) of a catalogue screen (1 = home, 100021 =
    /// movies, 100023 = shows, 100025 = kids, 100097 = TV shows). `more` is
    /// false on the last page.
    pub async fn screen(&self, screen_id: &str, page: i64) -> Result<(Vec<Rail>, bool), TvPlusError> {
        let e = self.endpoints();
        let mut req = self
            .http
            .get(format!("{}/screen/v2/{screen_id}", e.content))
            .query(&[("pageNo", page.to_string()), ("isKids", "false".to_string())]);
        for (k, v) in content_headers("Home") {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        check_status(&resp, "screen")?;
        let r: RailsResponse = resp.json().await?;
        let more = !r.data.is_empty();
        Ok((keep_playable(r.data), more))
    }

    /// A show's episodes; `season <= 0` means the default season.
    pub async fn episodes(&self, show_id: &str, season: i64) -> Result<Vec<VodItem>, TvPlusError> {
        let e = self.endpoints();
        let mut url = format!("{}/metadata/v2/metadata/Show/{}", e.content, urlencoding::encode(show_id));
        if season > 0 {
            url.push_str(&format!("?season={season}"));
        }
        #[derive(Deserialize)]
        struct Resp {
            data: Rail,
        }
        let mut req = self.http.get(url);
        for (k, v) in content_headers("Metadata") {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        check_status(&resp, "episodes")?;
        let r: Resp = resp.json().await?;
        Ok(r.data.items.into_iter().filter(|it| it.content_type == "Episode" && it.playable()).collect())
    }

    /// Headers for a Widevine license request for on-demand content
    /// (`k2/k.java`), varying by `PlaybackData.algo`. Mirrors
    /// `VODLicenseHeaders`.
    pub fn vod_license_headers(&self, d: &PlaybackData) -> Vec<(String, String)> {
        let cr = match self.credentials() {
            Some(c) => c,
            None => return Vec::new(),
        };
        let mut h = vec![
            ("os".to_string(), "android".to_string()),
            ("playbackToken".to_string(), d.playback_token.clone()),
            ("srno".to_string(), "230203144000".to_string()),
            ("usergroup".to_string(), "474537347347373".to_string()),
            ("deviceid".to_string(), self.device.android_id.clone()),
            ("channelid".to_string(), d.content_id.clone()),
            ("versionCode".to_string(), VERSION_CODE.to_string()),
            ("devicetype".to_string(), "tv".to_string()),
            ("uniqueid".to_string(), cr.user_id.clone()),
            ("ssotoken".to_string(), cr.sso_token.clone()),
        ];
        match d.algo {
            ALGO_JIO_VOD => {
                h.push(("lbCookie".to_string(), String::new()));
                h.push(("idamId".to_string(), String::new()));
                h.push(("jioId".to_string(), String::new()));
                h.push(("appId".to_string(), "jiovod".to_string()));
                h.push(("appKey".to_string(), "2ccce09e59153fc9".to_string()));
            }
            ALGO_ZEE5 => {
                h.push(("customData".to_string(), d.playback_token.clone()));
                h.push(("nl".to_string(), d.nl.clone()));
            }
            _ => {}
        }
        h
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Programme {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "startEpoch", default)]
    pub start_epoch: i64,
    #[serde(rename = "endEpoch", default)]
    pub end_epoch: i64,
    #[serde(default)]
    pub thumbnail: String,
}

pub fn to_xmltv(content_id: &str, category: &str, progs: &[Programme]) -> Vec<crate::epg::XmlProgramme> {
    progs
        .iter()
        .map(|p| {
            let icon = if p.thumbnail.ends_with('/') { String::new() } else { p.thumbnail.clone() };
            crate::epg::XmlProgramme {
                channel: channel_id(content_id),
                start: crate::epg::format_xmltv_time(p.start_epoch),
                stop: crate::epg::format_xmltv_time(p.end_epoch),
                title: p.title.clone(),
                desc: p.description.clone(),
                category: category.to_string(),
                icon,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_number_with_plus91() {
        assert_eq!(normalize_number("+919876543210").unwrap(), "9876543210");
        assert_eq!(normalize_number("9876543210").unwrap(), "9876543210");
        assert!(normalize_number("12345").is_err());
    }

    #[test]
    fn content_id_strips_prefix() {
        assert_eq!(content_id::content_id("tvp_302084"), Some("302084"));
        assert_eq!(content_id::content_id("154"), None);
    }

    #[test]
    fn normalize_name_matches_go_semantics() {
        assert_eq!(normalize_name("Star Plus HD"), "starplushd");
        assert_eq!(normalize_name("Zee & TV"), "zeeandtv");
    }

    #[test]
    fn exclusive_drops_duplicates_by_ext_id_and_name() {
        let jiotv = vec![crate::television::Channel {
            id: "154".to_string(),
            name: "Star Plus".to_string(),
            ..Default::default()
        }];
        let tvplus = vec![
            LiveChannel { content_id: "1".into(), ext_id: "154".into(), name: "Star Plus".into(), ..Default::default() },
            LiveChannel { content_id: "2".into(), name: "Zee5 Exclusive".into(), ..Default::default() },
            LiveChannel { content_id: "3".into(), name: "Test Channel".into(), ..Default::default() },
        ];
        let ex = exclusive(&tvplus, &jiotv);
        assert_eq!(ex.len(), 1);
        assert_eq!(ex[0].id, "tvp_2");
    }

    #[test]
    fn mirrors_maps_jiotv_id_to_content_id() {
        let jiotv = vec![crate::television::Channel { id: "154".into(), name: "Star Plus".into(), ..Default::default() }];
        let tvplus = vec![LiveChannel { content_id: "1".into(), ext_id: "154".into(), name: "Star Plus".into(), ..Default::default() }];
        let m = mirrors(&tvplus, &jiotv);
        assert_eq!(m.get("154"), Some(&"1".to_string()));
    }

    #[test]
    fn device_round_trips_through_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let d1 = Device::load_or_create(&store).unwrap();
        let d2 = Device::load_or_create(&store).unwrap();
        assert_eq!(d1.android_id, d2.android_id);
    }

    #[test]
    fn credentials_json_uses_go_field_names() {
        let cr = Credentials {
            number: "9876543210".into(),
            sso_token: "sso".into(),
            subscriber_id: "sub".into(),
            unique: "uniq".into(),
            user_id: "uid".into(),
            auth_token: "at".into(),
            refresh_token: "rt".into(),
        };
        let json = serde_json::to_string(&cr).unwrap();
        for key in ["ssoToken", "subscriberId", "userId", "authToken", "refreshToken"] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
    }

    #[test]
    fn credentials_needs_refresh_when_no_exp_claim() {
        let cr = Credentials { auth_token: "not-a-jwt".into(), ..Default::default() };
        assert!(cr.needs_refresh(SystemTime::now(), Duration::from_secs(3600)));
    }

    /// Exercises send_otp -> verify_otp -> exchange_token end to end against
    /// a local mock server standing in for tv.media.jio.com /
    /// jiotvapi.media.jio.com. No real number, OTP or credential appears
    /// here — everything is a redacted placeholder the mock server echoes
    /// back, per the project's rule against live JioTV+ calls.
    #[tokio::test]
    async fn full_login_flow_against_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let auth_server = MockServer::start().await;
        let user_service_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/apis/v3.2/stbotplogin/sendotp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "message": "ok", "identifier": "redacted-identifier", "fttxIds": []
            })))
            .mount(&auth_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/apis/v3.2/stbotplogin/verifyotp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ssoToken": "redacted-sso",
                "sessionAttributes": {"user": {"subscriberId": "redacted-sub", "unique": "redacted-uniq"}}
            })))
            .mount(&auth_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/loginotp/exchangetoken"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "authToken": "redacted-at", "refreshToken": "redacted-rt", "userId": "redacted-uid"
            })))
            .mount(&user_service_server)
            .await;

        let client = Client::new(reqwest::Client::new(), Device::new());
        client.set_endpoints(Endpoints {
            auth: auth_server.uri(),
            user_service: user_service_server.uri(),
            ..Default::default()
        });

        let sent = client.send_otp("9876543210", "").await.unwrap();
        assert_eq!(sent.identifier, "redacted-identifier");

        let creds = client.verify_otp("9876543210", &sent.identifier, "0000").await.unwrap();
        assert_eq!(creds.sso_token, "redacted-sso");
        assert_eq!(creds.auth_token, "redacted-at");
        assert_eq!(creds.refresh_token, "redacted-rt");
    }
}
