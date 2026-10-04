mod access;
mod api;
mod autostart;
mod catchup;
mod cli;
mod config;
mod custom_channels;
mod dash;
mod drm_channels;
mod epg;
mod extras;
mod extras_state;
mod keyed_locks;
mod login;
mod secureurl;
mod server;
mod state;
mod store;
mod stream;
mod television;
mod tls;
mod token_refresh;
mod tunnel;
mod unlock;
mod update;
mod vod;

#[cfg(feature = "full")]
mod ui_assets;

use std::io::Write;
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    // Must run before any other thread exists (see `unlock::init_local_offset`).
    unlock::init_local_offset();

    let args = cli::parse()?;

    if let cli::Command::Help = args.command {
        print_help();
        return Ok(());
    }

    let cfg = config::Config::load(args.config.as_deref())?;
    init_logging(&cfg);

    let path_prefix = resolve_path_prefix(&cfg)?;
    let store = Arc::new(store::Store::open(&path_prefix)?);
    let access = Arc::new(access::Access::new(store.clone()));
    let secure = Arc::new(secureurl::SecureUrl::new(cfg.disable_url_encryption));
    if cfg.disable_url_encryption {
        eprintln!(
            "Warning! URL encryption is disabled. Anyone can pass modified URLs to your server."
        );
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    match args.command {
        cli::Command::Serve(serve_args) => {
            if !args.skip_update_check {
                runtime.spawn(update::check_quietly());
            }
            runtime.block_on(serve(cfg, path_prefix, store, access, secure, serve_args))
        }
        cli::Command::LoginOtp => runtime.block_on(login_otp(store)),
        cli::Command::LoginReset => login_reset(&store),
        cli::Command::ExtrasLogin => runtime.block_on(extras_login_cli(&store)),
        cli::Command::ExtrasLogout => extras_logout_cli(&store),
        cli::Command::AdminPassword => admin_password(&access, &path_prefix),
        cli::Command::KeyShow => show_key(&access),
        cli::Command::KeyRotate => rotate_key(&access),
        cli::Command::EpgGenerate => {
            runtime.block_on(epg_generate(&cfg, &path_prefix, store, access, secure))
        }
        cli::Command::EpgDelete => epg_delete(&path_prefix),
        cli::Command::BackgroundStart { args } => background_start(&args, &path_prefix),
        cli::Command::BackgroundStop => background_stop(&path_prefix),
        cli::Command::Update { version } => runtime.block_on(update::run(version.as_deref())),
        cli::Command::Autostart { args: serve_args } => {
            autostart::install(&serve_args, args.config.as_deref(), &path_prefix)
        }
        cli::Command::AutostartRemove => autostart::remove(),
        cli::Command::Help => unreachable!(),
    }
}

/// Keeps the active context's XMLTV cache fresh. Identity validation happens
/// before the age check, so a young cache from another account or mode is
/// never reused.
async fn epg_task_loop(state: Arc<state::AppState>) {
    loop {
        if let Err(e) = epg::ensure_current_cache(&state).await {
            tracing::warn!("EPG generation failed: {e}");
        }

        let jitter_secs: i64 = {
            let mut b = [0u8; 8];
            rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
            (i64::from_le_bytes(b) % (2 * 60 * 60)) - 60 * 60 // +/- 1h
        };
        let sleep_secs = (24 * 60 * 60 + jitter_secs).max(60 * 60) as u64;
        tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;
    }
}

async fn epg_generate(
    cfg: &config::Config,
    path_prefix: &str,
    store: Arc<store::Store>,
    access: Arc<access::Access>,
    secure: Arc<secureurl::SecureUrl>,
) -> anyhow::Result<()> {
    let path = format!("{path_prefix}epg.xml.gz");
    println!("Deleting existing EPG file if exists");
    epg_delete_files(&path)?;
    println!("Generating new EPG file... this can take a few minutes.");
    let state = build_app_state(cfg, path_prefix, store, access, secure)?;
    epg::regenerate_for_state(&state).await?;
    println!("EPG file generated successfully at {path}");
    Ok(())
}

