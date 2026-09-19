//! Access gating: the `/k/<key>/` prefix IPTV players carry, and (full build)
//! the admin password + signed session cookie for the web UI. Mirrors
//! `internal/access` in the Go version, including the store keys
//! (`access_key`, `admin_password`, `session_secret`) so an existing store
//! keeps working.

use crate::store::Store;
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const SESSION_COOKIE: &str = "jiotv_session";
pub const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub const KEY_PREFIX: &str = "/k/";

const STORE_KEY_ACCESS: &str = "access_key";
const STORE_KEY_PASSWORD: &str = "admin_password";
const STORE_KEY_SECRET: &str = "session_secret";

const PBKDF2_ITERATIONS: u32 = 600_000;
const MIN_PASSWORD_LEN: usize = 8;
const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);

/// Paths that need no access key. A trailing "/" matches as a prefix.
pub const OPEN_PATHS: &[&str] = &[
    "/render.m3u8",
    "/render.ts",
    "/render.key",
    "/render.mpd",
    "/render.dash/",
    "/drm",
    "/dashtime",
    "/static/",
    "/favicon.ico",
    "/jtvimage/",
    "/jtvposter/",
    "/api/auth/",
    "/ui/",
    "/",
];

pub fn is_open(path: &str) -> bool {
    OPEN_PATHS.iter().any(|p| {
        path == *p || (*p != "/" && p.ends_with('/') && path.starts_with(p))
    })
}

#[derive(Debug, thiserror::Error)]
pub enum AccessError {
    #[error("the password needs at least {0} characters")]
    WeakPassword(usize),
    #[error("too many wrong passwords, try again later")]
    TooManyAttempts,
    #[error("no admin password set")]
    NoPassword,
    #[error("unknown password format")]
    UnknownFormat,
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
}

pub struct Access {
    store: std::sync::Arc<Store>,
    key: Mutex<Option<String>>,
    secret: Mutex<Option<Vec<u8>>>,
    logins: Mutex<HashMap<String, Vec<SystemTime>>>,
}

impl Access {
    pub fn new(store: std::sync::Arc<Store>) -> Access {
        Access {
            store,
            key: Mutex::new(None),
            secret: Mutex::new(None),
            logins: Mutex::new(HashMap::new()),
        }
    }

    pub fn key(&self) -> Result<String, AccessError> {
        let mut guard = self.key.lock().unwrap();
        if let Some(k) = guard.as_ref() {
            return Ok(k.clone());
        }
        if let Some(stored) = self.store.get_opt(STORE_KEY_ACCESS) {
            if !stored.is_empty() {
                *guard = Some(stored.clone());
                return Ok(stored);
            }
        }
        let next = random_hex(16);
        self.store.set(STORE_KEY_ACCESS, &next)?;
        *guard = Some(next.clone());
        Ok(next)
    }

    pub fn rotate(&self) -> Result<String, AccessError> {
        let next = random_hex(16);
        self.store.set(STORE_KEY_ACCESS, &next)?;
        *self.key.lock().unwrap() = Some(next.clone());
        Ok(next)
    }

    pub fn playlist_path(&self) -> Result<String, AccessError> {
        Ok(format!("{KEY_PREFIX}{}/playlist.m3u", self.key()?))
    }

    // ---- admin password ----

