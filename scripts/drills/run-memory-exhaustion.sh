#!/usr/bin/env bash
# Runs scripts/drills/memory-exhaustion.mjs inside a transient systemd scope that is the emulated 8 GB machine
# (outer safety net: nothing it starts can use more than 8 GiB of this host).
# KERNEL_MEMORY_MIN=2G sets the scope's MemoryMin the way the shipped unit does; unset, the kernel must raise its own protection (M4b).
# usage: run-memory-exhaustion.sh KERNEL_BINARY TAG [CELLS=M1,M2,M3,M6]   (state and evidence under ~/.chariox/dev/memceiling)
# Never run it against 20A7, 9AFB or the reviewer kernel: it starts its own kernel.
set -euo pipefail
kernel=${1:?kernel binary}; tag=${2:?tag}; cells=${3:-M1,M2,M3,M6}
tree=$(cd "$(dirname "$0")/../.." && pwd)
base=/root/.chariox/dev/memceiling
mkdir -p "$base/evidence/$tag"
systemd-run --unit="memceil-$tag" --collect --quiet \
  -p "Delegate=cpu memory pids" -p DelegateSubgroup=supervisor -p MemoryMax=8G -p MemorySwapMax=0 ${KERNEL_MEMORY_MIN:+-p MemoryMin=$KERNEL_MEMORY_MIN} \
  -p RuntimeMaxSec=1800 \
  /usr/bin/env -i PATH=/usr/local/bin:/usr/bin:/bin CELLS="$cells" STRICT_M4="${STRICT_M4:-1}" KERNEL_MEMORY_MIN="${KERNEL_MEMORY_MIN:-}" \
  /usr/bin/node "$tree/scripts/drills/memory-exhaustion.mjs" \
  --kernel-binary "$kernel" --label "$tag" --tag "$tag" --state-root "$base/state-$tag" --evidence-dir "$base/evidence/$tag" \
  --budget-high-gib "${BUDGET_HIGH_GIB:-6.5}" --head "$(git -C "$tree" rev-parse HEAD)"
echo "memceil-$tag.service $base/evidence/$tag"
