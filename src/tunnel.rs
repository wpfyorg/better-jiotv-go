//! Optional `cloudflared` tunnel, spawned only when `--tunnel` is passed.
//! Deliberately kept out of the dependency graph as a linked library: this
//! module just downloads/locates the `cloudflared` binary and drives it as a
//! child process, so neither build feature pulls in a tunnel crate.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

const RELEASE_BASE: &str = "https://github.com/cloudflare/cloudflared/releases/latest/download";

/// Picks the asset name cloudflared publishes for the current OS/arch.
fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("cloudflared-linux-amd64"),
        ("linux", "aarch64") => Some("cloudflared-linux-arm64"),
        ("linux", "arm") => Some("cloudflared-linux-arm"),
        ("macos", "x86_64") => Some("cloudflared-darwin-amd64.tgz"),
        ("macos", "aarch64") => Some("cloudflared-darwin-arm64.tgz"),
        _ => None,
    }
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    }
}

/// Finds `cloudflared` in `data_dir` or PATH, downloading the release binary
/// into `data_dir` if neither has it. Prints what it does, per the project's
/// hard rule that a download only happens on explicit `--tunnel`.
pub async fn ensure_cloudflared(data_dir: &Path) -> anyhow::Result<PathBuf> {
    let local = data_dir.join(binary_name());
    if local.exists() {
        return Ok(local);
    }
    if let Ok(path) = which(binary_name()) {
        return Ok(path);
    }

    let asset = asset_name().ok_or_else(|| {
        anyhow::anyhow!(
            "no cloudflared release is published for {}/{}; install cloudflared yourself and put it on PATH or in {}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            data_dir.display()
        )
    })?;
    let url = format!("{RELEASE_BASE}/{asset}");
    println!("Downloading cloudflared from {url} to {}", local.display());

    let bytes = reqwest::get(&url).await?.error_for_status()?.bytes().await?;
    std::fs::create_dir_all(data_dir)?;

    if asset.ends_with(".tgz") {
        extract_tgz_binary(&bytes, &local)?;
    } else {
        std::fs::write(&local, &bytes)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&local)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&local, perms)?;
    }

    Ok(local)
}

fn extract_tgz_binary(bytes: &[u8], out: &Path) -> anyhow::Result<()> {
    // macOS releases ship a .tgz containing a single `cloudflared` binary.
    // Rather than pull in a tar/gzip crate for a path most users on the
    // target OpenWrt router will never take, shell out to the system's own
    // tar (present on macOS and Linux).
    let tmp = tempfile::NamedTempFile::new()?;
    std::fs::write(tmp.path(), bytes)?;
    let dir = tempfile::tempdir()?;
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(tmp.path())
        .arg("-C")
        .arg(dir.path())
        .status()?;
    if !status.success() {
        anyhow::bail!("failed to extract cloudflared archive");
    }
    std::fs::copy(dir.path().join("cloudflared"), out)?;
    Ok(())
}

fn which(bin: &str) -> anyhow::Result<PathBuf> {
    let path_var = std::env::var_os("PATH").ok_or_else(|| anyhow::anyhow!("no PATH"))?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("{bin} not found on PATH")
}

pub struct Tunnel {
    child: Child,
}

impl Tunnel {
    /// Spawns a quick (trycloudflare.com) tunnel for `local_port`, returning
    /// once the public URL has been parsed from cloudflared's stderr.
    pub async fn spawn_quick(cloudflared: &Path, local_port: u16) -> anyhow::Result<(Tunnel, String)> {
        let mut child = Command::new(cloudflared)
            .args([
                "tunnel",
                "--no-autoupdate",
                "--url",
                &format!("http://127.0.0.1:{local_port}"),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stderr = child.stderr.take().expect("piped stderr");
        let url = wait_for_url(stderr).await?;
        Ok((Tunnel { child }, url))
    }

    /// Runs a named tunnel using a token (`--tunnel-token` / `JIOTV_TUNNEL_TOKEN`).
    pub async fn spawn_named(cloudflared: &Path, token: &str) -> anyhow::Result<Tunnel> {
        let child = Command::new(cloudflared)
            .args(["tunnel", "run", "--token", token])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Tunnel { child })
    }

    pub async fn kill(mut self) {
        let _ = self.child.kill().await;
    }
}

async fn wait_for_url(stderr: tokio::process::ChildStderr) -> anyhow::Result<String> {
    let mut lines = BufReader::new(stderr).lines();
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("timed out waiting for cloudflared to print a trycloudflare.com URL");
        }
        let line = tokio::time::timeout(remaining, lines.next_line()).await??;
        let Some(line) = line else {
            anyhow::bail!("cloudflared exited before printing a URL");
        };
        if let Some(url) = extract_trycloudflare_url(&line) {
            return Ok(url);
        }
    }
}

fn extract_trycloudflare_url(line: &str) -> Option<String> {
    let idx = line.find("https://")?;
    let rest = &line[idx..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let candidate = &rest[..end];
    if candidate.contains("trycloudflare.com") {
        Some(candidate.trim_end_matches(['.', ',', ')']).to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_url_from_a_log_line() {
        let line = "2024-01-01T00:00:00Z INF |  https://random-words-1234.trycloudflare.com  |";
        assert_eq!(
            extract_trycloudflare_url(line).unwrap(),
            "https://random-words-1234.trycloudflare.com"
        );
    }

    #[test]
    fn ignores_lines_without_the_domain() {
        assert!(extract_trycloudflare_url("https://example.com is not it").is_none());
        assert!(extract_trycloudflare_url("no url here").is_none());
    }

    #[test]
    fn asset_name_is_known_for_common_targets() {
        // Just exercises the match arms compile and return something for
        // linux/aarch64, the router's own target.
        let saved = (std::env::consts::OS, std::env::consts::ARCH);
        let _ = saved; // consts are compile-time; nothing to assert further here.
        assert!(asset_name().is_some() || cfg!(not(any(target_os = "linux", target_os = "macos"))));
    }
}
