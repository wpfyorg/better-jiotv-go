//! `jiotv autostart`: runs the server at boot.
//!
//! On Linux with systemd it installs a `jiotv` service: a system unit when
//! run as root, otherwise a user unit. The `JIOTV_*` environment of the
//! install command goes into an env file readable only by its owner, since
//! it can hold a tunnel token. On Termux it adds a line to the shell rc
//! file, like the Go version did.

use std::path::{Path, PathBuf};
use std::process::Command;

const SERVICE: &str = "jiotv";

struct Paths {
    unit: PathBuf,
    env: PathBuf,
    user: bool,
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        extern "C" {
            fn geteuid() -> u32;
        }
        // SAFETY: geteuid takes no arguments and cannot fail.
        unsafe { geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn has_systemd() -> bool {
    Path::new("/run/systemd/system").is_dir()
}

fn is_termux() -> bool {
    std::env::var("PREFIX").map(|p| p.contains("com.termux")).unwrap_or(false)
}

fn paths() -> anyhow::Result<Paths> {
    if is_root() {
        return Ok(Paths {
            unit: PathBuf::from(format!("/etc/systemd/system/{SERVICE}.service")),
            env: PathBuf::from(format!("/etc/{SERVICE}/{SERVICE}.env")),
            user: false,
        });
    }
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set"))?;
    let config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    Ok(Paths {
        unit: PathBuf::from(format!("{config}/systemd/user/{SERVICE}.service")),
        env: PathBuf::from(format!("{config}/{SERVICE}/{SERVICE}.env")),
        user: true,
    })
}

/// Whether a jiotv service is installed for this user (or system-wide).
pub fn installed() -> bool {
    has_systemd() && paths().map(|p| p.unit.exists()).unwrap_or(false)
}

pub fn restart_hint() -> String {
    match paths() {
        Ok(p) if p.user => format!("systemctl --user restart {SERVICE}"),
        _ => format!("sudo systemctl restart {SERVICE}"),
    }
}

/// Quotes one ExecStart argument for systemd.
fn quote(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@,+".contains(c)) {
        return arg.to_string();
    }
    let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$");
    format!("\"{escaped}\"")
}

fn unit_file(exec: &[String], env_file: &Path, working_dir: &Path, user: bool) -> String {
    let exec_line = exec.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
    let wanted_by = if user { "default.target" } else { "multi-user.target" };
    format!(
        "[Unit]\n\
         Description=JioTV Go\n\
         Wants=network-online.target\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={exec_line}\n\
         WorkingDirectory={}\n\
         EnvironmentFile=-{}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy={wanted_by}\n",
        working_dir.display(),
        env_file.display(),
    )
}

/// The JIOTV_* variables to keep, with the store path pinned so the service
/// finds the same logins whatever its HOME is.
fn env_file(path_prefix: &str) -> String {
    let mut vars: Vec<(String, String)> = std::env::vars().filter(|(k, _)| k.starts_with("JIOTV_") && k != "JIOTV_PATH_PREFIX").collect();
    vars.push(("JIOTV_PATH_PREFIX".to_string(), path_prefix.to_string()));
    vars.sort();
    let mut out = String::from("# Written by `jiotv autostart`.\n");
    for (k, v) in vars {
        let v = v.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!("{k}=\"{v}\"\n"));
    }
    out
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn termux_env_path() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".config/jiotv/jiotv.env"))
}

fn shell_env_file(path_prefix: &str) -> String {
    let mut vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| k.starts_with("JIOTV_") && k != "JIOTV_PATH_PREFIX")
        .collect();
    vars.push(("JIOTV_PATH_PREFIX".to_string(), path_prefix.to_string()));
    vars.sort();
    let mut out = String::from("# Written by `jiotv autostart`; keep this file private.\n");
    for (key, value) in vars {
        out.push_str(&format!("export {key}={}\n", shell_quote(&value)));
    }
    out
}

