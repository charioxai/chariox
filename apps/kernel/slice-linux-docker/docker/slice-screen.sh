#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="${CHARIOX_SLICE_ROOT:-/opt/chariox-slice}"
LOGS="$ROOT/logs"
if [[ -n "${CHARIOX_SLICE_PRIVATE_ROOT:-}" ]]; then
  LOGS="$CHARIOX_SLICE_PRIVATE_ROOT/runtime/logs"
fi
DISPLAY_ID="${CHARIOX_SLICE_DISPLAY:-:99}"
DISPLAY_MODE="${CHARIOX_SLICE_DISPLAY_MODE:-unknown}"
SCREEN_GEOMETRY="${CHARIOX_SLICE_SCREEN_GEOMETRY:-1280x800x24}"
SCREEN_SIZE="${SCREEN_GEOMETRY%x*}"
DISPLAY_SERVER="${CHARIOX_SLICE_DISPLAY_SERVER:-Xvfb}"
case "$DISPLAY_SERVER" in Xvfb|Xorg) ;; *) exit 2 ;; esac
VNC_PORT="${CHARIOX_SLICE_VNC_PORT:-5900}"
NOVNC_PORT="${CHARIOX_SLICE_NOVNC_PORT:-6080}"
VIEWER_BACKEND="${CHARIOX_SLICE_VIEWER_BACKEND:-selkies}"
CHROME_URL="${CHARIOX_SLICE_CHROME_URL:-about:blank}"
CHROME_PROFILE="${CHARIOX_SLICE_CHROME_PROFILE:-$HOME/.chariox/browser/chromium}"

export DISPLAY="$DISPLAY_ID"

case "$VIEWER_BACKEND" in
  novnc|selkies) ;;
  *) printf 'Unsupported slice viewer backend: %s\n' "$VIEWER_BACKEND" >&2; exit 2 ;;
esac

slice_selkies() {
  /opt/chariox-selkies/bin/python "$ROOT/slice-selkies.py" "$@"
}

mkdir -p "$LOGS" "$CHROME_PROFILE"

log() {
  printf '[slice-screen] %s\n' "$*" >&2
}

run_xdotool() {
  timeout --foreground 10s xdotool "$@"
}

