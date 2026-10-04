//! Encrypts stream-proxy URL parameters with a per-start random AES-256-CTR
//! key, mirroring `pkg/secureurl` in the Go version. Since the key is
//! generated fresh on every start and never persisted, encrypted URLs from a
//! previous run (or from the other implementation) are never valid, which is
//! the point: it stops an outside caller from injecting arbitrary URLs into
//! the open `/render.*` and `/drm` routes.

use aes::cipher::{KeyIvInit, StreamCipher};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::sync::RwLock;

type Aes256Ctr = ctr::Ctr128BE<aes::Aes256>;

const CONTEXT_PREFIX: &str = "jiotv-context:";

struct KeyState {
    key: [u8; 32],
    epoch: u64,
}

pub struct SecureUrl {
    state: RwLock<KeyState>,
    disable: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SecureUrlError {
    #[error("ciphertext too short")]
    TooShort,
    #[error("invalid base64")]
    Base64,
    #[error("invalid utf8")]
    Utf8,
    #[error("stale context")]
    StaleContext,
}

impl SecureUrl {
    /// Generates a fresh random key. `disable` mirrors
    /// `disable_url_encryption`: URLs are then only percent-encoded, which
    /// must never be combined with an open (no access-key) deployment.
    pub fn new(disable: bool) -> SecureUrl {
        SecureUrl {
            state: RwLock::new(KeyState {
                key: random_key(),
                epoch: 1,
            }),
            disable,
        }
    }

    /// Invalidates every encrypted proxy URL issued in the previous account
    /// or product context. This is also enforced when URL encryption is
    /// disabled, so that debug deployments do not accidentally keep stale
    /// account-scoped URLs alive across a context switch.
    pub fn rotate(&self) -> u64 {
        let mut state = self.state.write().unwrap();
        state.key = random_key();
        state.epoch = state.epoch.wrapping_add(1).max(1);
        state.epoch
    }

    pub fn current_epoch(&self) -> u64 {
        self.state.read().unwrap().epoch
    }

    pub fn encrypt(&self, input: &str) -> String {
        let state = self.state.read().unwrap();
        let payload = scoped_payload(state.epoch, input);
        if self.disable {
            return urlencoding::encode(&payload).into_owned();
        }
        let mut iv = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut iv);
        encrypt_with_iv(&payload, iv, &state.key)
    }

    /// Same ciphertext for the same input every time (derives the nonce from
    /// the key and the input instead of a random IV). Used for the DASH
    /// segment URLs a player might otherwise re-request as "new" segments
    /// across manifest refreshes.
    pub fn encrypt_deterministic(&self, input: &str) -> String {
        let state = self.state.read().unwrap();
        let payload = scoped_payload(state.epoch, input);
        if self.disable {
            return urlencoding::encode(&payload).into_owned();
        }
        let mut hasher = Sha256::new();
        hasher.update(state.key);
        hasher.update(payload.as_bytes());
        let sum = hasher.finalize();
        let mut iv = [0u8; 16];
        iv.copy_from_slice(&sum[..16]);
        encrypt_with_iv(&payload, iv, &state.key)
    }

    pub fn decrypt(&self, input: &str) -> Result<String, SecureUrlError> {
        let state = self.state.read().unwrap();
        if self.disable {
            let decoded = urlencoding::decode(input)
                .map(|s| s.into_owned())
                .map_err(|_| SecureUrlError::Utf8)?;
            return parse_scoped_payload(&decoded, state.epoch);
        }
        let raw = base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE, input)
            .map_err(|_| SecureUrlError::Base64)?;
        if raw.len() < 16 {
            return Err(SecureUrlError::TooShort);
        }
        let (iv, ct) = raw.split_at(16);
        let mut buf = ct.to_vec();
        let mut cipher = Aes256Ctr::new(state.key.as_ref().into(), iv.into());
        cipher.apply_keystream(&mut buf);
        let decoded = String::from_utf8(buf).map_err(|_| SecureUrlError::Utf8)?;
        parse_scoped_payload(&decoded, state.epoch)
    }
}

fn random_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

fn scoped_payload(epoch: u64, input: &str) -> String {
    format!("{CONTEXT_PREFIX}{epoch}:{input}")
}

fn parse_scoped_payload(payload: &str, epoch: u64) -> Result<String, SecureUrlError> {
    let expected = format!("{CONTEXT_PREFIX}{epoch}:");
    payload
        .strip_prefix(&expected)
        .map(str::to_string)
        .ok_or(SecureUrlError::StaleContext)
}

fn encrypt_with_iv(input: &str, iv: [u8; 16], key: &[u8; 32]) -> String {
    let mut buf = input.as_bytes().to_vec();
    let mut cipher = Aes256Ctr::new(key.into(), (&iv).into());
    cipher.apply_keystream(&mut buf);
    let mut out = Vec::with_capacity(16 + buf.len());
    out.extend_from_slice(&iv);
    out.extend_from_slice(&buf);
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = SecureUrl::new(false);
        let enc = s.encrypt("https://example.com/a?b=c");
        assert_eq!(s.decrypt(&enc).unwrap(), "https://example.com/a?b=c");
    }

    #[test]
    fn deterministic_is_stable() {
        let s = SecureUrl::new(false);
        let a = s.encrypt_deterministic("https://example.com/seg1.ts");
        let b = s.encrypt_deterministic("https://example.com/seg1.ts");
        assert_eq!(a, b);
        let c = s.encrypt_deterministic("https://example.com/seg2.ts");
        assert_ne!(a, c);
    }

    #[test]
    fn random_is_not_stable() {
        let s = SecureUrl::new(false);
        let a = s.encrypt("https://example.com/x");
        let b = s.encrypt("https://example.com/x");
        assert_ne!(a, b);
    }

    #[test]
    fn disabled_uses_percent_encoding() {
        let s = SecureUrl::new(true);
        let enc = s.encrypt("https://example.com/a b");
        assert_eq!(s.decrypt(&enc).unwrap(), "https://example.com/a b");
    }

    #[test]
    fn a_different_process_key_cannot_decrypt() {
        let a = SecureUrl::new(false);
        let b = SecureUrl::new(false);
        let enc = a.encrypt("https://example.com/x");
        // Garbage or a decode error either way; never the original text.
        if let Ok(s) = b.decrypt(&enc) {
            assert_ne!(s, "https://example.com/x");
        }
    }

    #[test]
    fn rotating_context_invalidates_old_encrypted_urls() {
        let s = SecureUrl::new(false);
        let old = s.encrypt("https://example.com/x");
        let old_epoch = s.current_epoch();
        assert!(s.rotate() > old_epoch);
        assert!(s.decrypt(&old).is_err());
        let fresh = s.encrypt("https://example.com/x");
        assert_eq!(s.decrypt(&fresh).unwrap(), "https://example.com/x");
    }

    #[test]
    fn rotating_context_invalidates_old_urls_when_encryption_is_disabled() {
        let s = SecureUrl::new(true);
        let old = s.encrypt("https://example.com/x");
        s.rotate();
        assert!(matches!(s.decrypt(&old), Err(SecureUrlError::StaleContext)));
    }
}
