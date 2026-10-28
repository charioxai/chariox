#!/usr/bin/env bash
# Run under the host's cargo slot wrapper. All scratch stays outside the repo.
set -euo pipefail
cd "$(dirname "$0")/../.."
: "${TMPDIR:?set TMPDIR outside the repository}"
: "${CHARIOX_APP_TOOLCHAIN:?build the pinned compiler first}"
: "${CARGO_TARGET_DIR:?run this script under the cargo build-slot wrapper}"
scratch=$(mktemp -d "$TMPDIR/sdk-components.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
export CHARIOX_SDK_COMPONENTS="${CHARIOX_SDK_COMPONENTS:-$scratch/components}"
mkdir -p "$CHARIOX_SDK_COMPONENTS"
export NODE_OPTIONS=--max-old-space-size=8192
export TZ=UTC CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=1
cargo build --locked -p chariox-app-sdk-tools
cp "$CARGO_TARGET_DIR/debug/chariox-app-component" "$scratch/check-component"
export CHARIOX_APP_COMPONENT_CHECK="$scratch/check-component"
for n in 1 2; do
  source_dir="$scratch/source-$n"
  cp -R packages/app-sdk-rust/template "$source_dir"
  python3 - "$source_dir/Cargo.toml" "$PWD/packages/app-sdk-rust" <<'PY'
import json, sys
from pathlib import Path
p = Path(sys.argv[1])
p.write_text(p.read_text().replace('path = ".."', 'path = ' + json.dumps(sys.argv[2])))
PY
  python3 packages/app-sdk-tools/build-rust.py "$source_dir/Cargo.toml" "$CHARIOX_SDK_COMPONENTS/rust-$n.wasm"
  cp "$CARGO_TARGET_DIR/wasm32-wasip2/release/chariox_todo.wasm" "$CHARIOX_SDK_COMPONENTS/rust-raw-$n.wasm"
  cp packages/app-sdk-js/template/app.ts "$source_dir/app.ts"
  node packages/app-sdk-tools/build-js.mjs "$source_dir/app.ts" "$CHARIOX_SDK_COMPONENTS/js-$n.wasm"
done
node packages/app-sdk-tools/build-js.mjs packages/app-sdk-tools/tests/fixtures/resources.ts "$CHARIOX_SDK_COMPONENTS/resources.wasm"
node --test --test-concurrency=2 packages/app-sdk-tools/test-build-js.mjs
node packages/app-sdk-tools/prepare-npm.mjs "$scratch"
sha256sum "$CHARIOX_SDK_COMPONENTS/"*.wasm
result=0
cargo test --locked -p chariox-app-sdk-tools --test components -- --include-ignored || result=1
cargo test --locked -p chariox-app-sdk-tools --features component-drills --test component-drill -- --nocapture || result=1
exit "$result"
