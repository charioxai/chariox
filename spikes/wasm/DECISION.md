# WebAssembly for Chariox Apps: decision (wasmspike r1, 2026-10-22)

For Miguel. Design: [DESIGN.md](DESIGN.md). Code: `spikes/wasm/` on branch
`wasmspike/r1`. Raw numbers: `/root/.codex/evidence/browser-resume-20260930/wasmspike/final3/`
(final run on commit `e3b693273`).

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
  cold start than today's Node worker (8–9 vs 36 ms; option 1: 62 ms),
  1.6–4.4x less RSS per App (6–13x less private memory), runaway code stopped
  in-process at its 1 s budget with no restart, and a real memory cap. Option 1 (wasm
  inside Node) is worse than both wasmtime and plain Node on almost every
  measure.
- **(c) JS/TS through componentize-js.** Works, with a real cost: CPU-heavy
  JS runs **~8x slower** than in Node (JSON-heavy ~2.4x), each JS App is a
  12 MB component, and npm packages that need Node built-ins will not run.
  Fine for typical I/O-bound Apps; this is the main trade-off for you.
- **(d) Kernel side stays.** Confirmed: the worker speaks the existing FD3
  wire and FD4 handshake on the kernel's own codec (`WorkerPeer`, in a
  harness that plays the kernel); no kernel or protocol change was needed to
  run either App. Wiring the new engine into the kernel's launch path is
  migration PR 3, not done here.

## Numbers (Linux, bubblewrap sandbox, median of 3 runs)

Hardware: AMD Ryzen 9 9950X (32 threads), 125 GiB, Ubuntu 26.04.1, kernel
7.0.0. b3 is shared: load average 13–15 during the final run (17–24 in
two earlier full runs with the same ratios), so absolute times are noisy; ratios are stable across runs. Same calls in every variant. Memory
budget per App: 128 MiB (wasm memory cap, or V8 heap for Node).

