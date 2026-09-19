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
- **Live HLS proxying**: `/live/:id`, `/live/:quality/:id`, `/render.m3u8`
  (manifest rewriting so every segment/key URI routes back through this
  server, HDNEA cookie caching and refresh-on-401/403/404 with a
  quality-fallback retry chain, a short "recently dead" cooldown per
  channel), `/render.ts`, `/render.key`.
- **DASH/Widevine proxying**: `/live/mpd/:id`, `/live/key/:id`,
  `/render.mpd` (BaseURL rewriting, injected UTCTiming, CDN publish-time
  clock tracking), `/render.dash/*` segment proxying, `/dashtime`, and the
  `/drm` Widevine license proxy (with the cookie-harvesting HEAD request and
  JioTV auth headers `DRMKeyHandler` sends).
- The static `drm_channels.go` DRM channel-ID list (`src/drm_channels.rs`,
  copied verbatim, 970 IDs), used by the playlist and `/live/mpd` routing.
- Custom channels (`custom_channels_file`, JSON only — see the config gap
  below).
- Catchup stream resolution (`/catchup/stream/:id`), redirecting into the
  same `/render.m3u8` pipeline as live channels.
- A reduced `EnsureFreshCredentials`: refreshes the JioTV access token when
  its own JWT `exp` claim is close, using the saved refresh token.
- EPG generation (`jiotv epg generate`/`epg delete`, and a background
  regenerate-if-missing-or-stale check on `serve` startup when `epg = true`)
  and `/epg.xml.gz`, `/epg/:channelID/:offset`, `/jtvposter/:date/:file`.

## What is NOT at parity yet (be aware before relying on this)

This is a partial rewrite. The pieces below exist in the Go version and do
**not** exist here yet, or exist at reduced fidelity:

- **No singleflight / request de-duplication.** The Go version funnels
  concurrent requests for the same channel's playback URL through a single
  upstream call (`singleflight`); this rewrite does not, so a burst of
  simultaneous requests for one channel makes that many upstream calls.
- **JioTV+ (`tvplus`)** — login, catalogue, mirrors, learned stream-type
  persistence, all of it — is not implemented. The CLI subcommand exists and
  prints a "not implemented" message. `tvPlusRoute`/`tvPlusKeyHeaders`-style
  branches in the Go stream/DRM handlers (TV+ CDN user-agent switching, the
  MPD→HLS/HLS→MPD fallback redirects for TV+-only channels) have no
  equivalent here.
- **On-demand (JioCinema/ZEE5/MX Player)** — the `/api/ott/*`, `/vod.m3u`,
  `/vod/:id`, `/vod/license/:id` surface is not implemented (`/api/ott/play/:id`
  returns 501).
- **No daily EPG regeneration scheduler.** The Go version reschedules
  itself ~24h out at a random off-peak time after every generation; this
  rewrite only checks once at `serve` startup (missing or >24h old triggers
  one background regeneration) and via the `epg generate` CLI command — a
  server left running for days without a restart will not refresh its EPG.
- The web EPG proxy (`/epg/:channelID/:offset`) does not do the Go version's
  "correct the day if the upstream API's clock lags" adjustment
  (`webEPGDayOffset`); it passes the upstream response straight through.
- **Catchup EPG browsing** (`/catchup/:id` listing page, catchup player
  pages) is not implemented — only the stream-resolution endpoint is. There
  is no template engine in this rewrite and the Svelte UI has no catchup
  browser yet.
- **Custom channels file format**: JSON only (`{"channels": [...]}`); the Go
  version also auto-detected YAML.
- **Sony DAI channels** (`sl*`-prefixed IDs backed by Google DAI HLS URLs,
  `SONY_CHANNELS`/`SONY_JIO_MAP` in the Go tree) are not implemented.
- **Premium providers** (SonyLIV/ZEE5-style content bundled into a JioTV
  account itself, distinct from JioTV+ on-demand — `PremiumProviders`,
  `/premium/*` in the Go tree) are not implemented.
- `update`, `epg`, `background`, `autostart` CLI subcommands are not ported.
- The Watch page in the Svelte UI still expects the old Go-template player
  pages; it has not been re-pointed at an in-app Shaka/hls.js player.
- Login credential refresh (`login::LoginClient::refresh`,
  `token_refresh::ensure_fresh`) only covers the JWT-`exp` case; the SSO
  token's own fallback-TTL refresh path (for non-JWT tokens) is not ported.
- `/render.mpd`'s CDN Set-Cookie is not forwarded to the client (segment
  auth is instead carried in the encrypted `/render.dash/.../hdnea/...` path
  segment); a client that needed the cookie directly against the CDN
  wouldn't get it.

None of the above were exercised against the real JioTV/JioTV+/ZEE5 APIs —
per the project's rules, this was built and tested with unit tests, a mock
HTTP server (`wiremock`), and manifest/URL-rewriting unit tests using
hand-written fixtures.
