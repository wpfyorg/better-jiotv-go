//! Command-line parsing with `lexopt`, mirroring the subcommands in
//! `main.go`.

pub enum Command {
    Serve(ServeArgs),
    LoginOtp,
    LoginReset,
    ExtrasLogin,
    ExtrasLogout,
    AdminPassword,
    KeyShow,
    KeyRotate,
    EpgGenerate,
    EpgDelete,
    BackgroundStart { args: String },
    BackgroundStop,
    Update { version: Option<String> },
    Autostart { args: String },
    AutostartRemove,
    Help,
}

pub struct ServeArgs {
    pub host: String,
    pub port: String,
    pub public: bool,
    pub tls: bool,
    pub tls_cert: String,
    pub tls_key: String,
    pub tunnel: bool,
    pub tunnel_token: Option<String>,
}

impl Default for ServeArgs {
    fn default() -> Self {
        ServeArgs {
            host: "localhost".to_string(),
            port: "5001".to_string(),
            public: false,
            tls: false,
            tls_cert: String::new(),
            tls_key: String::new(),
            tunnel: false,
            tunnel_token: std::env::var("JIOTV_TUNNEL_TOKEN").ok(),
        }
    }
}

pub struct Args {
    pub config: Option<String>,
    pub skip_update_check: bool,
    pub command: Command,
}

pub fn parse() -> anyhow::Result<Args> {
    use lexopt::prelude::*;
    let mut config = None;
    let mut skip_update_check = false;
    let mut command = None;
    let mut serve = ServeArgs::default();
    let mut background_args = String::new();
    let mut version = None;

    let mut parser = lexopt::Parser::from_env();
    let mut positionals: Vec<String> = Vec::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("config") | Short('c') => config = Some(parser.value()?.parse()?),
            Long("skip-update-check") => skip_update_check = true,
            Long("host") | Short('H') => serve.host = parser.value()?.parse()?,
            Long("port") | Short('p') => serve.port = parser.value()?.parse()?,
            Long("public") | Short('P') => serve.public = true,
            Long("tls") => serve.tls = true,
            Long("tls-cert") => serve.tls_cert = parser.value()?.parse()?,
            Long("tls-key") => serve.tls_key = parser.value()?.parse()?,
            Long("tunnel") => serve.tunnel = true,
            Long("tunnel-token") => serve.tunnel_token = Some(parser.value()?.parse()?),
            Long("args") | Short('a') => background_args = parser.value()?.parse()?,
            Long("version") => version = Some(parser.value()?.parse()?),
            Long("help") | Short('h') => {
                command = Some(Command::Help);
            }
            Value(v) => positionals.push(v.into_string().map_err(|_| anyhow::anyhow!("invalid arg"))?),
            _ => return Err(arg.unexpected().into()),
        }
    }

    if command.is_none() {
        command = Some(match positionals.first().map(String::as_str) {
            Some("serve") | Some("run") | Some("start") | None => {
                if serve.public {
                    serve.host = "[::]".to_string();
                }
                Command::Serve(serve)
            }
            Some("login") => match positionals.get(1).map(String::as_str) {
                Some("otp") | Some("o") | None => Command::LoginOtp,
                Some("reset") | Some("logout") | Some("lo") => Command::LoginReset,
                Some(other) => anyhow::bail!("unknown login subcommand: {other}"),
            },
            Some("extras") => match positionals.get(1).map(String::as_str) {
                Some("login") => Command::ExtrasLogin,
                Some("logout") => Command::ExtrasLogout,
                other => anyhow::bail!("usage: jiotv extras login|logout (got {other:?})"),
            },
            Some("admin") => match positionals.get(1).map(String::as_str) {
                Some("password") => Command::AdminPassword,
                other => anyhow::bail!("usage: jiotv admin password (got {other:?})"),
            },
            Some("key") => match positionals.get(1).map(String::as_str) {
                Some("rotate") => Command::KeyRotate,
                Some("show") | None => Command::KeyShow,
                Some(other) => anyhow::bail!("unknown key subcommand: {other}"),
            },
            Some("epg") => match positionals.get(1).map(String::as_str) {
                Some("generate") | Some("gen") | Some("g") => Command::EpgGenerate,
                Some("delete") | Some("del") | Some("d") => Command::EpgDelete,
                other => anyhow::bail!("usage: jiotv epg generate|delete (got {other:?})"),
            },
            Some("background") | Some("bg") => match positionals.get(1).map(String::as_str) {
                Some("start") | Some("run") | Some("r") => Command::BackgroundStart { args: background_args },
                Some("stop") | Some("k") | Some("kill") => Command::BackgroundStop,
                other => anyhow::bail!("usage: jiotv background start|stop (got {other:?})"),
            },
            Some("update") | Some("upgrade") | Some("u") => Command::Update { version },
            Some("autostart") | Some("as") => match positionals.get(1).map(String::as_str) {
                None | Some("install") | Some("enable") => Command::Autostart { args: background_args },
                Some("remove") | Some("disable") | Some("uninstall") => Command::AutostartRemove,
                Some(other) => anyhow::bail!("usage: jiotv autostart [--args \"...\"] | autostart remove (got {other})"),
            },
            Some(other) => anyhow::bail!("unknown command: {other}"),
        });
    }

    Ok(Args {
        config,
        skip_update_check,
        command: command.unwrap(),
    })
}
