#!/bin/sh
set -eu
: "${CARGO_TARGET_DIR:?run through cargo-slot.sh}"
: "${CHARIOX_WORKER_DRILL_DIR:?set an external evidence directory}"
mkdir -p "$CHARIOX_WORKER_DRILL_DIR"
cargo build --manifest-path packages/app-wasm-worker/tests/wasi-guest/Cargo.toml --target wasm32-unknown-unknown --release
cp "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/chariox_app_wasi_drill.wasm" "$CHARIOX_WORKER_DRILL_DIR/wasi.core.wasm"
cargo test -p chariox-kernel --lib wasi_http_policy_accepts -- --ignored --nocapture