const PID_FILE_NAME: &str = ".jiotv.pid";

/// Starts `serve` as a detached child process (re-invoking this same
/// binary) and records its PID, so `background stop` can find it later.
/// Mirrors `RunInBackground`.
fn background_start(args: &str, path_prefix: &str) -> anyhow::Result<()> {
    println!("Starting jiotv server in background...");
    let pid_path = format!("{path_prefix}{PID_FILE_NAME}");
    let exe = std::env::current_exe()?;

    let mut cmd_args: Vec<String> = vec!["--skip-update-check".to_string(), "serve".to_string()];
    cmd_args.extend(args.split_whitespace().map(str::to_string));

    let mut child = std::process::Command::new(&exe)
        .args(&cmd_args)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to start command: {e}"))?;

    std::fs::write(&pid_path, child.id().to_string())?;

    // Surface an immediate crash (bad flags, port in use) instead of
    // reporting success for a process that's already gone.
    std::thread::sleep(std::time::Duration::from_secs(1));
    if let Some(status) = child.try_wait()? {
        let _ = std::fs::remove_file(&pid_path);
        anyhow::bail!("server exited immediately after start: {status}");
    }

    println!("jiotv server started successfully in background.");
    Ok(())
}

/// Reads the PID file `background_start` wrote and kills that process.
/// Mirrors `StopBackground`.
fn background_stop(path_prefix: &str) -> anyhow::Result<()> {
    println!("Stopping jiotv server running in background...");
    let pid_path = format!("{path_prefix}{PID_FILE_NAME}");
    let pid_str = std::fs::read_to_string(&pid_path)
        .map_err(|e| anyhow::anyhow!("failed to read PID file: {e}"))?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("failed to parse PID file"))?;

    #[cfg(unix)]
    {
        // SAFETY: libc::kill with a valid PID and SIGTERM is the standard,
        // side-effect-contained way to ask another process to exit; no
        // pointers or shared memory are involved.
        let ret = unsafe { libc_kill(pid as i32, 15) };
        if ret != 0 {
            anyhow::bail!("failed to kill jiotv process {pid}");
        }
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()
            .map_err(|e| anyhow::anyhow!("failed to kill jiotv process {pid}: {e}"))?;
    }

    std::fs::remove_file(&pid_path)?;
    println!("jiotv server stopped successfully.");
    Ok(())
}

#[cfg(unix)]
extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

fn epg_delete(path_prefix: &str) -> anyhow::Result<()> {
    let path = format!("{path_prefix}epg.xml.gz");
    println!("Deleting existing EPG file if exists");
    let existed = std::path::Path::new(&path).exists();
    epg_delete_files(&path)?;
    println!(
        "{}",
        if existed {
            "EPG file deleted"
        } else {
            "EPG file does not exist"
        }
    );
    Ok(())
}

