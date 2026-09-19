//! A tiny TOML key/value store, file-compatible with the Go version's
//! `store_v4.toml` (`{ data = { k = "v", ... } }`), so an existing store
//! keeps working when a router is moved from the Go binary to this one.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Default, Serialize, Deserialize)]
struct FileFormat {
    #[serde(default)]
    data: BTreeMap<String, String>,
}

pub struct Store {
    path: PathBuf,
    inner: Mutex<FileFormat>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Returned by `get` (the non-`Option` companion to `get_opt`, which
    /// every current caller uses instead); kept as public API.
    #[allow(dead_code)]
    #[error("key not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Decode(#[from] toml::de::Error),
    #[error(transparent)]
    Encode(#[from] toml::ser::Error),
}

impl Store {
    pub fn open(path_prefix: &str) -> Result<Store, StoreError> {
        std::fs::create_dir_all(path_prefix)?;
        let path = PathBuf::from(path_prefix).join("store_v4.toml");
        let inner = if path.exists() {
            let text = std::fs::read_to_string(&path)?;
            toml::from_str(&text)?
        } else {
            let f = FileFormat::default();
            let store = Store {
                path: path.clone(),
                inner: Mutex::new(f),
            };
            store.save()?;
            return Ok(store);
        };
        Ok(Store {
            path,
            inner: Mutex::new(inner),
        })
    }

    fn save(&self) -> Result<(), StoreError> {
        let guard = self.inner.lock().unwrap();
        let text = toml::to_string(&*guard)?;
        std::fs::write(&self.path, text)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get(&self, key: &str) -> Result<String, StoreError> {
        let guard = self.inner.lock().unwrap();
        guard
            .data
            .get(key)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(key.to_string()))
    }

    pub fn get_opt(&self, key: &str) -> Option<String> {
        self.inner.lock().unwrap().data.get(key).cloned()
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), StoreError> {
        {
            let mut guard = self.inner.lock().unwrap();
            guard.data.insert(key.to_string(), value.to_string());
        }
        self.save()
    }

    pub fn delete(&self, key: &str) -> Result<(), StoreError> {
        {
            let mut guard = self.inner.lock().unwrap();
            guard.data.remove(key);
        }
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_values() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_str().unwrap()).unwrap();
        store.set("access_key", "abc123").unwrap();
        assert_eq!(store.get("access_key").unwrap(), "abc123");
        store.delete("access_key").unwrap();
        assert!(store.get("access_key").is_err());
    }

    #[test]
    fn reads_go_format_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store_v4.toml");
        std::fs::write(&path, "[data]\naccess_key = \"deadbeef\"\n").unwrap();
        let store = Store::open(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(store.get("access_key").unwrap(), "deadbeef");
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = Store::open(dir.path().to_str().unwrap()).unwrap();
            store.set("k", "v").unwrap();
        }
        let store = Store::open(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(store.get("k").unwrap(), "v");
    }
}
