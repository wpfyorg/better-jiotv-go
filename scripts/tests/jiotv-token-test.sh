#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
FIXTURE=$(mktemp -d)
trap 'rm -rf "$FIXTURE"' EXIT HUP INT TERM
mkdir -p "$FIXTURE/bin" "$FIXTURE/data"

cat > "$FIXTURE/service" <<'EOF'
#!/bin/sh
case "$1" in
  status) test -f "$MOCK_RUNNING" ;;
  stop) rm -f "$MOCK_RUNNING" ;;
  start) touch "$MOCK_RUNNING" ;;
  *) exit 1 ;;
esac
EOF
cat > "$FIXTURE/bin/uci" <<'EOF'
#!/bin/sh
case " $* " in
  *' get '*) printf '0\n' ;;
  *) exit 0 ;;
esac
EOF
for command in cp chmod mv; do
  cat > "$FIXTURE/bin/$command" <<'EOF'
#!/bin/sh
if [ "${MOCK_FAIL:-}" = "${0##*/}" ]; then
  case "$0" in
    */cp) case "$*" in *'.bak.'*) exit 1 ;; esac ;;
    */chmod|*/mv) exit 1 ;;
  esac
fi
exec "/bin/${0##*/}" "$@"
EOF
done
chmod +x "$FIXTURE/service" "$FIXTURE/bin/uci" "$FIXTURE/bin/cp" "$FIXTURE/bin/chmod" "$FIXTURE/bin/mv"

export PATH="$FIXTURE/bin:$PATH"
export MOCK_RUNNING="$FIXTURE/running"
export JIOTV_DATA_DIR="$FIXTURE/data"
export JIOTV_STORE="$FIXTURE/data/store_v4.toml"
export JIOTV_SERVICE="$FIXTURE/service"
printf 'ssoToken = "original"\n' > "$JIOTV_STORE"
printf 'extras_credentials = "replacement"\n' > "$FIXTURE/data/token-extra"

for failure in cp chmod mv; do
  touch "$MOCK_RUNNING"
  export MOCK_FAIL="$failure"
  if sh "$ROOT/scripts/jiotv-token" extra > "$FIXTURE/output" 2>&1; then
    echo "unexpected success with $failure failure" >&2
    exit 1
  fi
  test -f "$MOCK_RUNNING" || { echo "service remained stopped after $failure failure" >&2; exit 1; }
  grep -q '^ssoToken = "original"$' "$JIOTV_STORE" || { echo "store changed after $failure failure" >&2; exit 1; }
done
echo 'token-switch failure recovery OK (backup, chmod, mv)'