fn epg_delete_files(path: &str) -> anyhow::Result<()> {
    for candidate in [path.to_string(), format!("{path}.context.json")] {
        match std::fs::remove_file(candidate) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// A stable per-install device ID, matching the Go version's
/// `GetDeviceID`/`GenerateRandomString` (16 hex characters, persisted in the
/// store under `deviceId`).
fn device_id(store: &store::Store) -> anyhow::Result<String> {
    if let Some(id) = store.get_opt("deviceId") {
        if !id.is_empty() {
            return Ok(id);
        }
    }
    let mut bytes = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    let id = hex::encode(bytes);
    store.set("deviceId", &id)?;
    Ok(id)
}

fn resolve_path_prefix(cfg: &config::Config) -> anyhow::Result<String> {
    let prefix = if !cfg.path_prefix.is_empty() {
        cfg.path_prefix.clone()
    } else {
        default_path_prefix()?
    };
    // The OpenWrt default lives outside /root, so a directory created by a raw
    // install must not be readable by other users (the package applies 0700).
    if cfg.path_prefix.is_empty() && is_openwrt() {
        create_private_dir(&prefix)?;
    } else {
        std::fs::create_dir_all(&prefix)?;
    }
    Ok(if prefix.ends_with('/') {
        prefix
    } else {
        format!("{prefix}/")
    })
}

fn create_private_dir(path: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

fn default_path_prefix() -> anyhow::Result<String> {
    if is_openwrt() {
        return Ok("/etc/jiotv".to_string());
    }

    let home =
        home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve the user profile directory"))?;
    Ok(format!("{home}/.jiotv_go"))
}

fn is_openwrt() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/etc/openwrt_release").is_file()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn home_dir() -> Option<String> {
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Some(home);
        }
    }
    #[cfg(windows)]
    {
        if let Ok(home) = std::env::var("USERPROFILE") {
            if !home.is_empty() {
                return Some(home);
            }
        }
        let drive = std::env::var("HOMEDRIVE").ok()?;
        let path = std::env::var("HOMEPATH").ok()?;
        return Some(format!("{drive}{path}"));
    }
    #[cfg(not(windows))]
    None
}

fn init_logging(cfg: &config::Config) {
    if cfg.log_to_stdout {
        let level = if cfg.debug { "debug" } else { "info" };
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new(level))
            .try_init();
    }
}

async fn serve(
    cfg: config::Config,
    path_prefix: String,
    store: Arc<store::Store>,
    access: Arc<access::Access>,
    secure: Arc<secureurl::SecureUrl>,
    args: cli::ServeArgs,
) -> anyhow::Result<()> {
    let state = build_app_state(&cfg, &path_prefix, store, access.clone(), secure)?;
    if state.extras.enabled() {
        println!(
            "extras enabled{}",
            if state.extras.connected() {
                " (logged in)"
            } else {
                " (run `jiotv extras login`)"
            }
        );
    }

    let epg_path = format!("{path_prefix}epg.xml.gz");
    if cfg.epg || std::path::Path::new(&epg_path).exists() {
        if let Err(e) = epg::prepare_cache_for_state(&state).await {
            state.epg_state.invalidate();
            tracing::warn!("cannot validate the EPG cache for the active account: {e}");
        }
    }
    if cfg.epg {
        tokio::spawn(epg_task_loop(state.clone()));
    }

    if !cfg.disable_auth {
        let playlist = access.playlist_path()?;
        println!(
            "Playlist: http://{}:{}{playlist}",
            display_host(&args.host),
            args.port
        );
        #[cfg(feature = "full")]
        if !access.has_password() {
            let setup = playlist.trim_end_matches("playlist.m3u").to_string();
            // The setup link carries the access key and takes the new admin
            // password, so with TLS enabled it must be the HTTPS one.
            let origin = if args.tls {
                format!("https://{}:{}", reachable_host(&args.host), args.tls_port)
            } else {
                format!("http://{}:{}", display_host(&args.host), args.port)
            };
            println!("Web setup: {origin}{setup} (or run: jiotv admin password)");
        }
    } else {
        println!("Auth is disabled: playlist at /playlist.m3u");
    }

    let service = server::GatedService::new(state);

    let tls = if args.tls {
        if args.tls_port == args.port {
            anyhow::bail!("--tls-port must differ from --port");
        }
        let material =
            tls::load_or_create(&args.tls_cert, &args.tls_key, &path_prefix, &args.host)?;
        let tls_listener =
            tokio::net::TcpListener::bind(format!("{}:{}", args.host, args.tls_port)).await?;
        let origin = format!("https://{}:{}", reachable_host(&args.host), args.tls_port);
        if material.generated {
            println!("Generated a self-signed TLS certificate in {path_prefix}tls/");
        }
        println!("HTTPS: {origin}/ (browser UI; IPTV clients should keep using plain HTTP)");
        println!("TLS certificate SHA-256: {}", material.fingerprint);
        Some((tls_listener, material.config))
    } else {
        if !args.tls_cert.is_empty() || !args.tls_key.is_empty() {
            eprintln!("Warning: --tls-cert/--tls-key are ignored without --tls");
        }
        None
    };

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", args.host, args.port)).await?;

    let mut tunnel_handle = None;
    if args.tunnel {
        let data_dir =
            std::env::var("HOME").map(|h| std::path::PathBuf::from(h).join(".jiotv_go"))?;
        let cloudflared = tunnel::ensure_cloudflared(&data_dir).await?;
        if let Some(token) = &args.tunnel_token {
            let t = tunnel::Tunnel::spawn_named(&cloudflared, token).await?;
            tunnel_handle = Some(t);
            println!("Named tunnel running (see the Cloudflare dashboard for its hostname).");
        } else {
            let port: u16 = args.port.parse().unwrap_or(5001);
            let (t, url) = tunnel::Tunnel::spawn_quick(&cloudflared, port).await?;
            tunnel_handle = Some(t);
            let key_playlist = access
                .playlist_path()
                .unwrap_or_else(|_| "/playlist.m3u".to_string());
            println!("Public playlist: {url}{key_playlist}");
        }
    }

    tracing::info!("listening");
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let signal_tx = stop_tx.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = signal_tx.send(true);
    });
    let tls_task = tls.map(|(tls_listener, config)| {
        let svc = service.clone();
        tokio::spawn(tls::serve(
            tls_listener,
            config,
            move |peer| server::connection_service(svc.clone(), peer, true),
            stop_rx.clone(),
        ))
    });
    let mut http_stop = stop_rx;
    let serve_result = axum::serve(listener, server::WithConnectInfo::new(service))
        .with_graceful_shutdown(async move {
            let _ = http_stop.wait_for(|s| *s).await;
        })
        .await;
    let _ = stop_tx.send(true);
    if let Some(task) = tls_task {
        let _ = task.await;
    }

    if let Some(t) = tunnel_handle {
        t.kill().await;
    }
    serve_result.map_err(Into::into)
}

