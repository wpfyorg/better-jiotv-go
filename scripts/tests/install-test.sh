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
      echo '  "tag_name": "v1.3.3",'
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
# Stateful stand-in for the procd init script: JIOTV_TEST_INIT_STATE exists while "running".
state=${JIOTV_TEST_INIT_STATE:?}
[ -z "${JIOTV_TEST_INIT_LOG:-}" ] || printf '%s\n' "${1:-}" >>"$JIOTV_TEST_INIT_LOG"
case "${1:-}" in
  enable) ;;
  running) [ -f "$state" ] ;;
  start|restart) : >"$state" ;;
  stop) [ -z "${JIOTV_TEST_STOP_FAILS:-}" ] || exit 1; rm -f "$state" ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$tmp/bin/jiotv-init"

# The readiness check looks for a listening socket and the uci options.
cat >"$tmp/bin/netstat" <<'EOF'
#!/bin/sh
echo 'Active Internet connections (only servers)'
echo "tcp        0      0 0.0.0.0:5001            0.0.0.0:*               LISTEN      123/${JIOTV_TEST_PORT_OWNER:-jiotv}"
EOF
cat >"$tmp/bin/uci" <<'EOF'
#!/bin/sh
case "$*" in
  *jiotv.main.enabled) [ -z "${JIOTV_TEST_UCI_ENABLED:-}" ] || echo "$JIOTV_TEST_UCI_ENABLED" ;;
  *jiotv.main.tls) [ -z "${JIOTV_TEST_UCI_TLS:-}" ] || echo "$JIOTV_TEST_UCI_TLS" ;;
  *jiotv.main.host) [ -z "${JIOTV_TEST_UCI_HOST:-}" ] || echo "$JIOTV_TEST_UCI_HOST" ;;
  *network.lan.ipaddr) echo "${JIOTV_TEST_LAN_IP:-192.168.8.1/24}" ;;
esac
exit 0
EOF
chmod +x "$tmp/bin/netstat" "$tmp/bin/uci"

export JIOTV_TEST_INIT_STATE="$tmp/init-state"

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

# A desktop install tells the user to browse to localhost on this machine, and to
# use its own address from other devices (never a guessed one); slim has no UI.
JIOTV_TEST_OS=Darwin JIOTV_TEST_MACHINE=arm64 JIOTV_TEST_ASSET_NAME=jiotv-full-aarch64-apple-darwin \
  JIOTV_TEST_LOG="$tmp/log-desktop" JIOTV_INSTALL_DIR="$tmp/install-desktop" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >"$tmp/desktop-output"
grep -F "https://localhost:5443/" "$tmp/desktop-output" >/dev/null
grep -F "use this machine's address instead of localhost: https://<this-machine-ip>:5443/" "$tmp/desktop-output" >/dev/null
if grep -F "<host>" "$tmp/desktop-output" >/dev/null; then
  echo "the installer still prints the <host> placeholder" >&2
  exit 1
fi
JIOTV_TEST_OS=Darwin JIOTV_TEST_MACHINE=arm64 JIOTV_VARIANT=slim JIOTV_TEST_ASSET_NAME=jiotv-slim-aarch64-apple-darwin \
  JIOTV_TEST_LOG="$tmp/log-desktop-slim" JIOTV_INSTALL_DIR="$tmp/install-desktop-slim" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >"$tmp/desktop-slim-output"
grep -F "has no browser UI" "$tmp/desktop-slim-output" >/dev/null
if grep -F "Browser UI" "$tmp/desktop-slim-output" >/dev/null; then
  echo "a slim desktop install printed a browser UI address" >&2
  exit 1
fi
JIOTV_TEST_OS=Darwin JIOTV_TEST_MACHINE=arm64 JIOTV_INSTALL_TLS=0 JIOTV_TEST_ASSET_NAME=jiotv-full-aarch64-apple-darwin \
  JIOTV_TEST_LOG="$tmp/log-desktop-plain" JIOTV_INSTALL_DIR="$tmp/install-desktop-plain" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >"$tmp/desktop-plain-output"
