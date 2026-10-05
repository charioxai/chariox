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
export RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1 CARGO_PROFILE_TEST_DEBUG=0
slot-run cargo +1.88.0 test -p chariox-kernel --lib sudo --locked -- --nocapture
slot-run cargo +1.88.0 test -p chariox-kernel --lib local::api::tests::protocol_shapes --locked
slot-run cargo +1.88.0 test -p chariox-kernel --lib kernel_access_grants --locked
slot-run cargo +1.88.0 test -p chariox-kernel --lib meta_slash --locked
# Exercise authorization waits and no-replay behavior on real private sockets.
pnpm --filter @chariox/kernel-client run build
node --test packages/kernel-client/dist/ipc-control-response-replay.test.js packages/kernel-client/dist/ipc-unix-access.test.js packages/kernel-client/dist/kernel-authorization-request-policy.test.js packages/kernel-client/dist/websocket-pending-requests.test.js
