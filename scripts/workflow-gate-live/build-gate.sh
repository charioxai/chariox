#!/bin/bash
# Build the matrix gate component into $WFGATE_SCRATCH; run through the build-slot wrapper.
set -e
R=${WFGATE_SCRATCH:-/root/.chariox/dev/wfgate-r1}
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/../.."
cargo build --manifest-path "$here/matrix-gate/Cargo.toml" --release --target wasm32-unknown-unknown --target-dir "$CARGO_TARGET_DIR/wfgate-matrix"
cargo run -q -p chariox-app-sdk-tools --bin chariox-gate-component -- "$CARGO_TARGET_DIR/wfgate-matrix/wasm32-unknown-unknown/release/matrix_entry_gate.wasm" "$R/matrix-gate.wasm"
sha256sum "$R/matrix-gate.wasm"
