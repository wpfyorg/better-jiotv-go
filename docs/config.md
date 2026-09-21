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
| `epg`                     | `JIOTV_EPG`                       | `false`        | Generate `epg.xml.gz` at startup (if missing/stale) and roughly every 24h after. |
| `debug`                   | `JIOTV_DEBUG`                     | `false`        | Verbose logging. |
| `disable_ts_handler`      | `JIOTV_DISABLE_TS_HANDLER`        | `false`        | Serve `.ts`/`.aac` segment URLs straight from JioTV instead of proxying them through `/render.ts`. |
| `disable_logout`          | `JIOTV_DISABLE_LOGOUT`            | `false`        | Disable the logout button/API (JioTV and the extra source). |
| `drm`                     | `JIOTV_DRM`                       | `true`         | Enable DRM (Widevine DASH) channels; when off, every channel is offered as HLS only. |
| `extras`                  | `JIOTV_EXTRAS`                    | `false`        | Turn on the optional extra channel source (login, catalogue, playback, on-demand), the same way in as the panel's unlock code below. Needs `jiotv extras login` afterwards. The older names `tvplus` (config key) and `JIOTV_TVPLUS` (env var) still work, silently, if you're carrying over a config from before this option was renamed. |
| `disable_auth`            | `JIOTV_DISABLE_AUTH`              | `false`        | Serve without the `/k/<key>/` access key or the admin password. Only do this behind your own auth (reverse proxy, VPN, etc). |
| `title`                   | `JIOTV_TITLE`                     | `"JioTV Go"`   | Page title. |
| `disable_url_encryption`  | `JIOTV_DISABLE_URL_ENCRYPTION`    | `false`        | Turn off AES encryption of stream-proxy URL parameters (they're percent-encoded instead). Never combine with `disable_auth`. |
| `proxy`                   | `JIOTV_PROXY`                     | `""`           | Outbound proxy URL for JioTV API requests. |
| `path_prefix`             | `JIOTV_PATH_PREFIX`               | `~/.jiotv_go`  | Where `store_v4.toml` and other data live. |
| `log_path`                | `JIOTV_LOG_PATH`                  | `""`           | Reserved; not wired to a file sink yet (only stdout logging is implemented). |
| `log_to_stdout`           | `JIOTV_LOG_TO_STDOUT`             | `true`         | Log to stdout/stderr. |
| `custom_channels_file`    | `JIOTV_CUSTOM_CHANNELS_FILE`      | `""`           | Path to a JSON file of extra channels (`{"channels": [{"id","name","url","logo_url","category","language","is_hd"}]}`). YAML isn't supported (the Go version's other accepted format). |
| `default_categories`      | `JIOTV_DEFAULT_CATEGORIES`        | `[]`           | Category IDs to default the UI to (comma-separated in the env var). |
| `default_languages`       | `JIOTV_DEFAULT_LANGUAGES`         | `[]`           | Language IDs to default the UI to (comma-separated in the env var). |

`JIOTV_TUNNEL_TOKEN` (no config-file key) sets the token for a named
`cloudflared` tunnel; see the README's Tunnel section.

## Data directory

`path_prefix` (default `~/.jiotv_go`) holds:

- `store_v4.toml` — the key/value store: access key, admin password hash,
  session secret, whether the panel's unlock is currently active
  (`extras_unlocked`), and (once you've logged in to the extra source) its
  device identity, saved login and learned DASH/HLS map
  (`extras_device`/`extras_credentials`/`extras_stream_kinds`). A store from
  before this project's extra-source keys were renamed keeps working: the
  old names (`tvplus_device`, `tvplus_credentials`, `tvplus_dash`) are read
  as a fallback and migrated to the new names on first save. It does **not**
  reuse the Go version's separate plain-JioTV login credentials file — run
  `jiotv login otp` again after switching.
- `epg.xml.gz` — generated when `epg = true`.
- `.jiotv.pid` — written by `jiotv background start`, removed by `background
  stop`.
- `cloudflared` (or `cloudflared.exe`) — downloaded only when you pass
  `--tunnel` and it isn't already on `PATH`.

Never commit or share this directory: it holds your access key, your admin
password hash, and (once you've logged in) your JioTV and extra-source
tokens.

## The extra channel source, and its unlock code

Besides JioTV itself, the server can optionally carry channels and
on-demand titles from one extra source that needs its own login. It's off
by default. There are two ways to turn it on:

- **`extras = true` / `JIOTV_EXTRAS=true`** — always on, for a headless
  install (router, server with no browser access to the panel).
- **The panel's unlock code** — typed into the channel search box on the
  web UI's Channels page. It isn't a normal search term, so nothing is sent
  to the server unless what you typed has the shape described below; an
  ordinary search never leaves your browser. Once accepted, the extra
  source's section appears in Settings without a reload, and a "Lock"
  button there turns it back off (this only clears the panel's own unlock —
  it has no effect on the `extras` config/env switch).

The unlock code is not a secret hidden anywhere in this project — it's
computed from two things anyone administering the server can look up
themselves: **the server's own public IPv4 address** (not a LAN address —
what a site like `https://cloudflare.com/cdn-cgi/trace` reports for the
machine) and **the current date, in the server's local time**. The server
fetches its public IP from `https://cloudflare.com/cdn-cgi/trace` and caches
it for 10 minutes; if the machine is IPv6-only or the address can't be
fetched, no unlock code will work until it can, and the server says so
plainly in its response and logs.

Format, case-insensitive:

```
<octet1><MON><day>A<octet2>K<octet3>N<octet4>
```

- `<octet1>`, `<octet2>`, `<octet3>`, `<octet4>` are the server's public
  IPv4 address's four octets, in order.
- `<MON>` is the month, as the 3-letter English abbreviation (`SEP`) or the
  full name (`september`).
- `<day>` is the day of month, 1 or 2 digits, with or without a leading
  zero (`7` or `07`).
- The letters `A`, `K` and `N` between the numbers are fixed — they're part
  of the format, not placeholders.

Worked example: public IP `49.37.12.214` on 21 September gives
`49SEP21A37K12N214` (equivalently `49september21a37k12n214`). The date
accepts yesterday, today or tomorrow in the server's local time (each
checked against its own month name, so the code still works right at a
month boundary), to allow for clock/timezone slop between you and the
server.
