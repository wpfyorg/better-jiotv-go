//! EPG (Electronic Program Guide) generation and serving. Mirrors
//! `pkg/epg` and the EPG handlers in `internal/handlers/epg.go`. The
//! background regeneration loop (`main::epg_task_loop`) approximates the Go
//! version's "run again ~24h out at a random off-peak time" without
//! reproducing its exact hour arithmetic; see the README's parity notes.

use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::io::Write;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// One `<programme>` entry, public so `tvplus::to_xmltv` can hand in extra
/// entries from the TV+-only channels (mirrors `epg.Programme` in the Go
/// tree, used the same way by `TVPlusEPGSource`).
pub struct XmlProgramme {
    pub channel: String,
    pub start: String,
    pub stop: String,
    pub title: String,
    pub desc: String,
    pub category: String,
    pub icon: String,
}

struct XmlChannel {
    id: String,
    display: String,
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
pub fn format_xmltv_time(epoch_ms: i64) -> String {
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

/// Fetches the full channel list and every channel's EPG, appends any extra
/// (channel, programme) pairs from `extra_sources` (TV+'s catalogue, when
/// enabled — see `tvplus_state::epg_source`, mirroring `RegisterSource` /
/// `TVPlusEPGSource` in the Go tree), and renders the XMLTV document as a
/// string (without the leading `<?xml ...?>` header, added by the caller —
/// mirrors `genXML`/`GenXMLGz`).
pub async fn generate_xml(client: &reqwest::Client, extra_sources: Vec<(String, String)>, extra_programmes: Vec<XmlProgramme>) -> anyhow::Result<String> {
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
                programmes.push(XmlProgramme {
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
    programmes.extend(extra_programmes);

    if programmes.is_empty() {
        anyhow::bail!("no EPG programmes were fetched");
    }

    let extra_channels: Vec<XmlChannel> = extra_sources.into_iter().map(|(id, display)| XmlChannel { id, display }).collect();

    let mut xml = String::new();
    xml.push_str(r#"<tv version="" encoding="">"#);
    for ch in &channels.channels {
        xml.push_str(&format!(
            "<channel id=\"{}\"><display-name>{}</display-name></channel>",
            ch.channel_id,
            xml_escape(&ch.channel_name)
        ));
    }
    for ch in &extra_channels {
        xml.push_str(&format!("<channel id=\"{}\"><display-name>{}</display-name></channel>", xml_escape(&ch.id), xml_escape(&ch.display)));
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
    generate_xml_gz_with(client, path, Vec::new(), Vec::new()).await
}

pub async fn generate_xml_gz_with(
    client: &reqwest::Client,
    path: &str,
    extra_sources: Vec<(String, String)>,
    extra_programmes: Vec<XmlProgramme>,
) -> anyhow::Result<()> {
    let xml = generate_xml(client, extra_sources, extra_programmes).await?;
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
const IST_OFFSET_MS: i64 = 5 * 60 * 60 * 1000 + 30 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// How many days the getepg API's "today" lags the real date, derived from
/// the `serverDate` it stamps on its response (both compared in IST day
/// boundaries, matching the API's own). 0 means nothing to correct. Mirrors
/// `webEPGDayOffset`.
fn web_epg_day_offset(body: &[u8]) -> i64 {
    #[derive(Deserialize)]
    struct Entry {
        #[serde(rename = "serverDate", default)]
        server_date: String,
    }
    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        epg: Vec<Entry>,
    }
    let Ok(resp) = serde_json::from_slice::<Resp>(body) else { return 0 };
    let Some(first) = resp.epg.first() else { return 0 };
    if first.server_date.is_empty() {
        return 0;
    }
    let Some(server_day_start) = crate::dash::parse_rfc3339(&first.server_date) else { return 0 };
    let server_ms = server_day_start.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let epg_day = (server_ms + IST_OFFSET_MS) / DAY_MS;
    let today = (now_ms + IST_OFFSET_MS) / DAY_MS;
    (today - epg_day).max(0)
}

async fn fetch_web_epg(state: &AppState, channel_id: &str, offset: i64) -> Option<(StatusCode, Vec<u8>)> {
    let url = EPG_URL_FMT.replace("{offset}", &offset.to_string()).replace("{id}", channel_id);
    let resp = state.http.get(&url).header(header::USER_AGENT, "okhttp/4.12.0").send().await.ok()?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let bytes = resp.bytes().await.ok()?.to_vec();
    Some((status, bytes))
}

/// `GET /epg/:channelID/:offset` — proxies a single channel/day's schedule.
/// If the response's `serverDate` shows the API's "today" lagging real
/// time, re-fetches with the corrected offset so the schedule covers the
/// present moment, falling back to the original response if that retry
/// fails. Mirrors `WebEPGHandler`/`webEPGWithCorrectedDay`.
pub async fn web_epg_handler(Path((channel_id, offset)): Path<(String, String)>, State(state): State<Arc<AppState>>) -> Response {
    let channel_id_trimmed = channel_id.strip_prefix("sl").unwrap_or(&channel_id);
    if channel_id_trimmed.parse::<i64>().is_err() {
        return (StatusCode::BAD_REQUEST, "Invalid channel ID").into_response();
    }
    let Ok(offset_num) = offset.parse::<i64>() else {
        return (StatusCode::BAD_REQUEST, "Invalid offset").into_response();
    };

    let Some((status, body)) = fetch_web_epg(&state, channel_id_trimmed, offset_num).await else {
        return (StatusCode::BAD_GATEWAY, "upstream EPG request failed").into_response();
    };

    let (status, body) = if status == StatusCode::OK {
        let day_offset = web_epg_day_offset(&body);
        if day_offset > 0 {
            match fetch_web_epg(&state, channel_id_trimmed, offset_num + day_offset).await {
                Some((StatusCode::OK, adjusted)) => (StatusCode::OK, adjusted),
                _ => (status, body),
            }
        } else {
            (status, body)
        }
    } else {
        (status, body)
    };

    Response::builder().status(status).header(header::CONTENT_TYPE, "application/json").body(Body::from(body)).unwrap()
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

    #[test]
    fn day_offset_is_zero_for_current_server_date() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        let now_str = {
            let (y, m, d, hh, mm, ss) = crate::television::civil_from_unix((now / 1000) as u64);
            format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.000Z")
        };
        let body = format!(r#"{{"epg":[{{"serverDate":"{now_str}"}}]}}"#);
        assert_eq!(web_epg_day_offset(body.as_bytes()), 0);
    }

    #[test]
    fn day_offset_is_positive_when_server_date_is_stale() {
        let stale = "2020-01-01T00:00:00.000Z";
        let body = format!(r#"{{"epg":[{{"serverDate":"{stale}"}}]}}"#);
        assert!(web_epg_day_offset(body.as_bytes()) > 1000);
    }

    #[test]
    fn day_offset_is_zero_without_a_server_date() {
        assert_eq!(web_epg_day_offset(br#"{"epg":[]}"#), 0);
        assert_eq!(web_epg_day_offset(b"not json"), 0);
    }
}
