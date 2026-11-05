#!/usr/bin/env bash
# Run inside the repository's Cargo slot on shared builders.
set -euo pipefail
UNIVER_ROOT=$(cd "$(dirname "$0")/.." && pwd)
UNIVER_REPO=$(cd "$UNIVER_ROOT/../../.." && pwd)
UNIVER_OUTPUT=${1:?usage: build-backend.sh /absolute/path/to/bundle}
: "${TMPDIR:?set an App-owned scratch directory outside the repository}"
: "${CARGO_TARGET_DIR:?run inside the shared Cargo build slot}"
: "${CHARIOX_APP_COMPONENT_CHECK:?build the L1 component checker first}"
python3 - "$UNIVER_ROOT" <<'PY'
import hashlib, json, sys
from pathlib import Path
root = Path(sys.argv[1])
actual = hashlib.sha256((root / 'backend/Cargo.lock').read_bytes()).hexdigest()
if actual != json.loads((root / 'licenses.json').read_text())['backendLockSha256']:
    raise RuntimeError('unlisted Rust dependency: review the backend lock and licence inventory')
PY
mkdir -p "$UNIVER_OUTPUT/runtime"
python3 "$UNIVER_REPO/packages/app-sdk-tools/build-rust.py" "$UNIVER_ROOT/backend/Cargo.toml" "$UNIVER_OUTPUT/runtime/app.wasm"
