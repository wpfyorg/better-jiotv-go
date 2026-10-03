//! Optional HTTPS listener (`serve --tls`).
//!
//! Browsers only expose EME (Widevine) and Web Crypto (AES-128 HLS) in a
//! secure context, so the browser UI needs HTTPS when it is reached over a
//! LAN address. IPTV clients keep using the plain HTTP listener, since
//! most of them reject self-signed certificates.
//!
//! The HTTPS listener serves exactly the same service as the HTTP one. Each
//! request that arrives over it is tagged with the [`Https`] extension so
//! generated links render `https://` (the scheme is never taken from a
//! client-supplied header).

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use axum::body::Body;
use axum::http::Request;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;
use tower::Service;

pub const DEFAULT_TLS_PORT: &str = "5443";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Request extension marking a request that arrived over TLS.
#[derive(Clone, Copy, Debug)]
pub struct Https;

/// A ready-to-serve TLS configuration plus the leaf certificate fingerprint.
pub struct Material {
    pub config: Arc<rustls::ServerConfig>,
    /// Upper-case, colon-separated SHA-256 of the leaf certificate (DER).
    pub fingerprint: String,
    /// True when a new self-signed certificate was written by this call.
    pub generated: bool,
}

/// Resolves the certificate to serve.
///
/// * both `cert` and `key` given: use them (errors if unusable or mismatched);
/// * neither given: reuse `<data_dir>/tls/{cert,key}.pem`, creating a
///   self-signed pair when missing or unparsable;
/// * only one given: error.
pub fn load_or_create(
    cert: &str,
    key: &str,
    data_dir: &str,
    host: &str,
) -> anyhow::Result<Material> {
    match (cert.is_empty(), key.is_empty()) {
        (false, false) => {
            let (config, fingerprint) = load(Path::new(cert), Path::new(key))
                .with_context(|| format!("loading --tls-cert {cert} / --tls-key {key}"))?;
            Ok(Material {
                config,
                fingerprint,
                generated: false,
            })
        }
        (true, true) => {
            let dir = PathBuf::from(data_dir).join("tls");
            let (cert_path, key_path) = (dir.join("cert.pem"), dir.join("key.pem"));
            if cert_path.exists() && key_path.exists() {
                match load(&cert_path, &key_path) {
                    Ok((config, fingerprint)) => {
                        return Ok(Material {
                            config,
                            fingerprint,
                            generated: false,
                        })
                    }
                    Err(e) => eprintln!(
                        "Warning: stored TLS certificate in {} is unusable ({e:#}); regenerating",
                        dir.display()
                    ),
                }
            }
            generate_self_signed(&cert_path, &key_path, &certificate_names(host))?;
            let (config, fingerprint) = load(&cert_path, &key_path)
                .context("loading the freshly generated TLS certificate")?;
            Ok(Material {
                config,
                fingerprint,
                generated: true,
            })
        }
        _ => bail!("--tls-cert and --tls-key must be given together (or neither, for a self-signed certificate)"),
    }
}

fn load(cert_path: &Path, key_path: &Path) -> anyhow::Result<(Arc<rustls::ServerConfig>, String)> {
    let certs = CertificateDer::pem_file_iter(cert_path)
        .with_context(|| format!("reading {}", cert_path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("parsing {}", cert_path.display()))?;
    let leaf = certs
        .first()
        .with_context(|| format!("no certificate in {}", cert_path.display()))?;
    let fingerprint = fingerprint(leaf.as_ref());
    let key = PrivateKeyDer::from_pem_file(key_path)
        .with_context(|| format!("reading the private key {}", key_path.display()))?;
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .context("the certificate and private key do not match or are unsupported")?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok((Arc::new(config), fingerprint))
}

pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Subject alternative names for the generated certificate: localhost, the
/// loopback addresses, the machine hostname, the primary outbound local
/// address of each IP family, and `--host` when it is a concrete IP.
/// (Other interfaces are not enumerated; supply `--tls-cert/--tls-key` for
/// a certificate covering additional names.)
fn certificate_names(host: &str) -> Vec<String> {
    let mut names: Vec<String> = vec!["localhost".into(), "127.0.0.1".into(), "::1".into()];
    let mut push = |s: String| {
        if !names.contains(&s) {
            names.push(s);
        }
    };
    if let Some(h) = hostname() {
        push(h);
    }
    for ip in [outbound_ip("0.0.0.0:0", "192.0.2.1:9"), outbound_ip("[::]:0", "[2001:db8::1]:9")]
        .into_iter()
        .flatten()
    {
        push(ip.to_string());
    }
    if let Ok(ip) = host.trim_matches(|c| c == '[' || c == ']').parse::<IpAddr>() {
        if !ip.is_unspecified() {
            push(ip.to_string());
        }
    }
    names
}

/// The local address the OS would use for outbound traffic. A connected UDP
/// socket sends nothing; it only selects a route.
fn outbound_ip(bind: &str, target: &str) -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind(bind).ok()?;
    sock.connect(target).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn hostname() -> Option<String> {
    let raw = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/proc/sys/kernel/hostname").ok())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .or_else(|| {
            let out = std::process::Command::new("hostname").output().ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        })?;
    let h = raw.trim();
    let valid = !h.is_empty()
        && h.len() <= 253
        && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    valid.then(|| h.to_string())
}

