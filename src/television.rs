//! JioTV login, channel list and playlist generation. Mirrors
//! `pkg/television` and the login half of `pkg/utils` in the Go version. The
//! HTTP endpoints below are copied from
//! `internal/constants/urls/urls.go` in the Go tree (not called by tests;
//! tests exercise this module against a local mock server).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

pub const JIOTV_API_DOMAIN: &str = "jiotvapi.media.jio.com";
pub const CHANNELS_API_URL: &str = "https://jiotvapi.cdn.jio.com/apis/v3.1/getMobileChannelList/get/?langId=6&os=android&devicetype=phone&usertype=JIO&version=315&langId=6";
pub const REFRESH_TOKEN_URL: &str = "https://auth.media.jio.com/tokenservice/apis/v1/refreshtoken?langId=6";
pub const REFRESH_SSO_TOKEN_URL: &str = "https://tv.media.jio.com/apis/v2.0/loginotp/refresh?langId=6";
pub const PLAYBACK_API_PATH: &str = "/playback/apis/v1.1/geturl?langId=6";
pub const LOGIN_SEND_OTP_PATH: &str = "/userservice/apis/v1/loginotp/send";
pub const LOGIN_VERIFY_OTP_PATH: &str = "/userservice/apis/v1/loginotp/verify";

pub const CATEGORY_MAP: &[(i64, &str)] = &[
    (0, "All Categories"),
    (5, "Entertainment"),
    (6, "Movies"),
    (7, "Kids"),
    (8, "Sports"),
    (9, "Lifestyle"),
    (10, "Infotainment"),
    (12, "News"),
    (13, "Music"),
    (15, "Devotional"),
    (16, "Business"),
    (17, "Educational"),
    (18, "Shopping"),
    (19, "JioDarshan"),
];

pub const LANGUAGE_MAP: &[(i64, &str)] = &[
    (0, "All Languages"),
    (1, "Hindi"),
    (2, "Marathi"),
    (3, "Punjabi"),
    (4, "Urdu"),
    (5, "Bengali"),
    (6, "English"),
    (7, "Malayalam"),
    (8, "Tamil"),
    (9, "Gujarati"),
    (10, "Odia"),
    (11, "Telugu"),
    (12, "Bhojpuri"),
    (13, "Kannada"),
    (14, "Assamese"),
    (15, "Nepali"),
    (16, "French"),
    (18, "Other"),
];

pub fn category_name(id: i64) -> &'static str {
    CATEGORY_MAP.iter().find(|(k, _)| *k == id).map(|(_, v)| *v).unwrap_or("")
}

pub fn language_name(id: i64) -> &'static str {
    LANGUAGE_MAP.iter().find(|(k, _)| *k == id).map(|(_, v)| *v).unwrap_or("")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Channel {
    #[serde(rename = "channel_id", deserialize_with = "id_from_int_or_string")]
    pub id: String,
    #[serde(rename = "channel_name")]
    pub name: String,
    #[serde(rename = "channel_url", default)]
    pub url: String,
    #[serde(rename = "key_url", default)]
    pub key_url: String,
    #[serde(rename = "logoUrl", default)]
    pub logo_url: String,
    #[serde(rename = "channelCategoryId", default)]
    pub category: i64,
    #[serde(rename = "channelLanguageId", default)]
    pub language: i64,
    #[serde(rename = "isHD", default)]
    pub is_hd: bool,
    #[serde(rename = "isCatchupAvailable", default)]
    pub is_catchup_available: bool,
    #[serde(rename = "business_type", default)]
    pub business_type: String,
}

impl Channel {
    /// True for channels the playback API refuses without a separate
    /// subscription, derived from `business_type` (the API's own
    /// `is_premium` flag mispredicts about one in five channels; see the Go
    /// comment on `television.Channel.RequiresSubscription`).
    pub fn requires_subscription(&self) -> bool {
        self.business_type.trim().eq_ignore_ascii_case("premium")
    }
}

