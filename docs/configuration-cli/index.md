---
title: CLI Commands and Options
description: Reference for the current Rust jiotv command and its server flags.
category: Configuration
keywords: [CLI, serve, login, update, epg, key, autostart, TLS, port]
---

# CLI Commands and Options

Run `jiotv --help` to see the installed binary's command list. These commands are defined by the current Rust CLI.

| Command | What it does |
| --- | --- |
| `jiotv serve` | Start the HTTP server. Also accepted: `jiotv run`, `jiotv start` or no command. |
| `jiotv login otp` | Sign in to the JioTV provider account by OTP from the server terminal. |
| `jiotv login reset` | Clear JioTV login state. |
| `jiotv extras login` / `jiotv extras logout` | Connect/disconnect the optional Extras source when built/enabled. |
| `jiotv admin password` | Set an admin password from the CLI. |
| `jiotv key show` / `jiotv key rotate` | Show or replace the IPTV access key. Treat its output as confidential. |
| `jiotv epg generate` / `jiotv epg delete` | Generate the current guide or remove its cache. |
| `jiotv background start` / `jiotv background stop` | Start or stop a detached server process. |
| `jiotv update` | Check/install a release. `--version vX.Y.Z` requests a specific release. |
| `jiotv autostart` / `jiotv autostart remove` | Install/remove supported startup integration (systemd; Termux shell startup). |

All commands accept `--config PATH`. `--skip-update-check` suppresses the startup version check.

## Server options

| Flag | Default | Use |
| --- | --- | --- |
| `--host` / `-H` | `localhost` | IP or interface to bind. |
| `--port` / `-p` | `5001` | HTTP port. |
| `--public` / `-P` | off | Listen on `[::]` (all interfaces). Protect access before enabling. |
| `--tls` | off | Add HTTPS for browser playback. |
| `--tls-port` | `5443` | HTTPS listener port; different from HTTP. |
| `--tls-cert` / `--tls-key` | generated cert | Supply both PEM files with `--tls`. |
| `--tunnel` | off | Start a Cloudflare tunnel where supported. |
| `--tunnel-token` | unset | Named tunnel token; keep private. |

Example: `jiotv serve --host 127.0.0.1 --port 5001 --tls`. This serves HTTP on 5001 and HTTPS on 5443. When no certificate files are supplied, the server generates a self-signed certificate; browsers may show a warning until you trust it.

**Avoid exposing the service publicly** without strong controls. The access key and login are not a substitute for a network security plan. See [Network and HTTPS](../configuration-network-security/index.md).
