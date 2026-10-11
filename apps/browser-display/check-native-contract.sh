#!/bin/bash
# Actual native-command serde without building/running the whole kernel.
set -euo pipefail
cd "$(dirname "$0")/../.."
repo=$PWD
build=${CUMAC_BUILD_DIR:-/Users/miguel/.chariox/dev/cumac/build/native-contract}
mkdir -p "$build"
cat > "$build/Cargo.toml" <<EOF
[package]
name = "cumac-native-command-contract"
version = "0.0.0"
edition = "2021"
[workspace]
[[bin]]
name = "native-command-contract"
path = "$repo/apps/browser-display/native-command-contract.rs"
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
[profile.dev]
debug = 0
incremental = false
EOF
CARGO_TARGET_DIR="$build/target" cargo build --offline -j2 --manifest-path "$build/Cargo.toml"
CUMAC_NATIVE_COMMAND_CHECKER="$build/target/debug/native-command-contract" node --test apps/browser-display/native-presenter.test.mjs
