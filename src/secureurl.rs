//! Encrypts stream-proxy URL parameters with a per-start random AES-256-CTR
//! key, mirroring `pkg/secureurl` in the Go version. Since the key is
//! generated fresh on every start and never persisted, encrypted URLs from a
//! previous run (or from the other implementation) are never valid, which is
//! the point: it stops an outside caller from injecting arbitrary URLs into
//! the open `/render.*` and `/drm` routes.

use aes::cipher::{KeyIvInit, StreamCipher};
use rand::RngCore;
use sha2::{Digest, Sha256};

type Aes256Ctr = ctr::Ctr128BE<aes::Aes256>;

pub struct SecureUrl {
    key: [u8; 32],
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
}

impl SecureUrl {
    /// Generates a fresh random key. `disable` mirrors
    /// `disable_url_encryption`: URLs are then only percent-encoded, which
    /// must never be combined with an open (no access-key) deployment.
    pub fn new(disable: bool) -> SecureUrl {
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        SecureUrl { key, disable }
    }

    pub fn encrypt(&self, input: &str) -> String {
        if self.disable {
            return urlencoding::encode(input).into_owned();
        }
        let mut iv = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut iv);
        self.encrypt_with_iv(input, iv)
    }

    /// Same ciphertext for the same input every time (derives the nonce from
    /// the key and the input instead of a random IV). Used for the DASH
    /// segment URLs a player might otherwise re-request as "new" segments
    /// across manifest refreshes.
    pub fn encrypt_deterministic(&self, input: &str) -> String {
        if self.disable {
            return urlencoding::encode(input).into_owned();
        }
        let mut hasher = Sha256::new();
        hasher.update(self.key);
        hasher.update(input.as_bytes());
        let sum = hasher.finalize();
        let mut iv = [0u8; 16];
        iv.copy_from_slice(&sum[..16]);
        self.encrypt_with_iv(input, iv)
    }

    fn encrypt_with_iv(&self, input: &str, iv: [u8; 16]) -> String {
        let mut buf = input.as_bytes().to_vec();
        let mut cipher = Aes256Ctr::new((&self.key).into(), (&iv).into());
        cipher.apply_keystream(&mut buf);
        let mut out = Vec::with_capacity(16 + buf.len());
        out.extend_from_slice(&iv);
        out.extend_from_slice(&buf);
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE, out)
    }

    pub fn decrypt(&self, input: &str) -> Result<String, SecureUrlError> {
        if self.disable {
            return urlencoding::decode(input)
                .map(|s| s.into_owned())
                .map_err(|_| SecureUrlError::Utf8);
        }
        let raw = base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE, input)
            .map_err(|_| SecureUrlError::Base64)?;
        if raw.len() < 16 {
            return Err(SecureUrlError::TooShort);
        }
        let (iv, ct) = raw.split_at(16);
        let mut buf = ct.to_vec();
        let mut cipher = Aes256Ctr::new(self.key.as_ref().into(), iv.into());
        cipher.apply_keystream(&mut buf);
        String::from_utf8(buf).map_err(|_| SecureUrlError::Utf8)
    }
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
}
