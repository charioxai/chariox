#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="${CHARIOX_SLICE_ROOT:-/opt/chariox-slice}"
LOGS="$ROOT/logs"
DISPLAY_ID="${CHARIOX_SLICE_DISPLAY:-:99}"
DISPLAY_MODE="${CHARIOX_SLICE_DISPLAY_MODE:-unknown}"
SCREEN_GEOMETRY="${CHARIOX_SLICE_SCREEN_GEOMETRY:-1280x800x24}"
SCREEN_SIZE="${SCREEN_GEOMETRY%x*}"
VNC_PORT="${CHARIOX_SLICE_VNC_PORT:-5900}"
NOVNC_PORT="${CHARIOX_SLICE_NOVNC_PORT:-6080}"
DISPLAY_BACKEND="${CHARIOX_SLICE_DISPLAY_BACKEND:-novnc}"
SELKIES_PORT="${CHARIOX_SLICE_SELKIES_PORT:-$NOVNC_PORT}"
SELKIES_BIN="${CHARIOX_SLICE_SELKIES_BIN:-/opt/chariox-selkies/bin/selkies}"
SELKIES_PID_FILE="${CHARIOX_SLICE_SELKIES_PID_FILE:-$LOGS/selkies.pid}"
SELKIES_HEALTH_TIMEOUT="${CHARIOX_SLICE_SELKIES_HEALTH_TIMEOUT:-15}"
CHROME_URL="${CHARIOX_SLICE_CHROME_URL:-about:blank}"
CHROME_PROFILE="${CHARIOX_SLICE_CHROME_PROFILE:-$HOME/.config/chariox-slice-chromium}"
CHROME_TRUSTED_INSECURE_ORIGINS="${CHARIOX_SLICE_CHROME_TRUSTED_INSECURE_ORIGINS:-http://host.docker.internal:4321}"

export DISPLAY="$DISPLAY_ID"
START_IN_PROGRESS=0

mkdir -p "$LOGS" "$CHROME_PROFILE"

log() {
  printf '[slice-screen] %s\n' "$*" >&2
}

case "$DISPLAY_BACKEND" in
  novnc|selkies) ;;
  *)
    log "unknown display backend: $DISPLAY_BACKEND"
    exit 2
    ;;
esac

if [[ "$DISPLAY_BACKEND" == selkies ]]; then
  case "$SELKIES_PORT" in
    ''|*[!0-9]*) log "Selkies port must be numeric"; exit 2 ;;
  esac
  (( SELKIES_PORT >= 1 && SELKIES_PORT <= 65535 )) \
    || { log "Selkies port is outside the valid range"; exit 2; }
  case "$SELKIES_HEALTH_TIMEOUT" in
    ''|*[!0-9]*) log "Selkies health timeout must be numeric"; exit 2 ;;
  esac
  (( SELKIES_HEALTH_TIMEOUT >= 1 && SELKIES_HEALTH_TIMEOUT <= 120 )) \
    || { log "Selkies health timeout is outside the bounded range"; exit 2; }
fi

run_xdotool() {
  timeout 10s xdotool "$@"
}

start_process() {
  local name="$1"
  shift
  if pgrep -af "$name" >/dev/null; then
    return
  fi
  nohup "$@" >"$LOGS/$name.log" 2>&1 &
}

wait_for_display() {
  local attempt
  for attempt in $(seq 1 50); do
    if xdpyinfo -display "$DISPLAY_ID" >/dev/null 2>&1; then
      return
    fi
    sleep 0.1
  done
  log "X display $DISPLAY_ID did not become ready"
  tail -n 40 "$LOGS/xvfb.log" >&2 || true
  return 1
}

require_process() {
  local pattern="$1"
  local label="$2"
  local log_path="$3"
  if pgrep -af "$pattern" | grep -v defunct >/dev/null; then
    return 0
  fi
  log "$label did not stay running"
  tail -n 40 "$log_path" >&2 || true
  return 1
}

process_running() {
  pgrep -af "$1" | grep -v defunct >/dev/null
}

