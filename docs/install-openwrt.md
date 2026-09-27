# Install on OpenWrt

The Rust release binaries are a good fit for OpenWrt because Linux releases are built for musl. The project currently publishes raw binaries, not native OpenWrt packages.

Use the `full` build if you want the web UI. Use `slim` for a headless IPTV-only deployment.

## Supported router architectures

Check the router first:

```sh
uname -m
```

Map the result to the release asset:

| `uname -m` | Target |
|---|---|
| `x86_64` | `x86_64-unknown-linux-musl` |
| `aarch64`, `arm64` | `aarch64-unknown-linux-musl` |
| `armv7l`, `armv7*` | `armv7-unknown-linux-musleabihf` |

MIPS/MIPS64 and ARMv6 routers are not covered by the current release matrix.

## Install the raw release binary

The built-in updater defaults to releases from `wpfyorg/better-jiotv-go`. If your deployment publishes releases from another fork, change `REPO` below.

```sh
REPO=wpfyorg/better-jiotv-go
VARIANT=full

case "$(uname -m)" in
  x86_64) TARGET=x86_64-unknown-linux-musl ;;
  aarch64|arm64) TARGET=aarch64-unknown-linux-musl ;;
  armv7l|armv7*) TARGET=armv7-unknown-linux-musleabihf ;;
  *) echo "unsupported release architecture: $(uname -m)"; exit 1 ;;
esac

ASSET="jiotv-${VARIANT}-${TARGET}"
wget -O "/tmp/${ASSET}" "https://github.com/${REPO}/releases/latest/download/${ASSET}"
wget -O /tmp/SHA256SUMS "https://github.com/${REPO}/releases/latest/download/SHA256SUMS"
(cd /tmp && grep "  ${ASSET}$" SHA256SUMS | sha256sum -c -) || exit 1

cp "/tmp/${ASSET}" /usr/bin/jiotv
chmod 0755 /usr/bin/jiotv
/usr/bin/jiotv --help
```

The direct `wget` method assumes the release is publicly downloadable. For a private release repository, download the asset with authenticated GitHub access or build/copy the binary from another machine.

## Create persistent state and log in

Keep runtime credentials in a persistent private directory:

```sh
mkdir -p /etc/jiotv
chmod 0700 /etc/jiotv
export JIOTV_PATH_PREFIX=/etc/jiotv
```

Run the interactive login and enter the OTP yourself when prompted:

```sh
jiotv login otp
jiotv admin password
```

Optional settings can be exported before starting the service, for example:

```sh
export JIOTV_EPG=true
export JIOTV_EXTRAS=false
```

See [Configuration](config.md) for the complete list.

## Run at boot with `procd`

`jiotv autostart` is for systemd/Termux and does not install an OpenWrt service. Use `procd` on OpenWrt.

Create `/etc/init.d/jiotv`:

```sh
cat >/etc/init.d/jiotv <<'EOF'
#!/bin/sh /etc/rc.common

START=95
STOP=10
USE_PROCD=1

start_service() {
        procd_open_instance
        procd_set_param command /usr/bin/jiotv --skip-update-check serve --host 0.0.0.0 --port 5001
        procd_set_param env HOME=/root
        procd_set_param env JIOTV_PATH_PREFIX=/etc/jiotv
        procd_set_param env JIOTV_EPG=true
        procd_set_param respawn
        procd_set_param stdout 1
        procd_set_param stderr 1
        procd_close_instance
}
EOF

chmod 0755 /etc/init.d/jiotv
/etc/init.d/jiotv enable
/etc/init.d/jiotv start
```

Check it:

```sh
pgrep -af jiotv
logread -e jiotv
```

From another LAN device, open `http://ROUTER_LAN_IP:5001/`.

Do not set `JIOTV_DISABLE_AUTH=true` just because the service is LAN-only. Keep the access key/admin password unless another trusted authentication layer is in front of it.

## Firewall

If the router firewall blocks LAN access to port 5001, add a LAN-only rule rather than exposing the service on WAN. For example with UCI:

```sh
uci add firewall rule
uci set firewall.@rule[-1].name='Allow-JioTV-LAN'
uci set firewall.@rule[-1].src='lan'
uci set firewall.@rule[-1].proto='tcp'
uci set firewall.@rule[-1].dest_port='5001'
uci set firewall.@rule[-1].target='ACCEPT'
uci commit firewall
/etc/init.d/firewall restart
```

## Can this be installed with `apk` or `opkg`?

Yes, but package artifacts are not produced by this repository yet.

OpenWrt 25.12 and newer uses `apk`; OpenWrt 24.10 and older uses `opkg`. A proper package should install at least:

- `/usr/bin/jiotv`
- `/etc/init.d/jiotv`
- an optional `/etc/config/jiotv`/environment file
- package metadata declaring supported architectures

Once such artifacts are built, local installation would look like:

```sh
# OpenWrt 25.12+
apk add --allow-untrusted ./jiotv-*.apk

# OpenWrt 24.10 and older
opkg install ./jiotv_*.ipk
```

That is better than copying a binary by hand because the package manager can own upgrades/removal and install the `procd` service atomically. The current GitHub release workflow would need an additional packaging job before those commands become a real distribution path.

OpenWrt package-manager references:

- <https://openwrt.org/docs/guide-user/additional-software/managing_packages>
- <https://openwrt.org/docs/guide-user/additional-software/opkg-to-apk-cheatsheet>

## Updating

For a manually installed binary, either repeat the download/checksum steps above or run:

```sh
jiotv update
/etc/init.d/jiotv restart
```

If the release repository is private, set `JIOTV_UPDATE_TOKEN` for the update command. Package-managed installations should eventually be updated through their package feed instead of `jiotv update`.
