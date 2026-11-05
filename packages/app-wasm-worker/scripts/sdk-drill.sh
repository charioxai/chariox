#!/bin/sh
set -eu
: "${CHARIOX_SDK_CHECKOUT:?set the frozen L3 checkout}"
: "${CHARIOX_WORKER_SDK_DIR:?set the external evidence directory}"
: "${CHARIOX_C_TOOLCHAIN:?install the pinned C toolchain}"
for language in c cpp; do
  bash "$CHARIOX_SDK_CHECKOUT/packages/app-sdk-$language/build.sh" "$CHARIOX_WORKER_SDK_DIR/$language-template.wasm"
  extension=c
  [ "$language" = c ] || extension=cpp
  bash "$CHARIOX_SDK_CHECKOUT/packages/app-sdk-$language/build.sh" "$CHARIOX_WORKER_SDK_DIR/$language-drill.wasm" "$PWD/packages/app-wasm-worker/tests/sdk/drill.$extension"
done
cargo build --manifest-path packages/app-wasm-worker/tests/pre-cancel-guest/Cargo.toml --target wasm32-unknown-unknown --release
cp "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/chariox_app_pre_cancel_drill.wasm" "$CHARIOX_WORKER_SDK_DIR/pre-cancel.core.wasm"
cargo test -p chariox-app-wasm-worker --test sdk -- --ignored --nocapture
