# WebAssembly for Chariox Apps: decision (wasmspike r1, 2026-10-22)

For Miguel. Design: [DESIGN.md](DESIGN.md). Code: `spikes/wasm/` on branch
`wasmspike/r1`. Raw numbers: `/root/.codex/evidence/browser-resume-20260930/wasmspike/`.

## Summary

**Recommendation: yes to all four parts of the coordinator's proposal, with
one change: ship the WIT world as sync functions now and make WASI 0.3 async
the next major version.**

- **(a) Contract = Component Model WIT world.** Confirmed. A versioned
  `chariox:app` world maps 1:1 onto today's SDK and wire; it validates.
  Change: world 1.x is **sync-typed**, because the main JS toolchain
  (componentize-js 0.23) cannot build async worlds today (measured: it
  panics). Rust and wasmtime already run async (measured), so 2.0 = async.
- **(b) Host = wasmtime in a Rust worker.** Confirmed by the numbers: ~4x faster
  cold start than today's Node worker (10 vs 41 ms; option 1: 78 ms), 1.7–4.7x
  less RSS per App (6–15x less private memory), runaway code stopped
  in-process at its 1 s budget with no restart, and a real memory cap. Option 1 (wasm
  inside Node) is worse than both wasmtime and plain Node on almost every
  measure.
- **(c) JS/TS through componentize-js.** Works, with a real cost: CPU-heavy
  JS runs **~8.6x slower** than in Node (JSON-heavy ~2.5x), each JS App is a
  12 MB component, and npm packages that need Node built-ins will not run.
  Fine for typical I/O-bound Apps; this is the main trade-off for you.
- **(d) Kernel side stays.** Confirmed: the worker speaks the existing FD3
  wire and FD4 handshake on the kernel's own codec; no kernel or protocol
  change was needed to run either App.

## Numbers (Linux, bubblewrap sandbox, median of 3 runs)

Hardware: AMD Ryzen 9 9950X (32 threads), 125 GiB, Ubuntu 26.04.1, kernel
7.0.0. b3 is shared: load average 17–24 during every cell, so absolute times
are noisy; ratios are stable across runs. Same calls in every variant. Memory
budget per App: 128 MiB (wasm memory cap, or V8 heap for Node).