fn build_app_state(
    cfg: &config::Config,
    path_prefix: &str,
    store: Arc<store::Store>,
    access: Arc<access::Access>,
    secure: Arc<secureurl::SecureUrl>,
) -> anyhow::Result<Arc<state::AppState>> {
    let http = reqwest::Client::builder().build()?;
    let device_id = device_id(&store)?;
    let tv = Arc::new(television::Television::with_device_id(
        http.clone(),
        device_id,
    ));
    if let Some(creds) = login::load(&store) {
        tv.set_credentials(creds);
    }

    let custom_channels = Arc::new(custom_channels::CustomChannels::new());
    if !cfg.custom_channels_file.is_empty() {
        match custom_channels.load(&cfg.custom_channels_file) {
            Ok(n) => println!(
                "Loaded {n} custom channels from {}",
                cfg.custom_channels_file
            ),
            Err(e) => eprintln!(
                "Warning: could not load custom channels from {}: {e}",
                cfg.custom_channels_file
            ),
        }
    }

    let stored_override = match store.get_opt(unlock::STORE_KEY_UNLOCKED).as_deref() {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    };
    let extras_state = Arc::new(extras_state::ExtrasState::new(cfg.extras, stored_override));
    extras_state.init(&http, &store);

    Ok(Arc::new(state::AppState {
        config: cfg.clone(),
        path_prefix: path_prefix.to_string(),
        access,
        store,
        tv,
        secure,
        http: http.clone(),
        drm_channels: Default::default(),
        custom_channels,
        render_caches: Default::default(),
        dash_state: Default::default(),
        epg_state: Default::default(),
        extras: extras_state,
        vod_state: Default::default(),
        public_ip: Arc::new(unlock::PublicIp::new(http)),
        unlock_limiter: Arc::new(unlock::AttemptLimiter::default()),
    }))
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

/// The host to print in a URL meant to be opened from another device. A
/// wildcard bind address names no machine, so it becomes a placeholder.
fn reachable_host(host: &str) -> String {
    match host {
        "0.0.0.0" | "[::]" | "::" | "[::0]" | "0:0:0:0:0:0:0:0" => "<server-ip>".to_string(),
        other => display_host(other),
    }
}

fn display_host(host: &str) -> String {
    if host == "localhost" || host.is_empty() {
        "localhost".to_string()
    } else if host == "[::]" {
        "[::1]".to_string()
    } else {
        host.to_string()
    }
}

async fn login_otp(store: Arc<store::Store>) -> anyhow::Result<()> {
    print!("Enter your mobile number: +91 ");
    std::io::stdout().flush()?;
    let mut number = String::new();
    std::io::stdin().read_line(&mut number)?;
    let number = format!("+91{}", number.trim());

    println!("Sending OTP to your mobile number");
    let client = login::LoginClient::new(reqwest::Client::new());
    client.send_otp(&number).await?;
    println!("OTP sent to your mobile number");

    print!("Enter OTP: ");
    std::io::stdout().flush()?;
    let mut otp = String::new();
    std::io::stdin().read_line(&mut otp)?;
    let otp = otp.trim();

    let device_id = device_id(&store)?;
    let Some(creds) = client.verify_otp(&number, otp, &device_id).await? else {
        anyhow::bail!("the OTP is wrong or has expired");
    };
    login::save(&store, &creds, login::TOUCH_ALL)?;
    println!("Login successful");
    Ok(())
}

async fn extras_login_cli(store: &store::Store) -> anyhow::Result<()> {
    let device = extras::Device::load_or_create(store)?;
    let client = extras::Client::new(reqwest::Client::new(), device);

    print!("Mobile number registered to the fibre account: +91 ");
    std::io::stdout().flush()?;
    let mut number = String::new();
    std::io::stdin().read_line(&mut number)?;
    let number = number.trim().to_string();

    let mut resp = client
        .send_otp(&number, "")
        .await
        .map_err(|e| anyhow::anyhow!("could not send the OTP: {e}"))?;
    let conns = resp.connections();
    if !conns.is_empty() {
        println!("Connections on this number:");
        for (i, c) in conns.iter().enumerate() {
            let tail = if c.identifier.len() > 4 {
                &c.identifier[c.identifier.len() - 4..]
            } else {
                &c.identifier
            };
            println!(
                "  {}. {}, {}, line ending {tail}",
                i + 1,
                c.name,
                c.product_name
            );
        }
        print!("Choose a connection: ");
        std::io::stdout().flush()?;
        let mut pick = String::new();
        std::io::stdin().read_line(&mut pick)?;
        let pick: usize = pick
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("no such connection"))?;
        if pick < 1 || pick > conns.len() {
            anyhow::bail!("no such connection");
        }
        resp = client
            .send_otp(&number, &conns[pick - 1].identifier)
            .await
            .map_err(|e| anyhow::anyhow!("could not send the OTP: {e}"))?;
    }
    if resp.identifier.is_empty() {
        anyhow::bail!("extras did not send an OTP");
    }

    print!("OTP: ");
    std::io::stdout().flush()?;
    let mut otp = String::new();
    std::io::stdin().read_line(&mut otp)?;

    let result = client
        .verify_otp(&number, &resp.identifier, otp.trim())
        .await;
    if let Some(cr) = client.credentials() {
        cr.save(store)?;
    }
    match result {
        Ok(_) => println!("extras login saved. Restart the server to use it."),
        Err(e) => anyhow::bail!("login failed: {e}"),
    }
    Ok(())
}

