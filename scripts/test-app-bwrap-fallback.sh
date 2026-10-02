#!/usr/bin/env bash
# Takes the source/build scratch created by build-app-bwrap.sh.
set -euo pipefail
repo=$(cd -- "$(dirname -- "$0")/.." && pwd)
scratch=$1
cc=${CC:-cc}
common=(-U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0 -D_GNU_SOURCE -std=gnu11 -Wall -Wextra -Werror -O2 -ffunction-sections -fdata-sections -I "$scratch/source" -I "$scratch/build" -I "$repo/apps/app-worker/src")
"$cc" "${common[@]}" -Dsyscall=cx_test_syscall -Dopenat=cx_test_openat -c "$scratch/source/safe_openat.c" -o "$scratch/test-safe-openat.o"
"$cc" "${common[@]}" -c "$scratch/source/utils.c" -o "$scratch/test-utils.o"
"$cc" "${common[@]}" "$repo/apps/app-worker/tests/bwrap-openat-fallback.c" "$scratch/test-safe-openat.o" "$scratch/test-utils.o" -Wl,--gc-sections -lcap -o "$scratch/test-fallback"
"$scratch/test-fallback"