| metric | wasmtime + Rust | wasmtime + JS (componentize-js) | wasmtime + JS (componentize-js --aot) | Node 24.20 + jco Rust (JSPI) | Node 24.20 plain JS (today) |
|---|---|---|---|---|---|
| runs | 3 | 3 | 3 | 3 | 3 |
| cold start to ready, ms (AOT where wasm) | 9.9 [8.8–11.7] | 10.1 [9.7–13.0] | 13.5 [9.8–13.8] | 78.2 [71.3–79.4] | 40.9 [40.2–47.3] |
| cold start, compile at start (no AOT), ms | 31.1 [27.9–48.0] | 1034.5 [957.5–1290.1] | 1029.1 [857.7–1032.1] | n/a | n/a |
| warm instantiate, ms | 0.009 [0.009–0.010] | 0.012 [0.011–0.012] | 0.011 [0.011–0.011] | 0.230 [0.216–0.253] | n/a |
| RSS after ready, MiB | 11.7 [11.7–11.9] | 32.3 [32.2–32.4] | 43.2 [42.9–43.2] | 84.5 [84.1–85.0] | 54.5 [54.3–54.5] |
| private (anon) RSS after ready, MiB | 0.9 [0.9–0.9] | 2.1 [2.1–2.1] | 2.7 [2.7–2.7] | 35.7 [35.4–36.0] | 13.4 [13.4–13.4] |
| RSS after 1,000 calls + CPU tools, MiB | 22.3 [22.3–22.5] | 69.2 [65.8–70.0] | 82.2 [80.3–86.5] | 112.6 [112.5–116.4] | 112.5 [112.2–113.2] |
| tool call p50, ms | 0.029 [0.025–0.029] | 0.046 [0.042–0.046] | 0.053 [0.041–0.071] | 0.074 [0.071–0.083] | 0.043 [0.041–0.045] |
| tool call p95, ms | 0.049 [0.035–0.057] | 0.075 [0.057–0.077] | 0.085 [0.078–0.139] | 0.183 [0.173–0.184] | 0.109 [0.100–0.109] |
| tool call p99, ms | 0.095 [0.069–0.099] | 0.139 [0.114–0.145] | 0.201 [0.144–0.343] | 0.306 [0.301–0.318] | 0.192 [0.179–0.228] |
| JSON transform (10k records x5), ms | 43.0 [42.3–45.4] | 50.9 [47.3–58.0] | 44.0 [42.4–46.4] | 33.8 [30.5–34.4] | 19.9 [19.5–23.3] |
| JS-heavy compute (1M Map/string ops), ms | 70.5 [69.8–78.7] | 300.1 [290.6–309.2] | 298.6 [297.6–313.6] | 50.8 [49.1–51.1] | 34.9 [34.7–36.7] |
| runaway loop: error after, ms | 1005 [998–1006] | 1001 [1001–1007] | 1007 [1003–1010] | n/a | n/a |
| runaway loop: next call answered after, ms | 0.4 [0.3–0.4] | 1.4 [1.2–1.4] | 1.5 [1.4–2.1] | 101.1 [94.6–115.7] | 71.0 [69.8–77.1] |
| runaway loop: how | epoch trap, same process | epoch trap, same process | epoch trap, same process | SIGKILL at 1500 ms, reap 23 ms | SIGKILL at 1503 ms, reap 33 ms |
| 256 MiB byte buffers (limit 128) | APP_MEMORY_LIMIT, 31 ms, worker alive, peak RSS 137 MiB | APP_MEMORY_LIMIT, 29 ms, worker alive, peak RSS 158 MiB | APP_MEMORY_LIMIT, 31 ms, worker alive, peak RSS 165 MiB | allocated 256 MiB (not stopped), 403 ms, worker alive, peak RSS 338 MiB | allocated 256 MiB (not stopped), 69 ms, worker alive, peak RSS 308 MiB |
| 256 MiB small objects (limit 128) | APP_MEMORY_LIMIT, 100 ms, worker alive, peak RSS 117 MiB | APP_TRAPPED, 492 ms, worker alive, peak RSS 89 MiB | APP_TRAPPED, 692 ms, worker alive, peak RSS 90 MiB | allocated 256 MiB (not stopped), 10520 ms, worker alive, peak RSS 466 MiB | process died (exit status: 134), 234 ms, worker gone |
| state + broker HTTP correct | yes | yes | yes | yes | yes |
| load average during cells | 21.6 [13.4–24.4] | 21.4 [13.4–23.5] | 21.4 [13.3–23.5] | 21.3 [13.5–23.4] | 20.6 [14.1–22.1] |

Without bubblewrap (same cells): cold start to ready, ms (AOT where wasm); 1.6 [1.6–1.7]; 2.9 [2.6–3.8]; 3.9 [3.4–3.9]; 70.5 [57.2–75.9]; 30.7 [29.7–35.3]. Bubblewrap adds ~6 ms to the wasm worker and ~10 ms to Node.

Artifact size (worker binary: 22 MB stripped, includes the compiler used for install-time precompile; `node` 24.20: 126 MB):

- wasmtime + Rust: 121 KiB .wasm, 367 KiB .cwasm
- wasmtime + JS (componentize-js): 11816 KiB .wasm, 36359 KiB .cwasm
- wasmtime + JS (componentize-js --aot): 15136 KiB .wasm, 43412 KiB .cwasm
- Node 24.20 + jco Rust (JSPI): 1024 KiB package (+ Node runtime)
- Node 24.20 plain JS (today): 3 KiB package (+ Node runtime)