run_xdotool_utf8() {
  LC_ALL=C.UTF-8 run_xdotool "$@"
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
  tail -n 40 "$LOGS/display.log" >&2 || true
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

chromium_running() {
  # The lifetime supervisor carries Chromium's argv after the browser exits.
  # Match the browser executable, never that retiring Python owner.
  process_running "^(/[^[:space:]]*/)?chromium[[:space:]].*--user-data-dir=$CHROME_PROFILE"
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

stop_desktop_audio() {
  stop_process_pattern '(^|/)pulseaudio([[:space:]]|$)'
  if process_running '(^|/)pulseaudio([[:space:]]|$)'; then
    log "PulseAudio did not stop before snapshot"
    return 1
  fi
  # PulseAudio's per-machine runtime link points outside the durable home.
  # Remove only these links after its writers stop, never their targets or
  # persistent audio settings. The protected home scanner rejects them.
  local pulse_config="${XDG_CONFIG_HOME:-$HOME/.config}/pulse"
  local link
  for link in "$pulse_config"/*-runtime; do
    if [[ -L "$link" && "${link##*/}" =~ ^[a-f0-9]{32}-runtime$ ]]; then
      rm -- "$link"
    fi
  done
}

chromium_has_restorable_session() {
  # Browser.close can retire every tab-session file while session cookies
  # remain in the profile. Restore cookie policy for every owned cold launch
  # of an existing profile, independently of those tab files.
  [[ -d "$CHROME_PROFILE/Default" ]]
}

screen_missing_components() {
  local missing=()
  if ! xdpyinfo -display "$DISPLAY_ID" >/dev/null 2>&1; then
    missing+=("display")
  fi
  if ! process_running "$DISPLAY_SERVER $DISPLAY_ID"; then
    missing+=("display")
  fi
  if ! process_running '(^|/)tint2([[:space:]]|$)'; then
    missing+=("taskbar")
  fi
  if [[ "$VIEWER_BACKEND" == "selkies" ]]; then
    if ! slice_selkies status >/dev/null; then
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
  if ! chromium_running; then
    missing+=("chromium")
  fi
  if [[ "${#missing[@]}" -eq 0 ]]; then
    return 0
  fi
  printf '%s\n' "${missing[@]}"
}

tool_blocking_missing_components() {
  local missing=()
  if ! xdpyinfo -display "$DISPLAY_ID" >/dev/null 2>&1; then
    missing+=("display")
  fi
  if ! process_running "$DISPLAY_SERVER $DISPLAY_ID"; then
    missing+=("display")
  fi
  if ! chromium_running; then
    missing+=("chromium")
  fi
  if [[ "${#missing[@]}" -eq 0 ]]; then
    return 0
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

# Desktop startup, supervised recovery and URL forwarding share one Chromium
# launch configuration. `owned` starts one browser-lifecycle.py lifetime and
# prints its record; `forward` hands the URL to the running browser.
run_chromium() {
  local launch_mode="$1"
  shift
  local -a chrome_startup_target_args=()
  local -a chrome_launcher=(nohup chromium)
  if [[ "$launch_mode" == owned ]]; then
    chrome_launcher=(python3 "$ROOT/browser-lifecycle.py" start "$CHROME_PROFILE" "$LOGS/chromium-gui.log" chromium)
    # Wait for the previous owned child tree to retire before replacing it.
    # Start refuses a profile whose previous lifetime is not proven retired.
    python3 "$ROOT/browser-lifecycle.py" stop "$CHROME_PROFILE" >>"$LOGS/chromium-gui.log" 2>&1 \
      || log "previous browser lifetime is not retired"
    clear_chromium_profile_locks
    if chromium_has_restorable_session; then
      # App URLs cannot be restored until their verified assets and CDP Fetch
      # interception are reattached. This must finish before Chromium starts.
      local app_restore_mode
      app_restore_mode="$(node "$ROOT/browser-app-restore.mjs" "$CHROME_PROFILE")" || return "$?"
      case "$app_restore_mode" in
        restore) chrome_startup_target_args+=(--restore-last-session) ;;
        fresh) ;;
        *) log "invalid App restore result"; return 1 ;;
      esac
    fi
  fi
  if [[ "$#" -gt 0 ]]; then
    chrome_startup_target_args+=(--new-window -- "$@")
  elif [[ "${#chrome_startup_target_args[@]}" -eq 0 ]]; then
    chrome_startup_target_args=(-- "$CHROME_URL")
  fi

  chrome_launcher+=( \
    --user-data-dir="$CHROME_PROFILE" \
    --password-store=basic \
    --no-first-run \
    --no-default-browser-check \
    --disable-sync \
    --disable-dev-shm-usage \
    --disable-gpu \
    --remote-debugging-address=127.0.0.1 \
    --remote-debugging-port=9222 \
    "${chrome_startup_target_args[@]}" )
  if [[ "$launch_mode" == owned ]]; then
    "${chrome_launcher[@]}" 2>>"$LOGS/chromium-gui.log"
  else
    exec "${chrome_launcher[@]}" >>"$LOGS/chromium-gui.log" 2>&1
  fi
}

# Keep one owned browser lifetime running and relaunch it after the browser
# exits. browser-lifecycle.py owns, reaps and proves the retirement of each
# lifetime's process tree; this supervisor retires the current one on TERM.
supervise_chromium() {
  exec 3>"$LOGS/chromium-supervisor.lock"
  flock -w 10 3 || { log "browser supervisor lock timed out"; return 1; }
  printf '%s\n' "$$" >"$LOGS/chromium-supervisor.pid"
  local record="" waiter_pid="" backoff_pid=""
  trap retire_supervised_chromium TERM INT
  while true; do
    if record="$(run_chromium owned "$@" 3>&-)" && [[ -n "$record" ]]; then
      # Wait for this lifetime's browser by recorded identity, never by
      # pattern. A pidfd cannot follow a reused PID; recheck after opening it.
      python3 -B - "$ROOT/browser-lifecycle.py" "$record" 3>&- <<'PYTHON' &
import importlib.util, json, os, select, sys
spec = importlib.util.spec_from_file_location("browser_lifecycle", sys.argv[1])
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)
browser = json.loads(sys.argv[2]).get("browser")
try:
    descriptor = os.pidfd_open(browser["pid"])
