# Install on a homelab Linux server

This guide covers an always-on Linux host, VM, or LXC container on x86_64 or aarch64. The same approach works on most systemd distributions.

## Download the release

Use `full` when you want the web UI; use `slim` for a headless IPTV endpoint.

The built-in updater defaults to `wpfyorg/better-jiotv-go`; change `REPO` if your release is published elsewhere.

```sh
REPO=wpfyorg/better-jiotv-go
VARIANT=full

case "$(uname -m)" in
  x86_64) TARGET=x86_64-unknown-linux-musl ;;
  aarch64|arm64) TARGET=aarch64-unknown-linux-musl ;;
  armv7l|armv7*) TARGET=armv7-unknown-linux-musleabihf ;;
  *) echo "unsupported release architecture: $(uname -m)"; exit 1 ;;
esac

ASSET="jiotv-${VARIANT}-${TARGET}"
curl -fL -o "${ASSET}" "https://github.com/${REPO}/releases/latest/download/${ASSET}"
curl -fL -o SHA256SUMS "https://github.com/${REPO}/releases/latest/download/SHA256SUMS"
grep "  ${ASSET}$" SHA256SUMS | sha256sum -c -
sudo install -m 0755 "${ASSET}" /usr/local/bin/jiotv
```

## Run as a dedicated service user

```sh
sudo useradd --system --home /var/lib/jiotv --create-home --shell /usr/sbin/nologin jiotv 2>/dev/null || true
sudo chmod 0700 /var/lib/jiotv
sudo chown jiotv:jiotv /var/lib/jiotv
```

Perform the interactive setup as that user:

```sh
sudo -u jiotv env HOME=/var/lib/jiotv JIOTV_PATH_PREFIX=/var/lib/jiotv /usr/local/bin/jiotv login otp
sudo -u jiotv env HOME=/var/lib/jiotv JIOTV_PATH_PREFIX=/var/lib/jiotv /usr/local/bin/jiotv admin password
```

Enter the OTP yourself when prompted.

Create `/etc/systemd/system/jiotv.service`:

```ini
[Unit]
Description=JioTV
Wants=network-online.target
After=network-online.target

[Service]
User=jiotv
Group=jiotv
Environment=HOME=/var/lib/jiotv
Environment=JIOTV_PATH_PREFIX=/var/lib/jiotv
Environment=JIOTV_EPG=true
ExecStart=/usr/local/bin/jiotv --skip-update-check serve --host 0.0.0.0 --port 5001
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Enable it:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now jiotv
sudo systemctl status jiotv
```

Logs:

```sh
journalctl -u jiotv -f
```

## VM and LXC notes

- A VM needs only normal outbound internet access and inbound LAN access to the chosen listen port.
- An unprivileged LXC container works like any other Linux host; persist `/var/lib/jiotv` in the container or on a dedicated mount.
- Do not share the same state directory between concurrently running instances. It contains credentials, the access key, session secret, and generated files.

## Reverse proxy and HTTPS

For a LAN-only reverse proxy, bind JioTV to loopback instead:

```ini
ExecStart=/usr/local/bin/jiotv --skip-update-check serve --host 127.0.0.1 --port 5001
```

Then proxy to `127.0.0.1:5001` from Caddy, nginx, Traefik, HAProxy, or another frontend.

The Rust server currently serves plain HTTP. Although `--tls`, `--tls-cert`, and `--tls-key` are parsed by the CLI, native TLS is not wired into the Axum listener yet, so terminate HTTPS at the reverse proxy rather than relying on those flags.

There is one more current limitation: generated playlist URLs use the incoming `Host` but currently render an `http://` scheme. The web UI can still sit behind an HTTPS reverse proxy, but externally consumed M3U entries may need care until forwarded-scheme handling is implemented.

Keep JioTV's own authentication enabled unless the reverse proxy/VPN provides a trusted replacement. In particular, avoid combining `JIOTV_DISABLE_AUTH=true` with `JIOTV_DISABLE_URL_ENCRYPTION=true`.

## Updating

The service user normally cannot replace `/usr/local/bin/jiotv`, so update as an administrator:

```sh
sudo /usr/local/bin/jiotv update
sudo systemctl restart jiotv
```

For a private release repository, provide `JIOTV_UPDATE_TOKEN` (or `GITHUB_TOKEN`) to the update command.
