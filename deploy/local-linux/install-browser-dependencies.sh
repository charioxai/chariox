#!/usr/bin/env bash
# MP-08/MP-10/MP-11: the native keyboard channel uses /usr/bin/python3; the kernel's
# owned desktop (linux-owned-desktop.mjs) needs these system executables.
set -euo pipefail
root=${CHARIOX_LOCAL_INSTALL_ROOT:-}
python=$root/usr/bin/python3
apt=$root/usr/bin/apt-get
desktop=(Xvfb openbox bwrap dbus-daemon setxkbmap)
probe() {
  for name in "${desktop[@]}"; do [[ -x $root/usr/bin/$name ]] || return 1; done
  "$python" -I -B -c 'from Xlib import X, display; from Xlib.ext import xtest; from PIL import Image' >/dev/null 2>&1
}
if probe; then
  printf '[chariox-install] unchanged system browser dependencies\n'
  exit 0
fi
if [[ ! -x "$apt" ]]; then
  printf '[chariox-install] browser dependencies missing: install Python Xlib and Pillow for /usr/bin/python3 and %s, then re-run Setup\n' "${desktop[*]}" >&2
  exit 1
fi
printf '[chariox-install] install system browser dependencies\n'
DEBIAN_FRONTEND=noninteractive "$apt" update
DEBIAN_FRONTEND=noninteractive "$apt" install -y --no-install-recommends python3-xlib python3-pil xvfb openbox bubblewrap dbus-daemon x11-xkb-utils
probe || { printf '[chariox-install] browser dependencies still missing after installation\n' >&2; exit 1; }
