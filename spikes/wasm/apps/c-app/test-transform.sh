#!/usr/bin/env bash
# Needs wit-bindgen 0.63 and native clang with ASan/UBSan; no WASI SDK needed.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/c-transform-test.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
wit-bindgen c "$here/../../wit" --world app --out-dir "$scratch" >/dev/null
clang -O1 -g -fsanitize=address,undefined -fno-omit-frame-pointer \
  -Wall -Wextra -Wno-unused-parameter -Wno-unused-function -I"$scratch" \
  "$here/test-transform.c" -o "$scratch/test-transform"
# Nonzero fill makes the missing terminator reproducible even on fresh heaps.
ASAN_OPTIONS=malloc_fill_byte=190:detect_leaks=1 "$scratch/test-transform"
