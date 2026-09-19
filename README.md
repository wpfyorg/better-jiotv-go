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
jiotv tvplus login     # interactive OTP login to JioTV+ (needs tvplus = true)
jiotv tvplus logout    # delete the saved JioTV+ login
jiotv epg generate     # generate epg.xml.gz now
jiotv epg delete       # delete epg.xml.gz
jiotv background start [--args "..."]   # run `serve` detached, args passed through
jiotv background stop                   # stop it (reads the PID file background start wrote)
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
- The `/k/<key>` gate strips the prefix by hand in a `tower::Service`
  wrapped *around* the whole router (`server::GatedService`), before the
  router ever does its own path matching — not `Router::layer()`
  middleware (runs after matching; can't affect it) and not
  `Router::nest("/k/:key", ...)` (routes correctly, but silently adds an
  extra captured path parameter to every matched route, breaking any
  handler using `Path<String>`/`Path<(String, String)>` under the prefix —
  this shipped once and broke every IPTV stream route before a live check
  caught it). IPTV players and an authenticated admin session reach the
  same routes either way.
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
- `Watch.svelte`'s in-app player (the same pattern `VodPlayer.svelte`
  already used): Shaka Player for DASH + Widevine (license through
  `/live/key/:id`, reached with the same key/session as everything else),
  hls.js 1.7.3 for HLS (including HEVC-in-MPEG-TS, which newer hls.js and
  Chrome's native HLS can't play), falling back to a plain `<video src>`
  when neither can handle the stream. Backed by a new `/api/live/play/:id`
  endpoint returning `{dash, url, license}`, resolved the same way
  `/live/mpd/:id` is (DASH first via `get_drm_mpd`, HLS fallback). Shaka and
  hls.js themselves are vendored (not from a CDN) and served from
  `/static/external/...`, the same path the Go version used.
- On-demand playback via JioTV+ — JioCinema, ZEE5 and MX Player only (every
  other provider in the catalogue only opens a partner app and is filtered
  out): `/api/ott/search`, `/api/ott/screen/:id`, `/api/ott/show/:id`,
  `/api/ott/play/:id`, `/api/ott/license/:id` for the browser, `/vod.m3u`
  (6h cache) and `/vod/:id` for IPTV players, and `/vod/license/:id` /
  `/api/ott/license/:id` proxying the Widevine license to the title's own
  server — JioCinema's or **ZEE5's own** (the owner-approved exception to
  proxying only JioTV/JioTV+ hosts) — with the algo-specific headers
  (`appId`/`appKey` for JioCinema, `customData`/`nl` for ZEE5). Playback
  responses are cached 10 minutes.
- EPG generation (`jiotv epg generate`/`epg delete`, a background
  regenerate-if-missing-or-stale check on `serve` startup when `epg = true`,
  and a recurring ~24h background regeneration loop for as long as the
  server runs) and `/epg.xml.gz`, `/epg/:channelID/:offset` (with the
  upstream day-lag correction), `/jtvposter/:date/:file`.
- Concurrent-request de-duplication (a per-key async mutex map, same effect
  as Go's `singleflight`, no extra crate) on: JioTV's live-URL recovery
  refetch, and TV+'s catalogue/token-refresh/playback caches.
- `/render.mpd` forwards the CDN's `Set-Cookie` to the client (Domain
  stripped, Path rewritten to `/render.dash`).
- **JioTV+ (`tvplus`, off by default)**, compiled into both builds but only
  active when `tvplus = true`: device identity + saved-login store keys
  (`tvplus_device`, `tvplus_credentials`, `tvplus_dash`) byte-compatible
  with the Go tree; OTP login (`jiotv tvplus login` and
  `/api/tvplus/login/sendOTP`+`/verifyOTP`), `tvplus logout` and
  `/api/tvplus/logout`; token refresh (1h lead, de-duplicated); catalogue
  (6h cache) with `Mirrors`/`Exclusive`/`normalizeName`/test-channel
  filtering; routing (`tvp_` IDs always, a JioTV ID only when there's no
  JioTV login and TV+ carries it); playback (60s cache, de-duplicated) with
  learned per-channel DASH/HLS persisted to `tvplus_dash` and the
  MPD→HLS/HLS→MPD fallback redirects; the TV+ player User-Agent on TV+ CDN
  hosts for MPD/segment requests; TV+ license/key headers through `/drm` and
  `/live/key`; `isDRMChannel`/playlist-hiding (`channelPlayable`) using TV+
  state; TV+ channels folded into the EPG.

## What is NOT at parity yet (be aware before relying on this)

This is a partial rewrite. The pieces below exist in the Go version and do
**not** exist here yet, or exist at reduced fidelity:

- **TV+ is compiled into both builds**, not feature-gated out of `slim`
  (it's small — a JSON HTTP client — and disabled by default via
  `tvplus = false`, but it does add a little to the slim binary that a
  router deployment with TV+ permanently off will never use).
- The Sony DAI (`sl*`) channels and JioTV's own "premium providers"
  (SonyLIV/ZEE5 content bundled into a *JioTV* account, unrelated to TV+) are
  not part of the TV+ routing — see their own bullets below.
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
- **`update` is dropped, not ported.** The Go version downloaded a new
  release binary from GitHub and replaced itself; this rewrite has no
  release process to point that at yet, and "download and exec a binary
  fetched over the network" is exactly the kind of thing to not add
  speculatively. Update via your own package manager / redeploy instead.
- **`autostart` (the Termux/bash-profile convenience) is dropped, not
  ported.** It only ever added a line to `~/.bashrc`; low value relative to
  its slice of the rewrite, and easy to do by hand (`echo 'jiotv background
  start' >> ~/.bashrc`) if you want it.
- `background start`/`background stop` (run `serve` detached, stop it via a
  PID file) **are** ported — see the CLI list above.
- Login credential refresh (`login::LoginClient::refresh`,
  `token_refresh::ensure_fresh`) only covers the JWT-`exp` case; the SSO
  token's own fallback-TTL refresh path (for non-JWT tokens) is not ported.
- The daily EPG regeneration loop approximates the Go version's "random
  off-peak hour the next day" scheduling with a simpler "~24h +/- 1h
  jitter" sleep, rather than reproducing its exact hour arithmetic.

None of the above were exercised against the real JioTV/JioTV+/ZEE5 APIs —
per the project's rules, this was built and tested with unit tests, a mock
HTTP server (`wiremock`), and manifest/URL-rewriting unit tests using
hand-written fixtures.
