#!/bin/bash
# MP-08 / MP-09 / MP-10 / MP-11: real native policy and source-component drill.
# No sudo, host settings, account state or shared target directories.
set -euo pipefail
test "$(uname -s)" = Darwin || { echo 'Run this drill on macOS.' >&2; exit 2; }
repo=$(cd "$(dirname "$0")/../../.." && pwd)
test "$#" = 1 || { echo "usage: $0 ABSOLUTE_EVIDENCE_DIRECTORY" >&2; exit 2; }
evidence=$1
case "$evidence" in /*) ;; *) echo 'Evidence directory must be absolute.' >&2; exit 2;; esac
case "$evidence/" in "$repo/"*) echo 'Keep evidence outside the repository.' >&2; exit 2;; esac
for tool in cargo rustup codesign python3 shasum; do command -v "$tool" >/dev/null; done
mkdir -p "$evidence" "$HOME/.chariox/dev"
scratch=$(mktemp -d "$HOME/.chariox/dev/app-wasm-containment.XXXXXXXX")
cleanup() { rm -rf -- "$scratch"; }
trap cleanup EXIT
export CHARIOX_HOME="$scratch/home" TMPDIR="$scratch/tmp" CARGO_TARGET_DIR="$scratch/target"
export CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=1 RUST_MIN_STACK=16777216
export CHARIOX_WORKER_DRILL_DIR="$scratch/processes"
mkdir -p "$CHARIOX_HOME" "$TMPDIR" "$CHARIOX_WORKER_DRILL_DIR"
cd "$repo"
test -z "$(git status --porcelain)" || { echo 'Run the committed candidate from a clean worktree.' >&2; exit 2; }
{
    printf 'MP-08 / MP-09 / MP-10 / MP-11\n'
    git rev-parse HEAD
    git diff --stat
    uname -a
    sw_vers
    rustc --version
    printf 'CHARIOX_HOME=%s\n' "$CHARIOX_HOME"
} > "$evidence/source.txt"
rustup target list --installed | grep -qx wasm32-unknown-unknown || {
    echo 'Install the Rust wasm32-unknown-unknown target, then rerun.' >&2; exit 2;
}
cargo build --locked --manifest-path packages/app-wasm-worker/tests/guest/Cargo.toml \
    --target wasm32-unknown-unknown --release > "$evidence/guest-build.log" 2>&1
export CHARIOX_WORKER_DRILL_CORE="$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/chariox_app_worker_drill.wasm"
cargo test --locked -p chariox-app-wasm-worker --test containment --test process \
    --no-run --message-format=json > "$evidence/build.jsonl" 2> "$evidence/build.log"
artifact() {
    python3 - "$evidence/build.jsonl" "$1" <<'PY'
import json, sys
paths = [row['executable'] for line in open(sys.argv[1])
         if (row := json.loads(line)).get('reason') == 'compiler-artifact'
         and row.get('executable') and row['target']['name'] == sys.argv[2]]
assert len(paths) == 1, paths
print(paths[0])
PY
}
worker=$(artifact chariox-app-wasm-worker)
native=$(artifact containment)
process=$(artifact process)
# This is our own scratch executable. The red copy has no hardened-runtime
# policy; the green copy admits only the existing allow-jit entitlement.
export CHARIOX_UNCONTAINED_PROBE="$scratch/native-uncontained"
cp "$native" "$CHARIOX_UNCONTAINED_PROBE"
codesign --force --sign - --timestamp=none "$CHARIOX_UNCONTAINED_PROBE" 2> "$evidence/red-sign.log"
for binary in "$worker" "$native"; do
    codesign --force --sign - --timestamp=none --options runtime \
        --entitlements scripts/macos-app-worker.entitlements "$binary" 2>> "$evidence/sign.log"
    codesign --verify --strict "$binary" 2>> "$evidence/sign.log"
    shasum -a 256 "$binary" >> "$evidence/binaries.sha256"
done
set +e
printf '%s\n' 'signature probe' > "$scratch/signature-input"
PROBE_OUTSIDE="$scratch/signature-input" "$CHARIOX_UNCONTAINED_PROBE" --child authority green > "$evidence/signature-refusal.log" 2>&1
unsigned_result=$?
set -e
if test "$unsigned_result" = 0 || ! grep -q APP_SANDBOX_SIGNATURE "$evidence/signature-refusal.log"; then
    echo 'Unsigned/unhardened worker was not explicitly refused.' >&2
    exit 1
fi
set +e
"$native" > "$evidence/native.log" 2>&1
native_result=$?
"$process" --ignored --nocapture --test-threads=1 > "$evidence/process.log" 2>&1
process_result=$?
set -e
find "$CHARIOX_WORKER_DRILL_DIR" -name stderr.log -exec sh -c '
    for file do printf "worker stderr:\n"; cat "$file"; done
' sh {} + > "$evidence/worker-stderr.log"
printf '{"unsigned_signature_refused":true,"native_exit":%s,"process_exit":%s,"scratch_removed_on_exit":true}\n' \
    "$native_result" "$process_result" > "$evidence/result.json"
printf 'Native exit: %s; real worker exit: %s. Logs: %s\n' "$native_result" "$process_result" "$evidence"
test "$native_result" = 0 && test "$process_result" = 0
