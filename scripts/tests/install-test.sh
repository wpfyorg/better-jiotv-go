#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' 0 HUP INT TERM
mkdir "$tmp/bin"

cat >"$tmp/bin/uname" <<'EOF'
#!/bin/sh
case "$1" in
  -s) printf '%s\n' "${JIOTV_TEST_OS:-Linux}" ;;
  -m) printf '%s\n' "${JIOTV_TEST_MACHINE:-x86_64}" ;;
esac
EOF
cat >"$tmp/bin/curl" <<'EOF'
#!/bin/sh
out=
url=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out=$2; shift 2 ;;
    -*) shift ;;
    *) url=$1; shift ;;
  esac
done
printf '%s\n' "$url" >>"$JIOTV_TEST_LOG"
case "$url" in
  https://api.github.com/*)
    {
      echo '{'
      echo '  "tag_name": "v1.3.1",'
      echo '  "assets": ['
      if [ -n "${JIOTV_TEST_EXTRA_ASSET_NAME:-}" ]; then
        printf '    {"name": "%s"},\n' "$JIOTV_TEST_EXTRA_ASSET_NAME"
      fi
      printf '    {"name": "%s"}\n' "$JIOTV_TEST_ASSET_NAME"
      echo '  ]'
      echo '}'
    } >"$out"
    ;;
  */SHA256SUMS)
    if [ "${JIOTV_TEST_BAD_SUM:-0}" = 1 ]; then hash=$(printf bad | sha256sum | awk '{print $1}')
    else hash=$(printf 'test binary' | sha256sum | awk '{print $1}')
    fi
    printf '%s  %s\n' "$hash" "$JIOTV_TEST_ASSET_NAME" >"$out"
    ;;
  *) printf 'test binary' >"$out" ;;
esac
EOF
chmod +x "$tmp/bin/uname" "$tmp/bin/curl"

cat >"$tmp/bin/id" <<'EOF'
#!/bin/sh
[ "${1:-}" = -u ] && { echo 0; exit 0; }
exec /usr/bin/id "$@"
EOF
chmod +x "$tmp/bin/id"

cat >"$tmp/bin/jiotv-init" <<'EOF'
#!/bin/sh
[ -z "${JIOTV_TEST_INIT_LOG:-}" ] || printf '%s\n' "${1:-}" >>"$JIOTV_TEST_INIT_LOG"
case "${1:-}" in enable|restart) ;; *) exit 1 ;; esac
EOF
chmod +x "$tmp/bin/jiotv-init"

run_case() {
  expected=$1
  os=$2
  machine=$3
  prefix=$4
  shift 4
  install_dir="$tmp/install-$expected"
  log="$tmp/log-$expected"
  JIOTV_TEST_OS=$os JIOTV_TEST_MACHINE=$machine PREFIX=$prefix \
    JIOTV_TEST_ASSET_NAME=$expected JIOTV_TEST_LOG=$log \
    JIOTV_INSTALL_DIR="$install_dir" PATH="$tmp/bin:$PATH" \
    "$@" sh "$root/scripts/install.sh" >/dev/null
  test -x "$install_dir/jiotv"
  grep -F "/$expected" "$log" >/dev/null
  printf 'installer target OK: %s\n' "$expected"
}

run_case jiotv-full-x86_64-unknown-linux-musl Linux x86_64 '' env
run_case jiotv-slim-x86_64-apple-darwin Darwin x86_64 '' env JIOTV_VARIANT=slim
run_case jiotv-full-aarch64-linux-android Linux aarch64 /data/data/com.termux/files/usr env PREFIX=/data/data/com.termux/files/usr
run_case jiotv-full-armv7-linux-androideabi Linux armv7l /data/data/com.termux/files/usr env PREFIX=/data/data/com.termux/files/usr
run_case jiotv-full-x86_64-linux-android Linux x86_64 /data/data/com.termux/files/usr env PREFIX=/data/data/com.termux/files/usr
run_case jiotv-full-x86_64-unknown-linux-musl Linux x86_64 '' env JIOTV_REPO=owner/project JIOTV_VERSION=1.2.3
grep -F 'https://github.com/owner/project/releases/download/v1.2.3/' "$tmp/log-jiotv-full-x86_64-unknown-linux-musl" >/dev/null

if JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=mips PATH="$tmp/bin:$PATH" sh "$root/scripts/install.sh" >/dev/null 2>&1; then
  echo "unsupported architecture unexpectedly installed" >&2
  exit 1
fi

if JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=x86_64 JIOTV_TEST_ASSET_NAME=jiotv-full-x86_64-unknown-linux-musl \
  JIOTV_TEST_BAD_SUM=1 JIOTV_TEST_LOG="$tmp/bad-log" JIOTV_INSTALL_DIR="$tmp/bad-install" \
  PATH="$tmp/bin:$PATH" sh "$root/scripts/install.sh" >/dev/null 2>&1; then
  echo "checksum mismatch unexpectedly installed" >&2
  exit 1
