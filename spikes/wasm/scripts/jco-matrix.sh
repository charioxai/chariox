#!/usr/bin/env bash
# Option 1 on each Node line: does the jco/JSPI App start, and with which flag.
set -u
for spec in "v24.20.0|" "v24.20.0|--experimental-wasm-jspi" "v25.9.0|" "v26.12.1|"; do
  v=${spec%%|*}; f=${spec#*|}
  SPIKE_NODE=$SPIKE_ROOT/node/node-$v-linux-x64/bin/node SPIKE_NODE_FLAGS="$f" timeout 300 \
    "$CARGO_TARGET_DIR/release/chariox-wasm-harness" node-jco none 1 > "$1/jco-$v$f.json" 2> "$1/jco-$v$f.err"
  echo "node $v flags='$f' exit=$?"
done
