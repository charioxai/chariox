#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="${ARROBA_SLICE_ROOT:-/opt/arroba-slice}"
LOGS="$ROOT/logs"
DISPLAY_ID="${ARROBA_SLICE_DISPLAY:-:99}"
SCREEN_GEOMETRY="${ARROBA_SLICE_SCREEN_GEOMETRY:-1280x800x24}"
SCREEN_SIZE="${SCREEN_GEOMETRY%x*}"
VNC_PORT="${ARROBA_SLICE_VNC_PORT:-5900}"
NOVNC_PORT="${ARROBA_SLICE_NOVNC_PORT:-6080}"
CHROME_URL="${ARROBA_SLICE_CHROME_URL:-about:blank}"
CHROME_PROFILE="${ARROBA_SLICE_CHROME_PROFILE:-$HOME/.config/arroba-slice-chromium}"

export DISPLAY="$DISPLAY_ID"

mkdir -p "$LOGS" "$CHROME_PROFILE"

log() {
  printf '[slice-screen] %s\n' "$*" >&2
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

start_desktop() {
  pkill -f "Xvfb $DISPLAY_ID" >/dev/null 2>&1 || true
  pkill -f "openbox.*$DISPLAY_ID" >/dev/null 2>&1 || true
  pkill -f "x11vnc.*$DISPLAY_ID" >/dev/null 2>&1 || true
  pkill -f "websockify.*$NOVNC_PORT" >/dev/null 2>&1 || true
  pkill -f "chromium.*$CHROME_PROFILE" >/dev/null 2>&1 || true
  pkill -f "/usr/lib/chromium/chromium" >/dev/null 2>&1 || true
  rm -f "/tmp/.X${DISPLAY_ID#:}-lock" "/tmp/.X11-unix/X${DISPLAY_ID#:}"

  nohup Xvfb "$DISPLAY_ID" -screen 0 "$SCREEN_GEOMETRY" -ac +extension RANDR +extension XTEST >"$LOGS/xvfb.log" 2>&1 &
  wait_for_display

  nohup openbox >"$LOGS/openbox.log" 2>&1 &
  nohup x11vnc -display "$DISPLAY_ID" -localhost -nopw -forever -shared -rfbport "$VNC_PORT" >"$LOGS/x11vnc.log" 2>&1 &
  nohup websockify --web=/usr/share/novnc/ "0.0.0.0:$NOVNC_PORT" "127.0.0.1:$VNC_PORT" >"$LOGS/novnc.log" 2>&1 &

  nohup chromium \
    --user-data-dir="$CHROME_PROFILE" \
    --no-sandbox \
    --no-first-run \
    --no-default-browser-check \
    --disable-dev-shm-usage \
    --disable-gpu \
    --remote-debugging-address=127.0.0.1 \
    --remote-debugging-port=9222 \
    "$CHROME_URL" >"$LOGS/chromium-gui.log" 2>&1 &

  sleep 2
  require_process "Xvfb $DISPLAY_ID" "Xvfb" "$LOGS/xvfb.log"
  require_process "x11vnc.*$DISPLAY_ID" "x11vnc" "$LOGS/x11vnc.log"
  require_process "websockify.*$NOVNC_PORT" "noVNC websockify" "$LOGS/novnc.log"
  require_process "chromium.*$CHROME_PROFILE" "Chromium" "$LOGS/chromium-gui.log"
  status
}

status() {
  printf 'display=%s\n' "$DISPLAY_ID"
  printf 'screen=%s\n' "$SCREEN_SIZE"
  printf 'viewer=http://127.0.0.1:%s/vnc.html?host=127.0.0.1&port=%s&autoconnect=true&resize=scale\n' "$NOVNC_PORT" "$NOVNC_PORT"
  pgrep -af "Xvfb $DISPLAY_ID|openbox|x11vnc|websockify|chromium.*$CHROME_PROFILE" | grep -v defunct || true
}

stop_desktop() {
  pkill -f "Xvfb $DISPLAY_ID" >/dev/null 2>&1 || true
  pkill -f "openbox" >/dev/null 2>&1 || true
  pkill -f "x11vnc.*$DISPLAY_ID" >/dev/null 2>&1 || true
  pkill -f "websockify.*$NOVNC_PORT" >/dev/null 2>&1 || true
  pkill -f "chromium.*$CHROME_PROFILE" >/dev/null 2>&1 || true
}

screenshot() {
  local path="${1:-/tmp/arroba-slice-screenshot.png}"
  scrot -z "$path"
  printf '%s\n' "$path"
}

focus_chromium() {
  local window
  window="$(xdotool search --onlyvisible --class chromium 2>/dev/null | tail -n 1 || true)"
  if [[ -n "$window" ]]; then
    xdotool windowactivate --sync "$window" >/dev/null 2>&1 || true
    xdotool windowfocus --sync "$window" >/dev/null 2>&1 || true
    sleep 0.1
  fi
}

click() {
  focus_chromium
  xdotool mousemove "$1" "$2" click 1
}

double_click() {
  focus_chromium
  xdotool mousemove "$1" "$2" click --repeat 2 --delay 80 1
}

drag() {
  focus_chromium
  xdotool mousemove "$1" "$2" mousedown 1 mousemove --sync "$3" "$4" mouseup 1
}

move_mouse() {
  xdotool mousemove "$1" "$2"
}

scroll() {
  local amount="${1:-1}"
  local button=5
  if [[ "$amount" =~ ^- ]]; then
    button=4
    amount="${amount#-}"
  fi
  local i
  for i in $(seq 1 "$amount"); do
    xdotool click "$button"
  done
}

type_text() {
  focus_chromium
  xdotool type --clearmodifiers --delay 5 "$*"
}

key() {
  focus_chromium
  xdotool key --clearmodifiers "$1"
}

clipboard_get() {
  xclip -selection clipboard -out 2>/dev/null || true
}

clipboard_set() {
  printf '%s' "$*" | xclip -selection clipboard -in
}

clipboard_clear() {
  printf '' | xclip -selection clipboard -in
}

paste_stdin() {
  focus_chromium
  local previous
  previous="$(clipboard_get || true)"
  xclip -selection clipboard -in >/dev/null
  xdotool key --clearmodifiers ctrl+v
  sleep 0.1
  printf '%s' "$previous" | xclip -selection clipboard -in
}

ocr() {
  local image="${1:-/tmp/arroba-slice-screenshot.png}"
  if [[ ! -f "$image" ]]; then
    screenshot "$image" >/dev/null
  fi
  tesseract "$image" stdout -l eng 2>/dev/null
}

find_text() {
  local query="$1"
  local image="${2:-/tmp/arroba-slice-screenshot.png}"
  local tsv="/tmp/arroba-slice-ocr.tsv"
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
  chromium --user-data-dir="$CHROME_PROFILE" --no-sandbox --new-window "$1" >/dev/null 2>&1 &
  sleep 1
  focus_chromium
}

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
  ocr) shift; ocr "$@" ;;
  find-text|find_text) shift; find_text "$@" ;;
  open-url|open_url) shift; open_url "$@" ;;
  *)
    cat >&2 <<EOF
Usage: $(basename "$0") start|stop|status|screenshot|click|double-click|drag|move|scroll|type|key|clipboard-get|clipboard-set|clipboard-clear|paste-stdin|ocr|find-text|open-url
EOF
    exit 2
    ;;
esac