except (TypeError, ProcessLookupError):
    sys.exit(0)
if lifecycle.same_process(browser):
    select.select([descriptor], [], [])
PYTHON
      waiter_pid=$!
      wait "$waiter_pid" || true
      waiter_pid=""
      # Avoid a restart storm, and leave a bounded loss window for Room health.
      # The next lifetime start first retires this one, including descendants.
      log "Chromium exited; relaunching in 6 seconds"
    else
      log "Chromium lifetime did not start; retrying in 6 seconds"
    fi
    sleep 6 3>&- &
    backoff_pid=$!
    wait "$backoff_pid" || true
    backoff_pid=""
    set --
  done
}

# TERM/INT handler for supervise_chromium; its waiter and backoff are locals
# of that frame. Retirement uses the lifecycle's verified Browser.close and
# bounded settlement, never pattern-based termination.
retire_supervised_chromium() {
  trap - TERM INT
  local child retire_exit=0
  for child in "$waiter_pid" "$backoff_pid"; do
    [[ -z "$child" ]] || kill -TERM "$child" 2>/dev/null || true
    [[ -z "$child" ]] || wait "$child" || true
  done
  python3 "$ROOT/browser-lifecycle.py" stop "$CHROME_PROFILE" >>"$LOGS/chromium-gui.log" 2>&1 || retire_exit=$?
  rm -f "$LOGS/chromium-supervisor.pid"
  exit "$retire_exit"
}

# Serialize launchers, including an open-url during supervisor backoff. Child
# processes close this lock FD; it belongs only to this bounded launch request.
launch_chromium() (
  exec 4>"$LOGS/chromium-launch.lock"
  flock -w 10 4 || { log "browser launch lock timed out"; return 1; }
  if chromium_running; then
    run_chromium forward "$@" 4>&- &
    return
  fi
  stop_chromium_supervisor || return $?
  nohup bash "${BASH_SOURCE[0]}" supervise-browser "$@" 4>&- >>"$LOGS/chromium-supervisor.log" 2>&1 &
  for _ in $(seq 1 50); do
    chromium_running && return
    sleep 0.1
  done
  log "Chromium supervisor did not launch a browser"
  stop_chromium_supervisor
  return 1
)

# The lock cannot survive process death or a container restart. Clear stale
# markers while holding it, so a new owner's marker cannot be removed.
chromium_supervisor_active() {
  if flock -n "$LOGS/chromium-supervisor.lock" rm -f -- "$LOGS/chromium-supervisor.pid"; then
    return 1
  fi
  return 0
}

stop_chromium_supervisor() {
  local supervisor_pid
  local -a supervisor_args=()
  chromium_supervisor_active || return 0
  supervisor_pid="$(cat "$LOGS/chromium-supervisor.pid" 2>/dev/null || true)"
  if [[ ! "$supervisor_pid" =~ ^[1-9][0-9]*$ ]] \
    || ! mapfile -d '' -t supervisor_args 2>/dev/null <"/proc/$supervisor_pid/cmdline" \
    || [[ "${#supervisor_args[@]}" -lt 3 ]] \
    || [[ "${supervisor_args[0]##*/}" != bash || "${supervisor_args[2]:-}" != supervise-browser ]] \
    || [[ "$(readlink -f -- "${supervisor_args[1]:-}")" != "$(readlink -f -- "${BASH_SOURCE[0]}")" ]]; then
    chromium_supervisor_active || return 0
    log "browser supervisor identity could not be verified"
    return 1
  fi
  kill -TERM "$supervisor_pid" 2>/dev/null || true
  # Its TERM path retires the lifetime: a verified Browser.close (3 s) and
  # bounded descendant settlement (12 s), after any in-flight launch.
  for _ in $(seq 1 200); do
    chromium_supervisor_active || return 0
    sleep 0.1
  done
  log "Chromium supervisor did not stop"
  return 1
}

