#!/bin/sh
set -eu
: "${CARGO_TARGET_DIR:?run through cargo-slot.sh}"
: "${CHARIOX_WORKER_DRILL_DIR:?set an external evidence directory}"
for guest in wasi migration; do
    cargo build --manifest-path "packages/app-wasm-worker/tests/$guest-guest/Cargo.toml" --target wasm32-unknown-unknown --release
    cp "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/chariox_app_${guest}_drill.wasm" "$CHARIOX_WORKER_DRILL_DIR/$guest.core.wasm"
done
cargo test -p chariox-app-wasm-worker --test wasi -- --ignored --nocapture