fn termux_start_line(exe: &Path, env: &Path, serve_args: &str) -> String {
    format!(
        ". {}; {} background start --args {} {TERMUX_MARKER}\n",
        shell_quote(&env.display().to_string()),
        shell_quote(&exe.display().to_string()),
        shell_quote(serve_args),
    )
}

fn termux_rc_contents(existing: &str, start_line: &str) -> (String, bool) {
    let already_set_up = existing.contains(TERMUX_MARKER);
    let mut contents = if already_set_up {
        existing
            .lines()
            .filter(|line| !line.contains(TERMUX_MARKER))
            .map(|line| format!("{line}\n"))
            .collect()
    } else {
        existing.to_string()
    };
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(start_line);
    (contents, already_set_up)
}

fn write_private(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn systemctl(user: bool, args: &[&str]) -> anyhow::Result<()> {
    let mut cmd = Command::new("systemctl");
    if user {
        cmd.arg("--user");
    }
    let status = cmd.args(args).status().map_err(|e| anyhow::anyhow!("cannot run systemctl: {e}"))?;
    if !status.success() {
        anyhow::bail!("systemctl {} failed: {status}", args.join(" "));
    }
    Ok(())
}

/// `jiotv autostart [--args "serve flags"]`.
pub fn install(serve_args: &str, config: Option<&str>, path_prefix: &str) -> anyhow::Result<()> {
    if is_termux() {
        return install_termux(serve_args, path_prefix);
    }
    if !has_systemd() {
        anyhow::bail!(
            "systemd is not running here. On OpenWrt, the openwrt-jiotv-go package starts jiotv from /etc/init.d/jiotv_go; elsewhere, start `jiotv serve` from your init system"
        );
    }
    let p = paths()?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let mut exec = vec![exe.display().to_string()];
    if let Some(c) = config {
        exec.push("--config".to_string());
        exec.push(std::fs::canonicalize(c)?.display().to_string());
    }
    exec.push("--skip-update-check".to_string());
    exec.push("serve".to_string());
    exec.extend(serve_args.split_whitespace().map(str::to_string));

    let working_dir = PathBuf::from(path_prefix);
    std::fs::create_dir_all(&working_dir)?;
    write_private(&p.env, &env_file(path_prefix))?;
    if let Some(dir) = p.unit.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p.unit, unit_file(&exec, &p.env, &working_dir, p.user))?;

    systemctl(p.user, &["daemon-reload"])?;
    systemctl(p.user, &["enable", "--now", SERVICE])?;
    println!("Installed {} and started it.", p.unit.display());
    println!("Settings from JIOTV_* variables are in {} (readable only by you).", p.env.display());
    if p.user {
        println!("To keep it running after you log out and start it at boot, run once: sudo loginctl enable-linger $USER");
        println!("Logs: journalctl --user -u {SERVICE} -f");
    } else {
        println!("Logs: journalctl -u {SERVICE} -f");
    }
    Ok(())
}

/// `jiotv autostart remove`.
pub fn remove() -> anyhow::Result<()> {
    if is_termux() {
        return remove_termux();
    }
    if !has_systemd() {
        anyhow::bail!("systemd is not running here");
    }
    let p = paths()?;
    if !p.unit.exists() {
        println!("No {SERVICE} service is installed at {}.", p.unit.display());
        return Ok(());
    }
    let _ = systemctl(p.user, &["disable", "--now", SERVICE]);
    std::fs::remove_file(&p.unit)?;
    let _ = std::fs::remove_file(&p.env);
    systemctl(p.user, &["daemon-reload"])?;
    println!("Removed the {SERVICE} service.");
    Ok(())
}

const TERMUX_MARKER: &str = "# jiotv autostart";

fn termux_rc() -> anyhow::Result<PathBuf> {
    let prefix = std::env::var("PREFIX")?;
    Ok(PathBuf::from(format!("{prefix}/etc/bash.bashrc")))
}

