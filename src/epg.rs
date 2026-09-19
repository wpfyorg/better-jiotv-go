//! EPG (Electronic Program Guide) generation and serving. Mirrors
//! `pkg/epg` and the EPG handlers in `internal/handlers/epg.go`. The
//! daily-regeneration scheduler (`epg.Init`'s "run again ~24h from now at a
//! random off-peak time") is not ported — only on-demand generation (`epg
//! generate`) and a startup check for a missing/stale file. See the
//! README's parity notes.

use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::io::Write;
use std::sync::Arc;

const CHANNEL_LIST_URL: &str = "https://jiotv.data.cdn.jio.com/apis/v3.1/getMobileChannelList/get/?os=android&devicetype=phone&usertype=tvYR7NSNn7rymo3F";
const EPG_URL_FMT: &str = "https://jiotv.data.cdn.jio.com/apis/v1.3/getepg/get?offset={offset}&channel_id={id}";
pub const EPG_POSTER_URL: &str = "https://jiotv.catchup.cdn.jio.com/dare_images/shows/";

#[derive(Deserialize)]
struct ChannelObject {
    #[serde(rename = "channel_id")]
    channel_id: i64,
    #[serde(rename = "channel_name")]
    channel_name: String,
}

#[derive(Deserialize)]
struct ChannelsResponse {
    #[serde(rename = "result", default)]
    channels: Vec<ChannelObject>,
}

#[derive(Deserialize, Default)]
struct EpgObject {
    #[serde(rename = "startEpoch", default)]
    start_epoch: i64,
    #[serde(rename = "endEpoch", default)]
    end_epoch: i64,
    #[serde(rename = "showCategory", default)]
    show_category: String,
    #[serde(default)]
    description: String,
    #[serde(rename = "showname", default)]
    title: String,
    #[serde(rename = "episodePoster", default)]
    poster: String,
}

#[derive(Deserialize, Default)]
struct EpgResponse {
    #[serde(default)]
    epg: Vec<EpgObject>,
}

struct Programme {
    channel: String,
    start: String,
    stop: String,
    title: String,
    desc: String,
    category: String,
    icon: String,
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Formats a Unix-millisecond epoch as XMLTV's `"20060102150405 -0700"`
/// (always `+0000`/UTC here, since that's what this rewrite computes with).
fn format_xmltv_time(epoch_ms: i64) -> String {
    let secs = (epoch_ms / 1000).max(0) as u64;
    let (y, mo, d, hh, mm, ss) = crate::television::civil_from_unix(secs);
    format!("{y:04}{mo:02}{d:02}{hh:02}{mm:02}{ss:02} +0000")
}

async fn fetch_epg_for_channel(client: &reqwest::Client, channel_id: i64) -> Vec<EpgObject> {
    let mut out = Vec::new();
    for offset in 0..2 {
        let url = EPG_URL_FMT.replace("{offset}", &offset.to_string()).replace("{id}", &channel_id.to_string());
        let Ok(resp) = client.get(&url).header(header::USER_AGENT, "okhttp/4.12.0").send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        if let Ok(parsed) = resp.json::<EpgResponse>().await {
            out.extend(parsed.epg);
        }
    }
    out
}

/// Fetches the full channel list and every channel's EPG, and renders the
/// XMLTV document as a string (without the leading `<?xml ...?>` header,
/// added by the caller — mirrors `genXML`/`GenXMLGz`).
pub async fn generate_xml(client: &reqwest::Client) -> anyhow::Result<String> {
    let resp = client.get(CHANNEL_LIST_URL).send().await?.error_for_status()?;
    let channels: ChannelsResponse = resp.json().await?;

    let semaphore = Arc::new(tokio::sync::Semaphore::new(20));
    let mut tasks = Vec::new();
    for ch in &channels.channels {
        let client = client.clone();
        let id = ch.channel_id;
        let sem = semaphore.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.unwrap();
            (id, fetch_epg_for_channel(&client, id).await)
        }));
    }

    let mut programmes = Vec::new();
    for task in tasks {
        if let Ok((id, entries)) = task.await {
            for e in entries {
                programmes.push(Programme {
                    channel: id.to_string(),
                    start: format_xmltv_time(e.start_epoch),
                    stop: format_xmltv_time(e.end_epoch),
                    title: e.title,
                    desc: e.description,
                    category: e.show_category,
                    icon: format!("{EPG_POSTER_URL}{}", e.poster),
                });
            }
        }
    }

    if programmes.is_empty() {
        anyhow::bail!("no EPG programmes were fetched");
    }

    let mut xml = String::new();
    xml.push_str(r#"<tv version="" encoding="">"#);
    for ch in &channels.channels {
        xml.push_str(&format!(
            "<channel id=\"{}\"><display-name>{}</display-name></channel>",
            ch.channel_id,
            xml_escape(&ch.channel_name)
        ));
    }
    for p in &programmes {
        xml.push_str(&format!(
            "<programme channel=\"{}\" start=\"{}\" stop=\"{}\"><title lang=\"en\">{}</title><desc lang=\"en\">{}</desc><category lang=\"en\">{}</category><icon src=\"{}\"></icon></programme>",
            p.channel, p.start, p.stop, xml_escape(&p.title), xml_escape(&p.desc), xml_escape(&p.category), xml_escape(&p.icon)
        ));
    }
    xml.push_str("</tv>");
    Ok(xml)
}

