#!/bin/sh
set -eu

repo=${JIOTV_REPO:-wpfyorg/better-jiotv-go}
variant=${JIOTV_VARIANT:-full}
variant_explicit=0
[ -n "${JIOTV_VARIANT:-}" ] && variant_explicit=1
version=${JIOTV_VERSION:-latest}
install_tls=${JIOTV_INSTALL_TLS:-1}
start_service=${JIOTV_START_SERVICE:-1}
ready_timeout=${JIOTV_READY_TIMEOUT:-15}
# Onboarding: auto asks when a terminal is available, always forces it, never
# keeps the plain non-interactive install. Piped, redirected and CI runs get
# the old behaviour because they have no usable terminal.
onboard=${JIOTV_ONBOARD:-auto}
provider=${JIOTV_PROVIDER:-}
public_host=${JIOTV_PUBLIC_HOST:-}
tunnel=${JIOTV_TUNNEL:-}
tunnel_token=${JIOTV_TUNNEL_TOKEN:-}
case "$ready_timeout" in ''|*[!0-9]*) echo "JIOTV_READY_TIMEOUT must be a number of seconds" >&2; exit 2 ;; esac
# Readiness needs two consecutive good checks, so fewer than two seconds can never succeed.
[ "$ready_timeout" -ge 2 ] || ready_timeout=2

case "$install_tls" in 0|1) ;; *) echo "JIOTV_INSTALL_TLS must be 0 or 1" >&2; exit 2 ;; esac
case "$start_service" in 0|1) ;; *) echo "JIOTV_START_SERVICE must be 0 or 1" >&2; exit 2 ;; esac

