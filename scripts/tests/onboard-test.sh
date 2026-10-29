#!/bin/sh
# Drives scripts/install.sh through a pseudo-terminal so the onboarding flow is
# exercised the way a user sees it. Every prompt reads /dev/tty, so a pty is
# the only way to test it without a human at the keyboard. Keystrokes are only
# sent once the matching prompt has been rendered, which keeps arrow-key
# selection deterministic.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$tmp/bin"

cat >"$tmp/bin/uname" <<'EOF'
#!/bin/sh
case "$1" in
  -s) printf '%s\n' "Darwin" ;;
  -m) printf '%s\n' "arm64" ;;
esac
EOF
chmod +x "$tmp/bin/uname"

# The installer downloads a release asset; here it is a script that behaves
# like `jiotv` so the sign-in dialogs can be driven end to end.
cat >"$tmp/stub" <<'EOF'
#!/bin/sh
case "${1:-}" in
  --help)
    echo "USAGE:"
    echo "  jiotv login otp"
    echo "  jiotv tv+ login | tv+ logout"
    echo "  jiotv fastway login"
    echo "  jiotv admin password"
    ;;
  login)
    printf 'Enter your mobile number: +91 '
    read number
    printf 'Sending OTP to your mobile number\n'
    printf 'OTP sent to your mobile number\n'
    printf 'Enter OTP: '
    read otp
    [ -n "$otp" ] && echo "Login successful"
    ;;
  extras|tv+)
    printf 'Mobile number registered to the fibre account: +91 '
    read number
    if [ "${JIOTV_STUB_NO_CONNECTIONS:-0}" = 1 ]; then
      printf 'OTP: '
    else
      printf 'Connections on this number:\n'
      printf '  1. Home, JioFiber, line ending 1234\n'
      printf '  2. Office, JioFiber, line ending 5678\n'
      printf 'Choose a connection: '
      read pick
      printf 'OTP: '
    fi
    read otp
    echo "JioTV+ login saved. Restart the server to use it."
    ;;
  admin)
    printf 'New admin password: '
    read pass
    [ -n "$pass" ]
    ;;
  autostart)
    echo "service installation unavailable" >&2
    exit 1
    ;;
  key)
    echo "/k/secret-key/playlist.m3u"
    ;;
esac
EOF
chmod +x "$tmp/stub"

if command -v sha256sum >/dev/null 2>&1; then hash_cmd=sha256sum; else hash_cmd='shasum -a 256'; fi

cat >"$tmp/bin/curl" <<EOF
#!/bin/sh
out=
url=
while [ "\$#" -gt 0 ]; do
  case "\$1" in
    -o) out=\$2; shift 2 ;;
    -*) shift ;;
    *) url=\$1; shift ;;
  esac
done
hash() { $hash_cmd "\$1" | awk '{print \$1}'; }
case "\$url" in
  */SHA256SUMS)
    printf '%s  %s\n' "\$(hash "$tmp/stub")" "\$(cat "$tmp/asset-name")" >"\$out"
    ;;
  *)
    cp "$tmp/stub" "\$out"
    basename "\$url" >"$tmp/asset-name"
    ;;
esac
EOF
chmod +x "$tmp/bin/curl"

# Waits until the prompt has been drawn before typing at it, so arrow keys are
# read in raw mode instead of piling up in the terminal's line buffer.
wait_for() {
  n=0
  while [ "$n" -lt 120 ]; do
    grep -qF "$1" "$2" 2>/dev/null && return 0
    sleep 0.2
    n=$((n + 1))
  done
  echo "timed out waiting for: $1" >&2
  return 1
}

