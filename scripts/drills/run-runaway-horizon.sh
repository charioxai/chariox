#!/usr/bin/env bash
# Runs scripts/drills/runaway-horizon.mjs in a transient systemd unit that is a 24 GB machine (nothing it starts can use more
# than MemoryMax of this host; the hog cannot take b3). Row 541 round 2, cells R1-R5.
# usage: run-runaway-horizon.sh KERNEL_BINARY TAG [CELLS=R1,R2,R3,R4,R5]   (state and evidence under ~/.chariox/dev/orch15-admission541)
# Never run it against 20A7, 9AFB or the reviewer kernel: it starts its own kernel.
set -euo pipefail
kernel=${1:?kernel binary}; tag=${2:?tag}; cells=${3:-R1,R2,R3,R4,R5}
tree=$(cd "$(dirname "$0")/../.." && pwd)
base=/root/.chariox/dev/orch15-admission541
mkdir -p "$base/evidence/$tag"
systemd-run --unit="rh541-$tag" --collect --quiet --wait \
  -p "Delegate=cpu memory pids" -p DelegateSubgroup=supervisor -p "MemoryMax=24G" -p MemorySwapMax=0 \
  -p RuntimeMaxSec=1800 \
  /usr/bin/env -i PATH=/root/.local/bin:/usr/local/bin:/usr/bin:/bin CELLS="$cells" \
  /usr/bin/node "$tree/scripts/drills/runaway-horizon.mjs" \
  --kernel-binary "$kernel" --label "$tag" --tag "$tag" --state-root "$base/state-$tag" --evidence-dir "$base/evidence/$tag" \
  --head "$(git -C "$tree" rev-parse HEAD)"
echo "rh541-$tag.service $base/evidence/$tag"