start_desktop() {
  if chromium_running || process_running "$DISPLAY_SERVER $DISPLAY_ID" || process_running "x11vnc.*$DISPLAY_ID" || novnc_running; then
    stop_desktop
  fi
  # Stop an owned previous Selkies process even when switching to noVNC.
  if [[ -x /opt/chariox-selkies/bin/python ]]; then
    slice_selkies stop --allow-forced >/dev/null
  fi
  stop_process_pattern "websockify.*127\\.0\\.0\\.1:$VNC_PORT"
  stop_process_pattern "websockify.*$NOVNC_PORT"
  stop_process_pattern "x11vnc.*$DISPLAY_ID"
  stop_process_pattern "x11vnc.*$VNC_PORT"
  stop_process_pattern '(^|/)openbox([[:space:]]|$)'
  stop_process_pattern '(^|/)tint2([[:space:]]|$)'
  stop_desktop_audio
  stop_process_pattern "$DISPLAY_SERVER $DISPLAY_ID"
  rm -f "/tmp/.X${DISPLAY_ID#:}-lock" "/tmp/.X11-unix/X${DISPLAY_ID#:}"

  if [[ "$DISPLAY_SERVER" == "Xorg" ]]; then
    nohup Xorg "$DISPLAY_ID" -config "$ROOT/xorg-dummy.conf" -logfile "$LOGS/Xorg.log" -nolisten tcp -noreset -ac >"$LOGS/display.log" 2>&1 &
  else
    nohup Xvfb "$DISPLAY_ID" -screen 0 "$SCREEN_GEOMETRY" -ac +extension RANDR +extension XTEST >"$LOGS/display.log" 2>&1 &
  fi
  wait_for_display
  if [[ "$DISPLAY_SERVER" == "Xorg" ]]; then
    # Apply the configured initial geometry before any browser/viewer starts.
    # Subsequent geometry changes belong to the canonical controller path.
    /opt/chariox-selkies/bin/python - "$ROOT" "$SCREEN_SIZE" <<'PYTHON'
import importlib.util, pathlib, sys, time
sys.path.insert(0, sys.argv[1])
spec = importlib.util.spec_from_file_location("canonical_display", pathlib.Path(sys.argv[1]) / "canonical-display.py")
display = importlib.util.module_from_spec(spec)
spec.loader.exec_module(display)
width, height = map(int, sys.argv[2].split("x"))
display.dimensions(width, height)
display._deadline = time.monotonic() + 10
display.resize(width, height)
PYTHON
  fi

  # Desktop-launched programs inherit one session bus. Without it, ordinary
  # GTK applications cannot persist dconf settings. The supervisor stops the
  # bus when Openbox exits, so stop only Openbox rather than killing both.
  # MP-08 / MP-11: native observations attach to the SAME desktop session bus.
  local computer_runtime="$ROOT/private"
  if [[ -n "${CHARIOX_SLICE_PRIVATE_ROOT:-}" ]]; then computer_runtime="$CHARIOX_SLICE_PRIVATE_ROOT/runtime"; fi
  install -d -m 0700 "$computer_runtime"
  rm -f "$computer_runtime/desktop-session-address"
  nohup dbus-run-session -- sh -c 'umask 077; printf "%s" "$DBUS_SESSION_BUS_ADDRESS" > "$1"; exec openbox' sh "$computer_runtime/desktop-session-address" >"$LOGS/openbox.log" 2>&1 &
  nohup tint2 -c "$ROOT/tint2rc" >"$LOGS/taskbar.log" 2>&1 &
  if [[ "$VIEWER_BACKEND" == "selkies" ]]; then
    if ! slice_selkies start >/dev/null; then
      stop_desktop
      return 1
    fi
  else
    nohup x11vnc -display "$DISPLAY_ID" -xrandr resize -nonap -wait 100 -noxdamage -localhost -nopw -forever -shared -rfbport "$VNC_PORT" >"$LOGS/x11vnc.log" 2>&1 &
    nohup websockify --web=/usr/share/novnc/ "0.0.0.0:$NOVNC_PORT" "127.0.0.1:$VNC_PORT" >"$LOGS/novnc.log" 2>&1 &
  fi

  launch_chromium || return $?

  sleep 2
  require_process "$DISPLAY_SERVER $DISPLAY_ID" "$DISPLAY_SERVER" "$LOGS/display.log"
  require_process '(^|/)openbox([[:space:]]|$)' "Openbox" "$LOGS/openbox.log"
  require_process '(^|/)tint2([[:space:]]|$)' "Applications taskbar" "$LOGS/taskbar.log"
  if [[ "$VIEWER_BACKEND" == "selkies" ]]; then
    slice_selkies status >/dev/null
  else
    require_process "x11vnc.*$DISPLAY_ID" "x11vnc" "$LOGS/x11vnc.log"
    require_process "websockify.*$NOVNC_PORT" "noVNC websockify" "$LOGS/novnc.log"
  fi
  require_process "^(/[^[:space:]]*/)?chromium[[:space:]].*--user-data-dir=$CHROME_PROFILE" "Chromium" "$LOGS/chromium-gui.log"
  # MP-08 / MP-11: Xorg/desktop clients may install Mode_switch on code8.
  # Prepare one inert text slot after startup, before publishing readiness.
  CHARIOX_OWNED_VIRTUAL_DISPLAY=1 /usr/bin/python3 "$ROOT/slice-keyboard.py" prepare-owned-keymap
  status
}