selkies_pid_state_present() {
  [[ -e "$SELKIES_PID_FILE" || -L "$SELKIES_PID_FILE" ]]
}

selkies_process_start_ticks() {
  local pid="$1"
  [[ -r "/proc/$pid/stat" ]] || return 1
  awk '{ print $22 }' "/proc/$pid/stat"
}

selkies_read_pid_state() {
  local line_count
  [[ -f "$SELKIES_PID_FILE" && ! -L "$SELKIES_PID_FILE" ]] || {
    log "Selkies PID state is absent or unsafe"
    return 1
  }
  line_count="$(wc -l <"$SELKIES_PID_FILE" | tr -d ' ')"
  [[ "$line_count" == 2 ]] || {
    log "Selkies PID state is stale or malformed"
    return 1
  }
  SELKIES_PID="$(sed -n '1p' "$SELKIES_PID_FILE")"
  SELKIES_START_TICKS="$(sed -n '2p' "$SELKIES_PID_FILE")"
  [[ "$SELKIES_PID" =~ ^[1-9][0-9]*$ ]] \
    && [[ "$SELKIES_START_TICKS" =~ ^[0-9]+$ ]] \
    || { log "Selkies PID state is stale or malformed"; return 1; }
}

selkies_process_matches() {
  local pid="$1"
  local start_ticks="$2"
  local command_line
  [[ -r "/proc/$pid/cmdline" ]] || return 1
  [[ "$(selkies_process_start_ticks "$pid")" == "$start_ticks" ]] || return 1
  command_line="$(tr '\0' ' ' <"/proc/$pid/cmdline")"
  [[ "$command_line" == *"$SELKIES_BIN"* ]]
}

selkies_process_owned() {
  selkies_read_pid_state || return 1
  selkies_process_matches "$SELKIES_PID" "$SELKIES_START_TICKS"
}

selkies_require_binary() {
  [[ -x "$SELKIES_BIN" ]] || {
    log "Selkies binary is missing or not executable: $SELKIES_BIN"
    return 1
  }
  command -v curl >/dev/null 2>&1 || {
    log "curl is required for the Selkies health gate"
    return 1
  }
  command -v lsof >/dev/null 2>&1 || {
    log "lsof is required for the Selkies port-conflict gate"
    return 1
  }
}

selkies_port_is_free() {
  if lsof -nP -iTCP:"$SELKIES_PORT" -sTCP:LISTEN 2>/dev/null | tail -n +2 | grep -q .; then
    log "Selkies port is already in use: $SELKIES_PORT"
    return 1
  fi
}

selkies_health() {
  selkies_process_owned || return 1
  curl --fail --silent --show-error --max-time 1 \
    "http://127.0.0.1:$SELKIES_PORT/api/health" \
    | grep -Fxq 'OK'
}

selkies_write_pid_state() {
  local temporary
  [[ ! -L "$SELKIES_PID_FILE" ]] || {
    log "Selkies PID state is a symlink"
    return 1
  }
  temporary="$(mktemp "$SELKIES_PID_FILE.XXXXXX")" || return 1
  chmod 0600 "$temporary"
  printf '%s\n%s\n' "$SELKIES_PID" "$SELKIES_START_TICKS" >"$temporary"
  mv -f "$temporary" "$SELKIES_PID_FILE"
}

selkies_stop_pid() {
  local pid="$1"
  local start_ticks="$2"
  local attempt
  if ! selkies_process_matches "$pid" "$start_ticks"; then
    return 0
  fi
  kill -TERM "$pid" >/dev/null 2>&1 || true
  for attempt in $(seq 1 50); do
    if ! selkies_process_matches "$pid" "$start_ticks"; then
      return 0
    fi
    sleep 0.1
  done
  kill -KILL "$pid" >/dev/null 2>&1 || true
  for attempt in $(seq 1 20); do
    if ! selkies_process_matches "$pid" "$start_ticks"; then
      return 0
    fi
    sleep 0.1
  done
  log "Selkies process did not stop within the bounded timeout"
  return 1
}

