# WebAssembly App worker spike

Not part of the product build. Read [DECISION.md](DECISION.md) first, then
[DESIGN.md](DESIGN.md).

| Path | What |
| --- | --- |
| `wit/app.wit` | the spike's async world (`chariox:app@0.2.0`), built by every r2 App |
| `wit-full/app.wit` | the proposed async world 1.0 on WASI 0.3 (`deps/`), validated with wasm-tools |
| `wit-sync/app.wit` | r1's sync world, kept to rebuild the r1 Apps for comparison |
| `worker-async/` | `chariox-wasm-worker-async`: one instance per App serving concurrent calls, on the existing FD3/FD4 IPC |
| `worker/` | r1 sync worker (comparison only) |
| `harness/` | kernel side: real `WorkerPeer`, stand-in brokers, bubblewrap; r1 cells plus `cells.rs` |
| `apps/` | `rust-async`, `qjs-app` (QuickJS), `py-app` (Python) on the async world; r1 `rust-app`, `js-app` (StarlingMonkey), `node-app` (today's SDK), `jco-app` |
| `npm-compat/` | five npm packages on QuickJS and StarlingMonkey |
| `async-probe/` | r1's first component-model async probe |
| `scripts/` | artifact build, sweeps, summaries |

## Reproduce (Linux x64)

Tools (all under `$SPIKE_ROOT`): rustup toolchain 1.99.1 with `wasm32-wasip2`,
wasm-tools 1.262.0, Node 24.20.0, npm `@bytecodealliance/jco@1.39.0`,
`@bytecodealliance/componentize-js@0.23.0`,
`@bytecodealliance/preview2-shim@0.30.0` in `$SPIKE_ROOT/js`, componentize-py
0.25.1 in a venv at `$SPIKE_ROOT/py`, system `bwrap` 0.11.1, and
componentize-qjs built from git (the npm 0.4.5 release cannot build async
imports):

```sh
git clone https://github.com/andreiltd/componentize-qjs $SPIKE_ROOT/src-qjs/repo
(cd $SPIKE_ROOT/src-qjs/repo && git checkout cecfa26bd && \
  CARGO_TARGET_DIR=$SPIKE_ROOT/target-qjs cargo build --release --locked -p componentize-qjs-cli)
```

```sh
export SPIKE_ROOT=/var/tmp/wasmspike
export RUSTUP_HOME=$SPIKE_ROOT/rustup CARGO_HOME=$SPIKE_ROOT/cargo CARGO_TARGET_DIR=$SPIKE_ROOT/target
export PATH=$SPIKE_ROOT/bin:$CARGO_HOME/bin:$SPIKE_ROOT/js/node_modules/.bin:$SPIKE_ROOT/node/node-v24.20.0-linux-x64/bin:$PATH
export SPIKE_WORKER=$CARGO_TARGET_DIR/release/chariox-wasm-worker
export SPIKE_WORKER_ASYNC=$CARGO_TARGET_DIR/release/chariox-wasm-worker-async
export SPIKE_QJS=$SPIKE_ROOT/target-qjs/release/componentize-qjs
cd spikes/wasm
cargo build --release                       # workers, harness, async probe host
scripts/build-artifacts.sh                  # every App, .cwasm, jco output, Node packages
scripts/sweep-async.sh $SPIKE_ROOT/res/sweep2.jsonl   # r2: 3 interleaved runs, all cells
python3 scripts/summarize2.py $SPIKE_ROOT/res/sweep2.jsonl $SPIKE_ROOT/artifacts
```

r1 sweeps (`sweep.sh`, `concurrency.sh`, `jco-matrix.sh`) still work for the
sync variants.

One cell by hand: `$CARGO_TARGET_DIR/release/chariox-wasm-harness <variant> <bwrap|none> 1`
with variant `wasm-rust-async`, `wasm-qjs`, `wasm-py`, `wasm-rust`, `wasm-js`, `wasm-jsaot`, `node-jco` or `node-js`
(`SPIKE_NODE` = the node binary for the Node variants). Modes:
`<variant> bwrap 16 concurrency` and `wasm-js bwrap fetchstd` (r1); r2:
`<variant> bwrap shared <calls>`, `stream <MiB> <work>`, `subscribe <count> <work>`,
`cancel`, `blast <spin|bytes|objects>`, `hol` (set `SPIKE_ORIGIN_DELAY_MS`, as
`sweep-async.sh` does).

Async probe: build `async-probe/guest` with `cargo build --release --target
wasm32-wasip2`, then `spike-async-host <guest.wasm> <calls>`; for jco,
`jco transpile <guest.wasm> -o <dir> --name probe --instantiation async` and
`node async-probe/run-jco.mjs <dir> <calls>` (run from a directory where
`@bytecodealliance/preview2-shim` resolves).
