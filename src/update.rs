//! `jiotv update`: replaces this binary with a release from GitHub.
//!
//! Releases come from `JIOTV_UPDATE_REPO` (default below). Each release
//! carries one binary per build and target, named
//! `jiotv-<full|slim>-<target triple>`, plus a `SHA256SUMS` file. A private
//! repository needs a token in `JIOTV_UPDATE_TOKEN` (or `GITHUB_TOKEN`) with
//! read access to its contents.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;

const DEFAULT_REPO: &str = "wpfyorg/jiotv_go-tvplus";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The build this binary is: the update keeps it.
pub const VARIANT: &str = if cfg!(feature = "full") { "full" } else { "slim" };

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    url: String,
}

fn repo() -> String {
    std::env::var("JIOTV_UPDATE_REPO").ok().filter(|r| !r.is_empty()).unwrap_or_else(|| DEFAULT_REPO.to_string())
}

fn token() -> Option<String> {
    ["JIOTV_UPDATE_TOKEN", "GITHUB_TOKEN"].iter().find_map(|k| std::env::var(k).ok().filter(|t| !t.is_empty()))
}

/// The asset name for this binary, e.g. `jiotv-slim-aarch64-unknown-linux-musl`.
pub fn asset_name() -> String {
    format!("jiotv-{VARIANT}-{}", target_triple())
}

fn target_triple() -> &'static str {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "linux") => "aarch64-unknown-linux-musl",
        ("x86_64", "linux") => "x86_64-unknown-linux-musl",
        ("arm", "linux") => "armv7-unknown-linux-musleabihf",
        ("aarch64", "macos") => "aarch64-apple-darwin",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        _ => "unsupported",
    }
}

/// Parses "v1.2.3" or "1.2.3" into comparable numbers.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let v = v.split(['-', '+']).next()?;
    let mut parts = v.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)))
}

fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_version(tag), parse_version(current)) {
        (Some(t), Some(c)) => t > c,
        _ => false,
    }
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder().user_agent(format!("jiotv/{VERSION}")).timeout(Duration::from_secs(120)).build()?)
}

fn authed(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    let req = req.header("X-GitHub-Api-Version", "2022-11-28");
    match token() {
        Some(t) => req.bearer_auth(t),
        None => req,
    }
}

async fn fetch_release(client: &reqwest::Client, version: Option<&str>) -> anyhow::Result<Release> {
    let repo = repo();
    let url = match version {
        Some(v) => {
            let tag = if v.starts_with('v') { v.to_string() } else { format!("v{v}") };
            format!("https://api.github.com/repos/{repo}/releases/tags/{tag}")
        }
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };
    let resp = authed(client.get(&url).header("Accept", "application/vnd.github+json")).send().await?;
    match resp.status().as_u16() {
        200 => Ok(resp.json().await?),
        404 if token().is_none() => anyhow::bail!("no release found in {repo}. If the repository is private, set JIOTV_UPDATE_TOKEN"),
        404 => anyhow::bail!("no release found in {repo}"),
        401 | 403 => anyhow::bail!("GitHub refused the request ({}). Check JIOTV_UPDATE_TOKEN", resp.status()),
        _ => anyhow::bail!("GitHub answered {}", resp.status()),
    }
}

async fn download_asset(client: &reqwest::Client, asset: &Asset) -> anyhow::Result<Vec<u8>> {
    // The API asset URL works for private repositories too, given a token.
    let resp = authed(client.get(&asset.url).header("Accept", "application/octet-stream")).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("downloading {} failed: {}", asset.name, resp.status());
    }
    Ok(resp.bytes().await?.to_vec())
}

/// Finds the checksum for `name` in a SHA256SUMS file.
fn checksum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        (file == name).then(|| hash.to_lowercase())
    })
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes the new binary next to the running one and renames it over it,
/// which is safe while the old binary is running on Unix.
fn replace_executable(data: &[u8]) -> anyhow::Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let dir = exe.parent().ok_or_else(|| anyhow::anyhow!("cannot find the binary's directory"))?;
    let tmp = dir.join(format!(".{}.update", exe.file_name().and_then(|n| n.to_str()).unwrap_or("jiotv")));
    std::fs::write(&tmp, data).map_err(|e| anyhow::anyhow!("cannot write to {}: {e} (run with permission to replace the binary)", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    if let Err(e) = std::fs::rename(&tmp, &exe) {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("cannot replace {}: {e}", exe.display());
    }
    Ok(exe)
}

/// `jiotv update [--version vX.Y.Z]`.
pub async fn run(version: Option<&str>) -> anyhow::Result<()> {
    if target_triple() == "unsupported" {
        anyhow::bail!("no release builds for {}/{}", std::env::consts::OS, std::env::consts::ARCH);
    }
    let client = client()?;
    let release = fetch_release(&client, version).await?;
    if version.is_none() && !is_newer(&release.tag_name, VERSION) {
        println!("jiotv {VERSION} is up to date (latest release: {}).", release.tag_name);
        return Ok(());
    }
    let name = asset_name();
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| anyhow::anyhow!("release {} has no {name}", release.tag_name))?;
    let sums_asset = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS")
        .ok_or_else(|| anyhow::anyhow!("release {} has no SHA256SUMS; not installing an unverified binary", release.tag_name))?;

    println!("Downloading {name} from {} {}...", repo(), release.tag_name);
    let sums = String::from_utf8(download_asset(&client, sums_asset).await?)?;
    let want = checksum_for(&sums, &name).ok_or_else(|| anyhow::anyhow!("SHA256SUMS has no entry for {name}"))?;
    let data = download_asset(&client, asset).await?;
    let got = sha256_hex(&data);
    if got != want {
        anyhow::bail!("checksum mismatch for {name}: expected {want}, got {got}");
    }
    let path = replace_executable(&data)?;
    println!("Updated {} to {}.", path.display(), release.tag_name);
    if crate::autostart::installed() {
        println!("Restart the service to use it: {}", crate::autostart::restart_hint());
    } else {
        println!("Restart jiotv to use it.");
    }
    Ok(())
}

/// Prints a line when a newer release exists. Quiet on any failure, so a
/// server without internet access or a token starts as usual.
pub async fn check_quietly() {
    let Ok(client) = reqwest::Client::builder().user_agent(format!("jiotv/{VERSION}")).timeout(Duration::from_secs(5)).build() else {
        return;
    };
    if let Ok(release) = fetch_release(&client, None).await {
        if is_newer(&release.tag_name, VERSION) {
            println!("Update available: {} (you have {VERSION}). Run `jiotv update`.", release.tag_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0-rc1", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }

    #[test]
    fn reads_sha256sums() {
        let sums = "abc123  jiotv-full-x86_64-unknown-linux-musl\nDEF456 *jiotv-slim-aarch64-unknown-linux-musl\n";
        assert_eq!(checksum_for(sums, "jiotv-slim-aarch64-unknown-linux-musl").as_deref(), Some("def456"));
        assert_eq!(checksum_for(sums, "jiotv-full-aarch64-unknown-linux-musl"), None);
    }

    #[test]
    fn hashes() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
