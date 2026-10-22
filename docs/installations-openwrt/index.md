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

The service uses procd and starts at boot. Manage it with:

```sh
/etc/init.d/jiotv status
/etc/init.d/jiotv restart
logread -e jiotv
```

Settings are in `/etc/config/jiotv`; application state, including credentials and TLS material, is under `/etc/jiotv`. After changing UCI settings, run `uci commit jiotv` and `/etc/init.d/jiotv restart`. Do not expose the service to the internet.

To update, run the installer command again so the package manager updates the package. Do not use `jiotv update` for a package-managed installation. To uninstall, use `apk del jiotv` or `opkg remove jiotv`; use `jiotv-slim` if that was the installed package. Package removal preserves configuration and application data. See [OpenWrt maintenance](../maintenance-openwrt/index.md) for package-specific backup and removal details.

## Verify and troubleshoot

Run `/etc/init.d/jiotv status` and `logread -e jiotv` over SSH. Load the HTTPS page from a trusted LAN device; the slim package intentionally has no web interface. If installation reports an unsupported architecture, check `uname -m` and `opkg print-architecture` (or `apk --print-arch` where supported) against the package ABI. Do not force an incompatible package. See [OpenWrt problems](../troubleshooting-installation-startup/index.md).

Update by rerunning the supported package installer, then restart/check the service. Remove with the matching package manager, using `jiotv-slim` for the slim package. Preserve or back up `/etc/jiotv` and `/etc/config/jiotv` as described in [OpenWrt maintenance](../maintenance-openwrt/index.md).
