#!/bin/sh
set -eu

if [ "$#" -ne 5 ]; then
  echo "usage: $0 FORMAT TARGET SDK_VERSION BINARY_ARCH APP_VERSION" >&2
  exit 2
fi
format=$1
target=$2
sdk_version=$3
arch=$4
app_version=$5
case "$format" in opkg|apk) ;; *) echo "invalid package format: $format" >&2; exit 2 ;; esac

index="https://downloads.openwrt.org/releases/${sdk_version}/targets/${target}/"
listing=$(curl -fsSL "$index")
archive=$(printf '%s' "$listing" | grep -Eo 'openwrt-sdk-[^" ]+\.tar\.(zst|xz)' | head -n 1 || true)
[ -n "$archive" ] || { echo "no SDK archive found at $index" >&2; exit 1; }
mkdir -p /tmp/jiotv-sdk
curl -fsSL "${index}${archive}" -o /tmp/jiotv-sdk/sdk.tar
case "$archive" in
  *.tar.zst) tar --zstd -xf /tmp/jiotv-sdk/sdk.tar -C /tmp/jiotv-sdk ;;
  *.tar.xz) tar -xJf /tmp/jiotv-sdk/sdk.tar -C /tmp/jiotv-sdk ;;
esac
sdk=$(find /tmp/jiotv-sdk -mindepth 1 -maxdepth 1 -type d | head -n1)

case "$arch" in
  x86_64) triple=x86_64-unknown-linux-musl ;;
  aarch64) triple=aarch64-unknown-linux-musl ;;
  armv7) triple=armv7-unknown-linux-musleabihf ;;
esac
scripts/build-openwrt-packages.sh "$sdk" \
  "dist/bin/jiotv-full-${triple}" "dist/bin/jiotv-slim-${triple}" "$app_version"
