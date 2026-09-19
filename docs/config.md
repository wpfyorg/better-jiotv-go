# Configuration

Every option can be set as a `JIOTV_*` environment variable, or as a key in a
TOML config file. Environment variables win over the config file.

## Config file

`jiotv serve --config <path>` uses that file. Otherwise the server looks for,
in order: `jiotv_go.yml`, `jiotv_go.yaml`, `jiotv_go.toml`, `jiotv_go.json`,
`config.json`, `config.yml`, `config.toml`, `config.yaml` in the current
directory. Only **TOML** is actually parsed by this Rust version (the Go
version also accepted YAML/JSON via a generic config library); rename your
file to `.toml` if you're moving an existing YAML/JSON config over, or set
the equivalent `JIOTV_*` environment variables instead.

Example `jiotv_go.toml`:

```toml
title = "My JioTV"
epg = false
drm = true
disable_auth = false
path_prefix = "/data/jiotv"
default_categories = [5, 8]
default_languages = [1, 6]
```

## Options

| Key (TOML)               | Env var                          | Default        | Meaning |
|---------------------------|-----------------------------------|----------------|---------|
| `epg`                     | `JIOTV_EPG`                       | `false`        | Enable EPG generation. **Not implemented in this Rust version yet.** |
| `debug`                   | `JIOTV_DEBUG`                     | `false`        | Verbose logging. |
| `disable_ts_handler`      | `JIOTV_DISABLE_TS_HANDLER`        | `false`        | Reserved; the `.ts` segment proxy it refers to isn't implemented yet either. |
| `disable_logout`          | `JIOTV_DISABLE_LOGOUT`            | `false`        | Disable the logout button/API. |
| `drm`                     | `JIOTV_DRM`                       | `true`         | Enable DRM (Widevine) channels. **The DRM stream/license proxy isn't implemented yet**, so this currently has no effect. |
| `tvplus`                  | `JIOTV_TVPLUS`                    | `false`        | Enable JioTV+ channels. **Not implemented in this Rust version yet**; the flag is accepted but does nothing. |
| `disable_auth`            | `JIOTV_DISABLE_AUTH`              | `false`        | Serve without the `/k/<key>/` access key or the admin password. Only do this behind your own auth (reverse proxy, VPN, etc). |
| `title`                   | `JIOTV_TITLE`                     | `"JioTV Go"`   | Page title. |
| `disable_url_encryption`  | `JIOTV_DISABLE_URL_ENCRYPTION`    | `false`        | Turn off AES encryption of stream-proxy URL parameters (they're percent-encoded instead). Never combine with `disable_auth`. |
| `proxy`                   | `JIOTV_PROXY`                     | `""`           | Outbound proxy URL for JioTV API requests. |
| `path_prefix`             | `JIOTV_PATH_PREFIX`               | `~/.jiotv_go`  | Where `store_v4.toml` and other data live. |
| `log_path`                | `JIOTV_LOG_PATH`                  | `""`           | Reserved; not wired to a file sink yet (only stdout logging is implemented). |
| `log_to_stdout`           | `JIOTV_LOG_TO_STDOUT`             | `true`         | Log to stdout/stderr. |
| `custom_channels_file`    | `JIOTV_CUSTOM_CHANNELS_FILE`      | `""`           | Reserved; custom channels aren't implemented yet. |
| `default_categories`      | `JIOTV_DEFAULT_CATEGORIES`        | `[]`           | Category IDs to default the UI to (comma-separated in the env var). |
| `default_languages`       | `JIOTV_DEFAULT_LANGUAGES`         | `[]`           | Language IDs to default the UI to (comma-separated in the env var). |

`JIOTV_TUNNEL_TOKEN` (no config-file key) sets the token for a named
`cloudflared` tunnel; see the README's Tunnel section.

## Data directory

`path_prefix` (default `~/.jiotv_go`) holds:

- `store_v4.toml` — the key/value store (access key, admin password hash,
  session secret, saved JioTV login). Same file name and `{ data = {...} }`
  shape as the Go version, so a store carried over from Go keeps working for
  the settings this Rust version has ported (access key, admin password,
  session secret). It does **not** reuse the Go version's separate JioTV
  login credentials file — run `jiotv login otp` again after switching.
- `cloudflared` (or `cloudflared.exe`) — downloaded only when you pass
  `--tunnel` and it isn't already on `PATH`.

Never commit or share this directory: it holds your access key, your admin
password hash, and (once implemented further) your JioTV login token.
