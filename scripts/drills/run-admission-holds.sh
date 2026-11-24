#!/usr/bin/env bash
# Runs scripts/drills/admission-holds.mjs in a transient systemd unit that is the emulated 16 GB machine
# (nothing it starts can use more than MemoryMax of this host). Row 541 cells A1-A6.
# usage: run-admission-holds.sh KERNEL_BINARY TAG [CELLS=A6,A1,A3,A2,A4,A5]   (state and evidence under ~/.chariox/dev/orch15-admission541)
# Never run it against 20A7, 9AFB or the reviewer kernel: it starts its own kernel.
set -euo pipefail
kernel=${1:?kernel binary}; tag=${2:?tag}; cells=${3:-A6,A1,A3,A2,A4,A5}
tree=$(cd "$(dirname "$0")/../.." && pwd)
base=/root/.chariox/dev/orch15-admission541
mkdir -p "$base/evidence/$tag"
systemd-run --unit="adm541-$tag" --collect --quiet --wait \
  -p "Delegate=cpu memory pids" -p DelegateSubgroup=supervisor -p "MemoryMax=${MEMORY_MAX:-16G}" -p MemorySwapMax=0 \
  -p RuntimeMaxSec=2400 \
  /usr/bin/env -i PATH=/usr/local/bin:/usr/bin:/bin CELLS="$cells" \
  /usr/bin/node "$tree/scripts/drills/admission-holds.mjs" \
  --kernel-binary "$kernel" --label "$tag" --tag "$tag" --state-root "$base/state-$tag" --evidence-dir "$base/evidence/$tag" \
  --budget-high-gib "${BUDGET_HIGH_GIB:-12}" --heavy-gib "${HEAVY_GIB:-6}" --hog-gib "${HOG_GIB:-11}" --head "$(git -C "$tree" rev-parse HEAD)"
echo "adm541-$tag.service $base/evidence/$tag"
