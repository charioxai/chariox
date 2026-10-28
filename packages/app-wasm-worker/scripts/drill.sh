#!/bin/sh
# Invoke through cargo-slot.sh; preserve the built source App before releasing
# the shared target. State and logs belong outside the checkout.
set -eu
: "${CARGO_TARGET_DIR:?run through cargo-slot.sh}"
: "${CHARIOX_WORKER_DRILL_DIR:?set an external evidence directory}"
mkdir -p "$CHARIOX_WORKER_DRILL_DIR"
cargo build --manifest-path packages/app-wasm-worker/tests/guest/Cargo.toml --target wasm32-unknown-unknown --release
export CHARIOX_WORKER_DRILL_CORE="$CHARIOX_WORKER_DRILL_DIR/drill.core.wasm"
trap 'rm -f "$CHARIOX_WORKER_DRILL_CORE"' EXIT
cp "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/chariox_app_worker_drill.wasm" "$CHARIOX_WORKER_DRILL_CORE"
cargo test -p chariox-app-wasm-worker --test process -- --ignored --nocapture
sha256sum "$CARGO_TARGET_DIR/debug/chariox-app-wasm-worker"
