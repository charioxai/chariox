#!/usr/bin/env bash
# MP-08/MP-10/MP-11: the native keyboard channel uses /usr/bin/python3.
set -euo pipefail
root=${CHARIOX_LOCAL_INSTALL_ROOT:-}
python=$root/usr/bin/python3
apt=$root/usr/bin/apt-get
probe() {
  "$python" -I -B -c 'from Xlib import X, display; from Xlib.ext import xtest; from PIL import Image' >/dev/null 2>&1
}
if probe; then
  printf '[chariox-install] unchanged system Python browser dependencies\n'
  exit 0
fi
if [[ ! -x "$apt" ]]; then
  printf '[chariox-install] browser dependencies missing: install Python Xlib and Pillow for /usr/bin/python3, then re-run Setup\n' >&2
  exit 1
fi
printf '[chariox-install] install system Python browser dependencies\n'
DEBIAN_FRONTEND=noninteractive "$apt" update
DEBIAN_FRONTEND=noninteractive "$apt" install -y --no-install-recommends python3-xlib python3-pil
probe || { printf '[chariox-install] browser dependencies cannot be imported by /usr/bin/python3 after installation\n' >&2; exit 1; }