case "$variant" in full|slim) ;; *) echo "JIOTV_VARIANT must be full or slim" >&2; exit 2 ;; esac
case "$repo" in */*) ;; *) echo "JIOTV_REPO must be owner/repository" >&2; exit 2 ;; esac
case "$onboard" in auto|always|never) ;; *) echo "JIOTV_ONBOARD must be auto, always or never" >&2; exit 2 ;; esac
case "$provider" in tv+|tvplus) provider=extras ;; esac
case "$provider" in ''|jiotv|extras|fastway) ;; *) echo "JIOTV_PROVIDER must be jiotv, tv+ or fastway" >&2; exit 2 ;; esac
case "$tunnel" in ''|none|temp|named) ;; *) echo "JIOTV_TUNNEL must be none, temp or named" >&2; exit 2 ;; esac

sys=$(uname -s 2>/dev/null || echo unknown)
machine=$(uname -m 2>/dev/null || echo unknown)
termux=false
case "${PREFIX:-}" in *com.termux*) termux=true ;; esac
case "$sys" in Android) termux=true ;; esac

tmp=${TMPDIR:-/tmp}/jiotv-install-$$
mkdir -m 700 "$tmp"

# Restores the terminal and removes the working directory on every exit path,
# including a half-entered prompt.
ui_cleanup() {
  if [ -n "${ob_stopped_service:-}" ]; then ob_restart_existing_service >/dev/null 2>&1 || true; fi
  ui_raw_leave 2>/dev/null || true
  [ -z "${ob_fifo:-}" ] || exec 5>&- 2>/dev/null || true
  rm -rf "${tmp:-}" 2>/dev/null || true
}
trap 'ui_cleanup' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

say() { printf '%s==>%s %s\n' "$ui_grey" "$ui_reset" "$*"; }
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

# ---------------------------------------------------------------------------
# Terminal UI
#
# The onboarding flow is additive: every function below is only reached after
# `onboard_enabled` says there is a terminal and stdin is not the `curl | sh`
# pipe. Colours are plain ANSI so a shell script can do this without a Rust
# dependency, and every prompt reads from /dev/tty rather than stdin.
ui_esc=$(printf '\033')
ui_color=0
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-}" != dumb ]; then ui_color=1; fi
case "${FORCE_COLOR:-}" in 1) ui_color=1 ;; 0) ui_color=0 ;; esac
if [ "$ui_color" = 1 ]; then
  ui_bold=$(printf '\033[1m')
  ui_dim=$(printf '\033[2m')
  ui_red=$(printf '\033[31m')
  ui_green=$(printf '\033[32m')
  ui_yellow=$(printf '\033[33m')
  ui_cyan=$(printf '\033[36m')
  ui_grey=$(printf '\033[90m')
  ui_reset=$(printf '\033[0m')
else
  ui_bold=''
  ui_dim=''
  ui_red=''
  ui_green=''
  ui_yellow=''
  ui_cyan=''
  ui_grey=''
  ui_reset=''
fi

ui_stty_saved=
ui_raw_enter() {
  ui_stty_saved=$(stty -g </dev/tty 2>/dev/null) || { ui_stty_saved=; return 1; }
  [ -n "$ui_stty_saved" ] || return 1
  stty -icanon -echo min 1 time 0 </dev/tty 2>/dev/null && return 0
  stty raw -echo </dev/tty 2>/dev/null && return 0
  return 1
}
ui_raw_leave() {
  [ -z "${ui_stty_saved:-}" ] && return 0
  stty "$ui_stty_saved" </dev/tty 2>/dev/null || true
  ui_stty_saved=
}
ui_byte() { LC_ALL=C dd if=/dev/tty bs=1 count=1 2>/dev/null || true; }

ui_rule() { printf '%s  ────────────────────────────────────────────%s\n' "$ui_grey" "$ui_reset"; }
ui_ok() { printf '  %s✔%s %s\n' "$ui_green" "$ui_reset" "$*"; }
ui_warn() { printf '  %s!%s %s\n' "$ui_yellow" "$ui_reset" "$*"; }
ui_note() { printf '  %s%s%s\n' "$ui_grey" "$*" "$ui_reset"; }
ui_fail() { printf '  %s✘%s %s\n' "$ui_red" "$ui_reset" "$*"; }
ui_step() { printf '\n  %s%s◆%s %s\n\n' "$ui_bold" "$ui_cyan" "$ui_reset" "$*"; }

ui_banner() {
  printf '\n  %s%sJioTV%s %s%s installer%s\n' "$ui_bold" "$ui_cyan" "$ui_reset" "$ui_grey" "$1" "$ui_reset"
  ui_rule
}

# Draws the current menu into globals so a redraw never re-runs the prompt.
ui_menu_draw() {
  ui_i=1
  while [ "$ui_i" -le "$ui_n" ]; do
    ui_lbl=$(printf '%s' "$ui_labels" | awk -F'|' -v k="$ui_i" '{print $k}')
    if [ "$ui_i" -eq "$ui_idx" ]; then
      printf '  %s❯%s %s%s%s\n' "$ui_cyan" "$ui_reset" "$ui_bold" "$ui_lbl" "$ui_reset"
    else
      printf '    %s\n' "$ui_lbl"
    fi
    ui_i=$((ui_i + 1))
  done
}
ui_menu_redraw() {
  printf '%s[%dA' "$ui_esc" "$ui_n"
  ui_menu_draw
}

# Arrow keys, j/k, digits and Enter; Esc is deliberately not a cancel key
# because a bare escape byte cannot be told apart from the start of an arrow
# sequence without a read timeout, which POSIX sh has no portable way to set.
ui_choose_key() {
  ui_raw_enter || return 1
  ui_cr=$(printf '\r')
  ui_lf=$(printf '\n')
  ui_etx=$(printf '\003')
  while :; do
    ui_b=$(ui_byte)
    [ -n "$ui_b" ] || break
    case "$ui_b" in
      "$ui_cr"|"$ui_lf") break ;;
      "$ui_etx") ui_raw_leave; printf '\n'; exit 130 ;;
      "$ui_esc")
        ui_b2=$(ui_byte)
        if [ "$ui_b2" = '[' ]; then
          ui_b3=$(ui_byte)
          case "$ui_b3" in
            A) [ "$ui_idx" -gt 1 ] && ui_idx=$((ui_idx - 1)) ;;
            B) [ "$ui_idx" -lt "$ui_n" ] && ui_idx=$((ui_idx + 1)) ;;
          esac
        fi
        ui_menu_redraw
        ;;
      [1-9])
        if [ "$ui_b" -le "$ui_n" ]; then ui_idx=$ui_b; ui_menu_redraw; fi
        ;;
      j|J) [ "$ui_idx" -lt "$ui_n" ] && { ui_idx=$((ui_idx + 1)); ui_menu_redraw; } ;;
      k|K) [ "$ui_idx" -gt 1 ] && { ui_idx=$((ui_idx - 1)); ui_menu_redraw; } ;;
    esac
  done
  ui_raw_leave
  return 0
}

# ui_select "question" "value|value" "label|label" -> ui_sel_value, ui_sel_label
ui_select() {
  ui_prompt=$1
  ui_values=$2
  ui_labels=$3
  ui_n=$(printf '%s' "$ui_labels" | awk -F'|' '{print NF}')
  ui_idx=${ui_sel_default:-1}
  [ "$ui_idx" -ge 1 ] && [ "$ui_idx" -le "$ui_n" ] || ui_idx=1
  printf '  %s%s%s\n\n' "$ui_cyan" "$ui_prompt" "$ui_reset"
  ui_menu_draw
  ui_drawn=1
  if ! ui_choose_key; then
    ui_drawn=0
    # No raw mode here (no stty, odd terminal): numbered entry still works.
    printf '  %s1-%d then Enter:%s ' "$ui_dim" "$ui_n" "$ui_reset"
    ui_line=
    IFS= read -r ui_line </dev/tty || ui_line=
    case "$ui_line" in
      ''|*[!0-9]*) : ;;
      *)
        if [ "$ui_line" -ge 1 ] && [ "$ui_line" -le "$ui_n" ]; then ui_idx=$ui_line; fi
        ;;
    esac
  fi
  ui_sel_value=$(printf '%s' "$ui_values" | awk -F'|' -v k="$ui_idx" '{print $k}')
  ui_sel_label=$(printf '%s' "$ui_labels" | awk -F'|' -v k="$ui_idx" '{print $k}')
  if [ "$ui_drawn" = 1 ]; then
    printf '\033[%dA\033[J' "$ui_n"
  fi
  ui_ok "$ui_sel_label"
}

# ui_input "question" "default" -> ui_result
ui_input() {
  ui_result=
  ui_default=${2:-}
  printf '  %s%s%s\n  %s❯%s ' "$ui_cyan" "$1" "$ui_reset" "$ui_cyan" "$ui_reset"
  IFS= read -r ui_result </dev/tty || ui_result=
  ui_result=$(printf '%s' "$ui_result" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')
  [ -n "$ui_result" ] || ui_result=$ui_default
  if [ -n "$ui_result" ]; then
    printf '\033[1A\033[J  %s✔%s %s %s%s%s\n' "$ui_green" "$ui_reset" "$1" "$ui_bold" "$ui_result" "$ui_reset"
  fi
}

# ui_password "question" -> ui_result, never echoed, never stored in a shell log
ui_password() {
  ui_result=
  printf '  %s%s%s\n  %s❯%s ' "$ui_cyan" "$1" "$ui_reset" "$ui_cyan" "$ui_reset"
  ui_echo_off=0
  if stty -echo </dev/tty 2>/dev/null; then ui_echo_off=1; fi
  IFS= read -r ui_result </dev/tty || ui_result=
  [ "$ui_echo_off" = 0 ] || stty echo </dev/tty 2>/dev/null || true
  printf '\n'
  if [ -n "$ui_result" ]; then
    printf '\033[1A\033[J  %s✔%s %s %s(hidden)%s\n' "$ui_green" "$ui_reset" "$1" "$ui_dim" "$ui_reset"
  fi
}

ui_confirm() {
  if [ "${2:-yes}" = yes ]; then
    ui_sel_default=1
    ui_select "$1" "yes|no" "Yes|No"
  else
    ui_sel_default=2
    ui_select "$1" "yes|no" "Yes|No"
  fi
  unset ui_sel_default
  [ "$ui_sel_value" = yes ]
}

# A terminal we can drive, and a stdin that is not the install script itself.
onboard_enabled() {
  [ "$onboard" != never ] || return 1
  [ -r /dev/tty ] || return 1
  [ -t 1 ] || return 1
  return 0
}

# ---------------------------------------------------------------------------
# Onboarding questions
ob_vals=
ob_lbls=
ob_add() {
  if [ -z "$ob_vals" ]; then
    ob_vals=$1
    ob_lbls=$2
  else
    ob_vals="$ob_vals|$1"
    ob_lbls="$ob_lbls|$2"
  fi
}

ob_reset() { ob_vals=; ob_lbls=; }

ob_choose_variant() {
  [ "$variant_explicit" = 0 ] || return 0
  ob_reset
  ob_add full "Full   browser UI plus IPTV, needs the most space"
  ob_add slim "Slim   IPTV only, no browser UI, smallest download"
  if [ "$variant" = slim ]; then ui_sel_default=2; else ui_sel_default=1; fi
  ui_select "Which build?" "$ob_vals" "$ob_lbls"
  unset ui_sel_default
  variant=$ui_sel_value
}

# The downloaded binary decides which providers exist, so an unreleased one
# never shows up as a dead menu entry. `tv+ login` is the current name and
# `extras login` the previous one, which older binaries still use.
ob_binary_supports() {
  "$1" --help 2>/dev/null | grep -q -- "$2"
}

ob_choose_provider() {
  ob_bin=$1
  ob_reset
  ob_tvplus_args=
  if ob_binary_supports "$ob_bin" 'tv+ login'; then
    ob_tvplus_args='tv+ login'
  elif ob_binary_supports "$ob_bin" 'extras login'; then
    ob_tvplus_args='extras login'
  fi
  ob_add jiotv "JioTV       live channels and programme guide - any network"
  [ -n "$ob_tvplus_args" ] &&
    ob_add extras "JioTV+      extra channels, movies and shows - JioFiber/AirFiber"
  ob_binary_supports "$ob_bin" 'fastway login' &&
    ob_add fastway "Fastway     live TV from a Fastway subscription - Fastway/ACT"
  if [ -n "$provider" ]; then
    case "|$ob_vals|" in
      *"|$provider|"*) return 0 ;;
      *) provider= ;;
    esac
  fi
  case "$ob_vals" in
    *'|'*) ;;
    *) provider=$ob_vals; ob_name=$ob_vals; [ "$provider" = extras ] && ob_name='JioTV+'; ui_step "Using $ob_name (the only provider this build supports)"; return 0 ;;
  esac
  ui_note "Which login you have depends on your network: JioTV works anywhere,"
  ui_note "JioTV+ needs a JioFiber or AirFiber login, Fastway needs Fastway or ACT."
  printf '\n'
  ui_select "Which account are you setting up?" "$ob_vals" "$ob_lbls"
  provider=$ui_sel_value
}

# Best-effort LAN address so the summary shows a URL that actually opens.
detect_lan_ip() {
  ob_addr=
  if [ "$openwrt" = true ]; then
    router_ip
    return 0
  fi
  if command -v ip >/dev/null 2>&1; then
    ob_addr=$(ip -4 route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -n 1 || true)
    [ -n "$ob_addr" ] || ob_addr=$(ip -4 addr show scope global 2>/dev/null | sed -n 's/.*inet \([0-9.]*\).*/\1/p' | head -n 1 || true)
  fi
  if [ -z "$ob_addr" ] && command -v ifconfig >/dev/null 2>&1; then
    ob_nif=$(route -n get default 2>/dev/null | sed -n 's/.*interface: //p' | head -n 1 || true)
    if [ -n "$ob_nif" ] && command -v ipconfig >/dev/null 2>&1; then
      ob_addr=$(ipconfig getifaddr "$ob_nif" 2>/dev/null || true)
    fi
    [ -n "$ob_addr" ] || ob_addr=$(ifconfig 2>/dev/null | sed -n 's/.*inet \([0-9.]*\) .*/\1/p' | grep -v '^127\.' | head -n 1 || true)
  fi
  if [ -n "$ob_addr" ]; then printf '%s\n' "$ob_addr"; else printf 'localhost\n'; fi
}

