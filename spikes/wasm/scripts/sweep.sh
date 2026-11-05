#!/usr/bin/env bash
# Three interleaved runs of every variant/sandbox; one JSON line per cell.
# Usage: sweep.sh <out.jsonl>. Node variants use the product-pinned Node 24.20.0.
set -u
out=$1
node24=$SPIKE_ROOT/node/node-v24.20.0-linux-x64/bin/node
harness=$CARGO_TARGET_DIR/release/chariox-wasm-harness
for run in 1 2 3; do
  for sandbox in bwrap none; do
    for variant in wasm-rust wasm-js wasm-jsaot node-jco node-js; do
      load=$(cut -d' ' -f1 /proc/loadavg)
      line=$(SPIKE_NODE=$node24 timeout 300 "$harness" "$variant" "$sandbox" 1 2>> "$out.err")
      echo "{\"cell\":{\"run\":$run,\"load1\":$load,\"entry\":\"aot\"},\"r\":${line:-null}}" >> "$out"
    done
    # Compile-at-start (no install-time AOT) cold start, for comparison.
    for variant in wasm-rust wasm-js wasm-jsaot; do
      load=$(cut -d' ' -f1 /proc/loadavg)
      line=$(SPIKE_WASM_ENTRY=app.wasm timeout 300 "$harness" "$variant" "$sandbox" 1 2>> "$out.err")
      echo "{\"cell\":{\"run\":$run,\"load1\":$load,\"entry\":\"jit\"},\"r\":${line:-null}}" >> "$out"
    done
  done
done
