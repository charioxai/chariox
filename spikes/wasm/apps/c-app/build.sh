#!/usr/bin/env bash
# Builds the C App component: wit-bindgen c (callback ABI) + wasi-sdk 33 clang.
# usage: build.sh <out.wasm>   (needs wit-bindgen and wasm32-wasip2-clang on PATH)
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
gen=$(mktemp -d "${TMPDIR:-/tmp}/c-app-gen.XXXXXX")
trap 'rm -rf "$gen"' EXIT
wit-bindgen c "$here/../../wit" --world app --out-dir "$gen" >/dev/null
wasm32-wasip2-clang -O2 -Wall -Wno-unused-function -mexec-model=reactor -I"$gen" \
  -o "$1" "$here/app.c" "$gen/app.c" "$gen/app_component_type.o"
