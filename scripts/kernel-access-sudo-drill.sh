#!/usr/bin/env bash
set -euo pipefail
if [[ "$(uname -s)" != Linux ]]; then
  echo "Run the sudo drill on the Linux builder." >&2
  exit 1
fi
command -v slot-run >/dev/null
task_home=$(mktemp -d)
trap 'rm -rf -- "$task_home"' EXIT
export CHARIOX_HOME="$task_home"
export RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1
slot-run cargo +1.88.0 test -p chariox-kernel --lib sudo --locked -- --nocapture
slot-run cargo +1.88.0 test -p chariox-kernel --lib local::api::tests::protocol_shapes --locked
