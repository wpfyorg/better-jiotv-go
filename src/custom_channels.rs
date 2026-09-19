//! Custom channels loaded from `custom_channels_file` (JSON only in this
//! rewrite; the Go version also accepted YAML — see the README's parity
//! notes). Mirrors `LoadCustomChannels`/`GetCustomChannelByID` in
//! `pkg/television/television.go`.

use crate::television::Channel;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Deserialize)]
pub struct CustomChannelEntry {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default, rename = "logo_url")]
    pub logo_url: String,
    #[serde(default)]
    pub category: i64,
    #[serde(default)]
    pub language: i64,
    #[serde(default, rename = "is_hd")]
    pub is_hd: bool,
}

#[derive(Debug, Default, Deserialize)]
struct CustomChannelsConfig {
    #[serde(default)]
    channels: Vec<CustomChannelEntry>,
}

impl From<&CustomChannelEntry> for Channel {
    fn from(e: &CustomChannelEntry) -> Channel {
        Channel {
            id: e.id.clone(),
            name: e.name.clone(),
            url: e.url.clone(),
            logo_url: e.logo_url.clone(),
            category: e.category,
            language: e.language,
            is_hd: e.is_hd,
            ..Default::default()
        }
    }
}

#[derive(Default)]
pub struct CustomChannels {
    by_id: RwLock<HashMap<String, CustomChannelEntry>>,
}

impl CustomChannels {
    pub fn new() -> CustomChannels {
        CustomChannels::default()
    }

    /// Loads (or reloads) the channel list from `path` (JSON, `{"channels":
    /// [...]}`).
    pub fn load(&self, path: &str) -> anyhow::Result<usize> {
        let text = std::fs::read_to_string(path)?;
        let cfg: CustomChannelsConfig = serde_json::from_str(&text)?;
        let mut map = HashMap::new();
        for ch in &cfg.channels {
            map.insert(ch.id.clone(), ch.clone());
        }
        let count = map.len();
        *self.by_id.write().unwrap() = map;
        Ok(count)
    }

    pub fn get(&self, id: &str) -> Option<Channel> {
        self.by_id.read().unwrap().get(id).map(Channel::from)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.by_id.read().unwrap().contains_key(id)
    }

    pub fn all(&self) -> Vec<Channel> {
        self.by_id.read().unwrap().values().map(Channel::from).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_looks_up_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.json");
        std::fs::write(
            &path,
            r#"{"channels":[{"id":"custom1","name":"My Channel","url":"https://example.com/x.m3u8","logo_url":"https://example.com/logo.png","category":5,"language":1,"is_hd":true}]}"#,
        )
        .unwrap();

        let cc = CustomChannels::new();
        let n = cc.load(path.to_str().unwrap()).unwrap();
        assert_eq!(n, 1);
        assert!(cc.contains("custom1"));
        let ch = cc.get("custom1").unwrap();
        assert_eq!(ch.name, "My Channel");
        assert_eq!(ch.url, "https://example.com/x.m3u8");
        assert!(ch.is_hd);
        assert!(!cc.contains("nope"));
    }
}
