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

/// Refreshes the JioTV access token if it looks like a JWT that's within
/// `JWT_REFRESH_LEAD` of expiry (or isn't a JWT / is missing, in which case
/// the caller proceeds with what it has — matching the Go version's
/// "continue with the request, tokens might still work" fallback).
pub async fn ensure_fresh(state: &AppState) {
    let creds = state.tv.creds.read().unwrap().clone();
    let Some(creds) = creds else { return };
    if creds.refresh_token.is_empty() {
        return;
    }
    let Some(exp) = jwt_exp(&creds.access_token) else { return };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    if exp > now + JWT_REFRESH_LEAD.as_secs() {
        return;
    }
    let client = crate::login::LoginClient::new(state.http.clone());
    if let Ok(refreshed) = client.refresh(&creds).await {
        state.tv.set_credentials(refreshed.clone());
        let _ = crate::login::save(&state.store, &refreshed);
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
    fn non_jwt_token_has_no_exp() {
        assert_eq!(jwt_exp("not-a-jwt"), None);
    }
}
