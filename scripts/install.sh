#!/bin/sh
set -eu

repo=${JIOTV_REPO:-wpfyorg/better-jiotv-go}
variant=${JIOTV_VARIANT:-full}
version=${JIOTV_VERSION:-latest}
install_tls=${JIOTV_INSTALL_TLS:-1}
start_service=${JIOTV_START_SERVICE:-1}
ready_timeout=${JIOTV_READY_TIMEOUT:-15}
case "$ready_timeout" in ''|*[!0-9]*) echo "JIOTV_READY_TIMEOUT must be a number of seconds" >&2; exit 2 ;; esac
# Readiness needs two consecutive good checks, so fewer than two seconds can never succeed.
[ "$ready_timeout" -ge 2 ] || ready_timeout=2

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

# True when JioTV listens on the TCP port. When the socket listing names the
# owner it must be jiotv, so another daemon holding the port does not count;
# without owner information, or without a listing tool, the port alone decides.
port_listening() {
  if command -v netstat >/dev/null 2>&1; then listing=$(netstat -ltnp 2>/dev/null || true)
  elif command -v ss >/dev/null 2>&1; then listing=$(ss -ltnp 2>/dev/null || true)
  else return 0
  fi
  [ -n "$listing" ] || return 0
  owners=$(printf '%s\n' "$listing" | grep -E "[:.]$1[[:space:]]" || true)
  [ -n "$owners" ] || return 1
  if printf '%s\n' "$owners" | grep -Eq '[0-9]+/[^ ]+|pid='; then
    printf '%s\n' "$owners" | grep -q jiotv
  fi
}

# Process IDs of running jiotv servers (empty when pidof is unavailable).
jiotv_pids() { pidof jiotv 2>/dev/null || true; }

# Stop the service and wait until the process has really exited: procd only
# signals it on stop, so a lingering old process could otherwise be mistaken for
# a new one or overwrite state. Fails when it does not exit in time.
stop_service() {
  "$init_script" stop || return 1
  waited=0
  while [ "$waited" -lt "$ready_timeout" ]; do
    if [ -z "$(jiotv_pids)" ] && ! "$init_script" running >/dev/null 2>&1; then return 0; fi
    waited=$((waited + 1))
    sleep 1
  done
  return 1
}

# Print one uci option of the jiotv service, or the default when unavailable.
uci_opt() {
  value=$(uci -q get "jiotv.main.$1" 2>/dev/null || true)
  if [ -n "$value" ]; then echo "$value"; else echo "$2"; fi
}

# A uci boolean as 1 or 0, using the same spellings as OpenWrt's get_bool, so
# this agrees with what the init script does with the value.
uci_flag() {
  case "$(uci_opt "$1" "$2")" in
    1|on|true|yes|enabled) echo 1 ;;
    0|off|false|no|disabled) echo 0 ;;
    *) echo "$2" ;;
  esac
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

