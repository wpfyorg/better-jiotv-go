mod access;
mod api;
mod cli;
mod config;
mod login;
mod secureurl;
mod server;
mod state;
mod store;
mod television;
mod tunnel;

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
        cli::Command::Serve(serve_args) => runtime.block_on(serve(cfg, store, access, secure, serve_args)),
        cli::Command::LoginOtp => runtime.block_on(login_otp(store)),
        cli::Command::LoginReset => login_reset(&store),
        cli::Command::TvplusLogin | cli::Command::TvplusLogout => {
            eprintln!("JioTV+ login is not implemented in this Rust rewrite yet; see README's parity gaps.");
            Ok(())
        }
        cli::Command::AdminPassword => admin_password(&access),
        cli::Command::KeyShow => show_key(&access),
        cli::Command::KeyRotate => rotate_key(&access),
        cli::Command::Help => unreachable!(),
    }
}

fn resolve_path_prefix(cfg: &config::Config) -> anyhow::Result<String> {
    let prefix = if !cfg.path_prefix.is_empty() {
        cfg.path_prefix.clone()
    } else {
        let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("cannot resolve $HOME"))?;
        format!("{home}/.jiotv_go")
    };
    std::fs::create_dir_all(&prefix)?;
    Ok(prefix)
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
    store: Arc<store::Store>,
    access: Arc<access::Access>,
    secure: Arc<secureurl::SecureUrl>,
    args: cli::ServeArgs,
) -> anyhow::Result<()> {
    let http = reqwest::Client::builder().build()?;
    let tv = Arc::new(television::Television::new(http.clone()));
    if let Some(creds) = login::load(&store) {
        tv.set_credentials(creds);
    }

    let state = Arc::new(state::AppState {
        config: cfg.clone(),
        access: access.clone(),
        store,
        tv,
        secure,
        http,
        drm_channels: Default::default(),
    });

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
