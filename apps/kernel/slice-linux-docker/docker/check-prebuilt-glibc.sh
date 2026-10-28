#!/usr/bin/env bash
# A prebuilt kernel or relay runs inside the Debian 12 slice, so it may not
# need a newer glibc than the image provides. Binaries built on a newer host
# (Ubuntu 24.04 links glibc 2.38/2.39) fail at start; fail the image build
# instead, naming the supported ways to get compatible binaries.
set -euo pipefail

floor="${CHARIOX_GLIBC_FLOOR:-$(getconf GNU_LIBC_VERSION | awk '{print $2}')}"
rejected=0
for binary in "$@"; do
  required="$(objdump -T "$binary" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -Vu | tail -n 1)"
  if [[ -n "$required" && "$(printf '%s\n%s\n' "$required" "$floor" | sort -V | tail -n 1)" != "$floor" ]]; then
    echo "prebuilt $(basename "$binary") needs glibc $required but the slice image has glibc $floor" >&2
    rejected=1
  fi
done
if [[ "$rejected" != 0 ]]; then
  echo "Build the binaries inside the slice builder (docker build --target managed-release-artifacts)," >&2
  echo "or remove apps/kernel/slice-linux-docker/prebuilt/.managed-release to build in the image." >&2
  exit 1
fi
