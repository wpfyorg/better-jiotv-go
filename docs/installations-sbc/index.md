---
title: Install on Raspberry Pi and SBCs
description: Choose a supported JioTV build for Raspberry Pi and other Linux single-board computers.
category: Installation
keywords: [Raspberry Pi, SBC, ARM, ARMv7, aarch64, Linux]
---

# Install on Raspberry Pi and other SBCs

Use a supported Linux distribution such as Raspberry Pi OS, Debian, Ubuntu or Armbian. Check the operating system architecture with:

```sh
uname -m
```

Supported release targets include `aarch64` (64-bit ARM), `armv7l` (32-bit ARMv7), and `x86_64`. ARMv6 boards, including original Raspberry Pi and Pi Zero systems, are not supported by the published Linux release binaries. A Pi Zero 2 needs a 64-bit OS to use the aarch64 build.

Install using the [Linux guide](../installations-linux/index.md). The installer chooses the release target from the running system. Select `full` for the browser interface, or `slim` for an intentionally headless server. For an always-on systemd setup, follow [Background service](../maintenance-background-service/index.md).

For a system-wide service, choose a stable data directory and set `JIOTV_PATH_PREFIX` consistently before login and service setup. The service setup command saves the selected prefix. See [data, backups and logs](../configuration-storage-logs/index.md).

## Start, verify and maintain

After using the [Linux installation steps](../installations-linux/index.md), run `jiotv --help` to check the binary and `jiotv serve --host 0.0.0.0 --tls` for a trusted LAN installation. Visit `https://<sbc-lan-address>:5443/` from another device. If the binary reports an architecture or loader error, confirm the board's CPU and operating system with `uname -m`, then choose a matching release.

To update, stop the service, run `jiotv update`, restart it and check the browser page. To remove the installation, follow [update and uninstall](../maintenance-update-uninstall/index.md). For startup and missing dependency errors, see [installation troubleshooting](../troubleshooting-installation-startup/index.md).