status() {
  local missing
  missing="$(screen_missing_components)"
  printf 'display=%s\n' "$DISPLAY_ID"
  printf 'screen=%s\n' "$SCREEN_SIZE"
  printf 'mode=%s\n' "$DISPLAY_MODE"
  if [[ -z "$missing" ]]; then
    printf 'available=true\n'
    if [[ "$VIEWER_BACKEND" == "selkies" ]]; then
      printf 'viewer=http://127.0.0.1:%s/\n' "$NOVNC_PORT"
      return 0
    fi
    local viewer_port="$NOVNC_PORT"
    local discovered_port
    discovered_port="$(running_novnc_port)"
    if [[ -n "$discovered_port" ]]; then
      viewer_port="$discovered_port"
    fi
    printf 'viewer=http://127.0.0.1:%s/vnc.html?host=127.0.0.1&port=%s&autoconnect=true&resize=scale\n' "$viewer_port" "$viewer_port"
    pgrep -af "$DISPLAY_SERVER $DISPLAY_ID|openbox|x11vnc|websockify|chromium.*$CHROME_PROFILE" | grep -v defunct || true
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
  local supervisor_exit=0 browser_exit=0 streamer_exit=0 attempt
  # The supervisor retires its owned browser lifetime before it exits.
  stop_chromium_supervisor || supervisor_exit=$?
  # A lifetime can outlive its supervisor. Only the lifecycle may prove that
  # the browser tree exited; never fall back to pattern-based termination.
  if [[ "$supervisor_exit" -eq 0 ]]; then
    python3 "$ROOT/browser-lifecycle.py" stop "$CHROME_PROFILE" >/dev/null || browser_exit=$?
  fi
  if [[ -x /opt/chariox-selkies/bin/python ]]; then
    slice_selkies stop >/dev/null || streamer_exit=$?
  fi
  stop_desktop_audio
  stop_process_pattern "websockify.*127\\.0\\.0\\.1:$VNC_PORT"
  stop_process_pattern "websockify.*$NOVNC_PORT"
  stop_process_pattern "x11vnc.*$DISPLAY_ID"
  stop_process_pattern "x11vnc.*$VNC_PORT"
  stop_process_pattern '(^|/)openbox([[:space:]]|$)'
  stop_process_pattern '(^|/)tint2([[:space:]]|$)'
  stop_process_pattern "$DISPLAY_SERVER $DISPLAY_ID"
  if [[ "$supervisor_exit" -ne 0 ]]; then
    # Stopping the display above can let a slow browser finish the
    # supervisor's TERM path. Wait for that terminal proof before retaining
    # the earlier stop failure, then require the lifecycle's retirement proof.
    for attempt in $(seq 1 50); do
      chromium_supervisor_active || break
      sleep 0.1
    done
    if chromium_supervisor_active; then
      return "$supervisor_exit"
    fi
    python3 "$ROOT/browser-lifecycle.py" stop "$CHROME_PROFILE" >/dev/null || browser_exit=$?
  fi
  # Profile locks belong to the browser until its retirement is proven.
  if [[ "$browser_exit" -ne 0 ]]; then
    return "$browser_exit"
  fi
  clear_chromium_profile_locks
  return "$streamer_exit"
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

pointer_click() {
  require_screen_available
  local x="$1"
  local y="$2"
  local button_name="$3"
  local click_count="$4"
  local button
  case "$button_name" in
    left) button=1 ;;
    middle) button=2 ;;
    right) button=3 ;;
    *) printf 'pointer button must be left, middle, or right\n' >&2; return 2 ;;
  esac
  case "$click_count" in
    1|2) ;;
    *) printf 'pointer click count must be 1 or 2\n' >&2; return 2 ;;
  esac
  if [[ "${CHARIOX_COMPUTER_AGENT_INPUT:-}" == 1 ]]; then
    python3 "$ROOT/slice-keyboard.py" pointer-click "$button_name" "$click_count" "$x" "$y"
    return
  fi
  # xdotool also delays after the last release. A single click needs no
  # repeat interval; keep the double-click interval unchanged.
  local delay=0
  if [[ "$click_count" == 2 ]]; then
    delay=80
  fi
  run_xdotool mousemove "$x" "$y" click --repeat "$click_count" --delay "$delay" "$button"
}

