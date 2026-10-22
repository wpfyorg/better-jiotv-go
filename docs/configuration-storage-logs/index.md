---
title: Data Files, Backups and Logs
description: Locate JioTV's state files and collect safe diagnostics.
category: Configuration
keywords: [logs, store_v4.toml, data directory, backup, EPG cache, PID]
---

# Data Files, Backups and Logs

JioTV stores its access key, administrator password hash, device identity and provider session tokens in the runtime data directory. The Rust source resolves its default data directory to a per-user JioTV location; an OpenWrt package can use a different configured path. Set `JIOTV_PATH_PREFIX` (or TOML `path_prefix`) to choose another writable location.

Files may include:

| File | Purpose |
| --- | --- |
| `store_v4.toml` | Persistent state, tokens, access key and password hash. **Sensitive.** |
| `epg.xml.gz` | Generated XMLTV guide when enabled. |
| `epg.xml.gz.context.json` | Identity metadata for current account/source EPG cache. |
| `.jiotv.pid` | PID from `background start`. |
| `tls/` | Generated self-signed certificate and key when HTTPS is enabled. |

## Collect logs

If JioTV is running in your terminal, inspect the output there. For a short diagnostic run you can use:

```sh
JIOTV_DEBUG=true JIOTV_LOG_TO_STDOUT=true jiotv serve
```

Read the messages locally before sharing. Redact cookies, tokens, access-key URLs, OTPs, device identifiers and private server addresses. `JIOTV_LOG_PATH` exists in the config schema but does **not** currently implement file output.

For systemd or a container, see [background services](../maintenance-background-service/index.md) and [Docker maintenance](../maintenance-docker/index.md).

## Safe backup and restore

1. Stop JioTV so files are not changing while you copy them.
2. Copy the whole configured data directory into a private location; preserve its permissions.
3. Restore that directory to the same `path_prefix` before starting JioTV.
4. Test the admin login, account status and IPTV playlist link.
5. If restoring to another host, check network settings and rotate the playlist key if its URL may have escaped.

Never store these backups in public Git repositories. See [Backup and restore](../maintenance-backup-restore/index.md).
