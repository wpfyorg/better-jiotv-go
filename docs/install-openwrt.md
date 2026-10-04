# Install on OpenWrt

## Recommended: automatic install

SSH into the router as `root` and run:

```sh
wget -qO- https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.sh | sh
```

That is the recommended installation method. The script automatically:

- detects whether the router uses `apk` or `opkg`;
- reads the package ABI accepted by `apk` or `opkg` and rejects unsupported targets;
- downloads the matching `jiotv` package from the latest release;
- verifies it with `SHA256SUMS`;
- installs it with the router package manager;
- enables the JioTV service at boot;
- starts (or restarts, on an upgrade) the service and checks that it is listening, unless `option enabled '0'` is set in `/etc/config/jiotv`;
- prints the browser and playlist addresses for your router.

Set `JIOTV_START_SERVICE=0` to install and enable without starting the service.

After installation, set the admin password, then open the web UI and sign in; you enter the OTP yourself:

```sh
jiotv admin password
```

To sign in to JioTV from the terminal instead, stop the service first so it cannot overwrite the new login:

```sh
/etc/init.d/jiotv stop; sleep 3
jiotv login otp
/etc/init.d/jiotv start
```

Open the web UI over HTTPS:

```text
https://<router-ip>:5443/
```

The package enables HTTPS with a self-signed certificate (created on first start under `/etc/jiotv/tls/`). Browsers require HTTPS for protected playback, so accept the one-time certificate warning. IPTV apps should keep using the plain `http://<router-ip>:5001/` playlist. See [HTTPS in Usage](usage.md#https).

The normal `full` package includes the web UI. Most people should use it.

## Update

Run the same installer command again:

```sh
wget -qO- https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.sh | sh
```

It installs the latest package through `apk` or `opkg`, so the package database stays correct. Your `/etc/config/jiotv` settings and `/etc/jiotv` data are preserved.

Do not use `jiotv update` for an OpenWrt package installation.

## Uninstall

For newer apk-based OpenWrt:

```sh
apk del jiotv
```

For opkg-based OpenWrt:

```sh
opkg remove jiotv
```

Use `jiotv-slim` instead if you installed the slim package. Removal leaves `/etc/config/jiotv` and `/etc/jiotv` in place so saved settings and credentials are not silently deleted.

## Optional settings

OpenWrt service settings live in `/etc/config/jiotv`. The service uses `procd`, restarts automatically after crashes, and stores application data under `/etc/jiotv`.

HTTPS is controlled by `tls` (default `1`), `tls_port` (default `5443`), and optional `tls_cert` / `tls_key` paths to your own PEM files (both must be set). After editing, run `uci commit jiotv` and `/etc/init.d/jiotv restart`. Set `option tls '0'` to serve plain HTTP only.

If port `5001` or `5443` is blocked between LAN devices, add a firewall rule for the LAN zone only. Do not expose the service directly on WAN.

## Manual package install

Use this section only if the automatic installer cannot be used.

Open the [latest GitHub Release](https://github.com/wpfyorg/better-jiotv-go/releases/latest), then choose the package matching the router package manager and CPU:

- `.apk` for apk-based OpenWrt;
- `.ipk` for opkg-based OpenWrt;
- x86_64, `aarch64_cortex-a53`, or `arm_cortex-a7_neon-vfpv4` package ABI only.

Install the downloaded file with:

```sh
apk add --allow-untrusted ./package.apk
```

or:

```sh
opkg install ./package.ipk
```

Other OpenWrt package ABIs are not currently published; use the raw Linux musl fallback only when you understand the package-manager tradeoff.

## Raw binary fallback

The raw Linux musl binary is a last-resort fallback when no matching OpenWrt package is available. A raw install is not managed by `apk` or `opkg`, so prefer the package installer above whenever possible.