pointer_drag() {
  require_screen_available
  local from_x="$1"
  local from_y="$2"
  local to_x="$3"
  local to_y="$4"
  local button_name="$5"
  local button
  case "$button_name" in
    left) button=1 ;;
    middle) button=2 ;;
    right) button=3 ;;
    *) printf 'pointer button must be left, middle, or right\n' >&2; return 2 ;;
  esac
  run_xdotool mousemove "$from_x" "$from_y" mousedown "$button" mousemove --sync "$to_x" "$to_y" mouseup "$button"
}

pointer_scroll() {
  require_screen_available
  local x="$1"
  local y="$2"
  local horizontal_steps="$3"
  local vertical_steps="$4"
  if [[ ! "$horizontal_steps" =~ ^-?[0-9]+$ || ! "$vertical_steps" =~ ^-?[0-9]+$ ]]; then
    printf 'pointer scroll steps must be integers\n' >&2
    return 2
  fi
  if (( horizontal_steps == 0 && vertical_steps == 0 \
    || horizontal_steps < -120 || horizontal_steps > 120 \
    || vertical_steps < -120 || vertical_steps > 120 )); then
    printf 'pointer scroll steps must be nonzero and between -120 and 120\n' >&2
    return 2
  fi
  run_xdotool mousemove "$x" "$y"
  if (( horizontal_steps < 0 )); then
    run_xdotool click --repeat "$((-horizontal_steps))" --delay 20 6
  elif (( horizontal_steps > 0 )); then
    run_xdotool click --repeat "$horizontal_steps" --delay 20 7
  fi
  if (( vertical_steps < 0 )); then
    run_xdotool click --repeat "$((-vertical_steps))" --delay 20 4
  elif (( vertical_steps > 0 )); then
    run_xdotool click --repeat "$vertical_steps" --delay 20 5
  fi
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
  run_xdotool_utf8 type --clearmodifiers --delay 5 "$*"
}

