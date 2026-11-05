#!/bin/bash
# MP-08 / MP-09 / MP-10 / MP-11: real native policy and source-component drill.
# No sudo, host settings, account state or shared target directories.
#
# Run on an Apple-silicon (or Intel) Mac from a clean checkout of the candidate:
#   rustup toolchain install 1.99.1 --target wasm32-unknown-unknown
#   rustup run 1.99.1 bash packages/app-wasm-worker/scripts/macos-containment-drill.sh "$HOME/chariox-macos-evidence"
# Expected: the last line is
#   Native exit: 0; real worker exit: 0; installed App exit: 0; release gate exit: 0. Logs: <dir>
# and the script exits 0. In native.log every scenario reads "NATIVE <name>/<mode>: PASS"
# except memory, without-landlock and icache ("NOT RUN (Linux-only native probe)").
# "installation-files/green" proves, inside the signed, Seatbelt-contained probe:
# read/write/rename/remove beneath the installation's data and tmp work; a read,
# a write and a symlink outside them are refused; a TCP connect is refused; exec
# is refused. installed.log ends "test result: ok. 6 passed" and includes
# "DRILL native supervised preopens data/tmp persist across restart: PASS" (the
# real contained serve-supervised worker). release-gate.log shows
# every_build_profile_opens_the_worker_beside_the_kernel ... ok under --release.
set -euo pipefail
test "$(uname -s)" = Darwin || { echo 'Run this drill on macOS.' >&2; exit 2; }
repo=$(cd "$(dirname "$0")/../../.." && pwd)
test "$#" = 1 || { echo "usage: $0 ABSOLUTE_EVIDENCE_DIRECTORY" >&2; exit 2; }
evidence=$1
case "$evidence" in /*) ;; *) echo 'Evidence directory must be absolute.' >&2; exit 2;; esac
case "$evidence/" in "$repo/"*) echo 'Keep evidence outside the repository.' >&2; exit 2;; esac
for tool in cargo rustc rustup codesign python3 shasum; do command -v "$tool" >/dev/null; done
python3 - "$(rustc --version)" <<'PY'
import re, sys
version = re.match(r'^rustc (\d+)\.(\d+)\.', sys.argv[1])
if not version or tuple(map(int, version.groups())) < (1, 97):
    sys.exit(f'Wasmtime 50 requires Rust >= 1.97; active compiler is {sys.argv[1]}. '
             'Select a suitable rustup toolchain (for example rustup run 1.99.1 bash <drill> <evidence>) and rerun.')
PY
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
# Probe each syscall in a disposable child before spending time on compilation.
# These are diagnostics only: the signed worker still has to install its policy.
python3 - > "$evidence/rlimit-self-check.log" <<'PY'
import ctypes, os, resource
libc = ctypes.CDLL(None, use_errno=True)
class Limit(ctypes.Structure):
    _fields_ = [('soft', ctypes.c_uint64), ('hard', ctypes.c_uint64)]
for name, requested in [('RLIMIT_AS', 1 << 30),
                        ('RLIMIT_FSIZE', 8 << 20), ('RLIMIT_NOFILE', 64),
                        ('RLIMIT_CORE', 0)]:
    pid = os.fork()
    if pid == 0:
        operation = f'getrlimit({name})'
        try:
            kind = getattr(resource, name)
            old_soft, old_hard = resource.getrlimit(kind)
            bounded = lambda value, old: value if old == resource.RLIM_INFINITY else min(value, old)
            soft = bounded(bounded(requested, old_soft), old_hard)
            hard = bounded(requested, old_hard)
            operation = f'setrlimit({name}, soft={soft}, hard={hard})'
            result = libc.setrlimit(kind, ctypes.byref(Limit(soft, hard)))
            errno = ctypes.get_errno() if result else 0
            print(f'{operation}: {"FAIL" if result else "OK"} errno={errno} {os.strerror(errno)}', flush=True)
        except (OSError, ValueError) as error:
            print(f'{operation}: FAIL errno={getattr(error, "errno", None)} {error}', flush=True)
        os._exit(0)
    os.waitpid(pid, 0)
PY
cat "$evidence/rlimit-self-check.log"
rustup target list --installed | grep -qx wasm32-unknown-unknown || {
    echo 'Install the Rust wasm32-unknown-unknown target, then rerun.' >&2; exit 2;
}
cargo build --locked --manifest-path packages/app-wasm-worker/tests/guest/Cargo.toml \
    --target wasm32-unknown-unknown --release > "$evidence/guest-build.log" 2>&1
export CHARIOX_WORKER_DRILL_CORE="$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/chariox_app_worker_drill.wasm"
# The installed-App drill reads a real signed .cxapp built from the same core.
cargo run --locked -p chariox-app-wasm-worker --example package-drill -- \
    "$CHARIOX_WORKER_DRILL_DIR/package" > "$evidence/package.log" 2>&1
cargo test --locked -p chariox-app-wasm-worker --test containment --test process --test installed \
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
installed=$(artifact installed)
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
"$installed" --ignored --nocapture --test-threads=1 > "$evidence/installed.log" 2>&1
installed_result=$?
# Release kernels launch the worker installed beside them (no build-profile gate).
cargo test --locked --release -p chariox-app-runtime --lib \
    every_build_profile_opens_the_worker_beside_the_kernel > "$evidence/release-gate.log" 2>&1
release_result=$?
set -e
find "$CHARIOX_WORKER_DRILL_DIR" -name stderr.log -exec sh -c '
    for file do printf "worker stderr:\n"; cat "$file"; done
' sh {} + > "$evidence/worker-stderr.log"
printf '{"unsigned_signature_refused":true,"native_exit":%s,"process_exit":%s,"installed_exit":%s,"release_gate_exit":%s,"scratch_removed_on_exit":true}\n' \
    "$native_result" "$process_result" "$installed_result" "$release_result" > "$evidence/result.json"
printf 'Native exit: %s; real worker exit: %s; installed App exit: %s; release gate exit: %s. Logs: %s\n' \
    "$native_result" "$process_result" "$installed_result" "$release_result" "$evidence"
test "$native_result" = 0 && test "$process_result" = 0 && test "$installed_result" = 0 && test "$release_result" = 0
