# Installation

Pick the guide that matches where the server will run:

- [OpenWrt routers](install-openwrt.md) — raw musl binary, `procd` service, and notes on `apk`/`opkg` packaging.
- [Raspberry Pi and other SBCs](install-sbc.md) — Raspberry Pi OS, Debian/Ubuntu/Armbian-style systems, and systemd autostart.
- [Homelab Linux servers](install-homelab.md) — x86_64/aarch64 servers, VMs/LXC, dedicated service user, and reverse-proxy notes.

## Release variants

Releases contain two variants for each supported target:

- `full` — recommended for most users; includes the embedded Svelte web UI.
- `slim` — headless/IPTV-oriented build without the embedded web UI assets.

Current Linux release targets are:

| Machine | Release target |
|---|---|
| x86_64 / amd64 | `x86_64-unknown-linux-musl` |
| 64-bit ARM / arm64 / aarch64 | `aarch64-unknown-linux-musl` |
| 32-bit ARMv7 | `armv7-unknown-linux-musleabihf` |

There is currently no published MIPS, MIPS64, ARMv6, or 32-bit x86 release artifact.

After installation, see [Configuration](config.md) for `JIOTV_*` variables and TOML settings.