start_flow() {
  flow_dir=$1
  flow_out=$2
  rm -f "$tmp/in"
  mkfifo "$tmp/in"
  # The installer needs a terminal, so it runs under `script`. macOS ships BSD
  # script, which rejects a FIFO on stdin but accepts a pipe, so `cat` turns the
  # FIFO into one. util-linux script takes the command with -c instead.
  if script --version 2>/dev/null | grep -qi 'util-linux'; then
    JIOTV_INSTALL_DIR="$flow_dir" HOME="$flow_dir/home" XDG_CONFIG_HOME="$flow_dir/home/.config" JIOTV_TUNNEL="${JIOTV_TEST_TUNNEL:-}" JIOTV_TUNNEL_TOKEN="${JIOTV_TEST_TUNNEL_TOKEN:-}" PATH="$tmp/bin:$PATH" \
      sh -c 'cat "$1" | script -q -c "sh $2" /dev/null >"$3" 2>&1' _ "$tmp/in" "$root/scripts/install.sh" "$flow_out" &
  else
    JIOTV_INSTALL_DIR="$flow_dir" HOME="$flow_dir/home" XDG_CONFIG_HOME="$flow_dir/home/.config" JIOTV_TUNNEL="${JIOTV_TEST_TUNNEL:-}" JIOTV_TUNNEL_TOKEN="${JIOTV_TEST_TUNNEL_TOKEN:-}" PATH="$tmp/bin:$PATH" \
      sh -c 'cat "$1" | script -q /dev/null sh "$2" >"$3" 2>&1' _ "$tmp/in" "$root/scripts/install.sh" "$flow_out" &
  fi
  flow_pid=$!
  exec 6>"$tmp/in"
}
send() { printf '%s' "$1" >&6; }
enter() { printf '\r' >&6; }
finish_flow() { exec 6>&-; wait "$flow_pid" || true; }

# ---- default path: full build, JioTV account ------------------------------
start_flow "$tmp/install" "$tmp/out"
wait_for "Which build?" "$tmp/out"; enter
wait_for "Which account are you setting up?" "$tmp/out"; enter
wait_for "This machine's address" "$tmp/out"; send '192.168.1.9'; enter
wait_for "Open JioTV up beyond your LAN?" "$tmp/out"; enter
wait_for "Mobile number registered with JioTV" "$tmp/out"; send '9999999999'; enter
wait_for "Enter the OTP" "$tmp/out"; send '123456'; enter
wait_for "Choose an admin password" "$tmp/out"; send 'secret'; enter
wait_for "Installation complete" "$tmp/out"
finish_flow

grep -F "Signed in to JioTV" "$tmp/out" >/dev/null
grep -F "http://192.168.1.9:5001/k/secret-key/playlist.m3u" "$tmp/out" >/dev/null
test -x "$tmp/install/jiotv"
if [ "${JIOTV_TEST_SHOW:-0}" = 1 ]; then sed 's/\r$//' "$tmp/out"; fi
echo "onboarding default flow OK"

# ---- arrow keys: slim build, JioTV+ with a connection picker --------------
start_flow "$tmp/install-slim" "$tmp/out2"
wait_for "Which build?" "$tmp/out2"; send "$(printf '\033[B')"; enter
wait_for "Which account are you setting up?" "$tmp/out2"; send "$(printf '\033[B')"; enter
wait_for "This machine's address" "$tmp/out2"; send '192.168.1.9'; enter
wait_for "Open JioTV up beyond your LAN?" "$tmp/out2"; enter
wait_for "Mobile number registered to the fibre account" "$tmp/out2"; send '9999999999'; enter
wait_for "Choose the connection" "$tmp/out2"; send '2'; enter
wait_for "OTP" "$tmp/out2"; send '123456'; enter
wait_for "Installation complete" "$tmp/out2"
finish_flow

grep -F "Office, JioFiber" "$tmp/out2" >/dev/null
grep -F "Signed in to JioTV+" "$tmp/out2" >/dev/null
grep -F "http://192.168.1.9:5001/k/secret-key/playlist.m3u" "$tmp/out2" >/dev/null
grep -F "The slim build has no browser interface" "$tmp/out2" >/dev/null
if grep -F "Choose an admin password" "$tmp/out2" >/dev/null; then
  echo "a slim install asked for an admin password" >&2
  exit 1
fi
echo "onboarding arrow keys and JioTV+ flow OK"