    pub fn has_password(&self) -> bool {
        self.store
            .get_opt(STORE_KEY_PASSWORD)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    pub fn set_password(&self, password: &str) -> Result<(), AccessError> {
        if password.len() < MIN_PASSWORD_LEN {
            return Err(AccessError::WeakPassword(MIN_PASSWORD_LEN));
        }
        let mut salt = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut salt);
        let mut hash = [0u8; 32];
        pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_ITERATIONS, &mut hash);
        use base64::Engine;
        let encoded = format!(
            "pbkdf2-sha256${}${}${}",
            PBKDF2_ITERATIONS,
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(salt),
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(hash)
        );
        self.store.set(STORE_KEY_PASSWORD, &encoded)?;
        self.sign_out_everywhere()?;
        Ok(())
    }

    pub fn check_password(&self, password: &str) -> Result<bool, AccessError> {
        let encoded = self
            .store
            .get_opt(STORE_KEY_PASSWORD)
            .filter(|s| !s.is_empty())
            .ok_or(AccessError::NoPassword)?;
        let parts: Vec<&str> = encoded.split('$').collect();
        if parts.len() != 4 || parts[0] != "pbkdf2-sha256" {
            return Err(AccessError::UnknownFormat);
        }
        let iterations: u32 = parts[1].parse().map_err(|_| AccessError::UnknownFormat)?;
        use base64::Engine;
        let salt = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(parts[2])
            .map_err(|_| AccessError::UnknownFormat)?;
        let want = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(parts[3])
            .map_err(|_| AccessError::UnknownFormat)?;
        let mut got = vec![0u8; want.len()];
        pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, iterations, &mut got);
        use subtle::ConstantTimeEq;
        Ok(got.ct_eq(&want).into())
    }

    fn session_secret(&self) -> Result<Vec<u8>, AccessError> {
        let mut guard = self.secret.lock().unwrap();
        if let Some(s) = guard.as_ref() {
            return Ok(s.clone());
        }
        if let Some(stored) = self.store.get_opt(STORE_KEY_SECRET) {
            if let Ok(b) = hex::decode(&stored) {
                *guard = Some(b.clone());
                return Ok(b);
            }
        }
        drop(guard);
        self.new_secret()
    }

    fn new_secret(&self) -> Result<Vec<u8>, AccessError> {
        let mut b = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut b);
        self.store.set(STORE_KEY_SECRET, &hex::encode(&b))?;
        *self.secret.lock().unwrap() = Some(b.clone());
        Ok(b)
    }

    pub fn sign_out_everywhere(&self) -> Result<(), AccessError> {
        self.new_secret()?;
        Ok(())
    }

    fn sign(&self, payload: &str) -> Result<String, AccessError> {
        let secret = self.session_secret()?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret).expect("hmac key");
        mac.update(payload.as_bytes());
        use base64::Engine;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
    }

    pub fn new_session(&self, now: SystemTime) -> Result<String, AccessError> {
        let expires = now
            .checked_add(SESSION_TTL)
            .unwrap_or(now)
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let payload = expires.to_string();
        let sig = self.sign(&payload)?;
        Ok(format!("{payload}.{sig}"))
    }

    pub fn valid_session(&self, value: &str, now: SystemTime) -> bool {
        let Some((payload, sig)) = value.split_once('.') else {
            return false;
        };
        let Ok(expires) = payload.parse::<u64>() else {
            return false;
        };
        let now_secs = now.duration_since(UNIX_EPOCH).unwrap().as_secs();
        if now_secs >= expires {
            return false;
        }
        match self.sign(payload) {
            Ok(want) => {
                use subtle::ConstantTimeEq;
                sig.as_bytes().ct_eq(want.as_bytes()).into()
            }
            Err(_) => false,
        }
    }

    pub fn login(&self, ip: &str, password: &str, now: SystemTime) -> Result<bool, AccessError> {
        {
            let mut logins = self.logins.lock().unwrap();
            let recent = recent(&mut logins, ip, now);
            if recent.len() >= MAX_FAILURES {
                return Err(AccessError::TooManyAttempts);
            }
        }
        let ok = self.check_password(password)?;
        let mut logins = self.logins.lock().unwrap();
        if ok {
            logins.remove(ip);
        } else {
            let entry = logins.entry(ip.to_string()).or_default();
            entry.retain(|t| now.duration_since(*t).unwrap_or_default() < FAILURE_WINDOW);
            entry.push(now);
        }
        Ok(ok)
    }
}

fn recent(map: &mut HashMap<String, Vec<SystemTime>>, ip: &str, now: SystemTime) -> Vec<SystemTime> {
    let kept: Vec<SystemTime> = map
        .get(ip)
        .map(|v| {
            v.iter()
                .copied()
                .filter(|t| now.duration_since(*t).unwrap_or_default() < FAILURE_WINDOW)
                .collect()
        })
        .unwrap_or_default();
    map.insert(ip.to_string(), kept.clone());
    kept
}

fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn access() -> Access {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().to_str().unwrap()).unwrap());
        std::mem::forget(dir); // keep temp dir alive for the test
        Access::new(store)
    }

    #[test]
    fn key_is_created_and_stable() {
        let a = access();
        let k1 = a.key().unwrap();
        let k2 = a.key().unwrap();
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 32);
    }

    #[test]
    fn rotate_changes_key() {
        let a = access();
        let k1 = a.key().unwrap();
        let k2 = a.rotate().unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn open_paths_match_prefixes() {
        assert!(is_open("/"));
        assert!(is_open("/render.m3u8"));
        assert!(is_open("/jtvimage/foo.png"));
        assert!(!is_open("/playlist.m3u"));
        assert!(!is_open("/api/channels"));
    }

    #[test]
    fn password_round_trip() {
        let a = access();
        assert!(!a.has_password());
        a.set_password("hunter22").unwrap();
        assert!(a.has_password());
        assert!(a.check_password("hunter22").unwrap());
        assert!(!a.check_password("wrong").unwrap());
    }

    #[test]
    fn weak_password_rejected() {
        let a = access();
        assert!(matches!(
            a.set_password("short"),
            Err(AccessError::WeakPassword(_))
        ));
    }

    #[test]
    fn session_round_trip_and_expiry() {
        let a = access();
        a.set_password("hunter22").unwrap();
        let now = SystemTime::now();
        let session = a.new_session(now).unwrap();
        assert!(a.valid_session(&session, now));
        let later = now + Duration::from_secs(SESSION_TTL.as_secs() + 1);
        assert!(!a.valid_session(&session, later));
    }

    #[test]
    fn changing_password_invalidates_sessions() {
        let a = access();
        a.set_password("hunter22").unwrap();
        let now = SystemTime::now();
        let session = a.new_session(now).unwrap();
        a.set_password("hunter222").unwrap();
        assert!(!a.valid_session(&session, now));
    }

    #[test]
    fn rate_limits_after_five_failures() {
        let a = access();
        a.set_password("hunter22").unwrap();
        let now = SystemTime::now();
        for _ in 0..5 {
            assert!(!a.login("1.2.3.4", "wrong", now).unwrap());
        }
        assert!(matches!(
            a.login("1.2.3.4", "hunter22", now),
            Err(AccessError::TooManyAttempts)
        ));
    }
}
