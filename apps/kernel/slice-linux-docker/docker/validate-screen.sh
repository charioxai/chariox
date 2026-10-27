#!/usr/bin/env bash
set -Eeuo pipefail

SCREEN="${CHARIOX_SLICE_ROOT:-/opt/chariox-slice}/slice-screen.sh"
CDP="${CHARIOX_SLICE_ROOT:-/opt/chariox-slice}/browser-cdp.mjs"
TEST_PAGE="/tmp/chariox-slice-screen-test.html"
SHOT="/tmp/chariox-slice-screen-test.png"
PROFILE="/tmp/chariox-slice-chromium-validation"

find_or_retry() {
  local text="$1"
  local output="$2"
  local attempt
  for attempt in $(seq 1 10); do
    "$SCREEN" screenshot "$SHOT" >/dev/null
    if "$SCREEN" find-text "$text" "$SHOT" >"$output"; then
      return
    fi
    sleep 0.5
  done
  "$SCREEN" ocr "$SHOT" >&2 || true
  return 1
}

write_page() {
  cat >"$TEST_PAGE" <<'HTML'
<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <title>Chariox Slice Screen Test</title>
    <style>
      body { margin: 48px; font: 26px Arial, sans-serif; background: white; color: #111; }
      #status { margin-top: 32px; font-size: 34px; font-weight: 700; }
    </style>
  </head>
  <body>
    <h1>CHARIOX SLICE OCR READY</h1>
    <input id="input" autofocus placeholder="type here">
    <button id="button">PRESS SLICE BUTTON</button>
    <div id="status">WAITING FOR INPUT</div>
    <script>
      const input = document.getElementById("input");
      const status = document.getElementById("status");
      input.addEventListener("input", () => {
        status.textContent = "TYPED " + input.value.toUpperCase();
      });
      document.getElementById("button").addEventListener("click", () => {
        alert("CHARIOX SLICE DIALOG READY");
        status.textContent = "DIALOG ACCEPTED";
      });
      setTimeout(() => input.focus(), 300);
    </script>
  </body>
</html>
HTML
}

prepare() {
  rm -rf "$PROFILE"
  write_page
  CHARIOX_SLICE_CHROME_PROFILE="$PROFILE" \
  CHARIOX_SLICE_CHROME_URL="file://$TEST_PAGE" \
    "$SCREEN" start >/tmp/chariox-slice-screen-start.out
  sleep 3
  find_or_retry "CHARIOX SLICE OCR READY" /tmp/chariox-slice-find-heading.json
  printf 'ocr_heading=%s\n' "$(cat /tmp/chariox-slice-find-heading.json)"
  printf 'browser_opened_url=ok\n'
}

interact() {
"$CDP" click 117 170
sleep 0.3
"$CDP" type "sliceprobe"
"$CDP" key Shift
if ! "$CDP" text | grep -q "SLICEPROBE"; then
  "$SCREEN" ocr "$SHOT" >&2 || true
  exit 1
fi
printf 'browser_input=SLICEPROBE\n'

  "$CDP" click-selector "#button" >/dev/null
  "$CDP" cursor-status | grep -q '"visible":true'
  printf 'agent_cursor=visible\n'
  "$SCREEN" browser-dialog accept | grep -q '"ok":true'
  "$CDP" wait-text "DIALOG ACCEPTED" 5000 | grep -q '"ok":true'
  printf 'browser_dialog=accepted\n'

  "$SCREEN" clipboard-set "slice clipboard ok"
  clipboard="$("$SCREEN" clipboard-get)"
  [[ "$clipboard" == "slice clipboard ok" ]]
  printf 'clipboard=%s\n' "$clipboard"

  "$SCREEN" status | sed -n '1,12p'
}

case "${1:-full}" in
  prepare) prepare ;;
  interact) interact ;;
  full)
    prepare
    printf 'run_interact_in_separate_exec=required\n'
    ;;
  *)
    printf 'Usage: %s [prepare|interact|full]\n' "$(basename "$0")" >&2
    exit 2
    ;;
esac