fn id_from_int_or_string<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IdOrStr {
        Int(i64),
        Str(String),
    }
    Ok(match IdOrStr::deserialize(d)? {
        IdOrStr::Int(i) => i.to_string(),
        IdOrStr::Str(s) => s,
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChannelsResponse {
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub result: Vec<Channel>,
}

/// Holds JioTV credentials in memory. Persisted to the store by the caller
/// (see `crate::login`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub sso_token: String,
    pub crm: String,
    pub unique_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

pub struct Television {
    pub creds: RwLock<Option<Credentials>>,
    pub client: reqwest::Client,
}

impl Television {
    pub fn new(client: reqwest::Client) -> Television {
        Television {
            creds: RwLock::new(None),
            client,
        }
    }

    pub fn logged_in(&self) -> bool {
        self.creds.read().unwrap().is_some()
    }

    pub fn set_credentials(&self, c: Credentials) {
        *self.creds.write().unwrap() = Some(c);
    }

    pub fn clear_credentials(&self) {
        *self.creds.write().unwrap() = None;
    }

    fn auth_headers(&self) -> HashMap<String, String> {
        let mut h = HashMap::new();
        if let Some(c) = self.creds.read().unwrap().as_ref() {
            h.insert("ssoToken".into(), c.sso_token.clone());
            h.insert("crmid".into(), c.crm.clone());
            h.insert("uniqueId".into(), c.unique_id.clone());
            h.insert("accesstoken".into(), c.access_token.clone());
        }
        h
    }

    /// Fetches the full channel list. Works with or without a login (the
    /// list itself needs no auth in the Go version either).
    pub async fn channels(&self) -> anyhow::Result<ChannelsResponse> {
        self.channels_from(CHANNELS_API_URL).await
    }

    pub async fn channels_from(&self, url: &str) -> anyhow::Result<ChannelsResponse> {
        let mut req = self.client.get(url);
        for (k, v) in self.auth_headers() {
            req = req.header(k, v);
        }
        let resp = req.send().await?.error_for_status()?;
        Ok(resp.json::<ChannelsResponse>().await?)
    }
}

/// Generates an M3U playlist, mirroring `GenerateM3UPlaylist` in
/// `internal/handlers/handlers.go`: same `#EXTINF` attributes, same
/// `/live/...` and `/live/mpd/...` URL shapes, same KODIPROP block for DRM
/// channels.
pub struct PlaylistOptions<'a> {
    pub host_url: &'a str,
    pub quality: &'a str,
    pub split_category: &'a str,
    pub languages: &'a str,
    pub skip_genres: &'a str,
    pub sub_filter: &'a str,
}

pub fn generate_m3u_playlist(
    channels: &[Channel],
    opts: &PlaylistOptions,
    is_drm_channel: impl Fn(&str) -> bool,
    is_playable: impl Fn(&str) -> bool,
) -> String {
    let mut out = String::new();
    out.push_str("#EXTM3U x-tvg-url=\"");
    out.push_str(opts.host_url);
    out.push_str("/epg.xml.gz\"\n");
    let logo_url = format!("{}/jtvimage", opts.host_url);

    let langs: Vec<&str> = opts.languages.split(',').filter(|s| !s.is_empty()).collect();
    let skip_genres: Vec<&str> = opts.skip_genres.split(',').filter(|s| !s.is_empty()).collect();

    for ch in channels {
        if !is_playable(&ch.id) {
            continue;
        }
        if !opts.languages.is_empty() && !langs.contains(&language_name(ch.language)) {
            continue;
        }
        if !opts.skip_genres.is_empty() && skip_genres.contains(&category_name(ch.category)) {
            continue;
        }
        match opts.sub_filter {
            "hide" if ch.requires_subscription() => continue,
            "only" if !ch.requires_subscription() => continue,
            _ => {}
        }

        let (channel_url, kodi_props) = if is_drm_channel(&ch.id) {
            let mut url = format!("{}/live/mpd/{}", opts.host_url, ch.id);
            if !opts.quality.is_empty() {
                url.push_str("?q=");
                url.push_str(opts.quality);
            }
            let mut props = format!(
                "#KODIPROP:inputstream=inputstream.adaptive\n#KODIPROP:inputstream.adaptive.manifest_type=mpd\n#KODIPROP:inputstream.adaptive.license_type=com.widevine.alpha\n#KODIPROP:inputstream.adaptive.license_key={}/live/key/{}",
                opts.host_url, ch.id
            );
            if !opts.quality.is_empty() {
                props.push_str("?q=");
                props.push_str(opts.quality);
            }
            props.push('\n');
            (url, props)
        } else {
            let url = if !opts.quality.is_empty() {
                format!("{}/live/{}/{}.m3u8", opts.host_url, opts.quality, ch.id)
            } else {
                format!("{}/live/{}.m3u8", opts.host_url, ch.id)
            };
            (url, String::new())
        };

        let channel_logo_url = if ch.logo_url.starts_with("http://") || ch.logo_url.starts_with("https://") {
            ch.logo_url.clone()
        } else {
            format!("{}/{}", logo_url, ch.logo_url)
        };

        let group_title = match opts.split_category {
            "split" => format!("{} - {}", category_name(ch.category), language_name(ch.language)),
            "language" => language_name(ch.language).to_string(),
            _ => category_name(ch.category).to_string(),
        };

        out.push_str(&format!(
            "#EXTINF:-1 tvg-id=\"{}\" tvg-name=\"{}\" tvg-logo=\"{}\" tvg-language=\"{}\" tvg-type=\"{}\" group-title=\"{}\", {}\n{}{}\n",
            ch.id, ch.name, channel_logo_url, language_name(ch.language), category_name(ch.category),
            group_title, ch.name, kodi_props, channel_url
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: &str, name: &str, cat: i64, lang: i64) -> Channel {
        Channel {
            id: id.to_string(),
            name: name.to_string(),
            category: cat,
            language: lang,
            logo_url: "logo.png".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn parses_int_channel_id() {
        let json = r#"{"channel_id": 154, "channel_name": "Test", "logoUrl": "x.png"}"#;
        let ch: Channel = serde_json::from_str(json).unwrap();
        assert_eq!(ch.id, "154");
    }

    #[test]
    fn premium_business_type_requires_subscription() {
        let mut ch = channel("1", "A", 5, 1);
        ch.business_type = "PREMIUM".into();
        assert!(ch.requires_subscription());
        ch.business_type = "free".into();
        assert!(!ch.requires_subscription());
    }

    #[test]
    fn playlist_contains_extinf_and_live_url() {
        let channels = vec![channel("154", "Star Plus", 5, 1)];
        let opts = PlaylistOptions {
            host_url: "http://localhost:5001",
            quality: "",
            split_category: "",
            languages: "",
            skip_genres: "",
            sub_filter: "",
        };
        let m3u = generate_m3u_playlist(&channels, &opts, |_| false, |_| true);
        assert!(m3u.starts_with("#EXTM3U"));
        assert!(m3u.contains("tvg-id=\"154\""));
        assert!(m3u.contains("http://localhost:5001/live/154.m3u8"));
    }

    #[test]
    fn drm_channel_gets_mpd_and_kodiprops() {
        let channels = vec![channel("999", "DRM Ch", 5, 1)];
        let opts = PlaylistOptions {
            host_url: "http://h",
            quality: "",
            split_category: "",
            languages: "",
            skip_genres: "",
            sub_filter: "",
        };
        let m3u = generate_m3u_playlist(&channels, &opts, |_| true, |_| true);
        assert!(m3u.contains("http://h/live/mpd/999"));
        assert!(m3u.contains("KODIPROP:inputstream.adaptive.license_key=http://h/live/key/999"));
    }

    #[test]
    fn language_filter_excludes_other_languages() {
        let channels = vec![channel("1", "Hindi Ch", 5, 1), channel("2", "English Ch", 5, 6)];
        let opts = PlaylistOptions {
            host_url: "http://h",
            quality: "",
            split_category: "",
            languages: "Hindi",
            skip_genres: "",
            sub_filter: "",
        };
        let m3u = generate_m3u_playlist(&channels, &opts, |_| false, |_| true);
        assert!(m3u.contains("Hindi Ch"));
        assert!(!m3u.contains("English Ch"));
    }

    #[test]
    fn sub_filter_hides_premium() {
        let mut premium = channel("1", "Premium Ch", 5, 1);
        premium.business_type = "premium".into();
        let channels = vec![premium, channel("2", "Free Ch", 5, 1)];
        let opts = PlaylistOptions {
            host_url: "http://h",
            quality: "",
            split_category: "",
            languages: "",
            skip_genres: "",
            sub_filter: "hide",
        };
        let m3u = generate_m3u_playlist(&channels, &opts, |_| false, |_| true);
        assert!(!m3u.contains("Premium Ch"));
        assert!(m3u.contains("Free Ch"));
    }

    #[test]
    fn not_playable_channels_are_skipped() {
        let channels = vec![channel("1", "A", 5, 1)];
        let opts = PlaylistOptions {
            host_url: "http://h",
            quality: "",
            split_category: "",
            languages: "",
            skip_genres: "",
            sub_filter: "",
        };
        let m3u = generate_m3u_playlist(&channels, &opts, |_| false, |_| false);
        assert_eq!(m3u.trim(), "#EXTM3U x-tvg-url=\"http://h/epg.xml.gz\"");
    }
}
