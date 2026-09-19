mod access;
mod api;
mod catchup;
mod cli;
mod config;
mod custom_channels;
mod dash;
mod drm_channels;
mod epg;
mod login;
mod secureurl;
mod server;
mod state;
mod store;
mod stream;
mod television;
mod token_refresh;
mod tunnel;
mod tvplus;
mod tvplus_state;

#[cfg(feature = "full")]
mod ui_assets;

use std::io::Write;
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    let args = cli::parse()?;

    if let cli::Command::Help = args.command {
        print_help();
        return Ok(());
    }

    let cfg = config::Config::load(args.config.as_deref())?;
    init_logging(&cfg);

    if args.skip_update_check {
        tracing::info!("Skipping update check");
    }
    // No update command is implemented in this rewrite yet (see README).

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
        cli::Command::Serve(serve_args) => runtime.block_on(serve(cfg, path_prefix, store, access, secure, serve_args)),
        cli::Command::LoginOtp => runtime.block_on(login_otp(store)),
        cli::Command::LoginReset => login_reset(&store),
        cli::Command::TvplusLogin => runtime.block_on(tvplus_login_cli(&store)),
        cli::Command::TvplusLogout => tvplus_logout_cli(&store),
        cli::Command::AdminPassword => admin_password(&access),
        cli::Command::KeyShow => show_key(&access),
        cli::Command::KeyRotate => rotate_key(&access),
        cli::Command::EpgGenerate => runtime.block_on(epg_generate(&path_prefix)),
        cli::Command::EpgDelete => epg_delete(&path_prefix),
        cli::Command::Help => unreachable!(),
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
        let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("cannot resolve $HOME"))?;
        format!("{home}/.jiotv_go")
    };
    std::fs::create_dir_all(&prefix)?;
    Ok(if prefix.ends_with('/') { prefix } else { format!("{prefix}/") })
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

    let tvplus_state = Arc::new(tvplus_state::TvPlusState::new(cfg.tvplus));
    tvplus_state.init(&http, &store);
    if cfg.tvplus {
        println!("JioTV+ enabled{}", if tvplus_state.connected() { " (logged in)" } else { " (run `jiotv tvplus login`)" });
    }

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
        tvplus: tvplus_state,
    });

    if cfg.epg {
        let epg_path = format!("{path_prefix}epg.xml.gz");
        let needs_generation = match std::fs::metadata(&epg_path) {
            Ok(meta) => {
                let stale = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.elapsed().ok())
                    .map(|age| age > std::time::Duration::from_secs(24 * 60 * 60))
                    .unwrap_or(true);
                stale
            }
            Err(_) => true,
        };
        if needs_generation {
            let state_for_epg = state.clone();
            tokio::spawn(async move {
                println!("Generating EPG file in the background (JIOTV_EPG=true)...");
                let (extra_channels, extra_programmes) = if state_for_epg.tvplus.enabled() {
                    state_for_epg.tvplus.epg_source(&state_for_epg.tv).await.unwrap_or_else(|e| {
                        tracing::warn!("JioTV+ EPG source skipped: {e}");
                        (Vec::new(), Vec::new())
                    })
                } else {
                    (Vec::new(), Vec::new())
                };
                if let Err(e) = epg::generate_xml_gz_with(&http, &epg_path, extra_channels, extra_programmes).await {
                    tracing::warn!("EPG generation failed: {e}");
                } else {
                    println!("EPG file generated at {epg_path}");
                }
            });
        }
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

    let router = server::build_router(state);
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
    let serve_result = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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

    let creds = client.verify_otp(&number, otp).await?;
    login::save(&store, &creds)?;
    println!("Login successful");
    Ok(())
}

async fn tvplus_login_cli(store: &store::Store) -> anyhow::Result<()> {
    let device = tvplus::Device::load_or_create(store)?;
    let client = tvplus::Client::new(reqwest::Client::new(), device);

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
        anyhow::bail!("JioTV+ did not send an OTP");
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
        Ok(_) => println!("JioTV+ login saved. Restart the server to use it."),
        Err(e) => anyhow::bail!("login failed: {e}"),
    }
    Ok(())
}

fn tvplus_logout_cli(store: &store::Store) -> anyhow::Result<()> {
    tvplus::delete_credentials(store)?;
    println!("JioTV+ login deleted.");
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
        "jiotv - Stream JioTV on any device\n\nUSAGE:\n  jiotv [--config PATH] [--skip-update-check] <command>\n\nCOMMANDS:\n  serve [--host H] [--port P] [--public] [--tls] [--tls-cert] [--tls-key] [--tunnel] [--tunnel-token T]\n  login otp | login reset\n  tvplus login | tvplus logout   (not implemented yet)\n  admin password\n  key show | key rotate\n"
    );
}