What the rows mean: "cold start" is process spawn to `worker.ready` received
by the kernel side. "Warm instantiate" is a fresh App instance from an
already-loaded component. "Private RSS" excludes file-backed pages (the
mapped `.cwasm` or `node` binary) that the OS can share or drop. The runaway
budget is 1 s; Node cannot interrupt its own loop, so the supervisor kills the
process 0.5 s later and starts it again.

**Concurrency** (16 tool calls in flight, each waiting 50 ms on the HTTP
broker; 3 runs):

| variant | 16 calls done in, ms [min–max] | calls ok | RSS after, MiB |
|---|---|---|---|
| wasmtime + Rust, 1 instance | 831 [830–832] | 16/16 | 12 |
| wasmtime + Rust, pool of 16 | 54 [53–54] | 16/16 | 13 |
| wasmtime + JS, 1 instance | 836 [834–837] | 16/16 | 33 |
| wasmtime + JS, pool of 16 | 60 [59–62] | 16/16 | 69 |
| Node 24.20 + jco Rust | 866 [858–870] | 16/16 | 92 |
| Node 24.20 plain JS (today) | 64 [63–64] | 16/16 | 65 |

A sync world runs one call per instance at a time, in wasmtime and in jco
alike. The worker's instance pool (one fresh instance per concurrent call,
~10 µs each) removes the gap. An async world (2.0) would do it inside one
instance: the probe ran 64 concurrent calls on one Rust instance in 51.6 ms on
wasmtime 50 (serialized would be 3.2 s).

## Option 1 (wasm inside today's Node worker), as measured

- **Works**, including JSPI: Node **24.20.0 (our pin) already has JSPI on by
  default** (`WebAssembly.Suspending` present without a flag; same on 25.9
  and 26.12). The brief's "Node 25 needed" does not hold.
- **Slowest cold start** (78 ms vs 10 ms wasmtime, 41 ms plain Node) and
  **most memory** (85 MiB RSS, 36 MiB private) of all variants: V8 compiles
  the component and jco adds ~300 KB of glue JS per App.
- **No memory cap for wasm.** The V8 heap limit does not cover WebAssembly
  memory: the App allocated 256 MiB under a 128 MiB budget, peak RSS
  466 MiB. Only a cgroup kill (whole process) can stop it. Plain Node has the
  same hole for `ArrayBuffer`s (256 MiB allocated); small JS objects hit the
  heap limit and **abort the whole process** (exit 134 in 234 ms).
- **No in-process stop.** A runaway call needs a process kill plus restart
  (~23–33 ms to reap + ~70–100 ms to ready again); every other in-flight call of that App
  is lost. wasmtime: error after the 1.0 s budget, next call answered in
  0.4–1.5 ms (fresh instance), same process.
- **Keeps every Node liability.** The embedded `libnode` build (165-minute CI
  build, not yet produced), the bootstrap's permission patches, and Node
  upgrades: today's unmodified bootstrap **fails on Node 26.12** (its
  permission model now refuses the FD3 socket without `--allow-net`).
- **Async works too** (jco 1.39 ran the async probe concurrently on Node 24
  and 25), so async is not an argument for option 1.

## Risks

**Guest toolchains (checked 2026-10-22)**

| Language | Toolchain | State | Measured here |
| --- | --- | --- | --- |
| Rust | `wasm32-wasip2` target (rustc 1.99; `wasm32-wasip3` listed) + wit-bindgen 0.63 | mature; sync and async | yes: 121 KiB App, all tools |
| JS/TS | componentize-js 0.23 (StarlingMonkey, SpiderMonkey interpreter) | works, sync only, panics on async worlds; `--aot` (weval) gave no speedup here | yes |
| JS/TS | componentize-qjs 0.4.5 (QuickJS) | async exports supported | no |
| Go | TinyGo v0.43.0 (`-target=wasip2`) | usable; mainline Go builds wasip1 modules, not components | no |
| Python | componentize-py 0.25.1 (CPython inside the component) | usable, large components | no |
| C# | componentize-dotnet 0.8.0-preview | preview | no |
| Kotlin | Kotlin/Wasm (WasmGC; wasmtime 50 has GC on by default) | experimental for components | no |

