#!/bin/sh
# MP-08/MP-10/MP-11: check the loader and libraries in the actual slice runtime.
set -eu
runtime_bin=${1:-/opt/chariox-slice/bin}
if ! "$runtime_bin/chariox-kernel" --print-local-daemon-protocol-version >/dev/null \
    || ! "$runtime_bin/chariox-relay" --help >/dev/null; then
  echo 'Slice runtime binaries cannot start with this image libc/libraries. Rebuild the slice image from source without CHARIOX_PREBUILT_RUNTIME=1, or supply binaries built for Debian 12 (glibc 2.36) and its runtime libraries.' >&2
  exit 1
fi
