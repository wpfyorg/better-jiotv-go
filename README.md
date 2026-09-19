# jiotv (Rust)

A from-scratch Rust rewrite of JioTV Go. Streams JioTV as an M3U playlist for
any IPTV player, with an optional web UI. This branch has no Go code left —
see the end of this file for what still isn't at parity with the Go version.

## Builds

One crate, two Cargo features:

- **full** (default) — IPTV core + the Svelte web UI + the admin JSON API.
  Meant for a homelab machine.
  ```
  cargo build --release
  ```
- **slim** — IPTV core only, no UI assets, no admin API. Meant for a small
  router (OpenWrt/musl).
  ```
  cargo build --release --no-default-features --features slim
  ```

Release builds use `opt-level = "z"`, LTO, one codegen unit, `panic = "abort"`
and `strip = true` (see `[profile.release]` in `Cargo.toml`) to keep the
binary small.

### Cross-compiling for an OpenWrt router (aarch64 musl)

```
rustup target add aarch64-unknown-linux-musl
cargo install cargo-zigbuild   # needs zig on PATH
cargo zigbuild --release --no-default-features --features slim \
  --target aarch64-unknown-linux-musl
```

The full build cross-compiles the same way with `--features full` if you want
the UI on a router too (bigger binary, more RAM).

## Running

```
cargo run -- serve --host localhost --port 5001
```

On first run it creates `~/.jiotv_go/store_v4.toml` (or
`$JIOTV_PATH_PREFIX/store_v4.toml`), generates an access key, and prints the
keyed playlist URL:

```
Playlist: http://localhost:5001/k/<32-hex-key>/playlist.m3u
Web setup: http://localhost:5001/k/<32-hex-key>/   (full build, no password set yet)
```

Every request needs either that `/k/<key>/` prefix or (for the admin UI) a
signed-in session; see `docs/config.md` for `disable_auth` if you want to run
without either (e.g. behind your own reverse proxy's auth).

### CLI

```
jiotv serve [--host H] [--port P] [--public] [--tls] [--tls-cert C] [--tls-key K] [--tunnel] [--tunnel-token T]
jiotv login otp        # interactive OTP login to JioTV
jiotv login reset      # delete the saved login
jiotv admin password   # set/replace the full build's admin password
jiotv key show         # print the current keyed playlist URL
jiotv key rotate       # replace the access key (old playlist URLs stop working)
jiotv tvplus login|logout   # not implemented yet, see below
```

`--config <path>` and `--skip-update-check` are accepted at the top level, as
in the Go version.

### Tunnel

`serve --tunnel` looks for `cloudflared` in the data directory or on `PATH`;
if it's missing, it downloads the official release binary for your OS/arch
into the data directory (printing what it's downloading and to where), then
spawns a quick `trycloudflare.com` tunnel and prints the public keyed
playlist URL. `--tunnel-token <token>` (or `JIOTV_TUNNEL_TOKEN`) runs a named
tunnel instead. The child process is killed when the server exits. No tunnel
hostname is ever written into this repo.

The tunnel is deliberately just a downloaded binary spawned as a child
process — no tunnel crate is linked into either build.

## Configuration

See `docs/config.md`. In short: a `JIOTV_*` environment variable per option,
or a TOML config file (`jiotv_go.toml` / `config.toml` in the working
directory, or `--config <path>`). The store file (`store_v4.toml`) and every
`JIOTV_*` variable keep the same name as the Go version, so an existing data
directory keeps working.

## What's implemented

- Config loading (env vars + TOML file), matching the Go option names.
- The `store_v4.toml` key/value store, file-compatible with the Go version.
- The `/k/<key>/` access gate, admin password (PBKDF2-SHA256, 600k rounds)
  and signed 30-day session cookie, login rate limiting — all as in
  `internal/access` in the Go tree.
- AES-256-CTR URL encryption (`secureurl`), including the deterministic
  variant, with `disable_url_encryption` supported.
- JioTV OTP login (`login otp`), channel list fetch, and `playlist.m3u` /
  `/channels?type=m3u` generation with the `q`/`c`/`l`/`sg`/`sub` filters and
  the same M3U/KODIPROP format as the Go version.
- The admin JSON API (`/api/auth/*`, `/api/status`, `/api/channels`,
  `/api/key/rotate`, `/api/account/password`, `/api/jiotv/logout`) and the
  Svelte UI (`web/ui`, unchanged from the Go tree) served from the full
  binary.
- The `/k/:key` route nesting so IPTV players and an authenticated admin
  session can both reach the same playlist/channel routes, matching the Go
  gate's behaviour.
- The `--tunnel` / `--tunnel-token` cloudflared wrapper.
- `/jtvimage/:file` logo proxy.

## What is NOT at parity yet (be aware before relying on this)

This is a partial rewrite. The pieces below exist in the Go version and do
**not** exist here yet:

- **Live stream proxying** (`/live/:id`, `/live/:quality/:id`,
  `/render.m3u8`, `/render.ts`, `/render.key`, `/live/mpd/:id`,
  `/live/key/:id`, `/render.mpd`, `/render.dash/*`, `/dashtime`) and the DRM
  license proxy (`/drm`) are not implemented. The playlist links to these
  routes, but they 404. This is the biggest gap: without it, channels don't
  actually play yet.
- **JioTV+ (`tvplus`)** — login, catalogue, mirrors, learned stream-type
  persistence, all of it — is not implemented. The CLI subcommand exists and
  prints a "not implemented" message.
- **On-demand (JioCinema/ZEE5/MX Player)** — the `/api/ott/*`, `/vod.m3u`,
  `/vod/:id`, `/vod/license/:id` surface is not implemented (`/api/ott/play/:id`
  returns 501).
- **EPG generation** (`epg.xml.gz`) is not implemented; `epg generate`/`epg
  delete` CLI commands don't exist yet.
- **Catchup** is not implemented.
- **Custom channels** (`custom_channels_file`) and the hardcoded Sony DAI
  channel list are not implemented.
- **DRM channel detection** (`drm_channels.go`'s list) is not implemented;
  `AppState::is_drm_channel` always returns false, so the playlist never
  emits `/live/mpd/...` entries yet.
- `update`, `epg`, `background`, `autostart` CLI subcommands are not ported.
- The Watch page in the Svelte UI still expects a working `/mpd/:id` /
  `/live/...` backend; it has not been re-pointed at a Shaka/hls.js in-app
  player, since the server side it would call isn't implemented yet either.
- Login credential refresh (`login::LoginClient::refresh`) is implemented
  but not wired into a background refresh loop.

None of the above were exercised against the real JioTV/JioTV+/ZEE5 APIs —
per the project's rules, this was built and tested with unit tests and mock
servers only.
