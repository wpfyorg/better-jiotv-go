#!/bin/sh
set -eu

repo=${JIOTV_REPO:-wpfyorg/better-jiotv-go}
variant=${JIOTV_VARIANT:-full}
version=${JIOTV_VERSION:-latest}
install_tls=${JIOTV_INSTALL_TLS:-1}
start_service=${JIOTV_START_SERVICE:-1}

case "$install_tls" in 0|1) ;; *) echo "JIOTV_INSTALL_TLS must be 0 or 1" >&2; exit 2 ;; esac
case "$start_service" in 0|1) ;; *) echo "JIOTV_START_SERVICE must be 0 or 1" >&2; exit 2 ;; esac

case "$variant" in full|slim) ;; *) echo "JIOTV_VARIANT must be full or slim" >&2; exit 2 ;; esac
case "$repo" in */*) ;; *) echo "JIOTV_REPO must be owner/repository" >&2; exit 2 ;; esac

sys=$(uname -s 2>/dev/null || echo unknown)
machine=$(uname -m 2>/dev/null || echo unknown)
termux=false
case "${PREFIX:-}" in *com.termux*) termux=true ;; esac
case "$sys" in Android) termux=true ;; esac

tmp=${TMPDIR:-/tmp}/jiotv-install-$$
mkdir -m 700 "$tmp"
trap 'rm -rf "$tmp"' 0 HUP INT TERM

say() { printf '==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }

download() {
  if command -v curl >/dev/null 2>&1; then curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then wget -q "$1" -O "$2"
  else echo "curl or wget is required" >&2; return 1
  fi
}

verify_asset() {
  asset_name=$1
  sums_file=$2
  asset_file=$3
  expected=$(awk -v name="$asset_name" '$2 == name || $2 == "*" name { print $1; exit }' "$sums_file")
  [ -n "$expected" ] || { echo "SHA256SUMS has no entry for $asset_name" >&2; exit 1; }
  if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum "$asset_file" | awk '{print $1}')
  elif command -v shasum >/dev/null 2>&1; then actual=$(shasum -a 256 "$asset_file" | awk '{print $1}')
  else echo "sha256sum or shasum is required to verify the download" >&2; exit 1
  fi
  [ "$(printf '%s' "$expected" | tr 'A-F' 'a-f')" = "$(printf '%s' "$actual" | tr 'A-F' 'a-f')" ] || { echo "checksum mismatch for $asset_name" >&2; exit 1; }
}

size_kib() { echo $(( $(wc -c <"$1") / 1024 )); }

# True once the URL answers; used to confirm the service really came up.
http_ok() {
  if command -v curl >/dev/null 2>&1; then curl -fsS -o /dev/null --max-time 3 "$1" >/dev/null 2>&1
  elif command -v wget >/dev/null 2>&1; then wget -q -T 3 -O /dev/null "$1" >/dev/null 2>&1
  else return 1
  fi
}

# Print one uci option of the jiotv service, or the default when unavailable.
uci_opt() {
  value=$(uci -q get "jiotv.main.$1" 2>/dev/null || true)
  if [ -n "$value" ]; then echo "$value"; else echo "$2"; fi
}

# The router's LAN address, so the printed URLs can be opened as shown.
router_ip() {
  addr=$(uci -q get network.lan.ipaddr 2>/dev/null | head -n 1 || true)
  if [ -z "$addr" ] && command -v ip >/dev/null 2>&1; then
    addr=$(ip -4 addr show br-lan 2>/dev/null | sed -n 's/.*inet \([0-9.]*\).*/\1/p' | head -n 1)
  fi
  addr=${addr%%/*}
  if [ -n "$addr" ]; then echo "$addr"; else echo "<router-ip>"; fi
}

openwrt=false
if [ "${JIOTV_PLATFORM:-}" = openwrt ] || [ -r /etc/openwrt_release ]; then openwrt=true; fi

if [ "$openwrt" = true ]; then
  [ "$(id -u 2>/dev/null || echo 1)" = 0 ] || { echo "OpenWrt installation must be run as root" >&2; exit 1; }

  if command -v apk >/dev/null 2>&1; then
    package_manager=apk
    package_ext=apk
  elif command -v opkg >/dev/null 2>&1; then
    package_manager=opkg
    package_ext=ipk
  else
    echo "OpenWrt package manager not found (expected apk or opkg)" >&2
    exit 1
  fi

  package_arch=$(sed -n "s/^DISTRIB_ARCH='\([^']*\)'/\1/p" /etc/openwrt_release 2>/dev/null | head -n 1 || true)
  if [ -z "$package_arch" ]; then
    if [ "$package_manager" = apk ]; then
      package_arch=$(apk --print-arch 2>/dev/null || true)
    else
      package_arch=$(opkg print-architecture 2>/dev/null | awk '$1 == "arch" && $2 != "all" && ($3 + 0) >= best { best = $3 + 0; arch = $2 } END { print arch }')
    fi
  fi

  case "$package_arch" in
    aarch64|arm64) package_arch=aarch64_cortex-a53 ;;
    x86_64|aarch64_cortex-a53|arm_cortex-a7_neon-vfpv4) ;;
    *) echo "unsupported OpenWrt package architecture: ${package_arch:-unknown}" >&2; exit 1 ;;
  esac

  say "OpenWrt detected: package manager $package_manager, architecture $package_arch"
  if [ "$version" = latest ]; then
    release_api="https://api.github.com/repos/${repo}/releases/latest"
  else
    case "$version" in v*) tag=$version ;; *) tag="v${version}" ;; esac
    release_api="https://api.github.com/repos/${repo}/releases/tags/${tag}"
  fi
  download "$release_api" "$tmp/release.json"
  tag=$(sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/release.json" | head -n 1)
  [ -n "$tag" ] || { echo "could not determine the release version" >&2; exit 1; }
  say "Release $tag"

  package_name=jiotv
  [ "$variant" = slim ] && package_name=jiotv-slim
  if [ "$package_ext" = apk ]; then
    if [ "$variant" = full ]; then
      pattern="^jiotv-[0-9].*_${package_arch}\\.apk$"
    else
      pattern="^jiotv-slim-.*_${package_arch}\\.apk$"
    fi
  else
    pattern="^${package_name}_.*_${package_arch}\\.ipk$"
  fi
  asset=$(sed -n 's/.*"name":[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/release.json" | grep -E "$pattern" | head -n 1 || true)
  [ -n "$asset" ] || { echo "no $variant OpenWrt package found for $machine in $tag" >&2; exit 1; }

  base="https://github.com/${repo}/releases/download/${tag}"
  say "Downloading $asset"
  download "$base/$asset" "$tmp/$asset"
  note "$(size_kib "$tmp/$asset") KiB"
  download "$base/SHA256SUMS" "$tmp/SHA256SUMS"
  verify_asset "$asset" "$tmp/SHA256SUMS" "$tmp/$asset"
  say "Checksum verified (SHA-256)"

  other_package=jiotv-slim
  [ "$variant" = slim ] && other_package=jiotv
  say "Installing with $package_manager"
  if [ "$package_manager" = apk ]; then
    if apk info -e "$other_package" >/dev/null 2>&1; then apk del "$other_package"; fi
    apk add --allow-untrusted "$tmp/$asset"
  else
    if opkg status "$other_package" 2>/dev/null | grep -q '^Status: .* installed$'; then opkg remove "$other_package"; fi
    opkg install "$tmp/$asset"
  fi
  init_script=${JIOTV_INIT_SCRIPT:-/etc/init.d/jiotv}
  say "Enabling the service at boot"
  "$init_script" enable

  running=false
  if [ "$start_service" = 1 ]; then
    # restart, not start: an upgrade must replace a process that is already running.
    say "Starting the service"
    if "$init_script" restart; then
      http_port=$(uci_opt port 5001)
      tries=0
      while [ "$tries" -lt 15 ]; do
        if http_ok "http://127.0.0.1:${http_port}/"; then running=true; break; fi
        tries=$((tries + 1))
        sleep 1
      done
      if [ "$running" = true ]; then note "listening on port $http_port"
      else echo "warning: the service did not answer on port $http_port within 15 seconds" >&2
      fi
    else
      echo "warning: '$init_script restart' failed" >&2
    fi
  fi

  ip=$(router_ip)
  tls_on=$(uci_opt tls 1)
  http_port=$(uci_opt port 5001)
  tls_port=$(uci_opt tls_port 5443)
  echo
  if [ "$running" = true ]; then echo "JioTV ($variant, ${tag#v}) is installed and running."
  elif [ "$start_service" = 1 ]; then echo "JioTV ($variant, ${tag#v}) is installed, but the service is not answering yet."
  else echo "JioTV ($variant, ${tag#v}) is installed (not started: JIOTV_START_SERVICE=0)."
  fi
  echo
  if [ "$install_tls" = 1 ] && [ "$tls_on" != 0 ]; then
    echo "  Browser UI : https://${ip}:${tls_port}/  (HTTPS, self-signed certificate; accept the one-time warning)"
    echo "  IPTV apps  : http://${ip}:${http_port}/  (plain HTTP playlist)"
  else
    echo "  Browser UI : http://${ip}:${http_port}/  (browsers need HTTPS or localhost for DRM and encrypted HLS playback)"
    [ "$install_tls" = 1 ] || echo "  HTTPS instructions are off (JIOTV_INSTALL_TLS=0); the service setting is unchanged, so HTTPS stays on unless 'option tls 0' is set in /etc/config/jiotv."
  fi
  echo
  echo "Next steps:"
  if [ "$running" = true ]; then
    echo "  1. Open the browser UI and sign in (you enter the OTP yourself)."
  else
    echo "  1. Start the service: $init_script start  (then check: logread -e jiotv)"
    echo "     and sign in from the browser UI (you enter the OTP yourself)."
  fi
  echo "  2. Set the admin password: jiotv admin password"
  echo "  Prefer the terminal for signing in? Run 'jiotv login otp', then '$init_script restart'."
  echo
  echo "Service control: $init_script start|stop|restart    Logs: logread -e jiotv"
  [ "$start_service" = 0 ] || [ "$running" = true ] || exit 1
  exit 0
fi

case "$sys:$machine" in
  Linux:x86_64) target=x86_64-unknown-linux-musl ;;
  Linux:aarch64|Linux:arm64) target=aarch64-unknown-linux-musl ;;
  Linux:armv7l|Linux:armv7*) target=armv7-unknown-linux-musleabihf ;;
  Linux:i?86) target=i686-unknown-linux-musl ;;
  Darwin:x86_64) target=x86_64-apple-darwin ;;
  Darwin:arm64|Darwin:aarch64) target=aarch64-apple-darwin ;;
  Android:aarch64|Android:arm64|Linux:aarch64) target=aarch64-linux-android ;;
  Android:armv7l|Android:armv7*) target=armv7-linux-androideabi ;;
  Android:x86_64|Android:x86_64) target=x86_64-linux-android ;;
  *) echo "unsupported platform: $sys/$machine" >&2; exit 1 ;;
esac

# Termux reports Linux, so use its uname architecture to choose Android ABI.
if [ "$termux" = true ]; then
  case "$machine" in
    aarch64|arm64) target=aarch64-linux-android ;;
    armv7l|armv7*) target=armv7-linux-androideabi ;;
    x86_64) target=x86_64-linux-android ;;
    *) echo "unsupported Termux architecture: $machine" >&2; exit 1 ;;
  esac
fi

say "Detected $sys/$machine: installing the $variant build for $target"
asset="jiotv-${variant}-${target}"
if [ "$version" = latest ]; then
  base="https://github.com/${repo}/releases/latest/download"
else
  case "$version" in v*) tag=$version ;; *) tag="v${version}" ;; esac
  base="https://github.com/${repo}/releases/download/${tag}"
fi
say "Downloading $asset"
download "$base/$asset" "$tmp/$asset"
note "$(size_kib "$tmp/$asset") KiB"
download "$base/SHA256SUMS" "$tmp/SHA256SUMS"
verify_asset "$asset" "$tmp/SHA256SUMS" "$tmp/$asset"
say "Checksum verified (SHA-256)"

if [ -n "${JIOTV_INSTALL_DIR:-}" ]; then install_dir=$JIOTV_INSTALL_DIR
elif [ "$termux" = true ]; then install_dir=${PREFIX}/bin
elif [ "$(id -u 2>/dev/null || echo 1)" = 0 ] && [ -w /usr/local/bin ]; then install_dir=/usr/local/bin
else install_dir=${HOME:-}/.local/bin
fi
[ -n "$install_dir" ] || { echo "HOME is unset; set JIOTV_INSTALL_DIR" >&2; exit 1; }
mkdir -p "$install_dir"
if command -v install >/dev/null 2>&1; then install -m 0755 "$tmp/$asset" "$install_dir/jiotv"
else cp "$tmp/$asset" "$install_dir/jiotv" && chmod 0755 "$install_dir/jiotv"
fi
echo "Installed jiotv ($variant, $target) to $install_dir/jiotv"
case ":${PATH:-}:" in *":$install_dir:"*) ;; *) echo "Add $install_dir to PATH to run jiotv directly." ;; esac
if [ "$install_tls" = 1 ]; then
  echo "Next: jiotv login otp; jiotv admin password; jiotv serve --host 0.0.0.0 --tls"
  echo "Browser UI (HTTPS, self-signed certificate; accept the one-time warning): https://<host>:5443/"
  echo "IPTV apps (plain HTTP playlist): http://<host>:5001/"
else
  echo "Next: jiotv login otp; jiotv admin password; jiotv serve --host 0.0.0.0"
  echo "Browser UI: http://<host>:5001/ (browsers need HTTPS or localhost for DRM and encrypted HLS playback; add --tls to enable HTTPS)"
fi