# This machine's LAN address, for other devices on the network (best effort). A
# machine with Docker, a VPN or several adapters has more than one address, and the
# default route may point into a tunnel, so tunnel and container interfaces are skipped.
local_ip() {
  addr=
  if [ "$sys" = Darwin ]; then
    ifc=$(route -n get default 2>/dev/null | awk '/interface:/ { print $2; exit }' || true)
    case "$ifc" in utun*|ppp*|ipsec*|gif*|stf*) ifc= ;; esac
    if [ -n "$ifc" ]; then addr=$(ipconfig getifaddr "$ifc" 2>/dev/null || true); fi
    if [ -z "$addr" ]; then
      for ifc in en0 en1; do
        addr=$(ipconfig getifaddr "$ifc" 2>/dev/null || true)
        [ -z "$addr" ] || break
      done
    fi
  else
    if command -v ip >/dev/null 2>&1; then
      route_line=$(ip -4 route get 1.1.1.1 2>/dev/null | head -n 1 || true)
      route_dev=$(printf '%s\n' "$route_line" | sed -n 's/.* dev \([^ ]*\).*/\1/p')
      route_src=$(printf '%s\n' "$route_line" | sed -n 's/.* src \([0-9.]*\).*/\1/p')
      case "$route_dev" in
        ''|tun*|tap*|wg*|tailscale*|ppp*|ipsec*|zt*|utun*) ;;
        *) addr=$route_src ;;
      esac
      if [ -z "$addr" ]; then
        addr=$(ip -4 -o addr show scope global 2>/dev/null | awk '$2 !~ /^(lo|docker|br-|veth|virbr|tun|tap|wg|tailscale|ppp|ipsec|zt|utun)/ { split($4, a, "/"); print a[1]; exit }' || true)
      fi
    fi
    if [ -z "$addr" ] && command -v hostname >/dev/null 2>&1; then
      # Only a dotted IPv4 address is usable unbracketed in a URL.
      addr=$(hostname -I 2>/dev/null | tr ' ' '\n' | grep -E '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' | head -n 1 || true)
    fi
  fi
  if [ -n "$addr" ]; then echo "$addr"; else echo "<this-machine-ip>"; fi
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
  init_script=${JIOTV_INIT_SCRIPT:-/etc/init.d/jiotv}
  # Remember whether the service was already running: the package's own hook
  # starts it on install, which JIOTV_START_SERVICE=0 must not leave behind.
  was_running=false
  if [ -x "$init_script" ] && "$init_script" running >/dev/null 2>&1; then was_running=true; fi
  say "Installing with $package_manager"
  if [ "$package_manager" = apk ]; then
    if apk info -e "$other_package" >/dev/null 2>&1; then apk del "$other_package"; fi
    apk add --allow-untrusted "$tmp/$asset"
  else
    if opkg status "$other_package" 2>/dev/null | grep -q '^Status: .* installed$'; then opkg remove "$other_package"; fi
    opkg install "$tmp/$asset"
  fi
  say "Enabling the service at boot"
  "$init_script" enable

  running=false
  kept_running=false
  stop_failed=false
  disabled=false
  [ "$(uci_flag enabled 1)" = 1 ] || disabled=true
  if [ "$start_service" = 0 ]; then
    if [ "$was_running" != true ]; then
      # The package hook may have started it, or left it between procd respawns where
      # "running" is briefly false, so stop unconditionally rather than only if running.
      say "Making sure the service is stopped (JIOTV_START_SERVICE=0)"
      stop_service || stop_failed=true
    fi
    # What is left running now was running before and is deliberately untouched.
    if [ "$stop_failed" != true ] && "$init_script" running >/dev/null 2>&1; then kept_running=true; fi
  elif [ "$disabled" = true ]; then
    # The setting wins over a process started earlier. Stop unconditionally: a respawning
    # instance between attempts reports "not running" but would launch again.
    say "The service is disabled in /etc/config/jiotv (option enabled '0'); making sure it is stopped"
    stop_service || stop_failed=true
  else
    # Stop and wait before starting: an upgrade must replace the old process, and
    # only a process started after the old one is gone proves the new binary runs.
    say "Starting the service"
    http_port=$(uci_opt port 5001)
    if stop_service && "$init_script" start; then
      tries=0
      stable=0
      while [ "$tries" -lt "$ready_timeout" ]; do
        if "$init_script" running >/dev/null 2>&1 && port_listening "$http_port"; then stable=$((stable + 1)); else stable=0; fi
        # Ready only once it has stayed up across two checks, not just bound the port once.
        if [ "$stable" -ge 2 ]; then running=true; break; fi
        tries=$((tries + 1))
        sleep 1
      done
      if [ "$running" = true ]; then note "listening on port $http_port"
      else echo "warning: the service is not listening on port $http_port after $ready_timeout seconds" >&2
      fi
    else
      echo "warning: could not restart the service with '$init_script'" >&2
    fi
  fi

  # The service binds the configured host; only a wildcard bind is reachable at
  # the router's LAN address, so any other host is advertised as configured.
  bind_host=$(uci_opt host 0.0.0.0)
  case "$bind_host" in
    0.0.0.0|::|'[::]'|'[::0]'|::0|0:0:0:0:0:0:0:0) ip=$(router_ip) ;;
    \[*) ip=$bind_host ;;
    *:*) ip="[$bind_host]" ;;
    *) ip=$bind_host ;;
  esac
  tls_on=$(uci_flag tls 1)
  http_port=$(uci_opt port 5001)
  tls_port=$(uci_opt tls_port 5443)
  echo
  if [ "$stop_failed" = true ]; then echo "JioTV ($variant, ${tag#v}) is installed, but the service could not be stopped: run '$init_script stop'."
  elif [ "$running" = true ]; then echo "JioTV ($variant, ${tag#v}) is installed and running."
  elif [ "$kept_running" = true ]; then echo "JioTV ($variant, ${tag#v}) is installed; the service that was already running was left as it was (JIOTV_START_SERVICE=0)."
  elif [ "$disabled" = true ]; then echo "JioTV ($variant, ${tag#v}) is installed; the service is disabled in /etc/config/jiotv."
  elif [ "$start_service" = 1 ]; then echo "JioTV ($variant, ${tag#v}) is installed, but the service is not listening yet."
  else echo "JioTV ($variant, ${tag#v}) is installed (not started: JIOTV_START_SERVICE=0)."
  fi
  echo
  if [ "$variant" = slim ]; then
    echo "  IPTV apps  : http://${ip}:${http_port}/  (plain HTTP playlist; the slim build has no browser UI)"
  elif [ "$install_tls" = 1 ] && [ "$tls_on" = 1 ]; then
    echo "  Browser UI : https://${ip}:${tls_port}/  (HTTPS, self-signed certificate; accept the one-time warning)"
    echo "  IPTV apps  : http://${ip}:${http_port}/  (plain HTTP playlist)"
  else
    echo "  Browser UI : http://${ip}:${http_port}/  (browsers need HTTPS or localhost for DRM and encrypted HLS playback)"
    [ "$install_tls" = 1 ] || echo "  HTTPS instructions are off (JIOTV_INSTALL_TLS=0); the service setting is unchanged, so HTTPS stays on unless 'option tls 0' is set in /etc/config/jiotv."
  fi
  echo
  echo "Next steps:"
  step=1
  if [ "$disabled" = true ]; then
    echo "  $step. Enable the service: uci set jiotv.main.enabled=1 && uci commit jiotv && $init_script start"
    step=$((step + 1))
  elif [ "$running" != true ] && [ "$kept_running" != true ]; then
    echo "  $step. Start the service: $init_script start  (then check: logread -e jiotv)"
    step=$((step + 1))
  fi
  if [ "$variant" = slim ]; then
    echo "  $step. Sign in to JioTV from the terminal (you enter the OTP yourself). Stop the service first so it cannot overwrite the login:"
    echo "       $init_script stop; while pidof jiotv >/dev/null; do sleep 1; done; jiotv login otp; $init_script start"
  else
    echo "  $step. Set the admin password: jiotv admin password"
    step=$((step + 1))
    echo "  $step. Open the browser UI, log in with that password, then sign in to JioTV (you enter the OTP yourself)."
    echo "  To sign in to JioTV from the terminal instead, stop the service first so it cannot overwrite the login:"
    echo "    $init_script stop; while pidof jiotv >/dev/null; do sleep 1; done; jiotv login otp; $init_script start"
  fi
  echo
  echo "Service control: $init_script start|stop|restart    Logs: logread -e jiotv"
  [ "$stop_failed" != true ] || exit 1
  [ "$start_service" = 0 ] || [ "$disabled" = true ] || [ "$running" = true ] || exit 1
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
ip=$(local_ip)
if [ "$install_tls" = 1 ]; then
  echo "Next: jiotv login otp; jiotv admin password; jiotv serve --host 0.0.0.0 --tls"
  if [ "$variant" = slim ]; then
    echo "IPTV apps (plain HTTP playlist): http://localhost:5001/ on this machine, http://${ip}:5001/ from other devices. The slim build has no browser UI."
  else
    echo "Browser UI on this machine (HTTPS, self-signed certificate; accept the one-time warning): https://localhost:5443/"
    echo "From other devices on your network: https://${ip}:5443/ (browser), http://${ip}:5001/ (IPTV apps, plain HTTP playlist). The address is detected automatically; if it does not work, use the one your network assigned to this machine."
  fi
else
  echo "Next: jiotv login otp; jiotv admin password; jiotv serve --host 0.0.0.0"
  if [ "$variant" = slim ]; then
    echo "IPTV apps (plain HTTP playlist): http://localhost:5001/ on this machine, http://${ip}:5001/ from other devices. The slim build has no browser UI."
  else
    echo "Browser UI on this machine: http://localhost:5001/ (browsers need HTTPS or localhost for DRM and encrypted HLS playback; add --tls to enable HTTPS)"
    echo "From other devices on your network: http://${ip}:5001/"
  fi
fi