selkies_stop() {
  local stop_status=0
  if ! selkies_pid_state_present; then
    return 0
  fi
  selkies_read_pid_state || return 1
  if [[ -e "/proc/$SELKIES_PID" ]]; then
    selkies_process_owned || {
      log "Selkies PID state does not identify an owned process"
      return 1
    }
    selkies_stop_pid "$SELKIES_PID" "$SELKIES_START_TICKS" || stop_status=1
    [[ "$stop_status" == 0 ]] || return 1
  fi
  if [[ -L "$SELKIES_PID_FILE" ]]; then
    log "Selkies PID state became a symlink"
    return 1
  fi
  rm -f "$SELKIES_PID_FILE"
  return "$stop_status"
}

selkies_start() {
  local attempt
  local health_attempts=$((SELKIES_HEALTH_TIMEOUT * 10))
  SELKIES_START_TICKS=""
  selkies_require_binary
  if selkies_pid_state_present; then
    selkies_process_owned && selkies_health && return 0
    log "Selkies PID state is stale or the existing streamer is unhealthy"
    return 1
  fi
  selkies_port_is_free
  nohup "$SELKIES_BIN" \
    --addr=127.0.0.1 \
    --port="$SELKIES_PORT" \
    --mode=websockets \
    --encoder=h264enc \
    --use-cpu=true\|locked \
    --framerate=30 \
    --enable-https=false \
    --enable-basic-auth=false \
    --enable-resize=false\|locked \
    --enable-collab=false\|locked \
    --audio-enabled=false\|locked \
    --microphone-enabled=false\|locked \
    --webcam-enabled=false\|locked \
    --gamepad-enabled=false\|locked \
    --command-enabled=false\|locked \
    --file-transfers=none \
    --enable-clipboard=false \
    >"$LOGS/selkies.log" 2>&1 &
  SELKIES_PID="$!"
  for attempt in $(seq 1 10); do
    SELKIES_START_TICKS="$(selkies_process_start_ticks "$SELKIES_PID" || true)"
    [[ -n "$SELKIES_START_TICKS" ]] && break
    sleep 0.1
  done
  [[ -n "${SELKIES_START_TICKS:-}" ]] || {
    log "Selkies exited before PID state could be recorded"
    return 1
  }
  selkies_write_pid_state || {
    log "unable to record Selkies PID state"
    selkies_stop_pid "$SELKIES_PID" "$SELKIES_START_TICKS" || true
    return 1
  }
  for attempt in $(seq 1 "$health_attempts"); do
    if ! selkies_process_owned; then
      log "Selkies exited before becoming healthy"
      selkies_stop || true
      return 1
    fi
    if selkies_health; then
      return 0
    fi
    sleep 0.1
  done
  log "Selkies health check timed out after ${SELKIES_HEALTH_TIMEOUT}s"
  selkies_stop || true
  return 1
}

stop_process_pattern() {
  local pattern="$1"
  local attempt
  pkill -TERM -f "$pattern" >/dev/null 2>&1 || true
  for attempt in $(seq 1 30); do
    if ! process_running "$pattern"; then
      return 0
    fi
    sleep 0.1
  done
  pkill -KILL -f "$pattern" >/dev/null 2>&1 || true
  for attempt in $(seq 1 20); do
    if ! process_running "$pattern"; then
      return 0
    fi
    sleep 0.1
  done
}

running_novnc_port() {
  pgrep -af "websockify.*127\\.0\\.0\\.1:$VNC_PORT" \
    | grep -v defunct \
    | awk '{
        for (i = 1; i <= NF; i++) {
          if ($i ~ /^0\.0\.0\.0:[0-9]+$/) {
            sub(/^0\.0\.0\.0:/, "", $i)
            print $i
            exit
          }
        }
      }' \
    | head -n 1
}

novnc_running() {
  if process_running "websockify.*$NOVNC_PORT"; then
    return 0
  fi
  [[ -n "$(running_novnc_port)" ]]
}