grep -F "http://localhost:5001/" "$tmp/desktop-plain-output" >/dev/null

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
[ "${1:-}" != add ] || [ -z "${JIOTV_TEST_POSTINST_STARTS:-}" ] || : >"$JIOTV_TEST_INIT_STATE"
if [ "${1:-}" = info ] && [ "${2:-}" = -e ]; then
  [ "${3:-}" = "${JIOTV_TEST_INSTALLED_PACKAGE:-}" ]
  exit
fi
[ "${1:-}" = del ] && exit 0
test -f "${3:-}"
EOF
chmod +x "$tmp/bin/apk"
openwrt_apk=jiotv-1.3.3-r1_x86_64.apk
openwrt_slim_apk=jiotv-slim-1.3.3-r1_x86_64.apk
JIOTV_PLATFORM=openwrt JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=x86_64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_apk JIOTV_TEST_EXTRA_ASSET_NAME=$openwrt_slim_apk \
  JIOTV_TEST_INSTALLED_PACKAGE=jiotv-slim JIOTV_TEST_LOG="$tmp/openwrt-apk-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-apk-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  JIOTV_TEST_INIT_LOG="$tmp/openwrt-apk-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >"$tmp/openwrt-apk-output"
grep -F "del jiotv-slim" "$tmp/openwrt-apk-package" >/dev/null
grep -x "enable" "$tmp/openwrt-apk-init" >/dev/null
grep -x "start" "$tmp/openwrt-apk-init" >/dev/null
grep -F "is installed and running" "$tmp/openwrt-apk-output" >/dev/null
grep -F "http://192.168.8.1:5001/" "$tmp/openwrt-apk-output" >/dev/null
grep -F "Checksum verified" "$tmp/openwrt-apk-output" >/dev/null
grep -F "add --allow-untrusted" "$tmp/openwrt-apk-package" >/dev/null
grep -F "/$openwrt_apk" "$tmp/openwrt-apk-downloads" >/dev/null
if grep -F "/$openwrt_slim_apk" "$tmp/openwrt-apk-downloads" >/dev/null; then
  echo "full OpenWrt install selected the slim package" >&2
  exit 1
fi
openwrt_aarch64_apk=jiotv-1.3.3-r1_aarch64_cortex-a53.apk
JIOTV_PLATFORM=openwrt JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-aarch64-apk-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-aarch64-apk-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >/dev/null
grep -F "/$openwrt_aarch64_apk" "$tmp/openwrt-aarch64-apk-downloads" >/dev/null
# The package hook starts the service on install; JIOTV_START_SERVICE=0 must not leave it running.
rm -f "$JIOTV_TEST_INIT_STATE"
JIOTV_PLATFORM=openwrt JIOTV_START_SERVICE=0 JIOTV_TEST_POSTINST_STARTS=1 JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-nostart-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-nostart-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  JIOTV_TEST_INIT_LOG="$tmp/openwrt-nostart-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >"$tmp/openwrt-nostart-output"
grep -x "enable" "$tmp/openwrt-nostart-init" >/dev/null
grep -x "stop" "$tmp/openwrt-nostart-init" >/dev/null
if grep -x "start" "$tmp/openwrt-nostart-init" >/dev/null || [ -e "$JIOTV_TEST_INIT_STATE" ]; then
  echo "JIOTV_START_SERVICE=0 still started the service" >&2
  exit 1
