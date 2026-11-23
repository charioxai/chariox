#!/usr/bin/env bash
set -euo pipefail

image="${1:?usage: verify-image-layout.sh <image>}"
owner_helper="$(dirname "${BASH_SOURCE[0]}")/../../scripts/lib/drill-docker-owner.mjs"
label_lines="$(node "$owner_helper" labels verify-aegs-dummy-image "$$")"
labels=()
while IFS= read -r label; do labels+=("$label"); done <<< "$label_lines"
docker run --rm "${labels[@]}" --entrypoint /bin/sh "${image}" -c '
  test "$(id -u)" = "10001"
  test -d /var/lib/chariox/aegs
  test -w /var/lib/chariox/aegs
  test ! -e /var/lib/chariox/aeds
'