clear_chromium_profile_locks() {
  if [[ -d "$CHROME_PROFILE" ]]; then
    find "$CHROME_PROFILE" -maxdepth 1 \
      \( -name 'Singleton*' -o -name 'LOCK' -o -name 'lockfile' \) \
      -exec rm -rf {} + >/dev/null 2>&1 || true
  fi
}

configure_chromium_profile_preferences() {
  python3 - "$CHROME_PROFILE" <<'PY' >/dev/null 2>&1 || true
import json
import os
import sys

profile = sys.argv[1]
default_dir = os.path.join(profile, "Default")
os.makedirs(default_dir, exist_ok=True)
path = os.path.join(default_dir, "Preferences")
try:
    with open(path, "r", encoding="utf-8") as handle:
        prefs = json.load(handle)
except Exception:
    prefs = {}

signin = prefs.setdefault("signin", {})
signin["allowed"] = False
prefs.setdefault("sync", {})["requested"] = False
prefs["credentials_enable_service"] = False
profile_prefs = prefs.setdefault("profile", {})
profile_prefs["password_manager_enabled"] = False
profile_prefs["password_manager_leak_detection"] = False

tmp = f"{path}.tmp"
with open(tmp, "w", encoding="utf-8") as handle:
    json.dump(prefs, handle, separators=(",", ":"))
os.replace(tmp, path)
PY
}

screen_missing_components() {
  local missing=()
  if ! xdpyinfo -display "$DISPLAY_ID" >/dev/null 2>&1; then
    missing+=("display")
  fi
  if ! process_running "Xvfb $DISPLAY_ID"; then
    missing+=("xvfb")
  fi
  if [[ "$DISPLAY_BACKEND" == selkies ]]; then
    if ! selkies_health; then
      missing+=("selkies")
    fi
  else
    if ! process_running "x11vnc.*$DISPLAY_ID"; then
      missing+=("x11vnc")
    fi
    if ! novnc_running; then
      missing+=("novnc")
    fi
  fi
  if ! process_running "chromium.*$CHROME_PROFILE"; then
    missing+=("chromium")
  fi
  printf '%s\n' "${missing[@]}"
}

tool_blocking_missing_components() {
  local missing=()
  if ! xdpyinfo -display "$DISPLAY_ID" >/dev/null 2>&1; then
    missing+=("display")
  fi
  if ! process_running "Xvfb $DISPLAY_ID"; then
    missing+=("xvfb")
  fi
  if ! process_running "chromium.*$CHROME_PROFILE"; then
    missing+=("chromium")
  fi
  printf '%s\n' "${missing[@]}"
}

join_by_comma() {
  local IFS=,
  printf '%s' "$*"
}

require_screen_available() {
  local missing
  missing="$(tool_blocking_missing_components)"
  if [[ -z "$missing" ]]; then
    return 0
  fi
  status
  return 1
}

