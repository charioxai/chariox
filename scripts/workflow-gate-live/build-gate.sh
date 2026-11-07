#!/bin/bash
set -e
export RUSTUP_TOOLCHAIN=1.99.1
cd /root/work/agent-wfgate
cargo build --manifest-path /root/.chariox/dev/wfgate-r1/matrix-gate/Cargo.toml --release --target wasm32-unknown-unknown --target-dir "$CARGO_TARGET_DIR/wfgate-matrix"
cargo run -q -p chariox-app-sdk-tools --bin chariox-gate-component -- "$CARGO_TARGET_DIR/wfgate-matrix/wasm32-unknown-unknown/release/matrix_entry_gate.wasm" /root/.chariox/dev/wfgate-r1/matrix-gate.wasm
sha256sum /root/.chariox/dev/wfgate-r1/matrix-gate.wasm
