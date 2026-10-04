//! Catchup stream resolution: `/catchup/stream/:id.m3u8?start=&end=`.
//! Mirrors `CatchupStreamHandler` in `internal/handlers/catchup.go`. The
//! HTML catchup-browsing pages (`/catchup/:id` EPG listing, the catchup
//! player pages) are not ported — there is no template engine in this
//! rewrite and the Svelte UI has no catchup browser yet (see the README).

use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use std::sync::Arc;

#[derive(serde::Deserialize)]
pub struct CatchupQuery {
    start: Option<String>,
    end: Option<String>,
    srno: Option<String>,
}

fn to_jio_time(v: &str) -> String {
    // Accepts a millisecond Unix timestamp (as the UI would send) and
    // formats it as Jio's "20060102T150405"; if `v` isn't numeric, it's
    // passed through as-is (the caller may already have the right shape).
    match v.parse::<i64>() {
        Ok(ms) if ms > 0 => {
            let (y, m, d, hh, mm, ss) = crate::television::civil_from_unix((ms / 1000) as u64);
            format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}")
        }
        _ => v.to_string(),
    }
}

pub async fn catchup_stream_handler(
    axum::extract::Path(id): axum::extract::Path<String>,
    Query(q): Query<CatchupQuery>,
    State(state): State<Arc<AppState>>,
    prefix: Option<axum::Extension<crate::api::KeyPrefix>>,
) -> Response {
    let epoch = state.secure.current_epoch();
    let response = catchup_stream_inner(id, q, state.clone(), prefix).await;
    state.stable_since(epoch, response)
}

async fn catchup_stream_inner(
    id: String,
    q: CatchupQuery,
    state: Arc<AppState>,
    prefix: Option<axum::Extension<crate::api::KeyPrefix>>,
) -> Response {
    let id = id.trim_end_matches(".m3u8").to_string();
    if !state.channel_allowed(&id).await || crate::extras::content_id::content_id(&id).is_some() {
        return (
            StatusCode::NOT_FOUND,
            format!("Channel {id} is not available for TV catch-up"),
        )
            .into_response();
    }
    let (Some(start), Some(end)) = (q.start, q.end) else {
        return (StatusCode::BAD_REQUEST, "Missing start or end time").into_response();
    };
    let srno = q.srno.unwrap_or_default();

    crate::token_refresh::ensure_fresh(&state).await;

    let start_fmt = to_jio_time(&start);
    let end_fmt = to_jio_time(&end);

    let result = match state.tv.catchup_url(&id, &srno, &start_fmt, &end_fmt).await {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let target = if !result.bitrates.auto.is_empty() {
        result.bitrates.auto.clone()
    } else {
        result.result.clone()
    };
    if target.is_empty() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to get catchup URL from API",
        )
            .into_response();
    }

    let encrypted = state.secure.encrypt(&target);
    let prefix_str = prefix.as_ref().map(|p| p.0 .0.clone()).unwrap_or_default();
    let mut redirect = format!("{prefix_str}/render.m3u8?auth={encrypted}&channel_key_id={id}");
    if !result.hdnea.is_empty() && !target.contains("hdnea=") && !target.contains("__hdnea__=") {
        redirect.push_str(&format!("&hdnea={}", urlencoding::encode(&result.hdnea)));
    }
    Redirect::to(&redirect).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_millis_to_jio_timestamp() {
        // 1704164645000 ms == 2024-01-02T03:04:05Z
        assert_eq!(to_jio_time("1704164645000"), "20240102T030405");
    }

    #[test]
    fn passes_through_non_numeric_values() {
        assert_eq!(to_jio_time("20240102T030405"), "20240102T030405");
    }
}
