# WebAssembly for Chariox Apps: decision (wasmspike r2, 2026-10-22)

For Miguel. Design: [DESIGN.md](DESIGN.md). Code: `spikes/wasm/` on branch
`wasmspike/r2`. Raw numbers: `/root/.codex/evidence/browser-resume-20260930/wasmspike/r2/`
(`sweep2.jsonl`, `summary2.md`, `ENV.md`). r1 (sync world) is in git history
(`249de8384`).

## Summary

**Your async-first decision holds up.** One instance per App now serves
many agents at once, with shared in-memory state and no lost updates, on
Rust, JS (QuickJS) and Python. It does so with the same limits r1 had,
and no worse than Node anywhere except raw JS CPU speed.

- **Many agents, one instance, shared state: works.** 16 callers sharing a
  counter, a cache and one "connection" object finished in 52 ms on every
  wasm App (Node: 61 ms), with no lost updates. Memory: 13–39 MiB against
  Node's 66 MiB.
- **The real ceiling is the kernel, not the runtime.** Today's kernel peer
  allows **16** broker calls in flight per App, and Node's worker has its
  own cap of 16. At 64 callers, 48 get `BUSY` with *every* runtime. For
  "many agents" the kernel must queue instead of refusing (PR 2 below).
- **Streaming and subscriptions work, with backpressure.** A 50 MiB body
  streams at 260–280 MiB/s with **+1 to +4 MiB** of memory. Node's SDK fetch: 48 MiB/s,
  +37 MiB. A slow reader holds at most 4 chunks. Events arrive in order at
  ~42k/s, and a slow App slows the kernel's sends instead of growing memory.
- **Limits still work with several calls in flight.** A runaway call is
  stopped in-process at its 1 s CPU budget, and the worker names the
  culprit. The other calls on that instance are answered at once with a
  retryable `APP_INSTANCE_RESTARTED`. Their in-memory state is lost, as a
  Node crash loses it today. For JS and Python, an out-of-memory error fails
  only the offending call; the others and the shared state survive.