fn generate_self_signed(
    cert_path: &Path,
    key_path: &Path,
    names: &[String],
) -> anyhow::Result<()> {
    use rcgen::{CertificateParams, DnType, ExtendedKeyUsagePurpose, KeyPair, SanType};

    let mut sans = Vec::new();
    for n in names {
        match n.parse::<IpAddr>() {
            Ok(ip) => sans.push(SanType::IpAddress(ip)),
            Err(_) => {
                if let Ok(dns) = n.as_str().try_into() {
                    sans.push(SanType::DnsName(dns));
                }
            }
        }
    }
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.subject_alt_names = sans;
    params.distinguished_name.push(DnType::CommonName, "JioTV Go");
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(3650);
    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    if let Some(dir) = cert_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_private(key_path, key_pair.serialize_pem().as_bytes())?;
    write_private(cert_path, cert.pem().as_bytes())?;
    Ok(())
}

/// Writes via a temp file + rename; mode 0600 on unix.
fn write_private(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("pem.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

async fn stopped(rx: &mut watch::Receiver<bool>) {
    let _ = rx.wait_for(|s| *s).await;
}

/// Accept loop for the HTTPS listener. `make_service` produces the per-
/// connection service (given the peer address and `https = true`); `shutdown`
/// flips to true on shutdown, after which no new connections are accepted and
/// in-flight ones are drained gracefully.
pub async fn serve<S, F>(
    listener: TcpListener,
    config: Arc<rustls::ServerConfig>,
    make_service: F,
    mut shutdown: watch::Receiver<bool>,
) where
    F: Fn(SocketAddr) -> S + Send + Sync + 'static,
    S: Service<Request<Body>, Response = axum::response::Response, Error = std::convert::Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    let acceptor = TlsAcceptor::from(config);
    let make_service = Arc::new(make_service);
    let mut conns = tokio::task::JoinSet::new();
    loop {
        let (tcp, peer) = tokio::select! {
            r = listener.accept() => match r {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("TLS accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
            _ = stopped(&mut shutdown) => break,
        };
        let acceptor = acceptor.clone();
        let make_service = make_service.clone();
        let mut shutdown = shutdown.clone();
        conns.spawn(async move {
            let tls = match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    tracing::debug!("TLS handshake with {peer} failed: {e}");
                    return;
                }
                Err(_) => {
                    tracing::debug!("TLS handshake with {peer} timed out");
                    return;
                }
            };
            let svc = make_service(peer);
            let hyper_svc = service_fn(move |req: Request<Incoming>| {
                let mut svc = svc.clone();
                async move { svc.call(req.map(Body::new)).await }
            });
            let conn = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(tls), hyper_svc)
                .with_upgrades();
            tokio::pin!(conn);
            let res = tokio::select! {
                r = conn.as_mut() => r,
                _ = stopped(&mut shutdown) => {
                    conn.as_mut().graceful_shutdown();
                    conn.await
                }
            };
            if let Err(e) = res {
                tracing::debug!("TLS connection from {peer} ended: {e}");
            }
        });
        // Reap finished connections so the set does not grow unbounded.
        while conns.try_join_next().is_some() {}
    }
    drop(listener);
    while conns.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn generates_persists_and_reuses_a_self_signed_cert() {
        let dir = data_dir();
        let d = dir.path().to_str().unwrap();
        let a = load_or_create("", "", d, "192.168.1.50").unwrap();
        assert!(a.generated);
        let cert = dir.path().join("tls/cert.pem");
        let key = dir.path().join("tls/key.pem");
        assert!(cert.exists() && key.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(a.fingerprint.split(':').count(), 32);

        let b = load_or_create("", "", d, "192.168.1.50").unwrap();
        assert!(!b.generated);
        assert_eq!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn regenerates_when_stored_cert_is_unparsable() {
        let dir = data_dir();
        let d = dir.path().to_str().unwrap();
        let a = load_or_create("", "", d, "localhost").unwrap();
        std::fs::write(dir.path().join("tls/cert.pem"), "garbage").unwrap();
        let b = load_or_create("", "", d, "localhost").unwrap();
        assert!(b.generated);
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn explicit_pair_is_used_and_mismatch_or_partial_is_an_error() {
        let one = data_dir();
        let two = data_dir();
        load_or_create("", "", one.path().to_str().unwrap(), "localhost").unwrap();
        load_or_create("", "", two.path().to_str().unwrap(), "localhost").unwrap();
        let (c1, k1) = (one.path().join("tls/cert.pem"), one.path().join("tls/key.pem"));
        let k2 = two.path().join("tls/key.pem");
        let (c1s, k1s, k2s) = (
            c1.to_str().unwrap(),
            k1.to_str().unwrap(),
            k2.to_str().unwrap(),
        );

        let ok = load_or_create(c1s, k1s, "/nonexistent", "localhost").unwrap();
        assert!(!ok.generated);
        assert!(load_or_create(c1s, k2s, "/nonexistent", "localhost").is_err());

        let err = load_or_create(c1s, "", "/nonexistent", "localhost").err().unwrap();
        assert!(err.to_string().contains("must be given together"));
        let err = load_or_create("", k1s, "/nonexistent", "localhost").err().unwrap();
        assert!(err.to_string().contains("must be given together"));
    }

    #[test]
    fn names_include_loopback_and_concrete_host_ip() {
        let n = certificate_names("10.1.2.3");
        for want in ["localhost", "127.0.0.1", "::1", "10.1.2.3"] {
            assert!(n.iter().any(|x| x == want), "missing {want}");
        }
        assert!(!certificate_names("[::]").iter().any(|x| x == "::"));
    }
}
