//! Configuration: env vars (`JIOTV_*`) and an optional TOML/YAML/JSON config
//! file, mirroring the Go `internal/config` package. Only TOML is actually
//! parsed here (the Go version also read YAML/JSON via cleanenv); TOML is the
//! format the project's own docs recommend and the one `store_v4.toml` uses.

use serde::Deserialize;
use std::env;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub epg: bool,
    pub debug: bool,
    pub disable_ts_handler: bool,
    #[serde(default = "default_true")]
    pub disable_logout: bool,
    pub drm: bool,
    /// `extras` is the current config key; `tvplus` is accepted silently so
    /// an existing config file from before the rename keeps working.
    #[serde(alias = "tvplus")]
    pub extras: bool,
    pub disable_auth: bool,
    pub title: String,
    pub disable_url_encryption: bool,
    pub proxy: String,
    pub path_prefix: String,
    pub log_path: String,
    #[serde(default = "default_true")]
    pub log_to_stdout: bool,
    pub custom_channels_file: String,
    pub default_categories: Vec<i64>,
    pub default_languages: Vec<i64>,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            epg: false,
            debug: false,
            disable_ts_handler: false,
            disable_logout: false,
            drm: true,
            extras: false,
            disable_auth: false,
            title: "JioTV Go".to_string(),
            disable_url_encryption: false,
            proxy: String::new(),
            path_prefix: String::new(),
            log_path: String::new(),
            log_to_stdout: true,
            custom_channels_file: String::new(),
            default_categories: Vec::new(),
            default_languages: Vec::new(),
        }
    }
}

impl Config {
    /// Loads config from `filename` if given, else the first common config
    /// file in the current directory, else environment variables only.
    pub fn load(filename: Option<&str>) -> anyhow::Result<Config> {
        let mut cfg = Config::default();
        let path = filename
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .or_else(common_file);

        if let Some(path) = path {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("reading config file {path}: {e}"))?;
            cfg = toml::from_str(&text)
                .map_err(|e| anyhow::anyhow!("parsing config file {path}: {e}"))?;
        }
        cfg.apply_env();
        Ok(cfg)
    }

    fn apply_env(&mut self) {
        macro_rules! env_bool {
            ($field:ident, $name:literal) => {
                if let Ok(v) = env::var($name) {
                    if let Ok(b) = v.parse::<bool>() {
                        self.$field = b;
                    }
                }
            };
        }
        macro_rules! env_str {
            ($field:ident, $name:literal) => {
                if let Ok(v) = env::var($name) {
                    self.$field = v;
                }
            };
        }
        env_bool!(epg, "JIOTV_EPG");
        env_bool!(debug, "JIOTV_DEBUG");
        env_bool!(disable_ts_handler, "JIOTV_DISABLE_TS_HANDLER");
        env_bool!(disable_logout, "JIOTV_DISABLE_LOGOUT");
        env_bool!(drm, "JIOTV_DRM");
        // `JIOTV_EXTRAS` is the current env var; `JIOTV_TVPLUS` is accepted
        // silently as a fallback so an existing deployment keeps working.
        env_bool!(extras, "JIOTV_TVPLUS");
        env_bool!(extras, "JIOTV_EXTRAS");
        env_bool!(disable_auth, "JIOTV_DISABLE_AUTH");
        env_str!(title, "JIOTV_TITLE");
        env_bool!(disable_url_encryption, "JIOTV_DISABLE_URL_ENCRYPTION");
        env_str!(proxy, "JIOTV_PROXY");
        env_str!(path_prefix, "JIOTV_PATH_PREFIX");
        env_str!(log_path, "JIOTV_LOG_PATH");
        env_bool!(log_to_stdout, "JIOTV_LOG_TO_STDOUT");
        env_str!(custom_channels_file, "JIOTV_CUSTOM_CHANNELS_FILE");
        if let Ok(v) = env::var("JIOTV_DEFAULT_CATEGORIES") {
            self.default_categories = parse_int_list(&v);
        }
        if let Ok(v) = env::var("JIOTV_DEFAULT_LANGUAGES") {
            self.default_languages = parse_int_list(&v);
        }
    }
}

fn parse_int_list(v: &str) -> Vec<i64> {
    v.split(',')
        .filter_map(|s| s.trim().parse::<i64>().ok())
        .collect()
}

fn common_file() -> Option<String> {
    const NAMES: &[&str] = &[
        "jiotv_go.yml",
        "jiotv_go.yaml",
        "jiotv_go.toml",
        "jiotv_go.json",
        "config.json",
        "config.yml",
        "config.toml",
        "config.yaml",
    ];
    NAMES
        .iter()
        .find(|n| Path::new(n).exists())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // env::set_var is process-global; serialize tests that touch it.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_drm_is_true() {
        assert!(Config::default().drm);
    }

    #[test]
    fn env_overrides_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JIOTV_TITLE", "Test Title");
        std::env::set_var("JIOTV_DRM", "false");
        let cfg = Config::load(None).unwrap();
        assert_eq!(cfg.title, "Test Title");
        assert!(!cfg.drm);
        std::env::remove_var("JIOTV_TITLE");
        std::env::remove_var("JIOTV_DRM");
    }

    #[test]
    fn old_env_var_name_still_works() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JIOTV_TVPLUS", "true");
        let cfg = Config::load(None).unwrap();
        assert!(cfg.extras);
        std::env::remove_var("JIOTV_TVPLUS");
    }

    #[test]
    fn new_env_var_name_wins_over_old() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JIOTV_TVPLUS", "true");
        std::env::set_var("JIOTV_EXTRAS", "false");
        let cfg = Config::load(None).unwrap();
        assert!(!cfg.extras);
        std::env::remove_var("JIOTV_TVPLUS");
        std::env::remove_var("JIOTV_EXTRAS");
    }

    #[test]
    fn old_toml_key_name_still_works() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jiotv_go.toml");
        std::fs::write(&path, "tvplus = true\n").unwrap();
        let cfg = Config::load(Some(path.to_str().unwrap())).unwrap();
        assert!(cfg.extras);
    }

    #[test]
    fn toml_file_is_parsed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jiotv_go.toml");
        std::fs::write(&path, "title = \"From File\"\nepg = true\n").unwrap();
        let cfg = Config::load(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(cfg.title, "From File");
        assert!(cfg.epg);
    }
}
