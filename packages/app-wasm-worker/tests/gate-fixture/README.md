# Workflow gate fixture

This Rust guest exports the import-free `chariox:workflow-gate/gate@1.0.0`
world. It includes adversarial modes for the contained-worker regression; it is
not an advisory policy package.

Run from the repository root, through the shared Cargo slot wrapper. Keep
`TMPDIR` and the generated components outside the checkout:

```sh
cargo build --manifest-path packages/app-wasm-worker/tests/gate-fixture/Cargo.toml \
  --target wasm32-unknown-unknown --release
cp "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/workflow_gate_fixture.wasm" \
  "$TMPDIR/gate.core.wasm"
cargo run -p chariox-app-sdk-tools --bin chariox-gate-component -- \
  "$TMPDIR/gate.core.wasm" "$TMPDIR/gate.wasm"
cargo build --manifest-path examples/workflow-gates/advisory/Cargo.toml \
  --target wasm32-unknown-unknown --release
cargo run -p chariox-app-sdk-tools --bin chariox-gate-component -- \
  "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/advisory_entry_gate.wasm" "$TMPDIR/advisory-gate.wasm"
CHARIOX_GATE_COMPONENT="$TMPDIR/gate.wasm" \
CHARIOX_ADVISORY_GATE_COMPONENT="$TMPDIR/advisory-gate.wasm" cargo test \
  -p chariox-app-wasm-worker --features gate-drills --test gate-drill
```

The supervisor uses the verified native worker executable and its ordinary
owned-PID cancellation/reaping path. Validation and evaluation enter the same
mandatory R5 containment before compilation or Wasmtime threads. The gate
profile separately meters fuel, memory, tables, stack, output lifting and time;
it does not change the App profile or link App/WASI capabilities into a gate.