fi
test ! -e "$tmp/bad-install/jiotv"
echo "unsupported architecture and checksum rejection OK"

cat >"$tmp/bin/apk" <<'EOF'
#!/bin/sh
if [ "${1:-}" = --print-arch ]; then
  echo "${JIOTV_TEST_PACKAGE_ARCH:-x86_64}"
  exit 0
fi
printf '%s\n' "$*" >>"$JIOTV_TEST_PACKAGE_LOG"
if [ "${1:-}" = info ] && [ "${2:-}" = -e ]; then
  [ "${3:-}" = "${JIOTV_TEST_INSTALLED_PACKAGE:-}" ]
  exit
fi
[ "${1:-}" = del ] && exit 0
test -f "${3:-}"
EOF
chmod +x "$tmp/bin/apk"
openwrt_apk=jiotv-1.3.1-r1_x86_64.apk
openwrt_slim_apk=jiotv-slim-1.3.1-r1_x86_64.apk
JIOTV_PLATFORM=openwrt JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=x86_64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_apk JIOTV_TEST_EXTRA_ASSET_NAME=$openwrt_slim_apk \
  JIOTV_TEST_INSTALLED_PACKAGE=jiotv-slim JIOTV_TEST_LOG="$tmp/openwrt-apk-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-apk-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  JIOTV_TEST_INIT_LOG="$tmp/openwrt-apk-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >"$tmp/openwrt-apk-output"
grep -F "del jiotv-slim" "$tmp/openwrt-apk-package" >/dev/null
grep -x "enable" "$tmp/openwrt-apk-init" >/dev/null
grep -x "restart" "$tmp/openwrt-apk-init" >/dev/null
grep -F "is installed and running" "$tmp/openwrt-apk-output" >/dev/null
grep -F "http://<router-ip>:5001/" "$tmp/openwrt-apk-output" >/dev/null
grep -F "Checksum verified" "$tmp/openwrt-apk-output" >/dev/null
grep -F "add --allow-untrusted" "$tmp/openwrt-apk-package" >/dev/null
grep -F "/$openwrt_apk" "$tmp/openwrt-apk-downloads" >/dev/null
if grep -F "/$openwrt_slim_apk" "$tmp/openwrt-apk-downloads" >/dev/null; then
  echo "full OpenWrt install selected the slim package" >&2
  exit 1
fi
openwrt_aarch64_apk=jiotv-1.3.1-r1_aarch64_cortex-a53.apk
JIOTV_PLATFORM=openwrt JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-aarch64-apk-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-aarch64-apk-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >/dev/null
grep -F "/$openwrt_aarch64_apk" "$tmp/openwrt-aarch64-apk-downloads" >/dev/null
JIOTV_PLATFORM=openwrt JIOTV_START_SERVICE=0 JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-nostart-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-nostart-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  JIOTV_TEST_INIT_LOG="$tmp/openwrt-nostart-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >"$tmp/openwrt-nostart-output"
grep -x "enable" "$tmp/openwrt-nostart-init" >/dev/null
if grep -x "restart" "$tmp/openwrt-nostart-init" >/dev/null; then
  echo "JIOTV_START_SERVICE=0 still started the service" >&2
  exit 1
fi
grep -F "not started" "$tmp/openwrt-nostart-output" >/dev/null
if JIOTV_PLATFORM=openwrt JIOTV_TEST_PACKAGE_ARCH=aarch64_cortex-a72 \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-unsupported-package" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >/dev/null 2>&1; then
  echo "unsupported OpenWrt package ABI unexpectedly installed" >&2
  exit 1
fi
rm "$tmp/bin/apk"

cat >"$tmp/bin/opkg" <<'EOF'
#!/bin/sh
if [ "${1:-}" = print-architecture ]; then
  echo 'arch all 1'
  echo "arch ${JIOTV_TEST_PACKAGE_ARCH:-aarch64_cortex-a53} 10"
  exit 0
fi
printf '%s\n' "$*" >>"$JIOTV_TEST_PACKAGE_LOG"
if [ "${1:-}" = status ]; then
  [ "${2:-}" = "${JIOTV_TEST_INSTALLED_PACKAGE:-}" ] || exit 1
  echo 'Status: install user installed'
  exit 0
fi
[ "${1:-}" = remove ] && exit 0
test -f "${2:-}"
EOF
chmod +x "$tmp/bin/opkg"
openwrt_ipk=jiotv-slim_1.3.1-r1_aarch64_cortex-a53.ipk
JIOTV_PLATFORM=openwrt JIOTV_VARIANT=slim JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_ipk JIOTV_TEST_LOG="$tmp/openwrt-ipk-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-ipk-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >/dev/null
grep -F "install" "$tmp/openwrt-ipk-package" >/dev/null
grep -F "/$openwrt_ipk" "$tmp/openwrt-ipk-downloads" >/dev/null
rm "$tmp/bin/opkg"
echo "OpenWrt apk/opkg auto-selection OK"
