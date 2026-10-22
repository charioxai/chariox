#!/usr/bin/env bash
# 16 tool calls in flight, each waiting 50 ms on the HTTP broker; 3 runs.
# Usage: concurrency.sh <out.jsonl>
set -u
out=$1
harness=$CARGO_TARGET_DIR/release/chariox-wasm-harness
export SPIKE_NODE=$SPIKE_ROOT/node/node-v24.20.0-linux-x64/bin/node SPIKE_ORIGIN_DELAY_MS=50
for run in 1 2 3; do
  for pool in 1 16; do
    for variant in wasm-rust wasm-js; do
      SPIKE_WASM_POOL=$pool timeout 120 "$harness" "$variant" bwrap 16 concurrency 2>> "$out.err" | sed "s/^{/{\"pool\":$pool,\"run\":$run,/" >> "$out"
    done
  done
  for variant in node-jco node-js; do
    timeout 120 "$harness" "$variant" bwrap 16 concurrency 2>> "$out.err" | sed "s/^{/{\"pool\":null,\"run\":$run,/" >> "$out"
  done
done
# Standard fetch() in the JS component, through the broker only (3 runs).
for run in 1 2 3; do
  timeout 120 "$harness" wasm-js bwrap fetchstd 2>> "$out.err" | sed "s/^{/{\"run\":$run,/" >> "${out%.jsonl}-fetchstd.jsonl"
done
