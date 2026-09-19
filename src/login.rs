//! JioTV OTP login, token exchange and refresh. Endpoints and payload shapes
//! are copied from `pkg/utils/utils.go` (`LoginSendOTP`, `LoginVerifyOTP`)
//! and `internal/constants/urls/urls.go` in the Go tree. Never called with
//! real credentials by tests in this repo — see `tests/login_mock.rs` for
//! coverage against a local mock server.

use crate::television::{Credentials, JIOTV_API_DOMAIN, LOGIN_SEND_OTP_PATH, LOGIN_VERIFY_OTP_PATH, REFRESH_SSO_TOKEN_URL, REFRESH_TOKEN_URL};
use base64::Engine;
use serde::Deserialize;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

const USER_AGENT: &str = "okhttp/4.12.0";
const VERSION_CODE: &str = "422";

#[derive(Deserialize, Default)]
struct VerifyOtpResponse {
    #[serde(rename = "authToken", default)]
    auth_token: String,
    #[serde(rename = "refreshToken", default)]
    refresh_token: String,
    #[serde(rename = "ssoToken", default)]
    sso_token: String,
    #[serde(rename = "sessionAttributes", default)]
    session: SessionAttributes,
}

#[derive(Deserialize, Default)]
struct SessionAttributes {
    #[serde(default)]
    user: SessionUser,
}

#[derive(Deserialize, Default)]
struct SessionUser {
    #[serde(rename = "subscriberId", default)]
    subscriber_id: String,
    #[serde(default)]
    unique: String,
}

pub struct LoginClient {
    client: reqwest::Client,
    api_base: String,
    refresh_url: String,
    refresh_sso_url: String,
}

fn encode_number(number: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(number)
}

impl LoginClient {
    pub fn new(client: reqwest::Client) -> LoginClient {
        LoginClient {
            client,
            api_base: format!("https://{JIOTV_API_DOMAIN}"),
            refresh_url: REFRESH_TOKEN_URL.to_string(),
            refresh_sso_url: REFRESH_SSO_TOKEN_URL.to_string(),
        }
    }

    /// Overrides the API base and refresh URLs, for tests against a mock server.
    #[cfg(test)]
    pub fn with_base(client: reqwest::Client, base: &str) -> LoginClient {
        LoginClient {
            client,
            api_base: base.to_string(),
            refresh_url: format!("{base}/refresh"),
            refresh_sso_url: format!("{base}/refresh-sso"),
        }
    }

    fn app_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header("appname", "RJIL_JioTV").header("os", "android").header("devicetype", "phone")
    }

    /// Sends a login OTP to `number` (with country code, e.g. +91...).
    /// Mirrors `LoginSendOTP`: the API answers 204 when the OTP is sent.
    pub async fn send_otp(&self, number: &str) -> anyhow::Result<bool> {
        let url = format!("{}{LOGIN_SEND_OTP_PATH}", self.api_base);
        let resp = self.app_headers(self.client.post(url)).json(&json!({"number": encode_number(number)})).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("send OTP failed with status {}", resp.status());
        }
        Ok(true)
    }

    /// Verifies the OTP. Returns None when Jio rejects it. Mirrors
    /// `LoginVerifyOTP` (v1 endpoint: v2 needs an extra token exchange).
    pub async fn verify_otp(&self, number: &str, otp: &str, device_id: &str) -> anyhow::Result<Option<Credentials>> {
        let url = format!("{}{LOGIN_VERIFY_OTP_PATH}", self.api_base);
        let payload = json!({
            "number": encode_number(number),
            "otp": otp,
            "deviceInfo": {
                "consumptionDeviceName": "SM-G930F",
                "info": {"type": "android", "platform": {"name": "SM-G930F"}, "androidId": device_id},
            },
        });
        let resp = self.app_headers(self.client.post(url)).json(&payload).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("verify OTP failed with status {}", resp.status());
        }
        let body: VerifyOtpResponse = resp.json().await.unwrap_or_default();
        if body.auth_token.is_empty() {
            return Ok(None);
        }
        Ok(Some(Credentials {
            sso_token: body.sso_token,
            crm: body.session.user.subscriber_id,
            unique_id: body.session.user.unique,
            access_token: body.auth_token,
            refresh_token: body.refresh_token,
        }))
    }

    /// Gets a new access token. Mirrors `LoginRefreshAccessToken`.
    pub async fn refresh(&self, creds: &Credentials, device_id: &str) -> anyhow::Result<Credentials> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(rename = "authToken", default)]
            auth_token: String,
        }
        let resp = self
            .client
            .post(&self.refresh_url)
            .header("devicetype", "phone")
            .header("versionCode", VERSION_CODE)
            .header("os", "android")
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header("accesstoken", &creds.access_token)
            .json(&json!({"appName": "RJIL_JioTV", "deviceId": device_id, "refreshToken": creds.refresh_token}))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("access token refresh failed with status {}", resp.status());
        }
        let body: Resp = resp.json().await?;
        if body.auth_token.is_empty() {
            anyhow::bail!("access token not found in the refresh response");
        }
        Ok(Credentials { access_token: body.auth_token, ..creds.clone() })
    }

    /// Gets a new SSO token. Mirrors `LoginRefreshSSOToken`.
    pub async fn refresh_sso(&self, creds: &Credentials, device_id: &str) -> anyhow::Result<Credentials> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(rename = "ssoToken", default)]
            sso_token: String,
        }
        if creds.sso_token.is_empty() || creds.unique_id.is_empty() || device_id.is_empty() {
            anyhow::bail!("missing SSO token, unique ID or device ID");
        }
        let resp = self
            .client
            .get(&self.refresh_sso_url)
            .header("devicetype", "phone")
            .header("versionCode", VERSION_CODE)
            .header("os", "android")
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header("ssoToken", &creds.sso_token)
            .header("uniqueid", &creds.unique_id)
            .header("deviceid", device_id)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("SSO token refresh failed with status {}", resp.status());
        }
        let body: Resp = resp.json().await?;
        if body.sso_token.is_empty() {
            anyhow::bail!("SSO token not found in the refresh response");
        }
        Ok(Credentials { sso_token: body.sso_token, ..creds.clone() })
    }
}