computer_type_stdin() {
  require_screen_available
  # MP-11: slice-keyboard uses the kernel-admitted CHARIOX_COMPUTER_AGENT_INPUT
  # actor marker to bind and recheck native focus before every press. AT-SPI
  # is a system package; expose it without replacing the pinned keyboard backend.
  # Kernel enforces a length-derived deadline and immediate cancellation.
  # Standalone safety bound accommodates the full 64 KiB input contract.
  PYTHONPATH=/usr/lib/python3/dist-packages timeout --foreground --kill-after=1s 2h /opt/chariox-selkies/bin/python \
    "${BASH_SOURCE[0]%/*}/slice-keyboard.py"
}

computer_key_stdin() {
  require_screen_available
  local repeat="$1"
  local key=""
  IFS= read -r key || [[ -n "$key" ]]
  if [[ ! "$repeat" =~ ^[0-9]+$ ]] || (( repeat < 1 || repeat > 32 )); then
    printf 'computer key repeat must be between 1 and 32\n' >&2
    return 2
  fi
  if [[ -z "$key" || ${#key} -gt 128 || "$key" == -* || "$key" =~ [[:space:]] ]]; then
    printf 'computer key must be a non-whitespace native key name of at most 128 bytes\n' >&2
    return 2
  fi
  # MP-08/MP-11: use the same strict XTEST chords as host native input.
  # xdotool can return success for unknown lowercase keysyms without input.
  printf '%s' "$key" | PYTHONPATH=/usr/lib/python3/dist-packages timeout --foreground 10s /opt/chariox-selkies/bin/python \
    "${BASH_SOURCE[0]%/*}/slice-keyboard.py" key-repeat "$repeat"
}

computer_input_reset() {
  require_screen_available
  timeout --foreground 10s /opt/chariox-selkies/bin/python \
    "${BASH_SOURCE[0]%/*}/slice-keyboard.py" reset
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

require_clipboard_available() {
  require_screen_available
  if command -v xclip >/dev/null 2>&1; then
    return 0
  fi
  log "xclip is not available"
  return 1
}

computer_clipboard_write_stdin() (
  require_clipboard_available
  local input_path
  input_path="$(mktemp "${TMPDIR:-/tmp}/chariox-computer-clipboard.XXXXXX")"
  trap 'rm -f "$input_path"' EXIT
  cat >"$input_path"
  xclip -selection clipboard -in <"$input_path" >/dev/null 2>&1
)

computer_clipboard_read() {
  require_clipboard_available
  xclip -selection clipboard -out 2>/dev/null || true
}

paste_stdin() {
  require_screen_available
  local input
  input="$(cat)"
  if printf '%s' "$input" | node "$ROOT/browser-cdp.mjs" type-stdin >/dev/null 2>&1; then
    return 0
  fi
  focus_chromium
  printf '%s' "$input" | run_xdotool_utf8 type --clearmodifiers --delay 5 --file -
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

computer_secret_target() {
  require_screen_available
  /opt/chariox-selkies/bin/python "${BASH_SOURCE[0]%/*}/slice-keyboard.py" secret-target
}

computer_secret_paste_stdin() {
  require_screen_available
  if [[ $# != 1 ]]; then
    log "computer credential input requires an approved display target"
    return 2
  fi
  /opt/chariox-selkies/bin/python "${BASH_SOURCE[0]%/*}/slice-keyboard.py" secret "$1"
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
  python3 "$ROOT/slice-text-finder.py" --image "$image"
}

find_text() {
  require_screen_available
  local query="$1"
  local image="${2:-/tmp/chariox-slice-screenshot.png}"
  if [[ ! -f "$image" ]]; then
    screenshot "$image" >/dev/null
  fi
  python3 "$ROOT/slice-text-finder.py" --image "$image" "$query"
}

open_url() {
  # Opening a URL may recover the browser, but must not restart or create the
  # shared desktop. Other input operations still require the full screen state.
  if ! xdpyinfo -display "$DISPLAY_ID" >/dev/null 2>&1 || ! process_running "$DISPLAY_SERVER $DISPLAY_ID"; then
    status
    return 1
  fi
  if chromium_running && run_browser_cdp navigate "$1" >/dev/null 2>&1; then
    sleep 1
    focus_chromium
    return 0
  fi
  launch_chromium "$1"
  sleep 2
  require_process "^(/[^[:space:]]*/)?chromium[[:space:]].*--user-data-dir=$CHROME_PROFILE" "Chromium" "$LOGS/chromium-gui.log"
  focus_chromium
}

cleanup_failed_start() {
  local startup_status=$?
  trap - EXIT
  if [[ "$startup_status" -ne 0 ]]; then
    stop_desktop || true
  fi
  exit "$startup_status"
}

case "${1:-status}" in
  supervise-browser) shift; supervise_chromium "$@" ;;
  start)
    # Keep errexit active inside start_desktop. An `if start_desktop` wrapper
    # would suppress failures inside the function and could report success.
    trap cleanup_failed_start EXIT
    start_desktop
    ;;
  stop) stop_desktop ;;
  status) status ;;
  protected-screenshot|protected-ocr|protected-find-text)
    mode="${1#protected-}"; shift
    require_screen_available
    /usr/bin/python3 "$ROOT/slice-observation-mask.py" "$mode" "$@" ;;
  screenshot) shift; screenshot "$@" ;;
  click) shift; click "$@" ;;
  double-click|double_click) shift; double_click "$@" ;;
  pointer-click|pointer_click) shift; pointer_click "$@" ;;
  pointer-drag|pointer_drag) shift; pointer_drag "$@" ;;
  pointer-scroll|pointer_scroll) shift; pointer_scroll "$@" ;;
  drag) shift; drag "$@" ;;
  move|move_mouse) shift; move_mouse "$@" ;;
  scroll) shift; scroll "$@" ;;
  type|type_text) shift; type_text "$@" ;;
  computer-type-stdin|computer_type_stdin) computer_type_stdin ;;
  computer-key-stdin|computer_key_stdin) shift; computer_key_stdin "$@" ;;
  computer-clipboard-write-stdin|computer_clipboard_write_stdin) computer_clipboard_write_stdin ;;
  computer-clipboard-read|computer_clipboard_read) computer_clipboard_read ;;
  computer-key-hold-stdin) require_screen_available; /opt/chariox-selkies/bin/python "${BASH_SOURCE[0]%/*}/slice-keyboard.py" hold-key "$2" ;;
  pointer-hold) require_screen_available; /opt/chariox-selkies/bin/python "${BASH_SOURCE[0]%/*}/slice-keyboard.py" hold-button "$4" "$5" "$2" "$3" ;;
  computer-input-reset|computer_input_reset) computer_input_reset ;;
  key) shift; key "$@" ;;
  clipboard-get|clipboard_get) clipboard_get ;;
  clipboard-set|clipboard_set) shift; clipboard_set "$@" ;;
  clipboard-clear|clipboard_clear) clipboard_clear ;;
  paste-stdin|paste_stdin) paste_stdin ;;
  secret-paste-stdin|secret_paste_stdin) shift; secret_paste_stdin "$@" ;;
  secret-paste-submit-stdin|secret_paste_submit_stdin) shift; secret_paste_submit_stdin "$@" ;;
  computer-secret-target) computer_secret_target ;;
  computer-secret-paste-stdin|computer_secret_paste_stdin) shift; computer_secret_paste_stdin "$@" ;;
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
Usage: $(basename "$0") start|stop|status|screenshot|click|double-click|pointer-click|pointer-drag|pointer-scroll|drag|move|scroll|type|computer-type-stdin|key|computer-key-stdin|computer-key-hold-stdin|pointer-hold|computer-clipboard-write-stdin|computer-clipboard-read|computer-input-reset|clipboard-get|clipboard-set|clipboard-clear|paste-stdin|secret-paste-stdin|secret-paste-submit-stdin|computer-secret-paste-stdin|browser-status|browser-find|browser-fill|browser-click|browser-submit|browser-dialog|browser-text|browser-wait-text|browser-wait-selector|browser-wait-idle|ocr|find-text|open-url
EOF
    exit 2
    ;;
esac
