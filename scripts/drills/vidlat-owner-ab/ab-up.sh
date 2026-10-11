#!/bin/bash
# MP-08/MP-10/MP-11: public prebuilt runtimes only; no builds on the CCX43.
set -euo pipefail
umask 077
KIT=$(cd -- "$(dirname -- "$0")" && pwd)
: "${AB_STATE:?absolute disposable lane-owned state directory required}"
: "${AB_ORIGIN:?exact qualified validation origin required}"
# The enrolled Cloud profile supplies the relay. An explicit URL is only an assertion.
export AB_ORIGIN AB_RELAY_URL="${AB_RELAY_URL:-}"
export AB_API_ORIGIN="${AB_API_ORIGIN:-$AB_ORIGIN}"
: "${AB_SELKIES_ORIGIN:?valenv-gated Selkies origin required}"
: "${AB_GATEWAY_IP:?valenv gateway IPv4 required}"
export AB_KIT="$KIT" TMPDIR="$AB_STATE/tmp" NODE_OPTIONS=--max-old-space-size=4096
started=$(date +%s)
python3 "$KIT/ab-processes.py" preflight
test ! -e "$AB_STATE" || { echo 'MP-11: existing run; ab-down first' >&2; exit 2; }
mkdir -p "$TMPDIR" "$AB_STATE/logs" "$AB_STATE/runtime-libs"
printf 'vidlat-owner-ab\n' > "$AB_STATE/.vidlat-owned"
failure() { "$KIT/ab-down.sh" || true; }
trap failure ERR INT TERM
(cd "$KIT" && sha256sum --quiet -c SHA256SUMS)
node "$KIT/ab-auth-preflight.mjs"
if [ "${AB_SKIP_INSTALL:-0}" != 1 ]; then
  test "$(. /etc/os-release; printf '%s' "$ID:$VERSION_ID")" = ubuntu:26.04
  apt-get update > "$AB_STATE/logs/apt.private.log" 2>&1
  DEBIAN_FRONTEND=noninteractive apt-get install -y nodejs xvfb xauth x11-utils openbox xdotool curl bzip2 iptables libgbm1 libegl1 libgl1 libinput10 libudev1 libxkbcommon0 libva2 libx11-6 libxrandr2 libxtst6 libxfixes3 libxdamage1 libxcomposite1 libxext6 libpixman-1-0 libavcodec62 libavutil60 libavfilter11 libavformat62 libswresample6 libswscale9 libx264-165 pulseaudio-utils >> "$AB_STATE/logs/apt.private.log" 2>&1
  if ! command -v google-chrome >/dev/null; then
    curl --fail --location --max-time 120 https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb -o "$TMPDIR/chrome.deb"
    apt-get install -y "$TMPDIR/chrome.deb" >> "$AB_STATE/logs/apt.private.log" 2>&1
    rm "$TMPDIR/chrome.deb"
  fi
fi
# Cisco distribution condition: fetch directly, never redistribute its binary.
curl --fail --silent --show-error --location --max-time 120 https://ciscobinary.openh264.org/libopenh264-2.6.0-linux64.8.so.bz2 -o "$TMPDIR/openh264.bz2"
printf '%s  %s\n' 27ab53323c110b76214c1c72222f459d17febbcd1e252136cadc292b0308d75b "$TMPDIR/openh264.bz2" | sha256sum --quiet -c -
bzip2 -dc "$TMPDIR/openh264.bz2" > "$AB_STATE/runtime-libs/libopenh264.so.8"
rm "$TMPDIR/openh264.bz2"
# Relocate the prebuilt standalone Python, without pip or source compilation.
printf 'home = %s\ninclude-system-site-packages = false\nversion = 3.11.13\n' "$KIT/baseline/python/cpython-3.11.13-linux-x86_64-gnu/bin" > "$AB_STATE/pyvenv.cfg"
mkdir -p "$AB_STATE/python/bin" "$AB_STATE/python/lib"
cp "$AB_STATE/pyvenv.cfg" "$AB_STATE/python/pyvenv.cfg"
ln -s "$KIT/baseline/runtime-venv/bin/python" "$AB_STATE/python/bin/python"
ln -s "$KIT/baseline/runtime-venv/lib/python3.11" "$AB_STATE/python/lib/python3.11"
export LD_LIBRARY_PATH="$AB_STATE/runtime-libs:$KIT/runtime-libs:$KIT/baseline/runtime:$KIT/baseline/runtime-venv/lib/python3.11/site-packages/pixelflux.libs"
export CHARIOX_BROWSER_DISPLAY_OPENH264="$AB_STATE/runtime-libs/libopenh264.so.8"
"$AB_STATE/python/bin/python" -c 'import selkies,pixelflux,pcmflux' > "$AB_STATE/logs/import.private.log" 2>&1
nohup setsid node "$KIT/ab-supervisor.mjs" > "$AB_STATE/logs/supervisor.private.log" 2>&1 < /dev/null &
supervisor=$!
python3 "$KIT/ab-processes.py" record "$supervisor"
for n in $(seq 1 120); do
  test ! -f "$AB_STATE/FAILED" || { echo 'MP-10: runtime failed; inspect private log locally' >&2; exit 3; }
  if test -f "$AB_STATE/READY"; then break; fi
  sleep 1
done
test -f "$AB_STATE/READY"
node "$KIT/ab-control.mjs" health
elapsed=$(( $(date +%s) - started ))
test "$elapsed" -le 900 || { echo 'MP-10: startup exceeded 15 minutes' >&2; exit 4; }
printf 'MP-08/MP-10/MP-11: runtimes ready in %s seconds; human enrollment is next.\n' "$elapsed"
printf 'MP-08/MP-10: normal device enrollment: node %s/ab-control.mjs login\n' "$KIT"
trap - ERR INT TERM
