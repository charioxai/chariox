#!/usr/bin/env bash
# Real provider PTY, private throwaway kernel, test passkey, and chariox-shell.
set -euo pipefail
if [[ "$(uname -s)" != Linux ]]; then
  echo "Run the shell sudo drill on the Linux builder." >&2
  exit 1
fi
command -v slot-run >/dev/null
check_home=$(mktemp -d)
trap 'rm -rf -- "$check_home"' EXIT
export CHARIOX_LOG_DIR="$check_home/logs"
unset CHARIOX_KERNEL_LOCAL_AUTH_TOKEN CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE CHARIOX_RELAY_TOKEN CHARIOX_DAEMON_SOCKET
export CHARIOX_HOME="$check_home"
export RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
pnpm --filter @chariox/shell run build
export CHARIOX_SUDO_SHELL_CLI="$PWD/apps/shell/dist/shell.js"
# Shell calls use --kernel-url ws+unix://<kernel socket>; no bearer is sent.
slot-run cargo +1.88.0 test -p chariox-kernel --lib sudo_shell --locked -- --nocapture