start_desktop() {
  START_IN_PROGRESS=1
  local -a chrome_secure_context_args=()
  if [[ -n "$CHROME_TRUSTED_INSECURE_ORIGINS" ]]; then
    chrome_secure_context_args+=(
      "--unsafely-treat-insecure-origin-as-secure=$CHROME_TRUSTED_INSECURE_ORIGINS"
    )
  fi

  if selkies_pid_state_present || process_running "chromium.*$CHROME_PROFILE" || process_running "Xvfb $DISPLAY_ID" || process_running "x11vnc.*$DISPLAY_ID" || novnc_running; then
    if [[ "$DISPLAY_BACKEND" == selkies ]] || selkies_pid_state_present; then
      stop_desktop
    else
      stop_desktop || true
    fi
  fi
  stop_process_pattern "websockify.*127\\.0\\.0\\.1:$VNC_PORT"
  stop_process_pattern "websockify.*$NOVNC_PORT"
  stop_process_pattern "x11vnc.*$DISPLAY_ID"
  stop_process_pattern "x11vnc.*$VNC_PORT"
  stop_process_pattern "openbox"
  stop_process_pattern "chromium.*$CHROME_PROFILE"
  stop_process_pattern "/usr/lib/chromium/chromium"
  stop_process_pattern "Xvfb $DISPLAY_ID"
  clear_chromium_profile_locks
  configure_chromium_profile_preferences
  rm -f "/tmp/.X${DISPLAY_ID#:}-lock" "/tmp/.X11-unix/X${DISPLAY_ID#:}"

  nohup Xvfb "$DISPLAY_ID" -screen 0 "$SCREEN_GEOMETRY" -ac +extension RANDR +extension XTEST >"$LOGS/xvfb.log" 2>&1 &
  wait_for_display

  nohup openbox >"$LOGS/openbox.log" 2>&1 &
  if [[ "$DISPLAY_BACKEND" == selkies ]]; then
    selkies_start
  else
    nohup x11vnc -display "$DISPLAY_ID" -localhost -nopw -forever -shared -rfbport "$VNC_PORT" >"$LOGS/x11vnc.log" 2>&1 &
    nohup websockify --web=/usr/share/novnc/ "0.0.0.0:$NOVNC_PORT" "127.0.0.1:$VNC_PORT" >"$LOGS/novnc.log" 2>&1 &
  fi

  nohup chromium \
    --user-data-dir="$CHROME_PROFILE" \
    --no-sandbox \
    --password-store=basic \
    --no-first-run \
    --no-default-browser-check \
    --disable-sync \
    --disable-dev-shm-usage \
    --disable-gpu \
    --remote-debugging-address=127.0.0.1 \
    --remote-debugging-port=9222 \
    "${chrome_secure_context_args[@]}" \
    "$CHROME_URL" >"$LOGS/chromium-gui.log" 2>&1 &

  sleep 2
  require_process "Xvfb $DISPLAY_ID" "Xvfb" "$LOGS/xvfb.log"
  if [[ "$DISPLAY_BACKEND" == selkies ]]; then
    selkies_health || { log "Selkies failed its post-start health check"; return 1; }
  else
    require_process "x11vnc.*$DISPLAY_ID" "x11vnc" "$LOGS/x11vnc.log"
    require_process "websockify.*$NOVNC_PORT" "noVNC websockify" "$LOGS/novnc.log"
  fi
  require_process "chromium.*$CHROME_PROFILE" "Chromium" "$LOGS/chromium-gui.log"
  status
  START_IN_PROGRESS=0
}

status() {
  local missing
  missing="$(screen_missing_components)"
  printf 'display=%s\n' "$DISPLAY_ID"
  printf 'screen=%s\n' "$SCREEN_SIZE"
  printf 'mode=%s\n' "$DISPLAY_MODE"
  if [[ -z "$missing" ]]; then
    printf 'available=true\n'
    if [[ "$DISPLAY_BACKEND" == selkies ]]; then
      printf 'viewer=http://127.0.0.1:%s/\n' "$SELKIES_PORT"
      pgrep -af "Xvfb $DISPLAY_ID|openbox|$SELKIES_BIN|chromium.*$CHROME_PROFILE" | grep -v defunct || true
      return 0
    fi
    local viewer_port="$NOVNC_PORT"
    local discovered_port
    discovered_port="$(running_novnc_port)"
    if [[ -n "$discovered_port" ]]; then
      viewer_port="$discovered_port"
    fi
    printf 'viewer=http://127.0.0.1:%s/vnc.html?host=127.0.0.1&port=%s&autoconnect=true&resize=scale\n' "$viewer_port" "$viewer_port"
    pgrep -af "Xvfb $DISPLAY_ID|openbox|x11vnc|websockify|chromium.*$CHROME_PROFILE" | grep -v defunct || true
    return 0
  fi
  local missing_csv
  missing_csv="$(join_by_comma $missing)"
  printf 'available=false\n'
  printf 'missing=%s\n' "$missing_csv"
  printf 'message=slice screen is unavailable; missing %s\n' "$missing_csv"
  return 1
}