ob_choose_host() {
  [ -z "$public_host" ] || return 0
  ob_detected=$(detect_lan_ip)
  ui_step "Address other devices will use"
  ui_input "This machine's address on your network" "$ob_detected"
  case "$ui_result" in
    ''|*' '*|*/*) ui_warn "That is not an address; keeping $ob_detected"; public_host=$ob_detected ;;
    *) public_host=$ui_result ;;
  esac
  if [ "$public_host" = localhost ]; then
    ui_warn "Only this machine will be able to connect until you bind to a network address"
  fi
}

ob_choose_tunnel() {
  [ -z "$tunnel" ] || return 0
  ui_step "Access from outside your network"
  ui_select "Open JioTV up beyond your LAN?" \
    "none|temp|named" \
    "No, LAN only|Temporary Cloudflare URL, no account, new address every start|Permanent Cloudflare tunnel, needs a tunnel token"
  tunnel=$ui_sel_value
  case "$tunnel" in
    none) return 0 ;;
    temp)
      ui_warn "Anyone with the address can reach the sign-in page. Keep the admin password strong."
      ;;
    named)
      printf '  %sCreate a tunnel at dash.cloudflare.com, point its public hostname at\n  http://localhost:5001, then copy the token from the tunnel%s\n' "$ui_dim" "$ui_reset"
      ui_warn "Anyone with that hostname can reach the sign-in page. Keep the admin password strong."
      if [ -z "$tunnel_token" ]; then
        ui_password "Cloudflare tunnel token"
        tunnel_token=$ui_result
      fi
      case "$tunnel_token" in
        eyJ*) ;;
        '') ui_fail "No token entered; using a temporary URL instead"; tunnel=temp; tunnel_token= ;;
        *) ui_warn "That does not look like a Cloudflare tunnel token"; tunnel=temp; tunnel_token= ;;
      esac
      ;;
  esac
}

# ---------------------------------------------------------------------------
# Credentials
#
# The login commands read their answers from stdin, and the installer's stdin
# is the `curl | sh` pipe, so they are driven through a FIFO instead of being
# handed the terminal. That keeps one continuous wizard and never lets the
# child read the rest of this script as an answer.
ob_dialog_start() {
  ob_fifo=$tmp/login.fifo
  ob_log=$tmp/login.log
  rm -f "$ob_fifo"
  mkfifo "$ob_fifo"
  : >"$ob_log"
  "$@" <"$ob_fifo" >"$ob_log" 2>&1 &
  ob_pid=$!
  exec 5>"$ob_fifo"
}
ob_dialog_send() { printf '%s\n' "$1" >&5; }
ob_dialog_expect() {
  ob_waited=0
  while [ "$ob_waited" -lt "${2:-20}" ]; do
    grep -Eqi -- "$1" "$ob_log" 2>/dev/null && return 0
    if ! kill -0 "$ob_pid" 2>/dev/null; then
      grep -Eqi -- "$1" "$ob_log" 2>/dev/null && return 0
      return 1
    fi
    ob_waited=$((ob_waited + 1))
    sleep 1
  done
  return 1
}
ob_dialog_finish() {
  exec 5>&- 2>/dev/null || true
  ob_fifo=
  ob_status=0
  wait "$ob_pid" || ob_status=$?
  return "$ob_status"
}
ob_dialog_show() {
  [ -s "$ob_log" ] || return 0
  printf '%s\n' "$ui_dim"
  sed 's/^/  │ /' "$ob_log"
  printf '%s\n' "$ui_reset"
}

ob_creds_jiotv() {
  ob_bin=$1
  ui_step "Sign in to JioTV"
  ui_input "Mobile number registered with JioTV (+91)" ""
  ob_number=$ui_result
  case "$ob_number" in
    ''|*[!0-9]*) ui_fail "A mobile number is required for the OTP"; return 1 ;;
  esac
  ob_dialog_start "$ob_bin" login otp
  ob_dialog_send "$ob_number"
  if ob_dialog_expect 'OTP:' 40; then
    ui_ok "OTP sent to +91$ob_number"
    ui_password "Enter the OTP"
    ob_dialog_send "$ui_result"
  fi
  if ob_dialog_finish; then
    if grep -qi "Login successful" "$ob_log"; then ui_ok "Signed in to JioTV"; return 0; fi
  fi
  ob_dialog_show
  ui_fail "JioTV sign-in did not complete"
  return 1
}

ob_creds_extras() {
  ob_bin=$1
  ui_step "Sign in to JioTV+"
  ui_note "Use the JioFiber or AirFiber login for this connection."
  ui_input "Mobile number registered to the fibre account (+91)" ""
  ob_number=$ui_result
  case "$ob_number" in
    ''|*[!0-9]*) ui_fail "A mobile number is required for the OTP"; return 1 ;;
  esac
  [ -n "$ob_tvplus_args" ] || ob_tvplus_args='extras login'
  # shellcheck disable=SC2086  # tv+ is one argument pair, not a filename
  ob_dialog_start "$ob_bin" $ob_tvplus_args
  ob_dialog_send "$ob_number"
  # A number with no selectable connections goes straight to the OTP prompt.
  ob_ready=0
  if ob_dialog_expect 'Choose a connection:|OTP:' 40; then
    if grep -qi 'Choose a connection:' "$ob_log"; then
      ob_reset
      ob_i=1
      while :; do
        ob_line=$(sed -n "s/^  $ob_i\. \(.*\)$/\1/p" "$ob_log")
        [ -n "$ob_line" ] || break
        ob_add "$ob_i" "$ob_line"
        ob_i=$((ob_i + 1))
      done
      if [ -n "$ob_vals" ]; then
        ui_select "Choose the connection" "$ob_vals" "$ob_lbls"
        ob_dialog_send "$ui_sel_value"
      fi
      ob_dialog_expect 'OTP:' 40 && ob_ready=1
    else
      ob_ready=1
    fi
  fi
  if [ "$ob_ready" = 1 ]; then
    ui_ok "OTP sent to +91$ob_number"
    ui_password "Enter the OTP"
    ob_dialog_send "$ui_result"
  fi
  if ob_dialog_finish; then
    if grep -qi "login saved" "$ob_log"; then ui_ok "Signed in to JioTV+"; return 0; fi
  fi
  ob_dialog_show
  ui_fail "JioTV+ sign-in did not complete"
  return 1
}

ob_creds_fastway() {
  ob_bin=$1
  ui_step "Sign in to Fastway"
  ui_input "Fastway username" ""
  ob_user=$ui_result
  [ -n "$ob_user" ] || { ui_fail "A username is required"; return 1; }
  ui_password "Fastway password"
  ob_pass=$ui_result
  [ -n "$ob_pass" ] || { ui_fail "A password is required"; return 1; }
  ob_dialog_start "$ob_bin" fastway login
  ob_dialog_send "$ob_user"
  ob_dialog_send "$ob_pass"
  if ob_dialog_finish; then
    ui_ok "Signed in to Fastway"
    return 0
  fi
  ob_dialog_show
  ui_fail "Fastway sign-in did not complete"
  return 1
}

ob_admin_password() {
  ob_bin=$1
  [ "$variant" = full ] || return 0
  ui_step "Protect the browser interface"
  ui_password "Choose an admin password"
  ob_pass=$ui_result
  if [ -n "$ob_pass" ]; then
    if printf '%s\n' "$ob_pass" | "$ob_bin" admin password >/dev/null 2>&1; then
      ui_ok "Admin password saved"
    else
      ui_fail "Could not save the admin password; run: jiotv admin password"
    fi
  else
    ui_warn "No password set; run: jiotv admin password"
  fi
}

# Sign-in is part of the flow, so a failure is retried before the install is
# reported as incomplete rather than silently left half-configured.
ob_credentials() {
  ob_bin=$1
  ob_done=0
  ob_tries=0
  while [ "$ob_tries" -lt 3 ]; do
    case "$provider" in
      extras) ob_creds_extras "$ob_bin" && ob_done=1 ;;
      fastway) ob_creds_fastway "$ob_bin" && ob_done=1 ;;
      *) ob_creds_jiotv "$ob_bin" && ob_done=1 ;;
    esac
    [ "$ob_done" = 1 ] && break
    ob_tries=$((ob_tries + 1))
    [ "$ob_tries" -ge 3 ] && break
    ui_confirm "Try again?" yes || break
  done
  [ "$ob_done" = 1 ]
}

# ---------------------------------------------------------------------------
# Persisting the answers
ob_write_uci() {
  uci set jiotv.main.tv_plus="$( [ "$provider" = extras ] && echo 1 || echo 0 )"
  uci set jiotv.main.fastway="$( [ "$provider" = fastway ] && echo 1 || echo 0 )"
  case "$tunnel" in
    none) uci set jiotv.main.tunnel=0; uci delete jiotv.main.tunnel_token 2>/dev/null || true ;;
    temp) uci set jiotv.main.tunnel=1; uci delete jiotv.main.tunnel_token 2>/dev/null || true ;;
    named) uci set jiotv.main.tunnel=1; uci set jiotv.main.tunnel_token="$tunnel_token" ;;
  esac
  uci commit jiotv
}

ob_serve_args() {
  ob_args="--host 0.0.0.0"
  [ "$install_tls" = 1 ] && ob_args="$ob_args --tls"
  [ "$tunnel" = none ] || ob_args="$ob_args --tunnel"
  printf '%s\n' "$ob_args"
}

ob_shell_quote() {
  printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

ob_write_shell_env() {
  [ "$openwrt" = true ] && return 0
  ob_env_home=${HOME:-$install_dir}
  ob_env_dir=${XDG_CONFIG_HOME:-$ob_env_home/.config}/jiotv
  mkdir -p "$ob_env_dir"
  chmod 700 "$ob_env_dir" 2>/dev/null || true
  ob_env_file=$ob_env_dir/installer.env
  ob_tvplus_enabled=false
  ob_fastway_enabled=false
  [ "$provider" != extras ] || ob_tvplus_enabled=true
  [ "$provider" != fastway ] || ob_fastway_enabled=true
  (
    umask 077
    {
      printf 'export JIOTV_TVPLUS=%s\n' "$(ob_shell_quote "$ob_tvplus_enabled")"
      printf 'export JIOTV_FASTWAY=%s\n' "$(ob_shell_quote "$ob_fastway_enabled")"
      [ -z "$tunnel_token" ] || printf 'export JIOTV_TUNNEL_TOKEN=%s\n' "$(ob_shell_quote "$tunnel_token")"
    } >"$ob_env_file"
  )
  chmod 600 "$ob_env_file"
}

ob_install_service() {
  ob_bin=$1
  ob_service=0
  JIOTV_TVPLUS=$( [ "$provider" = extras ] && echo true || echo false )
  JIOTV_FASTWAY=$( [ "$provider" = fastway ] && echo true || echo false )
  export JIOTV_TVPLUS JIOTV_FASTWAY
  if [ -n "$tunnel_token" ]; then export JIOTV_TUNNEL_TOKEN="$tunnel_token"; fi
  ob_write_shell_env
  [ "$start_service" = 1 ] || return 0
  if "$ob_bin" autostart --args "$(ob_serve_args)" >"$tmp/autostart.log" 2>&1; then
    ui_ok "Installed as a service and started"
    ob_service=1
    ob_stopped_service=
  else
    ob_service=0
    if [ -n "${ob_stopped_service:-}" ]; then ob_restart_existing_service >/dev/null 2>&1 || true; fi
    ui_warn "Could not install a boot service here; use the start command in the summary"
  fi
}

ob_restart_existing_service() {
  case "${ob_stopped_service:-}" in
    system) systemctl start jiotv ;;
    user) systemctl --user start jiotv ;;
    *) return 1 ;;
  esac
  ob_stopped_service=
}

ob_jiotv_process_running() {
  if command -v pgrep >/dev/null 2>&1; then
    pgrep -x jiotv >/dev/null 2>&1
  elif command -v pidof >/dev/null 2>&1; then
    [ -n "$(pidof jiotv 2>/dev/null || true)" ]
  else
    return 1
  fi
}

ob_stop_existing_service() {
  ob_stopped_service=
  command -v systemctl >/dev/null 2>&1 || return 0
  if systemctl is-active --quiet jiotv 2>/dev/null; then
    ob_stopped_service=system
    systemctl stop jiotv || return 1
  elif systemctl --user is-active --quiet jiotv 2>/dev/null; then
    ob_stopped_service=user
    systemctl --user stop jiotv || return 1
  else
    # Do not sign in while an unmanaged/foreground server may still write its
    # in-memory store over the credentials being updated by this installer.
    ob_jiotv_process_running && return 1
    return 0
  fi
  ob_waited=0
  while [ "$ob_waited" -lt "$ready_timeout" ]; do
    case "$ob_stopped_service" in
      system) systemctl is-active --quiet jiotv 2>/dev/null || { ob_jiotv_process_running || return 0; } ;;
      user) systemctl --user is-active --quiet jiotv 2>/dev/null || { ob_jiotv_process_running || return 0; } ;;
    esac
    ob_waited=$((ob_waited + 1))
    sleep 1
  done
  return 1
}

ob_summary() {
  ob_bin=$1
  ob_ip=${public_host:-localhost}
  case "$ob_ip" in
    \[*\]) ob_url_ip=$ob_ip ;;
    *:*) ob_url_ip="[$ob_ip]" ;;
    *) ob_url_ip=$ob_ip ;;
  esac
  ob_http=5001
  ob_https=5443
  if [ "$openwrt" = true ]; then
    ob_http=$(uci_opt port 5001)
    ob_https=$(uci_opt tls_port 5443)
  fi
  printf '\n'
  ui_rule
  printf '  %s%sInstallation complete%s\n' "$ui_bold" "$ui_green" "$ui_reset"
  ui_rule
  printf '  %sbuild%s     %s %s%s%s\n' "$ui_dim" "$ui_reset" "$variant" "$ui_grey" "$sys/$machine" "$ui_reset"
  case "$provider" in
    extras) printf '  %saccount%s   JioTV+\n' "$ui_dim" "$ui_reset" ;;
    fastway) printf '  %saccount%s   Fastway\n' "$ui_dim" "$ui_reset" ;;
    *) printf '  %saccount%s   JioTV\n' "$ui_dim" "$ui_reset" ;;
  esac
  if [ "$variant" = full ]; then
    if [ "$install_tls" = 1 ] && [ "${ob_tls:-1}" = 1 ]; then
      printf '  %sbrowser%s   %shttps://%s:%s/%s\n' "$ui_dim" "$ui_reset" "$ui_bold" "$ob_ip" "$ob_https" "$ui_reset"
    else
      printf '  %sbrowser%s   %shttp://%s:%s/%s\n' "$ui_dim" "$ui_reset" "$ui_bold" "$ob_ip" "$ob_http" "$ui_reset"
    fi
  fi
  if "$ob_bin" key show >"$tmp/playlist-path" 2>/dev/null; then
    ob_playlist_path=$(cat "$tmp/playlist-path")
    case "$ob_playlist_path" in
      http://*|https://*) ob_playlist_url=$ob_playlist_path ;;
      /*) ob_playlist_url="http://$ob_url_ip:$ob_http$ob_playlist_path" ;;
      *) ob_playlist_url="http://$ob_url_ip:$ob_http/$ob_playlist_path" ;;
    esac
    printf '  %splaylist%s  %s\n' "$ui_dim" "$ui_reset" "$ob_playlist_url"
  else
    ui_warn "Could not get the authenticated playlist URL; run 'jiotv key show' after starting the server"
  fi
  case "$tunnel" in
    temp) printf '  %sremote%s    a trycloudflare.com address, printed by the service each start\n' "$ui_dim" "$ui_reset" ;;
    named) printf '  %sremote%s    your Cloudflare hostname\n' "$ui_dim" "$ui_reset" ;;
  esac
  printf '\n'
  if [ "$ob_service" != 1 ]; then
    if [ -n "${ob_env_file:-}" ]; then
      printf '  Start it with: %s. %s && jiotv serve %s%s\n' "$ui_bold" "$(ob_shell_quote "$ob_env_file")" "$(ob_serve_args)" "$ui_reset"
    else
      printf '  Start it with: %sjiotv serve %s%s\n' "$ui_bold" "$(ob_serve_args)" "$ui_reset"
    fi
  fi
  if [ "$variant" = full ]; then
    printf '  Open the address above and sign in with the admin password.\n'
  else
    printf '  The slim build has no browser interface; use the playlist above in your IPTV app.\n'
  fi
  printf '\n'
}

# ---------------------------------------------------------------------------
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

  if onboard_enabled; then
    ui_banner "OpenWrt · $package_manager · $package_arch"
    ob_choose_variant
  fi

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
  # Extract each name separately: GitHub can return all assets on one JSON line.
  asset=$(grep -oE '"name"[[:space:]]*:[[:space:]]*"[^"]*"' "$tmp/release.json" | sed 's/^[^"]*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)"$/\1/' | grep -E "$pattern" | head -n 1 || true)
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

  # Onboarding runs while the service is installed but not yet restarted, so a
  # terminal login cannot overwrite the state the server writes at start.
  ob_creds_done=0
  if onboard_enabled; then
    if ! stop_service; then
      ui_fail "Could not stop the existing service; account setup was not started"
      exit 1
    fi
    ob_choose_provider /usr/bin/jiotv
    ob_choose_host
    ob_choose_tunnel
    if ob_credentials /usr/bin/jiotv; then ob_creds_done=1; else
      ui_fail "The install is complete but the account is not connected"
    fi
    ob_admin_password /usr/bin/jiotv
    ob_write_uci
  fi

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
  [ -z "$public_host" ] || ip=$public_host
  tls_on=$(uci_flag tls 1)
  http_port=$(uci_opt port 5001)
  tls_port=$(uci_opt tls_port 5443)
  ob_tls=$tls_on

  if [ "${ob_creds_done:-0}" = 1 ]; then
    ob_service=1
    [ "$running" = true ] || ob_service=0
    ob_summary /usr/bin/jiotv
    [ "$stop_failed" != true ] || exit 1
    [ "$start_service" = 0 ] || [ "$disabled" = true ] || [ "$running" = true ] || exit 1
    exit 0
  fi

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
    echo "  IPTV apps  : use the playlist URL the service logs when it starts (logread -e jiotv | grep -i playlist), on http://${ip}:${http_port}/. The slim build has no browser UI."
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

if onboard_enabled; then
  ui_banner "$sys/$machine"
  ob_choose_variant
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
was_installed=false
[ -x "$install_dir/jiotv" ] && was_installed=true
ob_existing_update=false
if [ "$was_installed" = true ] && onboard_enabled; then
  ui_step "JioTV is already installed here"
  if ! ui_confirm "Update it in place and keep the current account?" yes; then
    ui_ok "Left as it was"
    exit 0
  fi
  ob_existing_update=true
fi
if [ "$ob_existing_update" = true ] && ! ob_stop_existing_service; then
  ui_fail "Could not stop the existing service; the installed binary was left unchanged"
  exit 1
fi
mkdir -p "$install_dir"
if command -v install >/dev/null 2>&1; then install -m 0755 "$tmp/$asset" "$install_dir/jiotv"
else cp "$tmp/$asset" "$install_dir/jiotv" && chmod 0755 "$install_dir/jiotv"
fi
echo "Installed jiotv ($variant, $target) to $install_dir/jiotv"
case ":${PATH:-}:" in *":$install_dir:"*) ;; *) echo "Add $install_dir to PATH to run jiotv directly." ;; esac

if onboard_enabled; then
  jiotv_bin=$install_dir/jiotv
  if [ "$ob_existing_update" = true ]; then
    if [ "$start_service" = 1 ] && ob_restart_existing_service; then
      ui_ok "Updated and restarted the existing JioTV service; account settings were kept"
    else
      ui_ok "Updated; the existing account settings were kept"
      ui_note "If JioTV was already running, restart it to load the updated binary."
      [ "$start_service" = 1 ] || ob_stopped_service=
    fi
    exit 0
  fi
  ob_choose_provider "$jiotv_bin"
  ob_choose_host
  ob_choose_tunnel
  if ! ob_stop_existing_service; then
    ui_fail "Could not stop the existing service; account setup was not started"
    exit 1
  fi
  JIOTV_TVPLUS=$( [ "$provider" = extras ] && echo true || echo false )
  JIOTV_FASTWAY=$( [ "$provider" = fastway ] && echo true || echo false )
  export JIOTV_TVPLUS JIOTV_FASTWAY
  if [ -n "$tunnel_token" ]; then export JIOTV_TUNNEL_TOKEN="$tunnel_token"; fi
  ob_write_shell_env
  if ob_credentials "$jiotv_bin"; then
    ob_admin_password "$jiotv_bin"
    ob_install_service "$jiotv_bin"
    ob_summary "$jiotv_bin"
    [ "$start_service" = 1 ] || ob_stopped_service=
    exit 0
  fi
  ui_fail "The install is complete but the account is not connected"
  case "$provider" in
    extras) ob_login_cmd=${ob_tvplus_args:-'extras login'} ;;
    fastway) ob_login_cmd='fastway login' ;;
    *) ob_login_cmd='login otp' ;;
  esac
  if [ -n "${ob_env_file:-}" ]; then
    printf '\n  Sign in when you are ready: . %s && jiotv %s\n\n' "$(ob_shell_quote "$ob_env_file")" "$ob_login_cmd"
  else
    printf '\n  Sign in when you are ready: %sjiotv %s%s\n\n' "$ui_bold" "$ob_login_cmd" "$ui_reset"
  fi
  exit 1
fi

if [ "$install_tls" = 1 ]; then
  echo "Next: jiotv login otp; jiotv admin password; jiotv serve --host 0.0.0.0 --tls"
  if [ "$variant" = slim ]; then
    echo "IPTV apps: use the playlist URL that jiotv serve prints when it starts, with localhost:5001 on this machine or this machine's address from other devices. The slim build has no browser UI."
  else
    echo "Browser UI on this machine (HTTPS, self-signed certificate; accept the one-time warning): https://localhost:5443/"
    echo "From other devices on your network, use this machine's address instead of localhost: https://<this-machine-ip>:5443/ (browser), http://<this-machine-ip>:5001/ (IPTV apps, plain HTTP playlist)."
  fi
else
  echo "Next: jiotv login otp; jiotv admin password; jiotv serve --host 0.0.0.0"
  if [ "$variant" = slim ]; then
    echo "IPTV apps: use the playlist URL that jiotv serve prints when it starts, with localhost:5001 on this machine or this machine's address from other devices. The slim build has no browser UI."
  else
    echo "Browser UI on this machine: http://localhost:5001/ (browsers need HTTPS or localhost for DRM and encrypted HLS playback; add --tls to enable HTTPS)"
    echo "From other devices on your network, use this machine's address instead of localhost: http://<this-machine-ip>:5001/"
  fi
fi
