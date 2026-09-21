//! Unlock gate for the optional extra channel source (`extras`).
//!
//! The feature stays off unless the `extras` config/env switch is set
//! (the way in for a headless/router install with no panel), or someone
//! enters the server's daily unlock code into the channel search box in the
//! panel. This is not a secret mechanism -- the format is documented here
//! and in `docs/config.md` -- it just needs the owner to be able to see the
//! server's own network, which a stranger can't.
//!
//! Code format, case-insensitive (compared lowercased):
//!
//! ```text
//! <octet1><MON><day>A<octet2>K<octet3>N<octet4>
//! ```
//!
//! where `<octet1..4>` are the server's public IPv4 address's four octets,
//! `<MON>` is the 3-letter or full English month name, and `<day>` is the
//! day of month (1 or 2 digits, leading zero optional). Example: public IP
//! `49.37.12.214` on 21 September gives `"49SEP21A37K12N214"`
//! (`49.SEP.21.A.37.K.12.N.214`). The date accepts yesterday, today or
//! tomorrow in the server's local time (each with its own month name, so it
//! still works right at a month boundary), and the IP is fetched from
//! `https://cloudflare.com/cdn-cgi/trace` and cached for ten minutes.

use std::net::Ipv4Addr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use subtle::ConstantTimeEq;

pub const STORE_KEY_UNLOCKED: &str = "extras_unlocked";

const PUBLIC_IP_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);

const MONTHS: [(&str, &str); 12] = [
    ("january", "jan"),
    ("february", "feb"),
    ("march", "mar"),
    ("april", "apr"),
    ("may", "may"),
    ("june", "jun"),
    ("july", "jul"),
    ("august", "aug"),
    ("september", "sep"),
    ("october", "oct"),
    ("november", "nov"),
    ("december", "dec"),
];

static LOCAL_OFFSET: OnceLock<time::UtcOffset> = OnceLock::new();

/// Captures the server's local UTC offset. Must be called once, at process
/// start, before any other thread exists -- `time`'s local-offset lookup is
/// only sound to call single-threaded. Falls back to UTC if it can't be
/// determined (e.g. the platform doesn't expose it), which just shifts the
/// accepted date window by the difference from local time.
pub fn init_local_offset() {
    let off = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let _ = LOCAL_OFFSET.set(off);
}

fn local_offset() -> time::UtcOffset {
    *LOCAL_OFFSET.get_or_init(|| time::UtcOffset::UTC)
}

/// The shape a candidate unlock code must have before we bother validating
/// it -- and, mirrored in the panel's JS, before the search box will POST it
/// to the server at all, so an ordinary channel search never leaves the
/// browser. Case-insensitive:
/// `^\d{1,3}[a-z]{3,9}\d{1,2}a\d{1,3}k\d{1,3}n\d{1,3}$`
pub fn looks_like_code(input: &str) -> bool {
    let s = input.trim();
    let b = s.as_bytes();
    let mut i = 0usize;

    fn digits(b: &[u8], i: &mut usize, max: usize) -> usize {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() && *i - start < max {
            *i += 1;
        }
        *i - start
    }

    if digits(b, &mut i, 3) == 0 {
        return false;
    }
    let alpha_start = i;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    if !(3..=9).contains(&(i - alpha_start)) {
        return false;
    }
    if digits(b, &mut i, 2) == 0 {
        return false;
    }
    if i >= b.len() || !b[i].eq_ignore_ascii_case(&b'a') {
        return false;
    }
    i += 1;
    if digits(b, &mut i, 3) == 0 {
        return false;
    }
    if i >= b.len() || !b[i].eq_ignore_ascii_case(&b'k') {
        return false;
    }
    i += 1;
    if digits(b, &mut i, 3) == 0 {
        return false;
    }
    if i >= b.len() || !b[i].eq_ignore_ascii_case(&b'n') {
        return false;
    }
    i += 1;
    if digits(b, &mut i, 3) == 0 {
        return false;
    }
    i == b.len()
}

fn candidate_codes(ip: Ipv4Addr, now: SystemTime) -> Vec<String> {
    let today = time::OffsetDateTime::from(now).to_offset(local_offset()).date();
    let dates = [today.previous_day(), Some(today), today.next_day()];
    let o = ip.octets();
    let mut out = Vec::new();
    for d in dates.into_iter().flatten() {
        let month_idx = d.month() as usize - 1;
        let (full, short) = MONTHS[month_idx];
        let day = d.day();
        let day_strs: Vec<String> = if day < 10 {
            vec![format!("{day:02}"), day.to_string()]
        } else {
            vec![day.to_string()]
        };
        for mon in [full, short] {
            for ds in &day_strs {
                out.push(format!("{}{mon}{ds}a{}k{}n{}", o[0], o[1], o[2], o[3]));
            }
        }
    }
    out
}

/// Whether `input` is a valid unlock code for `ip` on any of yesterday,
/// today or tomorrow (server local time). Compares every candidate in
/// constant time so a wrong guess doesn't reveal which part was wrong.
pub fn code_matches(ip: Ipv4Addr, now: SystemTime, input: &str) -> bool {
    let input = input.trim().to_ascii_lowercase();
    let mut ok = false;
    for candidate in candidate_codes(ip, now) {
        let eq: bool = input.as_bytes().ct_eq(candidate.as_bytes()).into();
        ok |= eq;
    }
    ok
}

