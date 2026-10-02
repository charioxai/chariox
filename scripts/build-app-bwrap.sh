#!/usr/bin/env bash
# Build the pinned App sandbox with the attested fail-closed ENOSYS fallback.
set -euo pipefail
repo=$(cd -- "$(dirname -- "$0")/.." && pwd)
scratch=$1
archive=$2
[[ "$scratch" = /* && ! -e "$scratch" ]]
mapfile -t pin < <(node -e 'const p=require(process.argv[1]).bubblewrap; console.log(p.sha256); console.log(p.size)' "$repo/apps/app-worker/sandbox.lock.json")
[[ "$(stat -c %s "$archive")" == "${pin[1]}" ]]
printf '%s  %s\n' "${pin[0]}" "$archive" | sha256sum --check --status
mkdir -p "$scratch/source"
tar -xJf "$archive" --strip-components=1 -C "$scratch/source"
patch --batch --fuzz=0 -d "$scratch/source" -p1 < "$repo/apps/app-worker/bubblewrap-openat.patch"
cp "$repo/apps/app-worker/src/bwrap_openat_fallback.h" "$scratch/source/"
meson setup "$scratch/build" "$scratch/source" --buildtype=release --wrap-mode=nodownload \
  -Dselinux=disabled -Dman=disabled -Dtests=false -Dbash_completion=disabled \
  -Dzsh_completion=disabled -Dassume_kernel=5.9.0
meson compile -C "$scratch/build" -j 1
