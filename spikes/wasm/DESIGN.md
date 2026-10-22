# Chariox Apps on WebAssembly: design (wasmspike r2, 2026-10-22)

Spike, not product code. Branch `wasmspike/r2` (from r1 `249de8384`, OSS main
`186c31efc`). Measured results and the recommendation: [DECISION.md](DECISION.md).
MP items: none of the Browser/Path-1 `MP-*` rows; this touches the App runtime
security anchors MP-11 keeps (sandbox, App capability admission).

r2 applies Miguel's decisions of 10-22: one runtime (wasmtime), one process
per App, **the App contract is async from the start** (WASI 0.3,
Component Model async, no sync 1.x), JS through **QuickJS**
(componentize-qjs), `engine: "node"` packages are rebuilt.

## 1. Shape in one paragraph

An App backend is a WebAssembly **component** targeting the async Chariox
WIT world `chariox:app/app@1.0.0` ([wit-full/app.wit](wit-full/app.wit)). A
Rust worker binary embeds **wasmtime 50**, loads the kernel-precompiled
component and runs **one instance per App that serves every call at once**:
each kernel request becomes a guest task, each broker call a concurrent host
call, so in-memory App state (caches, counters, a long-lived connection
object) is shared by all callers, as in today's Node worker. The worker speaks
the **existing** worker IPC: FD4 Ready/Continue (`CXAWR001`/`CXAWGO01`) and FD3
wire v1 on the kernel's own codec. The kernel side (brokers, authorization,
schema validation, receipts, signing/installation, the OS-sandboxed launcher,
`WorkerPeer`) stays. JS/TS Apps become components with componentize-qjs
(QuickJS-ng); Rust uses wit-bindgen; Python componentize-py.

## 2. The contract: the async Chariox WIT world

`wit-full/app.wit` is derived 1:1 from `@chariox/app-sdk` 0.8 and validates
with `wasm-tools 1.262` against the WASI 0.3.0 packages shipped with
wasmtime-wasi 50 (`wit-full/deps`). The spike builds a subset,
[wit/app.wit](wit/app.wit) (`chariox:app@0.2.0`: registry, state, http,
events, log, handlers), with all three guest languages.

Rules:

- **A call into the kernel is `async func`.** state, schedule, events,
  connections, files, http, host, validation. The guest awaits; the instance
  keeps serving other calls meanwhile.
- **A call that never leaves the worker stays a plain `func`.** registry,
  `log.write` (queued fire-and-forget), `events.occurrence-id` (pure).
- **Unbounded data is a `stream`.** HTTP request and response bodies
  (`http.fetch`), `files.atomic-replace` contents, live event feeds
  (`events.subscribe`). A `list<u8>` is only used where the kernel already
  caps the size.
- **Every handler export is `async` and gets `cancel: future<string>`.** It
  resolves with a reason when the kernel cancels the call (wire `cancel {id}`)
  or its deadline passes; it never resolves for a call that ends normally.
  SDKs expose it as `AbortSignal` (JS), a cancellation future (Rust) or a
  token (Python). Why an explicit future and not Component Model task
  cancellation: wasmtime 50 cannot cancel one guest task from the host
  (`call_concurrent` docs, wasmtime issue #11833; only dropping the whole
  store stops it). The explicit future works today on every toolchain that
  can race futures (§9). When wasmtime ships host-side `task.cancel`, the
  worker adds it after the grace period as a hard stop: additive, no world
  change.
- **JSON stays JSON**, **envelopes are typed**, **registration stays
  imperative** (`handlers.register` → `registry.*` → `worker.ready`),
  **migrations** use the small `app-migrations` world: unchanged from r1.

**Events have two paths.** Durable occurrences keep arriving as
`handlers.deliver-event` calls (receipts, retries, many in flight at once
now). `events.subscribe(name) -> stream<live-event>` is a new, non-durable
live feed for as long as the App holds the stream (presence, connection
pushes). Delivery is acknowledged to the kernel only once the event sits in
the stream's bounded buffer, so a slow App slows the sender instead of
growing memory (measured, DECISION.md).

### Versioning

