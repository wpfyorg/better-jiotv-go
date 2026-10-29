---
title: Install on OpenWrt
description: Install the JioTV package on supported OpenWrt routers and manage its procd service.
category: Installation
keywords: [OpenWrt, router, apk, opkg, procd, package]
---

# Install on OpenWrt

SSH to the router as `root`. The installer detects `apk` or `opkg`, chooses a supported package ABI, verifies the downloaded package checksum, and installs the package. Supported package architectures are x86_64, aarch64 Cortex-A53 and ARM Cortex-A7 NEON. Other router architectures are not published.

```sh
wget -qO- https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.sh | sh
```

By default, the installer enables the service and starts it. Open the browser interface at `https://<router-ip>:5443/` and accept the self-signed certificate warning. IPTV players use `http://<router-ip>:5001/`. The normal package is `full`; for headless installations, set `JIOTV_VARIANT=slim` before running the script. The slim package has no web interface and requires terminal OTP login.

## Guided setup

Over an SSH terminal the installer also asks how to set the router up: build (`full` or `slim`), account (JioTV, JioTV+, or Fastway when that package supports it), the address other devices will use, whether to expose it through Cloudflare, and the account sign-in. It stops the service before the terminal login and aborts if the service cannot be stopped, so a running process cannot overwrite the new login.

The provider and tunnel choices are written to UCI, so those settings survive a reboot and a package upgrade. The address shown during setup is an installer summary choice, not a UCI setting; configure `option host` manually if the service must bind to a particular address.

| Option | Values | Purpose |
| --- | --- | --- |
| `option tv_plus` | `0`, `1` | Enable the optional JioTV+ account (needs a JioFiber or AirFiber login). The previous option name `extras` is still read. |
| `option tunnel` | `0`, `1` | Start the optional Cloudflare tunnel with the service. |
| `option tunnel_token` | token | Named-tunnel token, passed to the service as an environment variable rather than a command-line argument. |

`option tunnel_token` is stored in `/etc/config/jiotv`, which is readable by root only. `option tunnel 1` without a token starts a temporary `trycloudflare.com` address that changes every start; check `logread -e cloudflared` for the current one. Read the exposure warning below before enabling either.

Piping the installer is still supported: prompts read `/dev/tty`, and with no terminal the package installs exactly as before. Use `JIOTV_ONBOARD=never` to skip the questions, or set `JIOTV_PROVIDER`, `JIOTV_PUBLIC_HOST`, `JIOTV_TUNNEL` and `JIOTV_TUNNEL_TOKEN` to answer them up front.

The service uses procd and starts at boot. Manage it with:

```sh
/etc/init.d/jiotv status
/etc/init.d/jiotv restart
logread -e jiotv
```

Settings are in `/etc/config/jiotv`; application state, including credentials and TLS material, is under `/etc/jiotv`. After changing UCI settings, run `uci commit jiotv` and `/etc/init.d/jiotv restart`.

## Exposure warning

A Cloudflare tunnel publishes the router's JioTV server on a public hostname. The access key in the playlist path and the admin password are then the only protection, and the DRM license proxy becomes reachable from the internet too. Use a long admin password, prefer the temporary URL when you only need short-lived access, and remove the tunnel when you are done. Without a tunnel, keep the service on your trusted LAN.

To update, run the installer command again so the package manager updates the package. Do not use `jiotv update` for a package-managed installation. To uninstall, use `apk del jiotv` or `opkg remove jiotv`; use `jiotv-slim` if that was the installed package. Package removal preserves configuration and application data. See [OpenWrt maintenance](../maintenance-openwrt/index.md) for package-specific backup and removal details.

## Verify and troubleshoot

Run `/etc/init.d/jiotv status` and `logread -e jiotv` over SSH. Load the HTTPS page from a trusted LAN device; the slim package intentionally has no web interface. If installation reports an unsupported architecture, check `uname -m` and `opkg print-architecture` (or `apk --print-arch` where supported) against the package ABI. Do not force an incompatible package. See [OpenWrt problems](../troubleshooting-installation-startup/index.md).

Update by rerunning the supported package installer, then restart/check the service. Remove with the matching package manager, using `jiotv-slim` for the slim package. Preserve or back up `/etc/jiotv` and `/etc/config/jiotv` as described in [OpenWrt maintenance](../maintenance-openwrt/index.md).