stop_desktop() {
  local selkies_status=0
  if selkies_pid_state_present; then
    selkies_stop || selkies_status=$?
  fi
  if process_running "chromium.*$CHROME_PROFILE"; then
    node "$ROOT/browser-cdp.mjs" close-browser >/dev/null 2>&1 || true
  fi
  local attempt
  for attempt in $(seq 1 80); do
    if ! process_running "chromium.*$CHROME_PROFILE"; then
      break
    fi
    sleep 0.1
  done
  pkill -TERM -f "chromium.*$CHROME_PROFILE" >/dev/null 2>&1 || true
  for attempt in $(seq 1 30); do
    if ! process_running "chromium.*$CHROME_PROFILE"; then
      break
    fi
    sleep 0.1
  done
  stop_process_pattern "chromium.*$CHROME_PROFILE"
  stop_process_pattern "/usr/lib/chromium/chromium"
  stop_process_pattern "websockify.*127\\.0\\.0\\.1:$VNC_PORT"
  stop_process_pattern "websockify.*$NOVNC_PORT"
  stop_process_pattern "x11vnc.*$DISPLAY_ID"
  stop_process_pattern "x11vnc.*$VNC_PORT"
  stop_process_pattern "openbox"
  stop_process_pattern "Xvfb $DISPLAY_ID"
  clear_chromium_profile_locks
  return "$selkies_status"
}

screenshot() {
  require_screen_available
  local path="${1:-/tmp/chariox-slice-screenshot.png}"
  scrot -z "$path"
  printf '%s\n' "$path"
}

focus_chromium() {
  local window
  window="$(run_xdotool search --onlyvisible --class chromium 2>/dev/null | tail -n 1 || true)"
  if [[ -n "$window" ]]; then
    run_xdotool windowactivate --sync "$window" >/dev/null 2>&1 || true
    run_xdotool windowfocus --sync "$window" >/dev/null 2>&1 || true
    sleep 0.1
  fi
}

click() {
  require_screen_available
  focus_chromium
  run_xdotool mousemove "$1" "$2" click 1
}

double_click() {
  require_screen_available
  focus_chromium
  run_xdotool mousemove "$1" "$2" click --repeat 2 --delay 80 1
}

drag() {
  require_screen_available
  focus_chromium
  run_xdotool mousemove "$1" "$2" mousedown 1 mousemove --sync "$3" "$4" mouseup 1
}

move_mouse() {
  require_screen_available
  run_xdotool mousemove "$1" "$2"
}

scroll() {
  require_screen_available
  local amount="${1:-1}"
  local button=5
  if [[ "$amount" =~ ^- ]]; then
    button=4
    amount="${amount#-}"
  fi
  local i
  for i in $(seq 1 "$amount"); do
    run_xdotool click "$button"
  done
}

type_text() {
  require_screen_available
  focus_chromium
  run_xdotool type --clearmodifiers --delay 5 "$*"
}

key() {
  require_screen_available
  focus_chromium
  run_xdotool key --clearmodifiers "$1"
}

clipboard_get() {
  require_screen_available
  xclip -selection clipboard -out 2>/dev/null || true
}

CLIPBOARD_OWNER_PID=""

put_clipboard_once() {
  local value="$1"
  printf '%s' "$value" | xclip -selection clipboard -in -loops 1 >/dev/null 2>&1 &
  CLIPBOARD_OWNER_PID="$!"
}

clipboard_set() {
  require_screen_available
  put_clipboard_once "$*"
}

clipboard_clear() {
  require_screen_available
  put_clipboard_once ''
}

paste_stdin() {
  require_screen_available
  local input
  input="$(cat)"
  if printf '%s' "$input" | node "$ROOT/browser-cdp.mjs" type-stdin >/dev/null 2>&1; then
    return 0
  fi
  focus_chromium
  printf '%s' "$input" | run_xdotool type --clearmodifiers --delay 5 --file -
}

secret_paste_stdin() {
  require_screen_available
  local selector="${1:-}"
  if [[ -n "$selector" ]]; then
    node "$ROOT/browser-cdp.mjs" secret-paste-stdin "$selector"
    return
  fi
  node "$ROOT/browser-cdp.mjs" secret-paste-stdin
}