| metric | wasmtime + Rust | wasmtime + JS (componentize-js) | wasmtime + JS (componentize-js --aot) | Node 24.20 + jco Rust (JSPI) | Node 24.20 plain JS (today) |
|---|---|---|---|---|---|
| runs | 3 | 3 | 3 | 3 | 3 |
| cold start to ready, ms (AOT where wasm) | 8.3 [7.2–11.0] | 9.1 [8.8–12.5] | 8.9 [8.3–10.5] | 62.2 [58.3–69.6] | 35.7 [33.3–35.7] |
| cold start, compile at start (no AOT), ms | 21.9 [21.6–38.4] | 889.3 [736.7–893.3] | 899.9 [883.4–937.0] | n/a | n/a |
| warm instantiate, ms | 0.009 [0.009–0.010] | 0.013 [0.013–0.019] | 0.013 [0.013–0.015] | 0.214 [0.178–0.220] | n/a |
| RSS after ready, MiB | 12.5 [12.4–12.7] | 33.2 [33.1–33.4] | 43.9 [43.7–43.9] | 84.4 [84.3–84.5] | 54.5 [54.5–54.5] |
| private (anon) RSS after ready, MiB | 1.0 [1.0–1.0] | 2.2 [2.2–2.2] | 2.9 [2.9–2.9] | 35.6 [35.6–35.7] | 13.4 [13.4–13.5] |
| RSS after 1,000 calls + CPU tools, MiB | 23.0 [23.0–23.1] | 69.9 [65.7–70.8] | 80.9 [76.8–81.8] | 114.1 [113.6–117.4] | 112.8 [112.5–112.8] |
| tool call p50, ms | 0.023 [0.022–0.026] | 0.049 [0.047–0.053] | 0.043 [0.040–0.049] | 0.067 [0.052–0.076] | 0.038 [0.037–0.039] |
| tool call p95, ms | 0.036 [0.028–0.053] | 0.089 [0.064–0.099] | 0.079 [0.061–0.094] | 0.146 [0.132–0.171] | 0.094 [0.089–0.102] |
| tool call p99, ms | 0.074 [0.060–0.103] | 0.149 [0.119–0.167] | 0.128 [0.107–0.167] | 0.313 [0.259–0.369] | 0.158 [0.156–0.172] |
| JSON transform (10k records x5), ms | 42.0 [40.1–44.5] | 45.1 [44.7–45.3] | 37.8 [34.7–38.8] | 32.3 [30.3–33.7] | 19.0 [18.4–21.4] |
| JS-heavy compute (1M Map/string ops), ms | 69.8 [67.8–75.8] | 281.3 [271.8–281.7] | 278.4 [274.9–287.6] | 48.3 [47.8–50.4] | 34.5 [34.4–35.1] |
| runaway loop: error after, ms | 1004 [997–1004] | 999 [998–1001] | 1005 [1003–1009] | n/a | n/a |
| runaway loop: next call answered after, ms | 0.3 [0.3–0.4] | 1.1 [0.9–1.4] | 1.3 [1.1–1.4] | 94.1 [89.5–105.8] | 55.3 [48.6–90.0] |
| runaway loop: how | epoch trap, same process | epoch trap, same process | epoch trap, same process | SIGKILL at 1501 ms, reap 25 ms | SIGKILL at 1501 ms, reap 15 ms |
| 256 MiB byte buffers (limit 128) | APP_MEMORY_LIMIT, 22 ms, worker alive, peak RSS 138 MiB | APP_MEMORY_LIMIT, 24 ms, worker alive, peak RSS 152 MiB | APP_MEMORY_LIMIT, 31 ms, worker alive, peak RSS 161 MiB | allocated 256 MiB (not stopped), 444 ms, worker alive, peak RSS 336 MiB | allocated 256 MiB (not stopped), 75 ms, worker alive, peak RSS 308 MiB |
| 256 MiB small objects (limit 128) | APP_MEMORY_LIMIT, 89 ms, worker alive, peak RSS 118 MiB | APP_TRAPPED, 464 ms, worker alive, peak RSS 90 MiB | APP_TRAPPED, 626 ms, worker alive, peak RSS 93 MiB | allocated 256 MiB (not stopped), 9566 ms, worker alive, peak RSS 466 MiB | process died (exit status: 134), 243 ms, worker gone |
| state + broker HTTP correct | yes | yes | yes | yes | yes |
| load average during cells | 14.4 [13.3–14.5] | 14.4 [13.3–14.5] | 13.8 [13.8–13.8] | 13.8 [13.8–14.2] | 13.7 [12.8–13.8] |

Without bubblewrap, cold start to ready (ms): 1.8 [1.7–1.8]; 3.0 [2.8–3.1]; 3.5 [3.3–27.8]; 53.5 [51.5–61.8]; 28.4 [26.4–30.2] (same column order). Bubblewrap adds ~6 ms to the wasm worker and ~7 ms to Node.

Artifact size (worker binary: 22 MB stripped, including the compiler used for install-time precompile; `node` 24.20: 126 MB):

- wasmtime + Rust: 121 KiB .wasm, 367 KiB .cwasm
- wasmtime + JS (componentize-js): 11831 KiB .wasm, 36404 KiB .cwasm
- wasmtime + JS (componentize-js --aot): 15180 KiB .wasm, 43501 KiB .cwasm
- Node 24.20 + jco Rust (JSPI): 1024 KiB package (+ Node runtime)
- Node 24.20 plain JS (today): 3 KiB package (+ Node runtime)

Tool-call round trips are tens of microseconds in every variant, far below
any network or provider latency: not a deciding factor.

What the rows mean: "cold start" is process spawn to `worker.ready` received
by the kernel side. "Warm instantiate" is a fresh App instance from an
already-loaded component. "Private RSS" excludes file-backed pages (the
mapped `.cwasm` or `node` binary) that the OS can share or drop. The runaway
budget is 1 s; Node cannot interrupt its own loop, so the supervisor kills the
process 0.5 s later and starts it again. "APP_TRAPPED" for JS small objects is
StarlingMonkey stopping at its own GC heap ceiling (an unhandlable-OOM
crash inside the sandboxed instance, below the 128 MiB cap); the worker
survives and starts a fresh instance either way.

