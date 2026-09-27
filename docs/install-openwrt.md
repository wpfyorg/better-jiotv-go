# Install on OpenWrt

Install the native package that matches the router CPU and package manager. OpenWrt 25.12 and newer uses `apk`; OpenWrt 24.10 and older uses `opkg`. Both `jiotv` (full, recommended) and `jiotv-slim` packages are published for x86_64, aarch64, and ARMv7 when available in the release.

## Detect package manager and architecture

```sh
if command -v apk >/dev/null 2>&1; then FORMAT=apk; else FORMAT=ipk; fi
uname -m
```

| `uname -m` | Rust binary family |
|---|---|
| `x86_64` | x86_64 |
| `aarch64`, `arm64` | aarch64 |
| `armv7l`, `armv7*` | ARMv7 |

The package release provides exact filenames and OpenWrt package architecture identifiers. Download the matching release asset from [GitHub Releases](https://github.com/wpfyorg/better-jiotv-go/releases/latest). Do not install packages for MIPS, MIPS64, ARMv6, or any architecture not listed in that release.

APK release filenames include the OpenWrt package architecture reported by the SDK, for example `jiotv-1.1.0-r1_x86_64.apk`. IPK release filenames keep the SDK-produced name, for example `jiotv_1.1.0-r1_x86_64.ipk`.

For apk-based OpenWrt:

```sh
wget https://github.com/wpfyorg/better-jiotv-go/releases/latest/download/<matching-package>.apk
apk add --allow-untrusted ./<matching-package>.apk
```

For opkg-based OpenWrt:

```sh
wget https://github.com/wpfyorg/better-jiotv-go/releases/latest/download/<matching-package>.ipk
opkg install ./<matching-package>.ipk
```

The package installs `/usr/bin/jiotv`, `/etc/init.d/jiotv`, and `/etc/config/jiotv`. Credentials and application data live under `/etc/jiotv`; upgrades preserve both the UCI config and application state.

## Login and start

```sh
jiotv login otp
jiotv admin password
/etc/init.d/jiotv enable
/etc/init.d/jiotv start
```

Enter the OTP yourself when prompted. Edit `/etc/config/jiotv` to set the listen host, port, EPG, extras, and other supported `JIOTV_*` settings. The service reads those values through UCI and runs under `procd` with respawn and log output.

From a LAN device, open `http://<router-lan-ip>:5001/`. If needed, add a firewall rule scoped to the LAN zone only; do not expose port 5001 on WAN. Keep JioTV access-key and admin authentication enabled.

## Update and remove

Update with the router's package manager using the matching package artifact:

```sh
apk add --allow-untrusted ./<new-package>.apk
# or
opkg install ./<new-package>.ipk
```

Do not use `jiotv update` for the package-owned `/usr/bin/jiotv`. To uninstall, stop the service and remove the package with `apk del jiotv` or `opkg remove jiotv` (use `jiotv-slim` for the slim package). Package removal leaves `/etc/config/jiotv` and `/etc/jiotv` so credentials and settings are not silently deleted.

## Raw binary fallback

Use this only if no package is available for the router's OpenWrt package architecture. The binary is available for x86_64, aarch64, and ARMv7 Linux musl targets. Download the matching `jiotv-{full|slim}-<target>` asset and `SHA256SUMS` from the release, verify the checksum, then install it as `/usr/bin/jiotv`.

Set `JIOTV_PATH_PREFIX=/etc/jiotv`, perform login, and configure `procd` manually. A raw installation is not owned by `apk` or `opkg`; update it using `jiotv update` or by repeating the verified download. See [Configuration](config.md) for supported environment variables.