fn install_termux(serve_args: &str, path_prefix: &str) -> anyhow::Result<()> {
    let rc = termux_rc()?;
    let existing = std::fs::read_to_string(&rc).unwrap_or_default();
    let env = termux_env_path()?;
    write_private(&env, &shell_env_file(path_prefix))?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let line = termux_start_line(&exe, &env, serve_args);
    let (contents, already_set_up) = termux_rc_contents(&existing, &line);
    std::fs::write(&rc, contents)?;
    if already_set_up {
        println!("Updated JioTV autostart in {} (environment in {}).", rc.display(), env.display());
    } else {
        println!("jiotv will start in the background when Termux opens ({}).", rc.display());
        println!("Settings from JIOTV_* variables are in {} (readable only by you).", env.display());
    }
    Ok(())
}

fn remove_termux() -> anyhow::Result<()> {
    let rc = termux_rc()?;
    let existing = std::fs::read_to_string(&rc).unwrap_or_default();
    let kept: String = existing.lines().filter(|l| !l.contains(TERMUX_MARKER)).map(|l| format!("{l}\n")).collect();
    std::fs::write(&rc, kept)?;
    if let Ok(env) = termux_env_path() {
        let _ = std::fs::remove_file(env);
    }
    println!("Removed autostart from {}.", rc.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_exec_arguments() {
        assert_eq!(quote("/usr/local/bin/jiotv"), "/usr/local/bin/jiotv");
        assert_eq!(quote("--port"), "--port");
        assert_eq!(quote("/opt/my dir/jiotv"), "\"/opt/my dir/jiotv\"");
        assert_eq!(quote("50%"), "\"50%%\"");
    }

    #[test]
    fn unit_file_has_the_service_basics() {
        let exec = vec!["/usr/bin/jiotv".to_string(), "serve".to_string(), "--port".to_string(), "5001".to_string()];
        let unit = unit_file(&exec, Path::new("/etc/jiotv/jiotv.env"), Path::new("/var/lib/jiotv"), false);
        assert!(unit.contains("ExecStart=/usr/bin/jiotv serve --port 5001\n"));
        assert!(unit.contains("EnvironmentFile=-/etc/jiotv/jiotv.env\n"));
        assert!(unit.contains("WantedBy=multi-user.target\n"));
        assert!(unit.contains("Restart=on-failure\n"));
    }

    #[test]
    fn env_file_pins_the_store_path() {
        let env = env_file("/var/lib/jiotv/");
        assert!(env.contains("JIOTV_PATH_PREFIX=\"/var/lib/jiotv/\"\n"));
    }

    #[test]
    fn shell_quote_preserves_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn termux_autostart_sources_private_environment_before_starting() {
        let key = "JIOTV_AUTOSTART_TEST_TOKEN";
        let previous = std::env::var_os(key);
        std::env::set_var(key, "token with 'quote'");
        let contents = shell_env_file("/data/data/com.termux/files/home/.jiotv/");
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }

        assert!(contents.contains("export JIOTV_PATH_PREFIX='/data/data/com.termux/files/home/.jiotv/'\n"));
        assert!(contents.contains("export JIOTV_AUTOSTART_TEST_TOKEN='token with '\\''quote'\\'''\n"));
        let line = termux_start_line(
            Path::new("/data/data/com.termux/files/usr/bin/jiotv"),
            Path::new("/data/data/com.termux/files/home/.config/jiotv/jiotv.env"),
            "--host 0.0.0.0 --tls --tunnel",
        );
        assert!(line.starts_with(". '/data/data/com.termux/files/home/.config/jiotv/jiotv.env'; "));
        assert!(line.contains("background start --args '--host 0.0.0.0 --tls --tunnel'"));
    }

    #[test]
    fn termux_autostart_replaces_stale_arguments() {
        let old = "alias ll='ls -l'\n/home/jiotv background start --args '--host 0.0.0.0' # jiotv autostart\n";
        let new = ". '/home/.config/jiotv/jiotv.env'; /home/jiotv background start --args '--host 0.0.0.0 --tunnel' # jiotv autostart\n";
        let (contents, was_set_up) = termux_rc_contents(old, new);
        assert!(was_set_up);
        assert_eq!(contents, "alias ll='ls -l'\n".to_string() + new);
    }
}
