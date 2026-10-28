#!/usr/bin/env bash
# MP-08/MP-10/MP-11 research only; run on b3 through the shared cargo slot.
# Requires rustc 1.99.1 + wasm32-wasip2 and wasi-sdk 33 in /var/tmp/texspike.
# Run prepare-guest.py first; this never builds or starts the product kernel.
set -euo pipefail
root=${TEX_SPIKE_ROOT:-/var/tmp/texspike}
sdk=$root/tools/wasi-sdk-33.0-x86_64-linux
fork=$root/tectonic
sysroot=$fork/wasi-deps/sysroot
export TMPDIR=/root/.chariox/dev/texspike/tmp CARGO_BUILD_JOBS=4
export CC_wasm32_wasip2=$sdk/bin/clang CXX_wasm32_wasip2=$sdk/bin/clang++ AR_wasm32_wasip2=$sdk/bin/llvm-ar
export CFLAGS_wasm32_wasip2="--target=wasm32-wasip2 --sysroot=$sdk/share/wasi-sysroot -isystem $fork/wasi-deps/sjlj-stub -I$sysroot/include -D_WASI_EMULATED_MMAN -D_WASI_EMULATED_SIGNAL"
export CXXFLAGS_wasm32_wasip2="$CFLAGS_wasm32_wasip2 -fno-exceptions"
export TECTONIC_DEP_BACKEND=manual
export FREETYPE2_INCLUDE_PATH="$sysroot/include/freetype2;$sysroot/include" FREETYPE2_LIB_DIR="$sysroot/lib"
export GRAPHITE2_INCLUDE_PATH="$sysroot/include" GRAPHITE2_LIB_DIR="$sysroot/lib"
export ICUUC_INCLUDE_PATH="$sysroot/include" ICUUC_LIB_DIR="$sysroot/lib"
export PNG_INCLUDE_PATH="$sysroot/include" PNG_LIB_DIR="$sysroot/lib"
export ZLIB_INCLUDE_PATH="$sysroot/include" ZLIB_LIB_DIR="$sysroot/lib"
export RUSTFLAGS="-L native=$sdk/share/wasi-sysroot/lib/wasm32-wasip2/noeh -L native=$sdk/share/wasi-sysroot/lib/wasm32-wasip2 -L native=$sysroot/lib -l static=c++ -l static=c++abi -l static=wasi-emulated-mman -l static=wasi-emulated-signal"
cd "$root/guest"
cargo build --release --locked --target wasm32-wasip2
mkdir -p "$root/artifacts"
cp "$CARGO_TARGET_DIR/wasm32-wasip2/release/texspike_guest.wasm" "$root/artifacts/app.wasm"
sha256sum "$root/artifacts/app.wasm"