/// Generates the EPG and writes it, gzip-compressed, to `path` (normally
/// `<path_prefix>/epg.xml.gz`).
pub async fn generate_xml_gz(client: &reqwest::Client, path: &str) -> anyhow::Result<()> {
    let xml = generate_xml(client).await?;
    let header = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\t<!DOCTYPE tv SYSTEM \"http://www.w3.org/2006/05/tv\">";
    let full = format!("{header}{xml}");

    let file = std::fs::File::create(path)?;
    let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    gz.write_all(full.as_bytes())?;
    gz.finish()?;
    Ok(())
}

/// `GET /epg.xml.gz` — serves the pre-generated file, or 404.
pub async fn epg_handler(State(state): State<Arc<AppState>>) -> Response {
    let path = format!("{}epg.xml.gz", state.path_prefix);
    match tokio::fs::read(&path).await {
        Ok(bytes) => Response::builder()
            .header(header::CONTENT_TYPE, "application/gzip")
            .header(header::CONTENT_ENCODING, "gzip")
            .body(Body::from(bytes))
            .unwrap(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            "EPG not found. Set JIOTV_EPG=true and restart, or run `jiotv epg generate`.",
        )
            .into_response(),
    }
}

/// `GET /epg/:channelID/:offset` — proxies a single channel/day's schedule,
/// mirroring `WebEPGHandler`'s day-offset correction (skipped here: this
/// rewrite passes the upstream response straight through — see README).
pub async fn web_epg_handler(Path((channel_id, offset)): Path<(String, String)>, State(state): State<Arc<AppState>>) -> Response {
    let channel_id = channel_id.strip_prefix("sl").unwrap_or(&channel_id);
    if channel_id.parse::<i64>().is_err() {
        return (StatusCode::BAD_REQUEST, "Invalid channel ID").into_response();
    }
    if offset.parse::<i64>().is_err() {
        return (StatusCode::BAD_REQUEST, "Invalid offset").into_response();
    }
    let url = EPG_URL_FMT.replace("{offset}", &offset).replace("{id}", channel_id);
    match state.http.get(&url).header(header::USER_AGENT, "okhttp/4.12.0").send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let bytes = resp.bytes().await.unwrap_or_default();
            Response::builder().status(status).header(header::CONTENT_TYPE, "application/json").body(Body::from(bytes)).unwrap()
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "upstream EPG request failed").into_response(),
    }
}

/// `GET /jtvposter/:date/:file`
pub async fn poster_handler(Path((date, file)): Path<(String, String)>, State(state): State<Arc<AppState>>) -> Response {
    let url = format!("{EPG_POSTER_URL}{date}/{file}");
    match state.http.get(&url).send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let ct = resp.headers().get(header::CONTENT_TYPE).cloned();
            let bytes = resp.bytes().await.unwrap_or_default();
            let mut builder = Response::builder().status(status);
            if let Some(ct) = ct {
                builder = builder.header(header::CONTENT_TYPE, ct);
            }
            builder.body(Body::from(bytes)).unwrap()
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "upstream poster request failed").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xml_special_characters() {
        assert_eq!(xml_escape("Tom & Jerry <Show>"), "Tom &amp; Jerry &lt;Show&gt;");
    }

    #[test]
    fn formats_xmltv_time() {
        // 2024-01-02T03:04:05Z in ms
        assert_eq!(format_xmltv_time(1704164645000), "20240102030405 +0000");
    }
}
