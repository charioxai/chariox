#!/usr/bin/env bash
# MP-08 / MP-09 / MP-10 / MP-11. Run serially; an unmet containment guarantee
# returns 1 after every renderer/check has written its evidence and cleaned up.
set -euo pipefail
if [[ $# != 3 ]]; then
  echo 'usage: bash packages/app-view-host/test/run-av1.sh <paired Cloud checkout> <new evidence directory> <new scratch directory>' >&2
  exit 2
fi
oss_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
cloud_root=$(realpath "$1")
evidence=$(realpath -m "$2")
scratch=$(realpath -m "$3")
if (( ${#scratch} + 4 > 60 )); then
  echo 'Choose a shorter scratch path, such as ~/.chariox/dev/appview-av1: Chrome needs a short TMPDIR for its Unix socket' >&2
  exit 2
fi
for directory in "$evidence" "$scratch"; do
  case "$directory/" in "$oss_root/"*|"$cloud_root/"*) echo 'Evidence and scratch must be outside both checkouts' >&2; exit 2;; esac
  if [[ -e "$directory" ]]; then echo "Use a new directory: $directory" >&2; exit 2; fi
done
mkdir -p "$evidence" "$scratch/tmp"
export CHARIOX_OSS_ROOT="$oss_root" CHARIOX_APP_VIEW_EVIDENCE="$evidence" CHARIOX_APP_VIEW_BUILDS="$scratch/builds"
export TMPDIR="$scratch/tmp" NODE_OPTIONS=--max-old-space-size=3072 CI=1
boundrun=${CHARIOX_BOUNDRUN:-/root/.chariox/dev/lanes-miguel-r28/agents/boundrun.sh}
if [[ ! -x "$boundrun" ]]; then echo 'Set CHARIOX_BOUNDRUN to the bounded Node/Chrome workload wrapper' >&2; exit 2; fi
step() {
  local name=$1 code=0
  shift
  "$boundrun" "$@" >"$evidence/$name.log" 2>&1 || code=$?
  printf '%s\t%s\n' "$name" "$code" >>"$evidence/steps.tsv"
  if [[ $code != 0 ]]; then tail -n 25 "$evidence/$name.log"; return "$code"; fi
}
{
  git -C "$oss_root" rev-parse HEAD
  git -C "$oss_root" status --porcelain
  git -C "$cloud_root" rev-parse HEAD
  git -C "$cloud_root" status --porcelain
  df -h /
  free -h
} >"$evidence/source-and-resources.txt"
cd "$oss_root"
step install-excalidraw corepack pnpm --dir examples/apps/excalidraw install --ignore-workspace --frozen-lockfile --ignore-scripts
step install-univer corepack pnpm --dir examples/apps/univer install --ignore-workspace --frozen-lockfile --ignore-scripts --config.auto-install-peers=false
step verifier node --test --test-concurrency=1 packages/app-view-host/test/*.test.mjs
# The installs skip lifecycle scripts, but the Univer licence inventory hashes esbuild with its native binary in place.
step install-univer-esbuild sh -c 'cd examples/apps/univer && node "$(node -p "require.resolve(\"esbuild/install.js\")")"'
step build-excalidraw node examples/apps/excalidraw/scripts/build-view.mjs "$scratch/builds/excalidraw" --client-view
step build-univer node examples/apps/univer/scripts/build-view.mjs "$scratch/builds/univer" --client-view
cd "$cloud_root"
step install-cloud corepack pnpm install --frozen-lockfile --ignore-scripts
step check-shared-host node apps/web/scripts/sync-app-view-host.mjs --check
step host-response node --test apps/web/scripts/app-view-host-response.test.mjs
step build-web corepack pnpm --workspace-concurrency=1 --filter @chariox-cloud/web... run build
step browser node apps/web/scripts/app-view-browser-drill.mjs
