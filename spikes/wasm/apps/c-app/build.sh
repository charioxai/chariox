#!/usr/bin/env bash
# The C spike graduated to the R1 SDK; keep one C build/runtime path.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
exec bash "$here/../../../../packages/app-sdk-c/build.sh" "$@"
