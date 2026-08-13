#!/usr/bin/env bash
set -euo pipefail

image="${1:?usage: verify-image-layout.sh <image>}"
docker run --rm --entrypoint /bin/sh "${image}" -c '
  test "$(id -u)" = "10001"
  test -d /var/lib/chariox/aegs
  test -w /var/lib/chariox/aegs
  test ! -e /var/lib/chariox/aeds
'
