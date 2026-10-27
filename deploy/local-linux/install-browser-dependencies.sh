#!/usr/bin/env bash
# MP-08/MP-10/MP-11: the native keyboard channel uses /usr/bin/python3; the kernel's
# owned desktop (linux-owned-desktop.mjs) needs these system executables; the pinned
# kernel Chromium (kernel-browser-chromium.lock.json) needs its own shared libraries,
# which a fresh server image lacks (it fails with "libatk-1.0.so.0: cannot open").
set -euo pipefail
root=${CHARIOX_LOCAL_INSTALL_ROOT:-}
python=$root/usr/bin/python3
apt=$root/usr/bin/apt-get
aptcache=$root/usr/bin/apt-cache
ldconfig=${CHARIOX_LOCAL_INSTALL_LDCONFIG:-ldconfig}
desktop=(Xvfb openbox bwrap dbus-daemon setxkbmap)
# Chrome for Testing's runtime libraries (the Playwright/Puppeteer Debian set). Ubuntu 24.04+
# renamed several with a t64 suffix: resolve_package picks whichever the archive has.
chromium_packages=(libasound2 libatk-bridge2.0-0 libatk1.0-0 libatspi2.0-0 libcairo2 libcups2 libdbus-1-3 libdrm2 libgbm1 libglib2.0-0 libnspr4 libnss3 libpango-1.0-0 libx11-6 libxcb1 libxcomposite1 libxdamage1 libxext6 libxtst6 libxfixes3 libxkbcommon0 libxrandr2)
chromium_sonames=(libasound.so.2 libatk-1.0.so.0 libatk-bridge-2.0.so.0 libatspi.so.0 libcups.so.2 libgbm.so.1 libnspr4.so libnss3.so libxkbcommon.so.0 libXcomposite.so.1 libXdamage.so.1 libXrandr.so.2 libX11.so.6 libXext.so.6 libXtst.so.6)
probe() {
  for name in "${desktop[@]}"; do [[ -x $root/usr/bin/$name ]] || return 1; done
  "$python" -I -B -c 'from Xlib import X, display; from Xlib.ext import xtest; from PIL import Image; import gi, pyatspi' >/dev/null 2>&1
}
probe_libraries() {
  local cache; cache=$("$ldconfig" -p 2>/dev/null) || return 1
  for name in "${chromium_sonames[@]}"; do grep -qF -- "$name" <<<"$cache" || return 1; done
}
resolve_package() {
  if "$aptcache" show "$1t64" >/dev/null 2>&1; then printf '%st64\n' "$1"; else printf '%s\n' "$1"; fi
}
if probe && probe_libraries; then
  printf '[chariox-install] unchanged system browser dependencies\n'
  exit 0
fi
if [[ ! -x "$apt" ]]; then
  printf '[chariox-install] browser dependencies missing: install Python Xlib and Pillow for /usr/bin/python3, %s and the Chromium libraries %s, then re-run Setup\n' "${desktop[*]}" "${chromium_packages[*]}" >&2
  exit 1
fi
printf '[chariox-install] install system browser dependencies\n'
# A fresh image has empty package lists: only after the update can apt-cache see the t64 names.
DEBIAN_FRONTEND=noninteractive "$apt" update
libraries=()
for name in "${chromium_packages[@]}"; do libraries+=("$(resolve_package "$name")"); done
DEBIAN_FRONTEND=noninteractive "$apt" install -y --no-install-recommends python3-xlib python3-pil python3-gi python3-pyatspi xvfb openbox bubblewrap dbus-daemon x11-xkb-utils "${libraries[@]}"
probe && probe_libraries || { printf '[chariox-install] browser dependencies still missing after installation\n' >&2; exit 1; }