secret_paste_submit_stdin() {
  require_screen_available
  local selector="${1:-}"
  if [[ -n "$selector" ]]; then
    node "$ROOT/browser-cdp.mjs" secret-paste-submit-stdin "$selector"
    return
  fi
  node "$ROOT/browser-cdp.mjs" secret-paste-submit-stdin
}

browser_status() {
  require_screen_available
  run_browser_cdp status
}

run_browser_cdp() {
  timeout --kill-after=1s 30s node "$ROOT/browser-cdp.mjs" "$@"
}

run_browser_cdp_wait() {
  local requested_ms="$1"
  shift
  if [[ ! "$requested_ms" =~ ^[0-9]+$ ]]; then
    requested_ms=10000
  fi
  local deadline_seconds=$(( (requested_ms + 999) / 1000 + 2 ))
  timeout --kill-after=1s "${deadline_seconds}s" node "$ROOT/browser-cdp.mjs" "$@"
}

browser_find() {
  require_screen_available
  local query="$1"
  local kind="${2:-any}"
  run_browser_cdp find "$query" "$kind"
}

browser_fill() {
  require_screen_available
  local selector="$1"
  shift
  run_browser_cdp fill "$selector" "$*"
}

browser_click() {
  require_screen_available
  run_browser_cdp click-selector "$1"
}

browser_submit() {
  require_screen_available
  run_browser_cdp submit "${1:-}"
}

browser_dialog() {
  require_screen_available
  local action="$1"
  local status
  case "$action" in
    accept|dismiss) ;;
    *)
      printf 'dialog action must be accept or dismiss\n' >&2
      return 2
      ;;
  esac
  set +e
  timeout --kill-after=1s 6s node "$ROOT/browser-cdp.mjs" dialog "$action" "${2:-}"
  status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    return 0
  fi
  focus_chromium
  case "$action" in
    accept) run_xdotool key --clearmodifiers Return ;;
    dismiss) run_xdotool key --clearmodifiers Escape ;;
  esac
  sleep 0.3
  printf '{"ok":true,"action":"%s","fallback":"xdotool","cdp_status":%s}' "$action" "$status"
}

browser_text() {
  require_screen_available
  run_browser_cdp text
}

browser_wait_text() {
  require_screen_available
  local requested_ms="${2:-10000}"
  run_browser_cdp_wait "$requested_ms" wait-text "$1" "$requested_ms"
}

browser_wait_selector() {
  require_screen_available
  local requested_ms="${2:-10000}"
  run_browser_cdp_wait "$requested_ms" wait-selector "$1" "$requested_ms"
}

browser_wait_idle() {
  require_screen_available
  local requested_ms="${1:-10000}"
  run_browser_cdp_wait "$requested_ms" wait-idle "$requested_ms"
}

ocr() {
  require_screen_available
  local image="${1:-/tmp/chariox-slice-screenshot.png}"
  if [[ ! -f "$image" ]]; then
    screenshot "$image" >/dev/null
  fi
  tesseract "$image" stdout -l eng 2>/dev/null
}

find_text() {
  require_screen_available
  local query="$1"
  local image="${2:-/tmp/chariox-slice-screenshot.png}"
  local tsv="/tmp/chariox-slice-ocr.tsv"
  if [[ ! -f "$image" ]]; then
    screenshot "$image" >/dev/null
  fi
  tesseract "$image" stdout -l eng tsv >"$tsv" 2>/dev/null
  python3 - "$query" "$tsv" <<'PY'
import csv
import json
import sys

query = sys.argv[1].lower()
path = sys.argv[2]
lines = {}

with open(path, newline="") as handle:
    for row in csv.DictReader(handle, delimiter="\t"):
        text = (row.get("text") or "").strip()
        if not text:
            continue
        key = (row.get("page_num"), row.get("block_num"), row.get("par_num"), row.get("line_num"))
        lines.setdefault(key, []).append(row)

def emit(text, rows):
    left = min(int(row["left"]) for row in rows)
    top = min(int(row["top"]) for row in rows)
    right = max(int(row["left"]) + int(row["width"]) for row in rows)
    bottom = max(int(row["top"]) + int(row["height"]) for row in rows)
    print(json.dumps({
        "text": text,
        "left": left,
        "top": top,
        "width": right - left,
        "height": bottom - top,
        "center_x": (left + right) // 2,
        "center_y": (top + bottom) // 2,
    }))

for rows in lines.values():
    words = [row for row in rows if (row.get("text") or "").strip()]
    for start in range(len(words)):
        for end in range(start + 1, len(words) + 1):
            text = " ".join((row.get("text") or "").strip() for row in words[start:end])
            if query in text.lower():
                emit(text, words[start:end])
                sys.exit(0)

for rows in lines.values():
    words = [row for row in rows if (row.get("text") or "").strip()]
    text = " ".join((row.get("text") or "").strip() for row in words)
    if query in text.lower():
        emit(text, words)
        sys.exit(0)

print(json.dumps(None))
sys.exit(1)
PY
}