| Thing | Rule |
| --- | --- |
| WIT package | `chariox:app@MAJOR.MINOR.0`; the manifest names the exact world. There is no sync 1.x: 1.0 is async. |
| Minor | Adds interfaces/functions only, gated with `@since(version = 1.N.0)`. A worker that knows 1.N links every 1.x world. Replaces today's `sdkVersion` feature detection. |
| Major | Any changed signature or removed function. The worker links two majors side by side for one deprecation window; install refuses unknown majors. Maps to `appContractVersion`. |
| WASI | The world imports WASI **0.3.0** (`wasi:http/client`, clocks, random, filesystem, cli). Toolchains still emit 0.2 imports for their own runtime (Rust std on `wasm32-wasip2`, QuickJS, CPython: measured); the worker links 0.2 as a compatibility layer until they move to 0.3 (`wasm32-wasip3`). |
| Wire | v1 plus two additions (§3): `events.deliver` for a subscribed name, and the worker acting on `cancel {id}`. Each follows the protocol-change rule (bump, snapshot, drill). |

## 3. SDK call → IPC message

The worker owns this mapping (spike: `worker-async/src/host.rs`). Shapes are
the wire shapes in `packages/app-sdk/WIRE.md`.

| WIT (guest calls) | Wire request (worker → kernel) |
| --- | --- |
| `registry.tool/event/lifecycle` | none; collected, then `worker.ready {tools, events, lifecycle}` |
| `state.get` / `state.transact` | `state.get {key}` / `state.transaction {...}` |
| `schedule.*`, `events.emit/status/retry/automations`, `connections.*`, `files.*`, `host.*`, `validation.*` | the same method as today, one call each (r1 table) |
| `http.fetch(req, body)` | `http.open` → `http.write`* (from the body stream) → `http.headers` → `http.read`* feeding the response stream → `http.cancel` when the stream is dropped, the call is cancelled or the body ends |
| `wasi:http/client.send` | same broker sequence (virtualized; r1 proved the 0.2 `outgoing-handler` route with StarlingMonkey's `fetch`) |
| `events.subscribe(name)` | none at subscribe time; the worker routes the kernel's `events.deliver {name, data}` for that name into the stream |
| `log.write` | `log.write {...}`, queued, never awaited by the guest |

| Kernel request (kernel → worker) | What the worker does |
| --- | --- |
| `tools.invoke {name, input}` + context | starts a guest task `handlers.call-tool(name, input, context, cancel)` on the App's one instance |
| `events.deliver` (durable occurrence) | starts `handlers.deliver-event(...)` |
| `events.deliver` for a subscribed live name | enqueues into the subscription; answers when enqueued (backpressure), `NO_SUBSCRIBER` (retryable) if none |
| `lifecycle.dispatch`, `schedule.wake` | start `handlers.lifecycle` / `handlers.deliver-wake` |
| `cancel {id}` | resolves that call's `cancel` future; after a 250 ms grace the worker answers for it if the guest did not |

## 4. Concurrency, limits and failure rules (the worker)

One `Store` and one instance per App, driven by `Store::run_concurrent`. The
IPC reader feeds requests into the store's event loop; each request is a
guest task; each import is an `Accessor`-based host function, so a waiting
call holds nothing. A component instance runs one task on the CPU at a time:
concurrency is cooperative, exactly like Node's event loop.

| Rule | Mechanism | Measured |
| --- | --- | --- |
| **Wait time** is bounded per call by the kernel deadline. | Tokio timer per call; at the deadline the worker resolves `cancel` and, after 250 ms grace, answers `APP_CALL_TIMEOUT` itself. A guest that ignores `cancel` leaves a parked task (wasmtime cannot drop one task) that is gone when the instance is next recycled. | DECISION.md, cancellation |
| **CPU time** is bounded per *slice*: the time the guest holds the CPU without returning to the event loop. | `call_hook` marks slice start/end; epoch ticks every 10 ms; the epoch callback yields to the host each tick (timers and IPC keep running) and traps when one slice exceeds the slice budget (spike: 1 s). | runaway loop stopped at 1.0 s, next call 0.2–1.4 ms |
| A **trap** ends the instance and every call on it. | wasmtime cannot remove one task from a store. The worker answers each in-flight call, builds a fresh instance and serves the queue. | 8 healthy calls answered with the culprit, same process |
| The **culprit** is named. | wasmtime's `TaskGroupHook` (feature `task-group-hook`) reports which call's task entered the CPU; the call holding it at the trap gets `APP_CALL_TIMEOUT` / `APP_MEMORY_LIMIT` / `APP_TRAPPED`; the others get `APP_INSTANCE_RESTARTED` with `retryable: true`. | attribution 100 % in the sweep |
| **Memory** is capped per App (one instance = one linear memory). | `ResourceLimiter`; a refused grow returns -1. GC guests (QuickJS, CPython) turn it into an exception inside the offending call: only that call fails (`APP_MEMORY_LIMIT`), the instance and its shared state live on. Rust aborts, which traps (rule above). | QuickJS/Python: 8/8 others ok, counter kept |
| **Backpressure** everywhere. | Response bodies: at most 4 × 64 KiB chunks read ahead of the guest. Subscriptions: 16 events buffered, kernel ack after enqueue. | 50 MiB stream: +1 MiB RSS |
| **Resources follow the call.** | A dropped body stream or a cancelled import drops the worker's broker-stream guard, which sends `http.cancel`. | Rust: all released 12 ms after cancel |

**Proposed rule for the other calls when one call fails** (for the product
worker):

1. A call that fails on its own (app-error, guest-handled OOM, cancel) never
   affects the others.
2. A trap (runaway slice, Rust OOM, guest bug) replaces the instance: the
   culprit gets the stop reason; every other in-flight call gets
   `APP_INSTANCE_RESTARTED` (retryable). The kernel retries a call once
   only if it is marked idempotent in the manifest (tools default: no);
   otherwise the agent sees the retryable error. In-memory state is lost,
   exactly as with a Node crash today; durable data belongs in `state`.
3. After a guest-handled OOM the instance keeps its (grown, never shrunk)
   memory. The worker recycles it at the next idle moment (no call in
   flight) if it is above 75 % of the cap.
4. Slice budget default 5 s, configurable per App up to the call deadline;
   the spike used 1 s to compare with r1. A slice that blocks longer than
   100 ms is logged with the call name so authors see head-of-line blocking.

**Kernel caps today.** `WorkerPeer` admits at most 16 concurrent broker
handlers per worker (`PeerLimits::validate`: `1..=16`) and answers `BUSY`
beyond that; today's Node worker has its own 16-handler cap
(`APP_BUSY app_handler_capacity_full`). So 64 agents waiting at once on
one App get 48 `BUSY` answers with either runtime (measured). "Many agents on
one App" needs a kernel change: queue broker calls (bounded, per App) instead
of refusing, and raise the cap. That is migration PR 3.

## 5. WASI, virtualized onto kernel brokers

| Interface | Implementation |
| --- | --- |
| `chariox:app/*` | worker → FD3 wire → kernel brokers |
| `wasi:http/client@0.3` (and 0.2 `outgoing-handler`) | the same broker stream as `http.fetch` (`WasiHttpHooks::send_request` in r1) |
| `wasi:filesystem` | two preopens only: the App's private `data` and `tmp`; fsync denied like the bootstrap; durable writes use `files.atomic-replace` |
| `wasi:clocks`, `wasi:random` | direct |
| `wasi:cli/stdout/stderr` | the supervisor's bounded log pipes; `environment`/`stdin` empty |
| `wasi:sockets` | **linked, every operation denied.** CPython (componentize-py) always imports it; the WASI context denies TCP, UDP and DNS, and bwrap has no network. Install refuses sockets for any other toolchain. |
| anything else | not linked; install refuses the component |

## 6. Package, manifest and install

- Manifest: `runtime: {engine: "wasm", entry: "runtime/app.wasm", world: "chariox:app/app@1.0.0"}`; `engine: "node"` is removed (packages are rebuilt).
- Install checks after signature verification: parse/validate, imports ⊆
  allowlist of the declared world (spike: `chariox-wasm-worker-async check`),
  required exports, size limits.
- **AOT precompile + cache** with the worker's exact engine config into the
  release store, keyed by (release digest, worker build, engine config); only
  kernel-produced `.cwasm` from the immutable store is ever loaded. Measured
  precompile: Rust 23 ms, QuickJS 122 ms, Python 0.7 s, StarlingMonkey 0.8 s.

## 7. Guest toolchains (pinned for the spike)

| Language | Toolchain | Async world | Notes |
| --- | --- | --- | --- |
| Rust | rustc 1.99.1, `wasm32-wasip2`, wit-bindgen 0.63.0 | yes | cancel by dropping the future: imports are cancelled too |
| JS/TS | componentize-qjs **git `cecfa26bd`** (main, 10-22; npm 0.4.5 cannot build async *imports*: its Wizer stubs them as sync, fixed upstream by #91, unreleased), rquickjs 0.14.0, quickjs-ng 0.16.2, wasmtime 50.0.0 inside, wasi-sdk 33 | yes | no Web APIs (`TextDecoder`, `crypto`, `fetch` absent); `u64` arrives as `BigInt`; `Promise.race` on `cancel` does not cancel the pending import |
| Python | componentize-py 0.25.1, CPython 3.14 | yes | 19 MB component; cannot cancel a pending future read (instance traps "resource has children"), so the Python SDK closes `cancel` and runs to completion |
| (r1) JS | componentize-js 0.23.0 (StarlingMonkey) | **no** (panics on async) | kept only as the comparison |

The JS SDK is a QuickJS-side layer: `register(chariox)` API unchanged,
`AbortSignal` from `cancel`, polyfills for `TextEncoder/TextDecoder`,
`crypto.getRandomValues` (over `wasi:random`) and `fetch` (over `http.fetch`),
`BigInt`→`number` for safe integers. `chariox app pack` bundles npm
dependencies with Rolldown (plain bundle; `jco componentize --bundle` with
the QuickJS backend emits `jco:node/*` imports that wasmtime cannot link,
measured).

## 8. What stays and what is replaced

| Stays (unchanged) | Replaced |
| --- | --- |
| Kernel brokers, authorization, schema validation, receipts, outbox/inbox, wakes | Embedded `libnode` runtime → one Rust worker binary |
| Package signing, publisher trust, release store, installation | `bootstrap.cjs` and its Node patches → host-side WASI policy |
| Launcher contract: FD3/FD4, clean env, bwrap, cgroup, RLIMITs, seccomp, Seatbelt | JS SDK transport → `ipc.rs` on the kernel's `wire` codec |
| Wire v1 and `WorkerPeer` (plus: queue instead of `BUSY`, §4) | `engine: "node"` → `engine: "wasm"` |
| `@chariox/app-sdk` **API** for JS authors | its implementation: a layer over the WIT imports inside the QuickJS component |

## 9. How the spike is built (what it does and does not prove)

- `worker-async/` — `chariox-wasm-worker-async`: wasmtime 50 with features
  `call-hook` and `task-group-hook`; one instance per App on
  `run_concurrent`; Accessor-based imports; host `StreamProducer`s for
  bodies and subscriptions; per-call cancel futures; slice budget; culprit
  attribution; `precompile`/`check`.
- `worker/` — the r1 sync worker, kept to measure the r1 Apps in the same run.
- `harness/` — plays the kernel with the real `WorkerPeer`/`Broker` and the
  kernel's `PeerLimits` defaults; bubblewrap with the launcher's namespace
  set. Cells: the r1 set, `shared`, `stream`, `subscribe`, `cancel`, `blast`,
  `hol` (`harness/src/cells.rs`). Brokers are stand-ins (state with version
  checks; an HTTP stream broker with a loopback origin and a synthetic large
  body).
- `apps/rust-async`, `apps/qjs-app`, `apps/py-app` — the same tools on the
  async world; `apps/node-app` — today's SDK, with the same `shared` and
  `stream` tools.
- `npm-compat/` — five npm packages on QuickJS and StarlingMonkey.

Not covered: the kernel's own launch path (needs the native launcher and an
enrolled runtime bundle), real brokers, seccomp/cgroup, macOS, `wasi:http`
0.3 client virtualization (r1 proved the 0.2 route), the `files` and
`schedule` imports, and C/C++ (Miguel: later).