# ---- JioTV+ with nothing to pick: must not wait for a picker --------------
JIOTV_STUB_NO_CONNECTIONS=1
JIOTV_TEST_TUNNEL_TOKEN="eyJdummy'quoted"
export JIOTV_STUB_NO_CONNECTIONS JIOTV_TEST_TUNNEL_TOKEN
start_flow "$tmp/install-noconn" "$tmp/out3"
wait_for "Which build?" "$tmp/out3"; enter
wait_for "Which account are you setting up?" "$tmp/out3"; send "$(printf '\033[B')"; enter
wait_for "This machine's address" "$tmp/out3"; send '192.168.1.9'; enter
wait_for "Open JioTV up beyond your LAN?" "$tmp/out3"; send "$(printf '\033[B\033[B')"; enter
wait_for "Mobile number registered to the fibre account" "$tmp/out3"; send '9999999999'; enter
wait_for "Enter the OTP" "$tmp/out3"; send '123456'; enter
wait_for "Choose an admin password" "$tmp/out3"; send 'secret'; enter
wait_for "Installation complete" "$tmp/out3"
finish_flow
unset JIOTV_STUB_NO_CONNECTIONS
unset JIOTV_TEST_TUNNEL_TOKEN
if [ "${JIOTV_TEST_SHOW:-0}" = 1 ]; then sed 's/\r$//' "$tmp/out3"; fi

grep -F "Signed in to JioTV+" "$tmp/out3" >/dev/null
env_file="$tmp/install-noconn/home/.config/jiotv/installer.env"
grep -F "export JIOTV_TVPLUS='true'" "$env_file" >/dev/null
grep -F "export JIOTV_TUNNEL_TOKEN='eyJdummy'\\''quoted'" "$env_file" >/dev/null
env -i PATH="$PATH" sh -c '. "$1"; test "$JIOTV_TUNNEL_TOKEN" = "$2" && test "$JIOTV_TVPLUS" = true' _ "$env_file" "eyJdummy'quoted"
if grep -F "eyJdummy'quoted" "$tmp/out3" >/dev/null; then
  echo "the tunnel token was printed in installer output" >&2
  exit 1
fi
env_mode=$(stat -c '%a' "$env_file" 2>/dev/null || stat -f '%Lp' "$env_file")
[ "$env_mode" = 600 ]
grep -F "$env_file" "$tmp/out3" >/dev/null
grep -F "jiotv serve --host 0.0.0.0 --tls --tunnel" "$tmp/out3" >/dev/null
if grep -F "Choose the connection" "$tmp/out3" >/dev/null; then
  echo "a connection picker appeared without any connections" >&2
  exit 1
fi
echo "onboarding JioTV+ without a picker OK"

echo "onboarding persists provider/tunnel settings and prints keyed playlist URL OK"

# ---- declining an update must leave the installed binary untouched --------
mkdir -p "$tmp/install-existing"
printf 'existing executable\n' >"$tmp/install-existing/jiotv"
chmod +x "$tmp/install-existing/jiotv"
start_flow "$tmp/install-existing" "$tmp/out4"
wait_for "Which build?" "$tmp/out4"; enter
wait_for "Update it in place and keep the current account?" "$tmp/out4"
send "$(printf '\033[B')"; enter
wait_for "Left as it was" "$tmp/out4"
finish_flow
grep -F "existing executable" "$tmp/install-existing/jiotv" >/dev/null
if grep -F "Installation complete" "$tmp/out4" >/dev/null; then
  echo "declining an update continued onboarding" >&2
  exit 1
fi
echo "declining an update leaves the installed binary untouched OK"

# ---- no terminal: the plain install must not prompt ----------------------
JIOTV_INSTALL_DIR="$tmp/install-plain" PATH="$tmp/bin:$PATH" \
  JIOTV_ONBOARD=always sh "$root/scripts/install.sh" >"$tmp/plain" 2>&1
if grep -F "Which build?" "$tmp/plain" >/dev/null; then
  echo "the installer prompted without a terminal" >&2
  exit 1
fi
grep -F "Installed jiotv (full, aarch64-apple-darwin)" "$tmp/plain" >/dev/null
echo "onboarding stays off without a terminal OK"
