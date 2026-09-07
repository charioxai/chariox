#!/usr/bin/env bash
# Packaging follows the pinned upstream scripts/ci/build-web.sh.
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
set -Eeuo pipefail

architecture="${1:?expected amd64 or arm64}"
cd /build
node fetch-selkies.mjs licenses /build/inputs
node fetch-selkies.mjs runtime /out/wheels "$architecture"
mkdir -p /build/source /out/licenses
tar -xzf /build/inputs/selkies.tar.gz --strip-components=1 -C /build/source
cp /build/locks/selkies-web-core.package-lock.json /build/source/addons/selkies-web-core/package-lock.json
cp /build/locks/selkies-dashboard.package-lock.json /build/source/addons/selkies-dashboard/package-lock.json

cd /build/source/addons/selkies-web-core
npm ci --no-audit --no-fund
npm run build
cd /build/source/addons/selkies-dashboard
npm ci --no-audit --no-fund
SELKIES_INJECT=1 npm run build

cd /build/source
mkdir -p addons/selkies-dashboard/dist/src src/selkies/selkies_web
cp addons/selkies-web-core/dist/selkies-core.js addons/selkies-dashboard/dist/src/
cp addons/universal-touch-gamepad/universalTouchGamepad.js addons/selkies-dashboard/dist/src/
cp -a addons/selkies-dashboard/dist/. src/selkies/selkies_web/
printf '%s\n' '"""Bundled Selkies web client."""' > src/selkies/selkies_web/__init__.py
printf '%s\n' '{"name":"Selkies","short_name":"Selkies","display":"fullscreen","background_color":"#000000","theme_color":"#000000","icons":[{"src":"icon-512.png","type":"image/png","sizes":"512x512"}],"start_url":"."}' > src/selkies/selkies_web/manifest.json
cp docs/assets/logo/icon-512x512.png src/selkies/selkies_web/icon-512.png
cp docs/assets/logo/favicon.ico src/selkies/selkies_web/favicon.ico

python3 -m venv /build/python
/build/python/bin/pip install --no-cache-dir build==1.3.0 setuptools==80.9.0 wheel==0.45.1
if ! /build/python/bin/python -m build --wheel --no-isolation --outdir /out > /build/wheel-build.log 2>&1; then
  tail -n 80 /build/wheel-build.log >&2
  exit 1
fi
printf 'Built pinned Selkies wheel\n'
cp /build/selkies.lock.json /out/licenses/sources.json
cp LICENSE /out/licenses/selkies-MPL-2.0.txt
cp src/selkies/Xlib/LICENSE /out/licenses/python-xlib-LGPL-3.0.txt
for project in pixelflux pcmflux; do
  revision="$(node -p "JSON.parse(require('fs').readFileSync('/build/selkies.lock.json')).$project.revision")"
  tar -xOf "/build/inputs/$project.tar.gz" "$project-$revision/LICENSE" > "/out/licenses/$project-MPL-2.0.txt"
  tar -xOf "/build/inputs/$project.tar.gz" "$project-$revision/LICENSES.md" > "/out/licenses/$project-dependencies.md"
done