fi
# A package that registered a respawning service which is momentarily "not running" is stopped too.
rm -f "$JIOTV_TEST_INIT_STATE"
JIOTV_PLATFORM=openwrt JIOTV_START_SERVICE=0 JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-respawn-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-respawn-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  JIOTV_TEST_INIT_LOG="$tmp/openwrt-respawn-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >/dev/null
grep -x "stop" "$tmp/openwrt-respawn-init" >/dev/null
# If the package-started service cannot be stopped the install must fail, not claim success.
rm -f "$JIOTV_TEST_INIT_STATE"
if JIOTV_PLATFORM=openwrt JIOTV_START_SERVICE=0 JIOTV_TEST_POSTINST_STARTS=1 JIOTV_TEST_STOP_FAILS=1 JIOTV_READY_TIMEOUT=1 JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-stopfail-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-stopfail-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  PATH="$tmp/bin:$PATH" sh "$root/scripts/install.sh" >"$tmp/openwrt-stopfail-output" 2>/dev/null; then
  echo "a service that could not be stopped was reported as installed successfully" >&2
  exit 1
fi
grep -F "could not be stopped" "$tmp/openwrt-stopfail-output" >/dev/null
# A service that was already running is left as it was.
: >"$JIOTV_TEST_INIT_STATE"
JIOTV_PLATFORM=openwrt JIOTV_START_SERVICE=0 JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-keep-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-keep-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  JIOTV_TEST_INIT_LOG="$tmp/openwrt-keep-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >"$tmp/openwrt-keep-output"
[ -e "$JIOTV_TEST_INIT_STATE" ]
grep -F "was already running was left as it was" "$tmp/openwrt-keep-output" >/dev/null
if grep -F "Start the service" "$tmp/openwrt-keep-output" >/dev/null; then
  echo "a preserved running service was reported as stopped" >&2
  exit 1
fi
if grep -x "stop" "$tmp/openwrt-keep-init" >/dev/null; then
  echo "an already running service was stopped by JIOTV_START_SERVICE=0" >&2
  exit 1
fi
grep -F "not started" "$tmp/openwrt-nostart-output" >/dev/null
# An upgrade stops the running service, waits for it, and only then starts it again.
: >"$JIOTV_TEST_INIT_STATE"
JIOTV_PLATFORM=openwrt JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-upgrade-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-upgrade-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  JIOTV_TEST_INIT_LOG="$tmp/openwrt-upgrade-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >/dev/null
awk '$1 == "stop" && !s { s = NR } $1 == "start" && !g { g = NR } END { exit !(s && g && s < g) }' "$tmp/openwrt-upgrade-init"
# A service disabled in /etc/config/jiotv stays stopped (a live one is stopped) and is not an install failure.
: >"$JIOTV_TEST_INIT_STATE"
JIOTV_PLATFORM=openwrt JIOTV_TEST_UCI_ENABLED=off JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-disabled-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-disabled-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  JIOTV_TEST_INIT_LOG="$tmp/openwrt-disabled-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >"$tmp/openwrt-disabled-output"
if grep -x "start" "$tmp/openwrt-disabled-init" >/dev/null; then
  echo "a disabled service was started" >&2
  exit 1
fi
grep -F "disabled in /etc/config/jiotv" "$tmp/openwrt-disabled-output" >/dev/null
grep -x "stop" "$tmp/openwrt-disabled-init" >/dev/null
[ ! -e "$JIOTV_TEST_INIT_STATE" ]
# Another daemon holding the port is not JioTV being ready.
if JIOTV_PLATFORM=openwrt JIOTV_TEST_PORT_OWNER=uhttpd JIOTV_READY_TIMEOUT=1 JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-owner-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-owner-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  PATH="$tmp/bin:$PATH" sh "$root/scripts/install.sh" >"$tmp/openwrt-owner-output" 2>/dev/null; then
  echo "a port held by another daemon was reported as JioTV running" >&2
  exit 1
fi
grep -F "not listening yet" "$tmp/openwrt-owner-output" >/dev/null
# The enable hint must also start the service.
grep -F "uci commit jiotv && " "$tmp/openwrt-disabled-output" | grep -F "start" >/dev/null
# Every uci spelling of false turns the HTTPS address off, as the init script does.
JIOTV_PLATFORM=openwrt JIOTV_TEST_UCI_TLS=off JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-notls-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-notls-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  PATH="$tmp/bin:$PATH" sh "$root/scripts/install.sh" >"$tmp/openwrt-notls-output"
if grep -F "https://" "$tmp/openwrt-notls-output" >/dev/null; then
  echo "HTTPS address printed although tls is off" >&2
  exit 1
