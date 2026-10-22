---
title: Configuration Reference
description: Verified TOML keys, JIOTV environment variables, defaults and accepted values.
category: Configuration
keywords: [config, TOML, JIOTV, environment variables, defaults, debug, extras]
---

# Configuration Reference

JioTV reads an optional **TOML** file and environment variables. Environment variables override the file. The Rust build only parses TOML, even though its automatic filename search also sees YAML/JSON names. A YAML or JSON file at a matching name will cause a parse error; convert it to TOML or use environment variables.

Explicit path: `jiotv --config /path/to/jiotv.toml serve`. The parser also accepts `jiotv serve --config /path/to/jiotv.toml`.

If `--config` is omitted, it searches the current working directory, in this order: `jiotv_go.yml`, `jiotv_go.yaml`, `jiotv_go.toml`, `jiotv_go.json`, `config.json`, `config.yml`, `config.toml`, `config.yaml`. Only choose a `.toml` filename.

## Example

```toml
# jiotv_go.toml (sample values; no credentials)
title = "Home TV"
epg = true
drm = true
extras = false
disable_auth = false
debug = false
log_to_stdout = true
default_categories = []
default_languages = []
```

Save, start with `jiotv --config ./jiotv_go.toml serve`, then open `http://localhost:5001/`.

## Complete configuration table

| TOML key | Environment variable | Default | Purpose and accepted values |
| --- | --- | --- | --- |
| `epg` | `JIOTV_EPG` | `false` | Boolean. Generate/refresh the XMLTV guide. |
| `debug` | `JIOTV_DEBUG` | `false` | Boolean. Show more diagnostic output. |
| `disable_ts_handler` | `JIOTV_DISABLE_TS_HANDLER` | `false` | Boolean. Avoid local TS/AAC segment proxying when supported; change only when diagnosing HLS. |
| `disable_logout` | `JIOTV_DISABLE_LOGOUT` | `false` | Boolean. Hide/disable JioTV logout when true. |
| `drm` | `JIOTV_DRM` | `true` | Boolean. Allow the protected DASH pathway; does not bypass DRM. |
| `extras` | `JIOTV_EXTRAS` | `false` | Boolean. Enable optional Extras account integration. Requires a build with Extras support. |
| `disable_auth` | `JIOTV_DISABLE_AUTH` | `false` | Boolean. Disable app access checks. Dangerous on reachable networks; use only with a trusted external access control. |
| `title` | `JIOTV_TITLE` | `"JioTV Go"` | Display title; any TOML string. |
| `disable_url_encryption` | `JIOTV_DISABLE_URL_ENCRYPTION` | `false` | Boolean. Disable encryption of local proxy parameters. Leave false. |
| `proxy` | `JIOTV_PROXY` | `""` | Optional outgoing proxy URL. |
| `path_prefix` | `JIOTV_PATH_PREFIX` | `""` | Directory for JioTV state. Empty uses the runtime's resolved default. |
| `log_path` | `JIOTV_LOG_PATH` | `""` | Reserved in this build; do not rely on file logging. |
| `log_to_stdout` | `JIOTV_LOG_TO_STDOUT` | `true` | Boolean. Emit logs to process output. |
| `custom_channels_file` | `JIOTV_CUSTOM_CHANNELS_FILE` | `""` | Optional path to custom-channel JSON file. |
| `default_categories` | `JIOTV_DEFAULT_CATEGORIES` | `[]` | Integer category IDs; environment form is comma-separated. |
| `default_languages` | `JIOTV_DEFAULT_LANGUAGES` | `[]` | Integer language IDs; environment form is comma-separated. |

Boolean environment values must be `true` or `false` in lowercase (as Rust's parser expects). Example: `JIOTV_EPG=true jiotv serve`. For integer lists, use `JIOTV_DEFAULT_LANGUAGES=1,6`.

Older deployments may use the TOML alias `tvplus` or `JIOTV_TVPLUS`. Prefer `extras` and `JIOTV_EXTRAS`; the newer environment variable takes precedence over the older one. `JIOTV_TUNNEL_TOKEN` is also accepted for the optional Cloudflare tunnel flag. Update uses `JIOTV_UPDATE_TOKEN` for private repository access, when required.

For runtime listening options, see [CLI commands](../configuration-cli/index.md); for protection see [Network and HTTPS](../configuration-network-security/index.md); for storage see [Data and logs](../configuration-storage-logs/index.md).
