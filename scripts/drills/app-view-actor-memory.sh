#!/bin/sh
# MP-08 / MP-09 / MP-10 / MP-11: an explicitly provisioned Linux memory drill.
# Build the kernel lib tests first; pass that test executable as the first argument.
set -eu
binary=${1:?pass the prebuilt kernel test executable}
shift
[ -x "$binary" ] || { echo 'kernel test executable is not executable' >&2; exit 2; }
[ -d /run/systemd/system ] || { echo 'this drill requires a running systemd manager' >&2; exit 2; }
[ "$#" -gt 0 ] || set -- mp08_a3_
export NODE_OPTIONS=--max-old-space-size=3072
exec systemd-run --scope --quiet -p MemoryMax=4G -p MemorySwapMax=0 \
  sh -c 'echo 0 > /proc/self/oom_score_adj; exec "$@"' sh \
  "$binary" --test-threads=1 "$@"
