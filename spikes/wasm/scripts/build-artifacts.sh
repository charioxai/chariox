#!/usr/bin/env bash
# Builds every spike artifact into $SPIKE_ROOT/artifacts. Needs env.sh tools.
set -euo pipefail
spike=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$spike/../.." && pwd)
out=$SPIKE_ROOT/artifacts
worker=$CARGO_TARGET_DIR/release/chariox-wasm-worker
rm -rf "$out" && mkdir -p "$out"/{wasm-rust,wasm-js,wasm-jsaot,wasm-rust-async,wasm-qjs,wasm-py,node-js/runtime,node-jco/runtime,runtime}

(cd "$spike/apps/rust-app" && cargo build --release --target wasm32-wasip2 -q)
cp "$CARGO_TARGET_DIR/wasm32-wasip2/release/spike_rust_app.wasm" "$out/wasm-rust/app.wasm"
(cd "$SPIKE_ROOT/js" && cp "$spike/scripts/build-js-app.mjs" . && SPIKE_ROOT="$spike/" node build-js-app.mjs "$out/wasm-js/app.wasm" \
  && SPIKE_ROOT="$spike/" node build-js-app.mjs "$out/wasm-jsaot/app.wasm" --aot)
for app in rust js jsaot; do
  "$worker" check "$out/wasm-$app/app.wasm"
  echo "wasm-$app $("$worker" precompile "$out/wasm-$app/app.wasm" "$out/wasm-$app/app.cwasm")"
done

# Async world (r2): Rust, QuickJS (componentize-qjs built from git, $SPIKE_QJS)
# and Python (componentize-py), precompiled by the async worker.
(cd "$spike/apps/rust-async" && cargo build --release --target wasm32-wasip2 -q)
cp "$CARGO_TARGET_DIR/wasm32-wasip2/release/spike_rust_async.wasm" "$out/wasm-rust-async/app.wasm"
(cd "$spike/apps/qjs-app" && time "$SPIKE_QJS" --wit "$spike/wit" --js app.js --world app -o "$out/wasm-qjs/app.wasm")
(time "$SPIKE_ROOT/py/bin/componentize-py" -d "$spike/wit" -w app componentize app -p "$spike/apps/py-app" -o "$out/wasm-py/app.wasm")
for app in rust-async qjs py; do
  "$SPIKE_WORKER_ASYNC" check "$out/wasm-$app/app.wasm"
  echo "wasm-$app $("$SPIKE_WORKER_ASYNC" precompile "$out/wasm-$app/app.wasm" "$out/wasm-$app/app.cwasm")"
done

async_imports='chariox:app/state@0.1.0#get chariox:app/state@0.1.0#transact chariox:app/http@0.1.0#send chariox:app/log@0.1.0#write'
jco transpile "$out/wasm-rust/app.wasm" -o "$out/node-jco/runtime/rust" --name app --instantiation async \
  --async-mode jspi --async-imports $async_imports \
  --async-exports 'chariox:app/handlers@0.1.0#call-tool' 'chariox:app/handlers@0.1.0#register' -q
cp "$spike/apps/jco-app/main.mjs" "$out/node-jco/runtime/main.mjs"
mkdir -p "$out/node-jco/node_modules/@bytecodealliance"
cp -r "$SPIKE_ROOT/js/node_modules/@bytecodealliance/preview2-shim" "$out/node-jco/node_modules/@bytecodealliance/"

cp "$spike/apps/node-app/main.mjs" "$spike/apps/js-app/tools.js" "$out/node-js/runtime/"
cp "$repo/apps/app-worker/src/bootstrap.cjs" "$repo/apps/app-worker/src/bootstrap-config.cjs" "$out/runtime/"
cp -r "$repo/packages/app-sdk" "$out/runtime/sdk"
rm -rf "$out/runtime/sdk/test" "$out/runtime/sdk/node_modules"
du -sb "$out"/* | sed "s|$out/||"
