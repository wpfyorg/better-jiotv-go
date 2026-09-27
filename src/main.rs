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
mod keyed_locks;
mod login;
mod secureurl;
mod server;
mod state;
mod store;
mod stream;
mod television;
mod token_refresh;
mod tunnel;
mod extras;
mod extras_state;
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
        eprintln!("Warning! URL encryption is disabled. Anyone can pass modified URLs to your server.");
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
        cli::Command::AdminPassword => admin_password(&access),
        cli::Command::KeyShow => show_key(&access),
        cli::Command::KeyRotate => rotate_key(&access),
        cli::Command::EpgGenerate => runtime.block_on(epg_generate(&path_prefix)),
        cli::Command::EpgDelete => epg_delete(&path_prefix),
        cli::Command::BackgroundStart { args } => background_start(&args, &path_prefix),
        cli::Command::BackgroundStop => background_stop(&path_prefix),
        cli::Command::Update { version } => runtime.block_on(update::run(version.as_deref())),
        cli::Command::Autostart { args: serve_args } => autostart::install(&serve_args, args.config.as_deref(), &path_prefix),
        cli::Command::AutostartRemove => autostart::remove(),
        cli::Command::Help => unreachable!(),
    }
}

/// Regenerates `epg.xml.gz` once if it's missing or more than a day old,
/// then keeps regenerating roughly once every 24h (jittered by up to an
/// hour either way, off-peak-ish like the Go version's random schedule)
/// for as long as the server runs. Mirrors `epg.Init`'s startup check plus
/// its "schedule the next run" loop, without reproducing its exact
/// day+1-at-a-random-hour arithmetic.
async fn epg_task_loop(state: Arc<state::AppState>, http: reqwest::Client, epg_path: String) {
    loop {
        let needs_generation = match std::fs::metadata(&epg_path) {
            Ok(meta) => meta
                .modified()
                .ok()
                .and_then(|m| m.elapsed().ok())
                .map(|age| age > std::time::Duration::from_secs(24 * 60 * 60))
                .unwrap_or(true),
            Err(_) => true,
        };
        if needs_generation {
            println!("Generating EPG file in the background (JIOTV_EPG=true)...");
            let (extra_channels, extra_programmes) = if state.extras.enabled() {
                state.extras.epg_source(&state.tv).await.unwrap_or_else(|e| {
                    tracing::warn!("extras EPG source skipped: {e}");
                    (Vec::new(), Vec::new())
                })
            } else {
                (Vec::new(), Vec::new())
            };
            match epg::generate_xml_gz_with(&http, &epg_path, extra_channels, extra_programmes).await {
                Ok(()) => println!("EPG file generated at {epg_path}"),
                Err(e) => tracing::warn!("EPG generation failed: {e}"),
            }
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

async fn epg_generate(path_prefix: &str) -> anyhow::Result<()> {
    let path = format!("{path_prefix}epg.xml.gz");
    println!("Deleting existing EPG file if exists");
    let _ = std::fs::remove_file(&path);
    println!("Generating new EPG file... this can take a few minutes.");
    let client = reqwest::Client::new();
    epg::generate_xml_gz(&client, &path).await?;
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
    let pid_str = std::fs::read_to_string(&pid_path).map_err(|e| anyhow::anyhow!("failed to read PID file: {e}"))?;
    let pid: u32 = pid_str.trim().parse().map_err(|_| anyhow::anyhow!("failed to parse PID file"))?;

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
    match std::fs::remove_file(&path) {
        Ok(()) => println!("EPG file deleted"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("EPG file does not exist"),
        Err(e) => return Err(e.into()),
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
        let home = home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve the user profile directory"))?;
        format!("{home}/.jiotv_go")
    };
    std::fs::create_dir_all(&prefix)?;
    Ok(if prefix.ends_with('/') { prefix } else { format!("{prefix}/") })
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
    let http = reqwest::Client::builder().build()?;
    let device_id = device_id(&store)?;
    let tv = Arc::new(television::Television::with_device_id(http.clone(), device_id));
    if let Some(creds) = login::load(&store) {
        tv.set_credentials(creds);
    }

    let custom_channels = Arc::new(custom_channels::CustomChannels::new());
    if !cfg.custom_channels_file.is_empty() {
        match custom_channels.load(&cfg.custom_channels_file) {
            Ok(n) => println!("Loaded {n} custom channels from {}", cfg.custom_channels_file),
            Err(e) => eprintln!("Warning: could not load custom channels from {}: {e}", cfg.custom_channels_file),
        }
    }

    let stored_unlocked = store.get_opt(unlock::STORE_KEY_UNLOCKED).as_deref() == Some("true");
    let extras_state = Arc::new(extras_state::ExtrasState::new(cfg.extras, stored_unlocked));
    extras_state.init(&http, &store);
    if cfg.extras || stored_unlocked {
        println!("extras enabled{}", if extras_state.connected() { " (logged in)" } else { " (run `jiotv extras login`)" });
    }
    let public_ip = Arc::new(unlock::PublicIp::new(http.clone()));
    let unlock_limiter = Arc::new(unlock::AttemptLimiter::default());

    let state = Arc::new(state::AppState {
        config: cfg.clone(),
        path_prefix: path_prefix.clone(),
        access: access.clone(),
        store,
        tv,
        secure,
        http: http.clone(),
        drm_channels: Default::default(),
        custom_channels,
        render_caches: Default::default(),
        dash_state: Default::default(),
        extras: extras_state,
        vod_state: Default::default(),
        public_ip,
        unlock_limiter,
    });

    if cfg.epg {
        let epg_path = format!("{path_prefix}epg.xml.gz");
        tokio::spawn(epg_task_loop(state.clone(), http, epg_path));
    }

    if !cfg.disable_auth {
        let playlist = access.playlist_path()?;
        println!("Playlist: http://{}:{}{playlist}", display_host(&args.host), args.port);
        #[cfg(feature = "full")]
        if !access.has_password() {
            let setup = playlist.trim_end_matches("playlist.m3u").to_string();
            println!(
                "Web setup: http://{}:{}{setup} (or run: jiotv admin password)",
                display_host(&args.host),
                args.port
            );
        }
    } else {
        println!("Auth is disabled: playlist at /playlist.m3u");
    }

    let service = server::GatedService::new(state);
    let listener = tokio::net::TcpListener::bind(format!("{}:{}", args.host, args.port)).await?;

    let mut tunnel_handle = None;
    if args.tunnel {
        let data_dir = std::env::var("HOME").map(|h| std::path::PathBuf::from(h).join(".jiotv_go"))?;
        let cloudflared = tunnel::ensure_cloudflared(&data_dir).await?;
        if let Some(token) = &args.tunnel_token {
            let t = tunnel::Tunnel::spawn_named(&cloudflared, token).await?;
            tunnel_handle = Some(t);
            println!("Named tunnel running (see the Cloudflare dashboard for its hostname).");
        } else {
            let port: u16 = args.port.parse().unwrap_or(5001);
            let (t, url) = tunnel::Tunnel::spawn_quick(&cloudflared, port).await?;
            tunnel_handle = Some(t);
            let key_playlist = access.playlist_path().unwrap_or_else(|_| "/playlist.m3u".to_string());
            println!("Public playlist: {url}{key_playlist}");
        }
    }

    tracing::info!("listening");
    let serve_result = axum::serve(listener, server::WithConnectInfo::new(service))
        .with_graceful_shutdown(shutdown_signal())
        .await;

    if let Some(t) = tunnel_handle {
        t.kill().await;
    }
    serve_result.map_err(Into::into)
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
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

    let mut resp = client.send_otp(&number, "").await.map_err(|e| anyhow::anyhow!("could not send the OTP: {e}"))?;
    let conns = resp.connections();
    if !conns.is_empty() {
        println!("Connections on this number:");
        for (i, c) in conns.iter().enumerate() {
            let tail = if c.identifier.len() > 4 { &c.identifier[c.identifier.len() - 4..] } else { &c.identifier };
            println!("  {}. {}, {}, line ending {tail}", i + 1, c.name, c.product_name);
        }
        print!("Choose a connection: ");
        std::io::stdout().flush()?;
        let mut pick = String::new();
        std::io::stdin().read_line(&mut pick)?;
        let pick: usize = pick.trim().parse().map_err(|_| anyhow::anyhow!("no such connection"))?;
        if pick < 1 || pick > conns.len() {
            anyhow::bail!("no such connection");
        }
        resp = client.send_otp(&number, &conns[pick - 1].identifier).await.map_err(|e| anyhow::anyhow!("could not send the OTP: {e}"))?;
    }
    if resp.identifier.is_empty() {
        anyhow::bail!("extras did not send an OTP");
    }

    print!("OTP: ");
    std::io::stdout().flush()?;
    let mut otp = String::new();
    std::io::stdin().read_line(&mut otp)?;

    let result = client.verify_otp(&number, &resp.identifier, otp.trim()).await;
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

fn admin_password(access: &access::Access) -> anyhow::Result<()> {
    let password = rpassword_prompt("New admin password: ")?;
    access.set_password(&password)?;
    println!("Admin password set.");
    Ok(())
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
         serve [--host H] [--port P] [--public] [--tls] [--tls-cert] [--tls-key] [--tunnel] [--tunnel-token T]\n  \
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