fn now_unix() -> String {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string()
}

/// Which refresh timestamps a save should reset to now.
#[derive(Clone, Copy)]
pub struct Touch {
    pub access: bool,
    pub sso: bool,
}

pub const TOUCH_ALL: Touch = Touch { access: true, sso: true };

/// Saves the login under the same store keys as the Go version
/// (`WriteJIOTVCredentials`), so either binary can use the store.
pub fn save(store: &crate::store::Store, creds: &Credentials, touch: Touch) -> anyhow::Result<()> {
    store.set("ssoToken", &creds.sso_token)?;
    store.set("crm", &creds.crm)?;
    store.set("uniqueId", &creds.unique_id)?;
    store.set("accessToken", &creds.access_token)?;
    store.set("refreshToken", &creds.refresh_token)?;
    let now = now_unix();
    if touch.access || store.get_opt("lastTokenRefreshTime").is_none() {
        store.set("lastTokenRefreshTime", &now)?;
    }
    if touch.sso || store.get_opt("lastSSOTokenRefreshTime").is_none() {
        store.set("lastSSOTokenRefreshTime", &now)?;
    }
    let _ = store.delete("jiotv_credentials");
    Ok(())
}

/// Loads the login saved by either this or the Go version.
pub fn load(store: &crate::store::Store) -> Option<Credentials> {
    let access_token = store.get_opt("accessToken").unwrap_or_default();
    let sso_token = store.get_opt("ssoToken").unwrap_or_default();
    if access_token.is_empty() && sso_token.is_empty() {
        // Written by earlier builds of this rewrite.
        return serde_json::from_str(&store.get_opt("jiotv_credentials")?).ok();
    }
    Some(Credentials {
        sso_token,
        crm: store.get_opt("crm").unwrap_or_default(),
        unique_id: store.get_opt("uniqueId").unwrap_or_default(),
        access_token,
        refresh_token: store.get_opt("refreshToken").unwrap_or_default(),
    })
}

/// Unix time of the last access or SSO token refresh, from the store.
pub fn last_refresh(store: &crate::store::Store, sso: bool) -> Option<u64> {
    let key = if sso { "lastSSOTokenRefreshTime" } else { "lastTokenRefreshTime" };
    store.get_opt(key)?.parse().ok()
}

pub fn clear(store: &crate::store::Store) -> anyhow::Result<()> {
    for key in ["ssoToken", "crm", "uniqueId", "accessToken", "refreshToken", "lastTokenRefreshTime", "lastSSOTokenRefreshTime", "jiotv_credentials"] {
        let _ = store.delete(key);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn send_and_verify_otp_against_mock_server() {
        use wiremock::matchers::{body_partial_json, header};
        let server = MockServer::start().await;
        let encoded = encode_number("+91XXXXXXXXXX");
        Mock::given(method("POST"))
            .and(path(super::super::television::LOGIN_SEND_OTP_PATH))
            .and(header("appname", "RJIL_JioTV"))
            .and(body_partial_json(serde_json::json!({"number": encoded})))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(super::super::television::LOGIN_VERIFY_OTP_PATH))
            .and(body_partial_json(serde_json::json!({"number": encoded, "otp": "0000", "deviceInfo": {"info": {"androidId": "dev"}}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ssoToken": "sso-redacted",
                "authToken": "at-redacted",
                "refreshToken": "rt-redacted",
                "sessionAttributes": {"user": {"subscriberId": "crm-redacted", "unique": "uid-redacted"}},
            })))
            .mount(&server)
            .await;

        let client = LoginClient::with_base(reqwest::Client::new(), &server.uri());
        assert!(client.send_otp("+91XXXXXXXXXX").await.unwrap());
        let creds = client.verify_otp("+91XXXXXXXXXX", "0000", "dev").await.unwrap().unwrap();
        assert_eq!(creds.sso_token, "sso-redacted");
        assert_eq!(creds.access_token, "at-redacted");
        assert_eq!(creds.crm, "crm-redacted");
        assert_eq!(creds.unique_id, "uid-redacted");
    }

    #[tokio::test]
    async fn rejected_otp_is_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(super::super::television::LOGIN_VERIFY_OTP_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"message": "Invalid OTP"})))
            .mount(&server)
            .await;
        let client = LoginClient::with_base(reqwest::Client::new(), &server.uri());
        assert!(client.verify_otp("+91XXXXXXXXXX", "1111", "dev").await.unwrap().is_none());
    }

    #[test]
    fn credentials_round_trip_through_go_store_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let creds = Credentials {
            sso_token: "a".into(),
            crm: "b".into(),
            unique_id: "c".into(),
            access_token: "d".into(),
            refresh_token: "e".into(),
        };
        save(&store, &creds, TOUCH_ALL).unwrap();
        assert_eq!(store.get_opt("accessToken").as_deref(), Some("d"));
        assert!(last_refresh(&store, true).is_some());
        let loaded = load(&store).unwrap();
        assert_eq!(loaded.access_token, "d");
        assert_eq!(loaded.unique_id, "c");
        clear(&store).unwrap();
        assert!(load(&store).is_none());
    }
}