fi
grep -F "Browser UI : http://" "$tmp/openwrt-notls-output" >/dev/null
# Every IPv6 wildcard spelling advertises the router address, and a one-second timeout still succeeds.
JIOTV_PLATFORM=openwrt JIOTV_TEST_UCI_HOST='[::0]' JIOTV_READY_TIMEOUT=1 JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-wild-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-wild-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  PATH="$tmp/bin:$PATH" sh "$root/scripts/install.sh" >"$tmp/openwrt-wild-output"
grep -F "https://192.168.8.1:5443/" "$tmp/openwrt-wild-output" >/dev/null
grep -F "is installed and running" "$tmp/openwrt-wild-output" >/dev/null
# A service bound to one address advertises that address, a wildcard bind the LAN address.
JIOTV_PLATFORM=openwrt JIOTV_TEST_UCI_HOST=127.0.0.1 JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-host-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-host-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  PATH="$tmp/bin:$PATH" sh "$root/scripts/install.sh" >"$tmp/openwrt-host-output"
grep -F "https://127.0.0.1:5443/" "$tmp/openwrt-host-output" >/dev/null
if grep -F "192.168.8.1" "$tmp/openwrt-host-output" >/dev/null; then
  echo "a loopback bind advertised the LAN address" >&2
  exit 1
fi
# A slim install has no browser UI, so its steps are terminal-only.
openwrt_slim_aarch64_apk=jiotv-slim-1.3.3-r1_aarch64_cortex-a53.apk
JIOTV_PLATFORM=openwrt JIOTV_VARIANT=slim JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 JIOTV_TEST_PACKAGE_ARCH=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_slim_aarch64_apk JIOTV_TEST_LOG="$tmp/openwrt-slim-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-slim-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" \
  PATH="$tmp/bin:$PATH" sh "$root/scripts/install.sh" >"$tmp/openwrt-slim-output"
if grep -E "Browser UI|browser UI|admin password" "$tmp/openwrt-slim-output" | grep -v "no browser UI" >/dev/null; then
  echo "slim install printed browser steps" >&2
  exit 1
fi
grep -F "stop; while pidof jiotv >/dev/null; do sleep 1; done; jiotv login otp" "$tmp/openwrt-slim-output" >/dev/null
# The admin password comes before signing in, and terminal login stops the service.
grep -n "Set the admin password" "$tmp/openwrt-apk-output" | grep -F "1." >/dev/null
grep -F "stop; while pidof jiotv >/dev/null; do sleep 1; done; jiotv login otp" "$tmp/openwrt-apk-output" >/dev/null
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
openwrt_ipk=jiotv-slim_1.3.3-r1_aarch64_cortex-a53.ipk
JIOTV_PLATFORM=openwrt JIOTV_VARIANT=slim JIOTV_TEST_OS=Linux JIOTV_TEST_MACHINE=aarch64 \
  JIOTV_TEST_ASSET_NAME=$openwrt_ipk JIOTV_TEST_LOG="$tmp/openwrt-ipk-downloads" \
  JIOTV_TEST_PACKAGE_LOG="$tmp/openwrt-ipk-package" JIOTV_INIT_SCRIPT="$tmp/bin/jiotv-init" PATH="$tmp/bin:$PATH" \
  sh "$root/scripts/install.sh" >/dev/null
grep -F "install" "$tmp/openwrt-ipk-package" >/dev/null
grep -F "/$openwrt_ipk" "$tmp/openwrt-ipk-downloads" >/dev/null
rm "$tmp/bin/opkg"
echo "OpenWrt apk/opkg auto-selection OK"
