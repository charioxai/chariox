# WebAssembly App worker spike

Not part of the product build. Read [DECISION.md](DECISION.md) first, then
[DESIGN.md](DESIGN.md).

| Path | What |
| --- | --- |
| `wit/app.wit` | the spike's subset of the Chariox world |
| `wit-full/app.wit` | the proposed full world 1.0 (validated with wasm-tools) |
| `worker/` | `chariox-wasm-worker`: wasmtime host on the existing FD3/FD4 worker IPC |
| `harness/` | kernel side: real `WorkerPeer`, stand-in brokers, launch with optional bubblewrap |
| `apps/` | the same tools as a Rust component, a componentize-js component, a plain Node App and a jco-wrapped Rust component |
| `async-probe/` | component-model async (WASI 0.3) on wasmtime and jco |
| `scripts/` | artifact build, sweep, concurrency, summaries |

## Reproduce (Linux x64)

Tools used (all under one scratch root, `$SPIKE_ROOT`): rustup toolchain
1.99.1 with `wasm32-wasip2`, wasm-tools 1.262.0, Node 24.20.0 / 25.9.0 /
26.12.1 tarballs, npm `@bytecodealliance/jco@1.39.0`,
`@bytecodealliance/componentize-js@0.23.0`,
`@bytecodealliance/preview2-shim@0.30.0` installed in `$SPIKE_ROOT/js`,
system `bwrap` 0.11.1.

```sh
export SPIKE_ROOT=/var/tmp/wasmspike
export RUSTUP_HOME=$SPIKE_ROOT/rustup CARGO_HOME=$SPIKE_ROOT/cargo CARGO_TARGET_DIR=$SPIKE_ROOT/target
export PATH=$SPIKE_ROOT/bin:$CARGO_HOME/bin:$SPIKE_ROOT/js/node_modules/.bin:$SPIKE_ROOT/node/node-v24.20.0-linux-x64/bin:$PATH
export SPIKE_WORKER=$CARGO_TARGET_DIR/release/chariox-wasm-worker
cd spikes/wasm
cargo build --release                       # worker, harness, async probe host
scripts/build-artifacts.sh                  # components, .cwasm, jco output, Node packages
scripts/sweep.sh $SPIKE_ROOT/res/sweep.jsonl          # 3 interleaved runs, every variant
scripts/concurrency.sh $SPIKE_ROOT/res/concurrency.jsonl
python3 scripts/summarize.py $SPIKE_ROOT/res/sweep.jsonl $SPIKE_ROOT/artifacts
python3 scripts/summarize_concurrency.py $SPIKE_ROOT/res/concurrency.jsonl
SPIKE_ROOT=$SPIKE_ROOT scripts/jco-matrix.sh <out-dir>  # jco App on Node 24/25/26
```

One cell by hand: `$CARGO_TARGET_DIR/release/chariox-wasm-harness <variant> <bwrap|none> 1`
with variant `wasm-rust`, `wasm-js`, `wasm-jsaot`, `node-jco` or `node-js`
(`SPIKE_NODE` = the node binary for the Node variants). Modes:
`<variant> bwrap 16 concurrency` and `wasm-js bwrap fetchstd`.

Async probe: build `async-probe/guest` with `cargo build --release --target
wasm32-wasip2`, then `spike-async-host <guest.wasm> <calls>`; for jco,
`jco transpile <guest.wasm> -o <dir> --name probe --instantiation async` and
`node async-probe/run-jco.mjs <dir> <calls>` (run from a directory where
`@bytecodealliance/preview2-shim` resolves).
