#!/usr/bin/env bash
# r2: three interleaved runs of the r1 cells (bubblewrap) for the async-world
# Apps next to the r1 sync wasm Apps and today's Node App, then the async
# cells. One JSON line per cell. Usage: sweep-async.sh <out.jsonl>
set -u
out=$1
export SPIKE_NODE=$SPIKE_ROOT/node/node-v24.20.0-linux-x64/bin/node
harness=$CARGO_TARGET_DIR/release/chariox-wasm-harness
cell() {
  local load line
  load=$(cut -d' ' -f1 /proc/loadavg)
  line=$(timeout 300 "$harness" "$@" 2>> "$out.err")
  echo "{\"cell\":{\"run\":$run,\"load1\":$load,\"args\":\"$*\",\"entry\":\"${SPIKE_WASM_ENTRY:-app.cwasm}\",\"delay\":\"${SPIKE_ORIGIN_DELAY_MS:-0}\"},\"r\":${line:-null}}" >> "$out"
}
for run in 1 2 3; do
  for variant in wasm-rust-async wasm-qjs wasm-py wasm-rust wasm-js node-js; do
    cell "$variant" bwrap 1
  done
  for variant in wasm-rust-async wasm-qjs wasm-py; do
    SPIKE_WASM_ENTRY=app.wasm cell "$variant" bwrap 1
    cell "$variant" none 1
  done
  cell node-js none 1
  export SPIKE_ORIGIN_DELAY_MS=50
  for variant in wasm-rust-async wasm-qjs wasm-py node-js; do
    cell "$variant" bwrap shared 16
    cell "$variant" bwrap shared 64
    cell "$variant" bwrap shared 64 50
    cell "$variant" bwrap hol
  done
  cell node-js bwrap stream 50 0
  for variant in wasm-rust-async wasm-qjs wasm-py; do
    cell "$variant" bwrap stream 50 0
    cell "$variant" bwrap stream 50 20
    cell "$variant" bwrap subscribe 1000 0
    cell "$variant" bwrap subscribe 300 200000
    cell "$variant" bwrap cancel
  done
  export SPIKE_ORIGIN_DELAY_MS=300
  for variant in wasm-rust-async wasm-qjs wasm-py; do
    for culprit in spin bytes objects; do cell "$variant" bwrap blast "$culprit"; done
  done
  unset SPIKE_ORIGIN_DELAY_MS
done