**Concurrency** (16 tool calls in flight, each waiting 50 ms on the HTTP
broker; 3 runs):

| variant | 16 calls done in, ms [min–max] | calls ok | RSS after, MiB |
|---|---|---|---|
| wasmtime + Rust, 1 instance | 825 [825–828] | 16/16 | 13 |
| wasmtime + Rust, pool of 16 | 53 [53–54] | 16/16 | 14 |
| wasmtime + JS, 1 instance | 831 [828–834] | 16/16 | 34 |
| wasmtime + JS, pool of 16 | 60 [60–61] | 16/16 | 70 |
| Node 24.20 + jco Rust | 854 [854–859] | 16/16 | 92 |
| Node 24.20 plain JS (today) | 61 [61–65] | 16/16 | 65 |

A sync world runs one call per instance at a time, in wasmtime and in jco
alike. The worker's instance pool (one fresh instance per concurrent call,
~10 µs each) removes the gap. An async world (2.0) would do it inside one
instance: the probe ran 64 concurrent calls on one Rust instance in 51.6 ms on
wasmtime 50 (serialized would be 3.2 s).

## Option 1 (wasm inside today's Node worker), as measured

- **Works**, including JSPI: Node **24.20.0 (our pin) already has JSPI on by
  default** (`WebAssembly.Suspending` present without a flag; same on 25.9
  and 26.12). The brief's "Node 25 needed" does not hold.
- **Slowest cold start** (62 ms vs 8–9 ms wasmtime, 36 ms plain Node) and
  **most memory** (85 MiB RSS, 36 MiB private) of all variants: V8 compiles
  the component and jco adds ~300 KB of glue JS per App.
- **No memory cap for wasm.** The V8 heap limit does not cover WebAssembly
  memory: the App allocated 256 MiB under a 128 MiB budget, peak RSS
  466 MiB. Only a cgroup kill (whole process) can stop it. Plain Node has the
  same hole for `ArrayBuffer`s (256 MiB allocated); small JS objects hit the
  heap limit and **abort the whole process** (exit 134 in 243 ms).
- **No in-process stop.** A runaway call needs a process kill plus restart
  (15–25 ms to reap, next call answered 55–94 ms after the kill); every
  other in-flight call of that App is lost. wasmtime: error after the 1.0 s budget, next call answered in
  0.3–1.3 ms (fresh instance), same process.
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

Rows marked "no": versions checked on their registries today; state from the
projects' own docs, not run here.

Note: `cargo-component` (last release 2025-03) is no longer needed; the
plain `wasm32-wasip2` target produces components.

**Debugging for authors.** A trap reaches the App's log with a wasm
backtrace; Rust keeps symbol names when not stripped. JS errors from
StarlingMonkey carry `file:line` stacks. There is no live debugger in the
worker, but there is none today either (the sandboxed Node worker denies the
inspector). Mitigation: keep the JS SDK's `register(chariox)` API so authors
can unit-test JS Apps in plain Node with an SDK fake, and give `chariox app
dev` a local worker with verbose traps.

**JS speed and size.** Measured: JS-heavy compute 281 ms vs 35 ms in Node;
JSON transform 45 ms vs 19 ms; 12 MB `.wasm`, 36 MB `.cwasm`, ~1 s
precompile per install; 33 MiB RSS at ready but only 2 MiB private (the rest
is the mapped engine code). A typical App waits on brokers, providers and
the network far more than it computes, so this is acceptable for most Apps;
a CPU-heavy JS App should be written in Rust or wait for a JIT-capable JS
engine.

**Ecosystem.** JS Apps lose Node built-ins (`node:fs`, `Buffer`, `net`…).
The SDK covers files (WASI filesystem), HTTP and state, but npm packages that
import Node built-ins fail at build time. HTTP is the good news: a plain
`fetch()` inside the JS component went through the kernel HTTP broker (the
worker routes `wasi:http` there), with no SDK call and no network access in
the sandbox.

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

1. **JS speed vs one runtime.** Accept ~8x slower CPU-heavy JS (and no Node
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
