#!/bin/sh
# MP-08/MP-10/MP-11: signed installed actions reach A2.
# Run through cargo-slot.sh; keep generated artifacts outside the checkout.
set -eu
: "${CARGO_TARGET_DIR:?use cargo-slot.sh}"
: "${CHARIOX_WORKER_DRILL_DIR:?set an external drill directory}"
mkdir -p "$CHARIOX_WORKER_DRILL_DIR/bin"
cargo build -p chariox-app-wasm-worker
cargo build --manifest-path packages/app-wasm-worker/tests/guest/Cargo.toml --target wasm32-unknown-unknown --release
export CHARIOX_WORKER_DRILL_CORE="$CHARIOX_WORKER_DRILL_DIR/install.core.wasm"
cp "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/chariox_app_worker_drill.wasm" "$CHARIOX_WORKER_DRILL_CORE"
cargo test -p chariox-kernel --lib --no-run --message-format=json > "$CHARIOX_WORKER_DRILL_DIR/kernel-tests.jsonl"
# Keep the executable and its native worker together before releasing the slot.
python3 - "$CHARIOX_WORKER_DRILL_DIR" "$CARGO_TARGET_DIR" <<'PY'
import json
import pathlib
import shutil
import sys
root, target = map(pathlib.Path, sys.argv[1:])
artifacts = [json.loads(line) for line in (root / "kernel-tests.jsonl").read_text().splitlines()]
executables = [entry["executable"] for entry in artifacts
               if entry.get("reason") == "compiler-artifact"
               and entry.get("target", {}).get("name") == "chariox_kernel"
               and entry.get("profile", {}).get("test") and entry.get("executable")]
if len(executables) != 1:
    raise SystemExit("expected exactly one kernel library test executable")
shutil.copy2(executables[0], root / "bin/kernel-tests")
worker = root / "bin/chariox-app-wasm-worker"
worker.unlink(missing_ok=True)
shutil.copy2(target / "debug/chariox-app-wasm-worker", worker)
worker.chmod(0o500)
PY
"$CHARIOX_WORKER_DRILL_DIR/bin/kernel-tests" signed_installed_ui_actions_reach_the_real_view_context --ignored --nocapture --test-threads=1
