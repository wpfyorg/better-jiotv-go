#!/bin/sh
set -eu

repo=${JIOTV_REPO:-wpfyorg/better-jiotv-go}
variant=${JIOTV_VARIANT:-full}
version=${JIOTV_VERSION:-latest}

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

openwrt=false
if [ "${JIOTV_PLATFORM:-}" = openwrt ] || [ -r /etc/openwrt_release ]; then openwrt=true; fi

if [ "$openwrt" = true ]; then
  [ "$(id -u 2>/dev/null || echo 1)" = 0 ] || { echo "OpenWrt installation must be run as root" >&2; exit 1; }

  case "$machine" in
    x86_64) package_arch=x86_64 ;;
    aarch64|arm64) package_arch=aarch64_cortex-a53 ;;
    armv7l|armv7*) package_arch=arm_cortex-a7_neon-vfpv4 ;;
    *) echo "unsupported OpenWrt architecture: $machine" >&2; exit 1 ;;
  esac

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

  if [ "$version" = latest ]; then
    release_api="https://api.github.com/repos/${repo}/releases/latest"
  else
    case "$version" in v*) tag=$version ;; *) tag="v${version}" ;; esac
    release_api="https://api.github.com/repos/${repo}/releases/tags/${tag}"
  fi
  download "$release_api" "$tmp/release.json"
  tag=$(sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/release.json" | head -n 1)
  [ -n "$tag" ] || { echo "could not determine the release version" >&2; exit 1; }

  package_name=jiotv
  [ "$variant" = slim ] && package_name=jiotv-slim
  if [ "$package_ext" = apk ]; then
    pattern="^${package_name}-.*_${package_arch}\\.apk$"
  else
    pattern="^${package_name}_.*_${package_arch}\\.ipk$"
  fi
  asset=$(sed -n 's/.*"name":[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/release.json" | grep -E "$pattern" | head -n 1 || true)
  [ -n "$asset" ] || { echo "no $variant OpenWrt package found for $machine in $tag" >&2; exit 1; }

  base="https://github.com/${repo}/releases/download/${tag}"
  download "$base/$asset" "$tmp/$asset"
  download "$base/SHA256SUMS" "$tmp/SHA256SUMS"
  verify_asset "$asset" "$tmp/SHA256SUMS" "$tmp/$asset"

  if [ "$package_manager" = apk ]; then
    apk add --allow-untrusted "$tmp/$asset"
  else
    opkg install "$tmp/$asset"
  fi
  init_script=${JIOTV_INIT_SCRIPT:-/etc/init.d/jiotv}
  "$init_script" enable
  echo "Installed JioTV ($variant) for OpenWrt."
  echo "Next: jiotv login otp"
  echo "Then: jiotv admin password"
  echo "Then: /etc/init.d/jiotv start"
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

asset="jiotv-${variant}-${target}"
if [ "$version" = latest ]; then
  base="https://github.com/${repo}/releases/latest/download"
else
  case "$version" in v*) tag=$version ;; *) tag="v${version}" ;; esac
  base="https://github.com/${repo}/releases/download/${tag}"
fi
download "$base/$asset" "$tmp/$asset"
download "$base/SHA256SUMS" "$tmp/SHA256SUMS"
verify_asset "$asset" "$tmp/SHA256SUMS" "$tmp/$asset"

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
echo "Next: jiotv login otp; jiotv admin password; jiotv serve"
