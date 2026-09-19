//! JioTV OTP login, token exchange and refresh. Endpoints and payload shapes
//! are copied from `pkg/utils/utils.go` (`LoginSendOTP`, `LoginVerifyOTP`)
//! and `internal/constants/urls/urls.go` in the Go tree. Never called with
//! real credentials by tests in this repo — see `tests/login_mock.rs` for
//! coverage against a local mock server.

use crate::television::{Credentials, JIOTV_API_DOMAIN, LOGIN_SEND_OTP_PATH, LOGIN_VERIFY_OTP_PATH, REFRESH_TOKEN_URL};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct SendOtpPayload<'a> {
    number: &'a str,
}

#[derive(Deserialize)]
struct SendOtpResponse {
    #[allow(dead_code)]
    code: Option<String>,
}

#[derive(Serialize)]
struct VerifyOtpPayload<'a> {
    number: &'a str,
    #[serde(rename = "otp")]
    otp: &'a str,
}

#[derive(Deserialize, Debug)]
pub struct VerifyOtpResponse {
    pub ssotoken: Option<String>,
    #[serde(rename = "authToken")]
    pub auth_token: Option<String>,
    #[serde(rename = "refreshToken")]
    pub refresh_token: Option<String>,
    pub crm: Option<String>,
    #[serde(rename = "uniqueId")]
    pub unique_id: Option<String>,
    /// Present on the wire; not consulted (verify_otp's caller reports
    /// success/failure via the `Result` itself instead).
    #[allow(dead_code)]
    pub status: Option<String>,
}

pub struct LoginClient {
    client: reqwest::Client,
    api_base: String,
    refresh_url: String,
}

impl LoginClient {
    pub fn new(client: reqwest::Client) -> LoginClient {
        LoginClient {
            client,
            api_base: format!("https://{JIOTV_API_DOMAIN}"),
            refresh_url: REFRESH_TOKEN_URL.to_string(),
        }
    }

    /// Overrides the API base and refresh URL, for tests against a mock server.
    #[cfg(test)]
    pub fn with_base(client: reqwest::Client, base: &str) -> LoginClient {
        LoginClient {
            client,
            api_base: base.to_string(),
            refresh_url: format!("{base}/refresh"),
        }
    }

    pub async fn send_otp(&self, number: &str) -> anyhow::Result<bool> {
        let url = format!("{}{LOGIN_SEND_OTP_PATH}", self.api_base);
        let resp = self
            .client
            .post(url)
            .json(&SendOtpPayload { number })
            .send()
            .await?
            .error_for_status()?;
        let _: SendOtpResponse = resp.json().await.unwrap_or(SendOtpResponse { code: None });
        Ok(true)
    }

    pub async fn verify_otp(&self, number: &str, otp: &str) -> anyhow::Result<Credentials> {
        let url = format!("{}{LOGIN_VERIFY_OTP_PATH}", self.api_base);
        let resp = self
            .client
            .post(url)
            .json(&VerifyOtpPayload { number, otp })
            .send()
            .await?
            .error_for_status()?;
        let body: VerifyOtpResponse = resp.json().await?;
        Ok(Credentials {
            sso_token: body.ssotoken.unwrap_or_default(),
            crm: body.crm.unwrap_or_default(),
            unique_id: body.unique_id.unwrap_or_default(),
            access_token: body.auth_token.unwrap_or_default(),
            refresh_token: body.refresh_token.unwrap_or_default(),
        })
    }

    pub async fn refresh(&self, creds: &Credentials) -> anyhow::Result<Credentials> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(rename = "refreshToken")]
            refresh_token: &'a str,
        }
        let resp = self
            .client
            .post(&self.refresh_url)
            .header("accesstoken", &creds.access_token)
            .json(&Body { refresh_token: &creds.refresh_token })
            .send()
            .await?
            .error_for_status()?;
        let body: VerifyOtpResponse = resp.json().await?;
        Ok(Credentials {
            access_token: body.auth_token.unwrap_or_else(|| creds.access_token.clone()),
            refresh_token: creds.refresh_token.clone(),
            ..creds.clone()
        })
    }
}

/// Persists credentials as a JSON blob under the `jiotv_credentials` store
/// key. Not required to match the Go binary's on-disk format byte for byte
/// (the Go code stays deleted on this branch); this is a fresh, simpler
/// format for the Rust store.
pub fn save(store: &crate::store::Store, creds: &Credentials) -> anyhow::Result<()> {
    let json = serde_json::to_string(creds)?;
    store.set("jiotv_credentials", &json)?;
    Ok(())
}

pub fn load(store: &crate::store::Store) -> Option<Credentials> {
    let json = store.get_opt("jiotv_credentials")?;
    serde_json::from_str(&json).ok()
}

pub fn clear(store: &crate::store::Store) -> anyhow::Result<()> {
    let _ = store.delete("jiotv_credentials");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn send_and_verify_otp_against_mock_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(super::super::television::LOGIN_SEND_OTP_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"code": "0000"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(super::super::television::LOGIN_VERIFY_OTP_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ssotoken": "sso-redacted",
                "authToken": "at-redacted",
                "refreshToken": "rt-redacted",
                "crm": "crm-redacted",
                "uniqueId": "uid-redacted",
                "status": "success",
            })))
            .mount(&server)
            .await;

        let client = LoginClient::with_base(reqwest::Client::new(), &server.uri());
        assert!(client.send_otp("+91XXXXXXXXXX").await.unwrap());
        let creds = client.verify_otp("+91XXXXXXXXXX", "0000").await.unwrap();
        assert_eq!(creds.sso_token, "sso-redacted");
        assert_eq!(creds.access_token, "at-redacted");
    }

    #[test]
    fn credentials_round_trip_through_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().to_str().unwrap()).unwrap();
        let creds = Credentials {
            sso_token: "a".into(),
            crm: "b".into(),
            unique_id: "c".into(),
            access_token: "d".into(),
            refresh_token: "e".into(),
        };
        save(&store, &creds).unwrap();
        let loaded = load(&store).unwrap();
        assert_eq!(loaded.access_token, "d");
        clear(&store).unwrap();
        assert!(load(&store).is_none());
    }
}