Note: `cargo-component` (last release 2025-03) is no longer needed; the
plain `wasm32-wasip2` target produces components.

**Debugging for authors.** A trap reaches the App's log with a wasm
backtrace; Rust keeps symbol names when not stripped. JS errors from
StarlingMonkey carry `file:line` stacks. There is no live debugger in the
worker, but there is none today either (the sandboxed Node worker denies the
inspector). Mitigation: keep the JS SDK's `register(chariox)` API so authors
can unit-test JS Apps in plain Node with an SDK fake, and give `chariox app
dev` a local worker with verbose traps.

**JS speed and size.** Measured: JS-heavy compute 300 ms vs 35 ms in Node;
JSON transform 51 ms vs 20 ms; 12 MB `.wasm`, 36 MB `.cwasm`, ~1 s
precompile per install; 33 MiB RSS at ready but only 2 MiB private (the rest
is the mapped engine code). A typical App waits on brokers, providers and
the network far more than it computes, so this is acceptable for most Apps;
a CPU-heavy JS App should be written in Rust or wait for a JIT-capable JS
engine.

**Ecosystem.** JS Apps lose Node built-ins (`node:fs`, `Buffer`, `net`…).
The SDK covers files (WASI filesystem), HTTP (`fetch` → broker) and state,
but npm packages that import Node built-ins fail at build time.

**Maturity of wasmtime's async.** It ran our probe correctly, but its own
docs still call component-model async "very incomplete". Another reason to
keep 1.x sync.

## Migration plan (bounded PRs)

Sizes: S ≈ 1–2 days, M ≈ 3–5 days, L ≈ 1–2 weeks, for one lane.

1. **WIT world 1.0 + allowlist** (S): `packages/app-wit` with
   `wit-full/app.wit`, versioning tests, the import allowlist.
2. **Wasm worker** (L): `chariox-wasm-worker` from the spike: every import of
   the world, limits, instance pool, `precompile`/`check`; tests on the real
   `WorkerPeer` with fixture brokers.
3. **Kernel install + launch** (L): manifest `engine: "wasm"`, package
   validation, install-time precompile into the release store, launch through
   the existing launcher (FD3/FD4, bwrap, cgroup). Protocol bump if any
   client-visible App shape changes.
4. **Sandbox profiles** (M): seccomp list for the worker (no runtime JIT, so
   stricter than Node), macOS Seatbelt and signing.
5. **WASI virtualization** (M): `wasi:http` onto the HTTP broker (standard
   `fetch` in JS goes through it), filesystem preopens with fsync deny.
6. **JS SDK as a component + packer** (M): same `@chariox/app-sdk` API over
   the WIT imports; `chariox app pack` builds JS (componentize-js) and Rust
   components; templates.
7. **Port Apps and drill** (M): first-party/example Apps, real-path drills
   (kernel, client, provider).
8. **Delete the Node engine** (S, deletion): `libnode` build pipeline,
   `bootstrap.cjs`, Node permission patches, `engine: "node"`.

About 6–9 weeks for one lane; 2 and 3 can run in parallel after 1.

## Open questions for Miguel

1. **JS speed vs one runtime.** Accept ~8.6x slower CPU-heavy JS (and no Node
   built-ins) for one runtime and real limits? Or keep Node as a second
   engine for JS only?
2. **Sync now, async later?** Ship world 1.x sync (works with every
   toolchain today) and async 2.0 later, or go async-only now (Rust and
   QuickJS JS only)?
3. **One process per App** (as today, recommended for v1) or many Apps per
   worker process later (~1 MiB private memory per Rust App)?
4. **macOS**: running precompiled code needs the same executable-memory
   entitlement question Node's JIT has today. OK to treat it the same way?
5. **Languages at launch**: Rust and JS/TS only, Go/Python later?
6. **Existing `engine: "node"` packages**: rebuild them, or keep a
   deprecation window?
