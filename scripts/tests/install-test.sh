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