fn extras_logout_cli(store: &store::Store) -> anyhow::Result<()> {
    extras::delete_credentials(store)?;
    println!("extras login deleted.");
    Ok(())
}

fn login_reset(store: &store::Store) -> anyhow::Result<()> {
    println!("Deleting existing login file if exists");
    login::clear(store)?;
    println!("We have successfully logged you out. Please login again.");
    Ok(())
}

fn admin_password(access: &access::Access, path_prefix: &str) -> anyhow::Result<()> {
    let password = rpassword_prompt("New admin password: ")?;
    let init = std::path::Path::new("/etc/init.d/jiotv");
    if is_openwrt() && init.exists() {
        // The running service caches the store in memory and rewrites the whole
        // file on every update, so the password must be written while it is
        // stopped, through a freshly loaded store.
        let restarted = with_service_stopped(init, std::time::Duration::from_secs(10), || {
            let store = Arc::new(store::Store::open(path_prefix)?);
            access::Access::new(store).set_password(&password)?;
            Ok(())
        })?;
        if restarted {
            println!("Admin password set. JioTV service restarted.");
        } else {
            println!("Admin password set.");
        }
        return Ok(());
    }
    access.set_password(&password)?;
    println!("Admin password set.");
    Ok(())
}

/// Runs `change` with the init-script service stopped, then starts it again
/// only if it was running. Returns whether the service was restarted.
fn with_service_stopped(
    init: &std::path::Path,
    stop_timeout: std::time::Duration,
    change: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<bool> {
    let is_running = || -> std::io::Result<bool> {
        Ok(std::process::Command::new(init)
            .arg("running")
            .status()?
            .success())
    };
    let was_running = is_running()?;
    if was_running {
        let stop = std::process::Command::new(init).arg("stop").status()?;
        if !stop.success() {
            anyhow::bail!("could not stop the JioTV service ({stop}); nothing was changed");
        }
        // procd only signals the process on `stop`; wait until it has exited so
        // its cached store cannot be written over the new one.
        let deadline = std::time::Instant::now() + stop_timeout;
        while is_running()? {
            if std::time::Instant::now() >= deadline {
                anyhow::bail!("the JioTV service did not stop in time; nothing was changed");
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
    let result = change();
    if was_running {
        let start = std::process::Command::new(init).arg("start").status()?;
        if !start.success() {
            anyhow::bail!("the change was applied, but starting the JioTV service failed: {start}");
        }
    }
    result.map(|()| was_running)
}

fn rpassword_prompt(prompt: &str) -> anyhow::Result<String> {
    // No terminal-echo suppression dependency is pulled in for this small,
    // interactive-only path; input is simply read as a line. Documented as
    // a known UX gap versus the Go CLI (which also just used Scanln here).
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

fn show_key(access: &access::Access) -> anyhow::Result<()> {
    println!("{}", access.playlist_path()?);
    Ok(())
}

fn rotate_key(access: &access::Access) -> anyhow::Result<()> {
    access.rotate()?;
    println!("{}", access.playlist_path()?);
    Ok(())
}

fn print_help() {
    println!(
        "jiotv - Stream JioTV on any device\n\n\
         USAGE:\n  jiotv [--config PATH] [--skip-update-check] <command>\n\n\
         COMMANDS:\n  \
         serve [--host H] [--port P] [--public] [--tls [--tls-port P] [--tls-cert F --tls-key F]] [--tunnel] [--tunnel-token T]\n  \
         \x20 --tls            also serve HTTPS (default port 5443) next to plain HTTP; browsers need\n  \
         \x20                  HTTPS for DRM/encrypted-HLS playback, IPTV apps can keep using HTTP\n  \
         \x20 --tls-port P     HTTPS port (default 5443)\n  \
         \x20 --tls-cert/--tls-key  PEM files to use (both or neither); without them a self-signed\n  \
         \x20                  certificate is created in <data dir>/tls/ on first start and reused\n  \
         login otp | login reset\n  \
         extras login | extras logout   (off by default; needs extras = true / JIOTV_EXTRAS=true)\n  \
         epg generate | epg delete\n  \
         admin password\n  \
         key show | key rotate\n  \
         background start [--args \"...\"] | background stop\n  \
         update [--version vX.Y.Z]      (needs JIOTV_UPDATE_TOKEN for a private repo)\n  \
         autostart [--args \"...\"] | autostart remove   (systemd service; Termux: shell rc)\n"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    mod service {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

        /// A fake init script that logs each action and reports `running`
        /// according to the presence of a `running` marker file.
        fn fake_init(dir: &std::path::Path, running: bool) -> std::path::PathBuf {
            let marker = dir.join("running");
            if running {
                std::fs::write(&marker, "").unwrap();
            }
            let script = dir.join("jiotv.init");
            std::fs::write(
                &script,
                format!(
                    "#!/bin/sh\necho \"$1\" >> '{log}'\ncase \"$1\" in\n running) [ -f '{m}' ] ;;\n stop) rm -f '{m}' ;;\n start) : > '{m}' ;;\nesac\n",
                    log = dir.join("calls").display(),
                    m = marker.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            script
        }

        /// The stop/start actions issued, ignoring `running` status polls.
        fn actions(dir: &std::path::Path) -> Vec<String> {
            std::fs::read_to_string(dir.join("calls"))
                .unwrap_or_default()
                .lines()
                .filter(|l| *l != "running")
                .map(str::to_owned)
                .collect()
        }

        #[test]
        fn running_service_is_stopped_for_the_change_then_started() {
            let dir = tempfile::tempdir().unwrap();
            let init = fake_init(dir.path(), true);
            let marker = dir.path().join("running");
            let restarted = with_service_stopped(&init, TIMEOUT, || {
                assert!(
                    !marker.exists(),
                    "service must be stopped during the change"
                );
                Ok(())
            })
            .unwrap();
            assert!(restarted);
            assert_eq!(actions(dir.path()), ["stop", "start"]);
        }

        #[test]
        fn change_waits_for_the_service_to_exit_and_is_skipped_if_it_never_does() {
            let dir = tempfile::tempdir().unwrap();
            let init = fake_init(dir.path(), true);
            // `stop` succeeds but the process lingers: make it a no-op.
            std::fs::write(
                &init,
                format!(
                    "#!/bin/sh\necho \"$1\" >> '{}'\ncase \"$1\" in running) exit 0 ;; esac\n",
                    dir.path().join("calls").display()
                ),
            )
            .unwrap();
            let mut changed = false;
            let err = with_service_stopped(&init, std::time::Duration::from_millis(300), || {
                changed = true;
                Ok(())
            })
            .unwrap_err();
            assert!(err.to_string().contains("did not stop in time"));
            assert!(
                !changed,
                "must not touch the store while the service lingers"
            );
            assert!(!actions(dir.path()).contains(&"start".to_string()));
        }

        #[test]
        fn private_dir_is_created_with_owner_only_access() {
            let dir = tempfile::tempdir().unwrap();
            let target = dir.path().join("etc").join("jiotv");
            create_private_dir(target.to_str().unwrap()).unwrap();
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "group/other must have no access");
        }

        #[test]
        fn stopped_service_stays_stopped() {
            let dir = tempfile::tempdir().unwrap();
            let init = fake_init(dir.path(), false);
            let restarted = with_service_stopped(&init, TIMEOUT, || Ok(())).unwrap();
            assert!(!restarted);
            assert!(actions(dir.path()).is_empty());
        }

        #[test]
        fn failed_change_still_restores_a_running_service() {
            let dir = tempfile::tempdir().unwrap();
            let init = fake_init(dir.path(), true);
            let err =
                with_service_stopped(&init, TIMEOUT, || anyhow::bail!("disk full")).unwrap_err();
            assert!(err.to_string().contains("disk full"));
            assert_eq!(actions(dir.path()), ["stop", "start"]);
        }
    }

    #[test]
    fn wildcard_binds_print_a_placeholder_instead_of_the_bind_address() {
        for wildcard in ["0.0.0.0", "[::]", "::"] {
            assert_eq!(reachable_host(wildcard), "<server-ip>", "{wildcard}");
        }
        assert_eq!(reachable_host("localhost"), "localhost");
        assert_eq!(reachable_host(""), "localhost");
        assert_eq!(reachable_host("192.168.1.10"), "192.168.1.10");
    }
}