open_url() {
  require_screen_available
  if node "$ROOT/browser-cdp.mjs" navigate "$1" >/dev/null 2>&1; then
    sleep 1
    focus_chromium
    return 0
  fi
  chromium --user-data-dir="$CHROME_PROFILE" --no-sandbox --password-store=basic --new-window "$1" >/dev/null 2>&1 &
  sleep 1
  focus_chromium
}

cleanup_failed_start() {
  local status=$?
  if [[ "$START_IN_PROGRESS" == 1 && "$DISPLAY_BACKEND" == selkies ]]; then
    START_IN_PROGRESS=0
    set +e
    stop_desktop
  fi
  exit "$status"
}

trap cleanup_failed_start EXIT

case "${1:-status}" in
  start) start_desktop ;;
  stop) stop_desktop ;;
  status) status ;;
  screenshot) shift; screenshot "$@" ;;
  click) shift; click "$@" ;;
  double-click|double_click) shift; double_click "$@" ;;
  drag) shift; drag "$@" ;;
  move|move_mouse) shift; move_mouse "$@" ;;
  scroll) shift; scroll "$@" ;;
  type|type_text) shift; type_text "$@" ;;
  key) shift; key "$@" ;;
  clipboard-get|clipboard_get) clipboard_get ;;
  clipboard-set|clipboard_set) shift; clipboard_set "$@" ;;
  clipboard-clear|clipboard_clear) clipboard_clear ;;
  paste-stdin|paste_stdin) paste_stdin ;;
  secret-paste-stdin|secret_paste_stdin) shift; secret_paste_stdin "$@" ;;
  secret-paste-submit-stdin|secret_paste_submit_stdin) shift; secret_paste_submit_stdin "$@" ;;
  browser-status|browser_status) browser_status ;;
  browser-find|browser_find) shift; browser_find "$@" ;;
  browser-fill|browser_fill) shift; browser_fill "$@" ;;
  browser-click|browser_click) shift; browser_click "$@" ;;
  browser-submit|browser_submit) shift; browser_submit "$@" ;;
  browser-dialog|browser_dialog) shift; browser_dialog "$@" ;;
  browser-text|browser_text) browser_text ;;
  browser-wait-text|browser_wait_text) shift; browser_wait_text "$@" ;;
  browser-wait-selector|browser_wait_selector) shift; browser_wait_selector "$@" ;;
  browser-wait-idle|browser_wait_idle) shift; browser_wait_idle "$@" ;;
  ocr) shift; ocr "$@" ;;
  find-text|find_text) shift; find_text "$@" ;;
  open-url|open_url) shift; open_url "$@" ;;
  *)
    cat >&2 <<EOF
Usage: $(basename "$0") start|stop|status|screenshot|click|double-click|drag|move|scroll|type|key|clipboard-get|clipboard-set|clipboard-clear|paste-stdin|secret-paste-stdin|secret-paste-submit-stdin|browser-status|browser-find|browser-fill|browser-click|browser-submit|browser-dialog|browser-text|browser-wait-text|browser-wait-selector|browser-wait-idle|ocr|find-text|open-url
EOF
    exit 2
    ;;
esac