- **Cancellation works fully only in Rust today.** wasmtime 50 cannot cancel
  one guest task from the host (issue #11833), so the contract carries an
  explicit `cancel` future. Rust drops the work and its broker streams in
  12 ms. In QuickJS the call returns at once, but the import it was waiting
  on keeps running until it ends (6 s here). componentize-py traps if a
  cancel read is abandoned, so Python Apps run to completion.
- **QuickJS vs StarlingMonkey.** QuickJS is 8x smaller (1.5 vs 11.8 MB), uses
  half the memory (16 vs 33 MiB), compiles 7x faster at install (0.12 vs
  0.8 s), and is equally fast on JS-heavy code. It is **2.3x slower on
  JSON** (102 vs 44 ms; Node 17 ms). Both run 4 of 5 popular npm packages.
- **QuickJS toolchain risk is real but contained.** The npm release cannot
  build our world: async *imports* were fixed upstream on 10-21 and are not
  released yet. The tool has essentially one maintainer. We pin a git
  commit, build it ourselves (5 min), and it is only a build tool: the
  components it emits are standard, so the worker does not depend on it.
- **Python works** (componentize-py 0.25.1, built in 1.5 s, far under 45 min):
  19 MB App, 36 MiB RSS, 4 ms cold start without bubblewrap.

## Numbers (Linux, bubblewrap sandbox, median [min–max] of 3 runs)

Hardware: AMD Ryzen 9 9950X (32 threads), 125 GiB, kernel 7.0.0, shared b3
(load 5.7–10 during the sweep). All six variants ran interleaved in the same
sweep, with the same calls. Memory cap per App: 128 MiB (wasm memory cap,
V8 heap for Node). CPU slice budget: 1 s (the same 1 s as r1's per-call
budget).

| metric | Rust (async) | JS QuickJS (async) | Python (async) | Rust (r1 sync) | JS StarlingMonkey (r1 sync) | Node 24.20 (today) |
|---|---|---|---|---|---|---|
| cold start to ready, ms | 5.9 [5.4–6.1] | 5.6 [5.6–7.1] | 8.1 [8.1–9.3] | 6.0 [5.4–9.1] | 6.9 [6.3–8.5] | 29.2 [29.1–35.3] |
| cold start without bubblewrap, ms | 1.4 [1.4–1.6] | 1.9 [1.8–2.5] | 4.1 [3.7–4.4] | – | – | 26.0 [25.1–31.6] |
| cold start, no install-time AOT, ms | 23.9 | 110.5 | 683.4 | – | (r1: 889) | – |
| warm instantiate, ms | 0.010 | 0.047 | 0.381 | 0.008 | 0.012 | – |
| RSS after ready, MiB | 12.7 | 15.7 | 35.9 | 12.3 | 33.0 | 55.6 |
| private (anon) RSS after ready, MiB | 1.1 | 1.4 | 3.7 | 1.0 | 2.2 | 13.4 |
| RSS after 1,000 calls + CPU tools, MiB | 23.5 | 30.9 | 46.0 | 22.9 | 69.7 | 114.7 |
| tool call p50 / p95 / p99, ms | 0.030 / 0.047 / 0.068 | 0.041 / 0.066 / 0.118 | 0.048 / 0.067 / 0.103 | 0.022 / 0.027 / 0.046 | 0.046 / 0.076 / 0.124 | 0.034 / 0.066 / 0.126 |
| JSON transform (10k records x5), ms | 37.6 [37.0–39.1] | 101.5 [101.0–109.4] | 107.5 [104.9–114.5] | 38.1 | 43.9 [41.9–47.4] | 17.2 [17.0–20.7] |
| JS-heavy compute (1M Map/string ops), ms | 65.6 | 249.8 [246.5–250.8] | 577.5 | 66.3 | 266.4 [265.7–287.9] | 32.7 |
| runaway loop: stopped after, ms | 1005 | 1008 | 1005 | 1003 | 1004 | killed at 1501 |
| runaway loop: next call answered after, ms | 0.1 | 0.3 | 1.3 | 0.3 | 0.9 | 58.8 (restart) |
| 256 MiB byte buffers (cap 128) | `APP_MEMORY_LIMIT`, peak 138 MiB | `APP_MEMORY_LIMIT`, 142 | `APP_MEMORY_LIMIT`, 153 | `APP_MEMORY_LIMIT`, 138 | `APP_MEMORY_LIMIT`, 153 | **not stopped**, 310 MiB |
| 256 MiB small objects (cap 128) | `APP_MEMORY_LIMIT` | `APP_MEMORY_LIMIT` | `APP_MEMORY_LIMIT` | `APP_MEMORY_LIMIT` | `APP_TRAPPED` (engine heap ceiling) | **process aborted** |
| .wasm / install-time .cwasm | 186 KiB / 578 KiB | 1.5 MB / 4.6 MB | 19 MB / 36 MB | 121 KiB / 366 KiB | 11.8 MB / 36 MB | 3 KiB + Node |
| install-time precompile | 23 ms | 122 ms | 684 ms | 17 ms | 820 ms | – |

Tool-call round trips stay in the tens of microseconds everywhere.

### Many agents on one instance

Each call bumps a shared counter, uses one shared "connection" object, waits
50 ms on the HTTP broker, then writes a shared cache.

| | Rust | QuickJS | Python | Node (today) |
|---|---|---|---|---|
| 16 callers: all done in, ms | 52.5 [51.5–52.8] | 52.9 [51.8–52.9] | 52.4 [52.1–52.6] | 61.2 [56.5–61.3] |
| 16 callers: no lost updates, one connection | yes, yes | yes, yes | yes, yes | yes, yes |
| 16 callers: RSS, MiB | 13.4 | 16.4 | 39.4 | 65.8 |
| 64 callers: calls ok | 16 / 64 (`BUSY`) | 16 / 64 (`BUSY`) | 16 / 64 (`BUSY`) | 16 / 64 (`APP_BUSY`) |

### Streaming and subscriptions

| | Rust | QuickJS | Python | Node (SDK fetch) |
|---|---|---|---|---|
| 50 MiB body, MiB/s | 261 [252–295] | 270 [269–273] | 280 [265–286] | 48 [48–49] |
| worker RSS during it, MiB | 12.6 → 13.3 | 15.6 → 16.6 | 35.9 → 39.8 | 55.5 → 93.0 |
| slow reader: chunks read ahead (cap 4) | 4 | 4 | 4 | – |
| slow reader result | done in 845 ms | stopped by the 1 s CPU slice | stopped by the 1 s CPU slice | – |
| 1,000 events: per second, in order | 42.9k, yes | 43.7k, yes | 41.5k, yes | – |
| slow reader: kernel's delivery ack p50 | 0.16 ms | 12 ms | 35 ms | – |

The slow JS/Python readers show a real rule question. A guest whose
stream reads find data already waiting never hands the CPU back, so one
slice accumulates. A 1 s slice is too tight for heavy stream processing;
DESIGN.md §4 proposes a 5 s default.

### When one call goes wrong (9 calls in flight on one instance)

| bad call | Rust | QuickJS | Python |
|---|---|---|---|
| runaway loop | bad call `APP_CALL_TIMEOUT` at 1.0 s; 8 others `APP_INSTANCE_RESTARTED` (retryable); shared state lost | same | same |
| 256 MiB buffers | bad call `APP_MEMORY_LIMIT` in 26 ms; 8 others `APP_INSTANCE_RESTARTED`; state lost (Rust aborts = trap) | bad call `APP_MEMORY_LIMIT` in 22 ms; **8 others ok; state kept**; RSS stays 143 MiB | same as QuickJS; RSS stays 154 MiB |
| cancel 2 of 10 calls | 8 others ok; both calls and their broker streams released in 12 ms | 8 others ok; worker answers at once; the waiting import keeps its broker stream until it ends (6.0 s) | 8 others ok; cancel not delivered (runs to completion) |
| CPU-heavy call (3M ops) | echo waits 191 ms behind it | 742 ms | 1001 ms (the call is stopped at the 1 s slice) |

The CPU-heavy row is the price of cooperative concurrency, and Node has it
too (its echo waited 95 ms behind the same call): one instance runs one
piece of code at a time.

**Proposed rule** (details in DESIGN.md §4):
1. A call that fails on its own never affects the others.
2. A trap replaces the instance. The culprit gets the stop reason (named
   through wasmtime's task-group hook, correct in every run); the others
   get retryable `APP_INSTANCE_RESTARTED`. The kernel retries only tools
   marked idempotent.
3. After an out-of-memory error the guest handled itself, the instance
   lives on and is recycled at the next idle moment if it is above 75 % of
   its cap.
4. The CPU slice defaults to 5 s, configurable per App up to the call
   deadline.

## QuickJS vs StarlingMonkey

| | QuickJS (componentize-qjs) | StarlingMonkey (componentize-js 0.23) | Node 24.20 |
|---|---|---|---|
| async world | **yes** | no (panics) | – |
| JSON transform / JS-heavy compute, ms | 101.5 / 249.8 | 43.9 / 266.4 | 17.2 / 32.7 |
| RSS after ready / after work, MiB | 15.7 / 30.9 | 33.0 / 69.7 | 55.6 / 114.7 |
| component / precompiled size | 1.5 MB / 4.6 MB | 11.8 MB / 36 MB | – |
| install precompile / cold start without AOT | 122 ms / 110 ms | 820 ms / 889 ms | – |
| out of memory (small objects) | handled in the call, instance lives | engine heap ceiling, instance trapped | process aborts |
| npm: zod, date-fns, lodash-es, nanoid, marked | 4/5 (nanoid: no `crypto.getRandomValues`) | 4/5 (marked: no `\p{…}` in regexes) | 5/5 |
| Web APIs | none (`TextDecoder`, `crypto`, `fetch` absent; SDK polyfills) | `fetch`, `TextDecoder`, `crypto` built in | all |

npm details and commands: `spikes/wasm/npm-compat/README.md`. Bundle with
plain Rolldown. `jco componentize --bundle` with the QuickJS backend emits
`jco:node/*` imports that wasmtime cannot link.

## QuickJS toolchain: risks and fallback

- **Pins used:** componentize-qjs git `cecfa26bd` (2026-10-22), rquickjs
  0.14.0, quickjs-ng 0.16.2, wasi-sdk 33, built against wasmtime 50.0.0.
  The npm 0.4.5 release fails on our world: Wizer stubs async imports as
  sync. That was issue #89, fixed by #91 on 10-21, and v0.5.0 is still an
  open release PR.
- **Single maintainer:** 8.7k lines of Rust, 58 of 60 human commits by one
  author (a jco maintainer has 2). quickjs-ng itself is an active
  multi-maintainer project.
- **Version pinning:** we build componentize-qjs from a pinned commit in our
  toolchain image and record the commit in each package's build metadata.
  We move to npm releases once v0.5.0 ships.
- **Fallback if it stalls:** the output is a standard component and the
  worker never sees the tool. So (1) we can fork and maintain it, since it
  is small and sits on rquickjs and quickjs-ng; (2) or we switch JS Apps to
  ComponentizeJS when it gains async (trigger below). JS Apps rebuild;
  nothing in the world or the worker changes.
- **Reevaluate trigger:** a ComponentizeJS/StarlingMonkey release that builds
  `spikes/wasm/wit/app.wit` (async imports and exports) unchanged. Then run
  `scripts/sweep-async.sh` with it as a fourth wasm variant. Switch if it
  keeps StarlingMonkey's ~2.3x JSON advantage and the size and memory cost
  stays acceptable (today 8x bigger, 2x the RSS).

## Other toolchain risks

| Language | Async world today | Measured here |
| --- | --- | --- |
| Rust | yes (wit-bindgen 0.63, `wasm32-wasip2`) | all cells, full cancel |
| JS/TS | yes (componentize-qjs, pinned git) | all cells; cancel does not stop the pending import |
| Python | yes (componentize-py 0.25.1) | all cells; cannot cancel (traps on an abandoned future read), 19 MB, imports `wasi:sockets` (linked, every operation denied) |
| Go, C/C++, C#, Kotlin | later (your Q5) | not run |

**wasmtime gaps that shaped the design:**
- no host-side cancel of one guest task (#11833), hence the explicit `cancel` future;
- a trap kills every task in the store, hence the restart rule;
- the culprit is only known through the opt-in `task-group-hook` feature.

## Migration plan (bounded PRs)

Sizes: S ≈ 1–2 days, M ≈ 3–5 days, L ≈ 1–2 weeks, for one lane.

1. **Async WIT world 1.0 + allowlist** (S): `packages/app-wit` from
   `wit-full/app.wit` (+ WASI 0.3 deps), versioning tests, import allowlist.
2. **Kernel concurrency for many agents** (M): `WorkerPeer` queues broker
   calls per App (bounded) instead of `BUSY`, cap raised to 64; the
   `events.deliver` live-subscription path and worker-side `cancel {id}`
   handling. Protocol bump, snapshot, drill.
3. **Async wasm worker** (L): from `worker-async/`. Every import of the world,
   host streams, cancel future + grace, CPU slices, culprit attribution,
   OOM recycling, `precompile`/`check`. Tests on the real `WorkerPeer`.
4. **Kernel install + launch** (L): `engine: "wasm"`, install checks, the
   precompile cache in the release store, launch through the native launcher
   (FD3/FD4, bwrap, cgroup).
5. **Sandbox profiles** (M): seccomp for the worker, macOS Seatbelt and the
   executable-memory entitlement (handled like Node's JIT, your Q4).
6. **WASI virtualization** (M): `wasi:http/client@0.3` and the 0.2
   `outgoing-handler` onto the broker; filesystem preopens.
7. **JS SDK on QuickJS + packer** (L): the `register(chariox)` API over the
   WIT imports; `AbortSignal` from `cancel`; polyfills (`TextEncoder/Decoder`,
   `crypto.getRandomValues`, `fetch`); `BigInt` → number; Rolldown bundling in
   `chariox app pack`; pinned componentize-qjs build; templates. Rust SDK
   crate (S inside this PR).
8. **Port Apps + real-path drills** (M): first-party Apps rebuilt (no
   `engine: "node"` window, your Q6); drills with the real kernel, client,
   provider and many agents on one App.
9. **Delete the Node engine** (S): `libnode` build, `bootstrap.cjs`, Node
   patches, `engine: "node"`.

About 8–10 weeks for one lane. PRs 2, 3 and 7 can run in parallel after 1.
Python and the other languages come later as their own S/M PRs (SDK layer
plus a template).

## Owner questions (each with my recommendation)

1. **How many agents may one App serve at once?** Today the kernel refuses
   above 16 waiting calls. *Recommend:* 64 in flight per App, then a short
   bounded queue, then `BUSY`.
2. **Retry calls lost to another call's crash?** *Recommend:* the kernel
   retries once only tools marked `idempotent` in the manifest; others
   return the retryable error to the agent.
3. **CPU slice default.** *Recommend:* 5 s by default, an App may raise it
   to the call deadline (max 30 s); slices over 100 ms are logged.
4. **Python timing.** It works today, except cancel. *Recommend:* not at
   launch; ship it as a preview after Rust and JS, once componentize-py can
   abandon a future read.
