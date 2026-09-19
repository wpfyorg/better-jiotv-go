//! Proactive JioTV access-token refresh, a reduced version of
//! `EnsureFreshCredentials`/`ForceRefreshCredentials` in
//! `internal/handlers/drm.go`. This rewrite skips the fallback-TTL path for
//! tokens that aren't JWTs (the Go version's SSO-token handling in
//! particular) and the "next validation time" scheduling cache — every
//! stream request just checks the access token's own `exp` claim and
//! refreshes when it's near.

use crate::state::AppState;
use base64::Engine;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const JWT_REFRESH_LEAD: Duration = Duration::from_secs(30);
const ACCESS_FALLBACK_TTL: u64 = 2 * 60 * 60;
const ACCESS_FALLBACK_LEAD: u64 = 10 * 60;
const SSO_FALLBACK_TTL: u64 = 24 * 60 * 60;
const SSO_FALLBACK_LEAD: u64 = 60 * 60;

fn jwt_exp(token: &str) -> Option<u64> {
    let mut parts = token.split('.');
    let (_h, payload, _s) = (parts.next()?, parts.next()?, parts.next()?);
    let mut padded = payload.to_string();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    let decoded = base64::engine::general_purpose::URL_SAFE
        .decode(&padded)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload))
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get("exp")?.as_u64()
}

/// Mirrors `shouldRefreshToken`: a JWT is refreshed shortly before its
/// `exp`; any other token once `fallback_ttl - fallback_lead` has passed
/// since it was last refreshed (or when that time is unknown).
fn should_refresh(token: &str, last_refresh: Option<u64>, fallback_ttl: u64, fallback_lead: u64, now: u64) -> bool {
    if token.is_empty() {
        return true;
    }
    if let Some(exp) = jwt_exp(token) {
        return exp <= now + JWT_REFRESH_LEAD.as_secs();
    }
    match last_refresh {
        Some(last) => last + fallback_ttl - fallback_lead <= now,
        None => true,
    }
}

/// Refreshes the JioTV access and SSO tokens when they are due, like
/// `EnsureFreshTokens`. Failures are logged and the request carries on with
/// the tokens it has.
pub async fn ensure_fresh(state: &AppState) {
    let _guard = state.tv.refresh_lock.lock().await;
    let Some(mut creds) = state.tv.creds.read().unwrap().clone() else { return };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let client = crate::login::LoginClient::new(state.http.clone());
    let device_id = state.tv.device_id.clone();

    let access_due = !creds.refresh_token.is_empty()
        && should_refresh(&creds.access_token, crate::login::last_refresh(&state.store, false), ACCESS_FALLBACK_TTL, ACCESS_FALLBACK_LEAD, now);
    if access_due {
        match client.refresh(&creds, &device_id).await {
            Ok(refreshed) => {
                creds = refreshed;
                let _ = crate::login::save(&state.store, &creds, crate::login::Touch { access: true, sso: false });
                state.tv.set_credentials(creds.clone());
            }
            Err(e) => tracing::warn!("JioTV access token refresh failed: {e}"),
        }
    }

    let sso_due = !creds.sso_token.is_empty()
        && should_refresh(&creds.sso_token, crate::login::last_refresh(&state.store, true), SSO_FALLBACK_TTL, SSO_FALLBACK_LEAD, now);
    if sso_due {
        match client.refresh_sso(&creds, &device_id).await {
            Ok(refreshed) => {
                let _ = crate::login::save(&state.store, &refreshed, crate::login::Touch { access: false, sso: true });
                state.tv.set_credentials(refreshed);
            }
            Err(e) => tracing::warn!("JioTV SSO token refresh failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_exp_from_a_jwt() {
        // header.payload.signature, payload = {"exp":1700000000}, unsigned
        // (a signature isn't needed since this only reads the claim).
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":1700000000}"#);
        let token = format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig");
        assert_eq!(jwt_exp(&token), Some(1700000000));
    }

    #[test]
    fn refresh_policy_matches_go() {
        let now = 1_000_000;
        assert!(should_refresh("", None, 7200, 600, now));
        assert!(should_refresh("opaque", None, 7200, 600, now));
        assert!(!should_refresh("opaque", Some(now - 60), 7200, 600, now));
        assert!(should_refresh("opaque", Some(now - 6700), 7200, 600, now));
    }

    #[test]
    fn non_jwt_token_has_no_exp() {
        assert_eq!(jwt_exp("not-a-jwt"), None);
    }
}
