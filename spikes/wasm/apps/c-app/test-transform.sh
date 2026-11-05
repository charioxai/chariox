#!/usr/bin/env bash
# R0's transform regression now runs through the SDK's actual exported handlers.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
exec bash "$here/../../../../packages/app-sdk-c/test-native.sh" "$@"