/// Fetches and caches the machine's public IPv4 address from Cloudflare's
/// trace endpoint. IPv6-only or unreachable machines have no unlock code --
/// `get()` returns `None` and callers should say so plainly.
pub struct PublicIp {
    http: reqwest::Client,
    cached: Mutex<Option<(Ipv4Addr, SystemTime)>>,
}

impl PublicIp {
    pub fn new(http: reqwest::Client) -> PublicIp {
        PublicIp { http, cached: Mutex::new(None) }
    }

    pub async fn get(&self) -> Option<Ipv4Addr> {
        if let Some((ip, at)) = *self.cached.lock().unwrap() {
            if SystemTime::now().duration_since(at).unwrap_or_default() < PUBLIC_IP_TTL {
                return Some(ip);
            }
        }
        let ip = fetch(&self.http).await;
        if let Some(ip) = ip {
            *self.cached.lock().unwrap() = Some((ip, SystemTime::now()));
        }
        ip
    }
}

async fn fetch(http: &reqwest::Client) -> Option<Ipv4Addr> {
    let resp = http.get("https://cloudflare.com/cdn-cgi/trace").send().await.ok()?;
    let text = resp.text().await.ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("ip=") {
            if let Ok(v4) = rest.trim().parse::<Ipv4Addr>() {
                return Some(v4);
            }
        }
    }
    None
}

/// Per-IP rate limiting for unlock attempts, mirroring `access::Access`'s
/// admin-login limiter (5 failures / 10 minutes).
#[derive(Default)]
pub struct AttemptLimiter {
    failures: Mutex<std::collections::HashMap<String, Vec<SystemTime>>>,
}

impl AttemptLimiter {
    pub fn allowed(&self, ip: &str, now: SystemTime) -> bool {
        let mut map = self.failures.lock().unwrap();
        let kept: Vec<SystemTime> = map
            .get(ip)
            .map(|v| v.iter().copied().filter(|t| now.duration_since(*t).unwrap_or_default() < FAILURE_WINDOW).collect())
            .unwrap_or_default();
        map.insert(ip.to_string(), kept.clone());
        kept.len() < MAX_FAILURES
    }

    pub fn record_failure(&self, ip: &str, now: SystemTime) {
        let mut map = self.failures.lock().unwrap();
        let entry = map.entry(ip.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t).unwrap_or_default() < FAILURE_WINDOW);
        entry.push(now);
    }

    pub fn record_success(&self, ip: &str) {
        self.failures.lock().unwrap().remove(ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_matches_examples() {
        assert!(looks_like_code("49SEP21A37K12N214"));
        assert!(looks_like_code("49sep21a37k12n214"));
        assert!(looks_like_code("1jan1a1k1n1"));
        assert!(!looks_like_code("hello world"));
        assert!(!looks_like_code("49.37.september.12.214.21"));
        assert!(!looks_like_code(""));
    }

    #[test]
    fn shape_rejects_ordinary_search_terms() {
        for q in ["Star Sports", "hbo", "news18", "espn 2", "123"] {
            assert!(!looks_like_code(q), "should not look like a code: {q}");
        }
    }

    #[test]
    fn worked_example_matches() {
        let ip: Ipv4Addr = "49.37.12.214".parse().unwrap();
        // 21 September 2026, noon UTC.
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1789992000);
        assert!(code_matches(ip, now, "49SEP21A37K12N214"));
        assert!(code_matches(ip, now, "49september21a37k12n214"));
        assert!(code_matches(ip, now, "  49sep21a37k12n214  "));
    }

    #[test]
    fn wrong_code_rejected() {
        let ip: Ipv4Addr = "49.37.12.214".parse().unwrap();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1789992000);
        assert!(!code_matches(ip, now, "49SEP21A37K12N215"));
        assert!(!code_matches(ip, now, "1SEP21A37K12N214"));
        assert!(!code_matches(ip, now, "49OCT21A37K12N214"));
        assert!(!code_matches(ip, now, "not a code"));
    }

    #[test]
    fn accepts_yesterday_today_tomorrow_across_month_boundary() {
        let ip: Ipv4Addr = "1.2.3.4".parse().unwrap();
        // 2026-09-30 12:00 UTC; tomorrow crosses into October.
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1790769600);
        assert!(code_matches(ip, now, "1SEP29A2K3N4"), "yesterday");
        assert!(code_matches(ip, now, "1SEP30A2K3N4"), "today");
        assert!(code_matches(ip, now, "1OCT1A2K3N4"), "tomorrow, new month");
        assert!(!code_matches(ip, now, "1SEP1A2K3N4"));
    }

    #[test]
    fn rate_limiter_blocks_after_five_failures() {
        let l = AttemptLimiter::default();
        let now = SystemTime::now();
        for _ in 0..5 {
            assert!(l.allowed("1.2.3.4", now));
            l.record_failure("1.2.3.4", now);
        }
        assert!(!l.allowed("1.2.3.4", now));
    }

    #[test]
    fn rate_limiter_success_clears_failures() {
        let l = AttemptLimiter::default();
        let now = SystemTime::now();
        for _ in 0..4 {
            l.record_failure("5.6.7.8", now);
        }
        l.record_success("5.6.7.8");
        assert!(l.allowed("5.6.7.8", now));
    }
}
