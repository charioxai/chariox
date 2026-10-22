# Chariox Apps on WebAssembly: design (wasmspike r1, 2026-10-22)

Spike, not product code. Base: OSS main `186c31efc`. Measured results and the
recommendation are in [DECISION.md](DECISION.md). MP items: none of the
Browser/Path-1 `MP-*` rows; this touches the App runtime security anchors that
MP-11 keeps (sandbox, App capability admission).

## 1. Shape in one paragraph

An App backend is a WebAssembly **component** that targets a versioned Chariox
WIT world (`chariox:app/app@1.0.0`, [wit-full/app.wit](wit-full/app.wit)). A
new Rust worker binary embeds **wasmtime**, loads the kernel-precompiled
component, and speaks the **existing** worker IPC: FD4 Ready/Continue
(`CXAWR001` record, `CXAWGO01` continuation) and the FD3 wire v1
(`chariox-app-runtime::wire`). Every WIT import is one existing wire method,
so the kernel side (brokers, authorization, schema validation, receipts,
signing/installation, the OS-sandboxed launcher, `WorkerPeer`) does not change.
JS/TS Apps become components with componentize-js (StarlingMonkey); Rust, Go,
Python, C# use their own component toolchains. One runtime for every language.

## 2. The contract: the Chariox WIT world

`wit-full/app.wit` is derived 1:1 from `@chariox/app-sdk` 0.8
(`packages/app-sdk/src/index.d.ts`) and validates with `wasm-tools 1.262`.
The spike builds a subset of it (`wit/app.wit`: registry, state, http, log,
handlers).

Rules:

- **JSON stays JSON.** Tool input/output, event payloads, state values and log
  fields are `type json = string`. Their schemas are already signed JSON
  Schema in the package and validated by the kernel; the wire carries JSON.
  A WIT record per tool would duplicate the schema and break every time an App
  changes a tool. The host parses guest JSON once, with the wire rules (depth
  64, safe integers, no duplicate keys), before any byte is sent: the same
  "an App mistake fails one call, never the channel" rule the JS SDK has.
- **Envelopes are typed.** Errors (`app-error {code, message, retryable}`),
  invocation context, state records/transactions, wakes, occurrences, HTTP
  requests/responses, validation operations are WIT records/variants, so Rust,
  Go, Python and C# authors get real types and the host never guesses.
- **Registration stays imperative.** The guest exports `handlers.register`,
  which calls `registry.tool/event/lifecycle` (the SDK's `tools.register`).
  The worker then sends `worker.ready {tools, events, lifecycle}` exactly as
  the bootstrap does, so the kernel's declaration check is unchanged.
- **Migrations** are separate components on a smaller world
  (`app-migrations`: imports only `state`, exports `migration.migrate`), which
  mirrors the kernel rule that only `state.get`, `state.transaction` and
  `migration.step` are admitted before readiness.

### Versioning

| Thing | Rule |
| --- | --- |
| WIT package | `chariox:app@MAJOR.MINOR.0`. The manifest names the exact world it targets. |
| Minor | Adds interfaces/functions only, gated with `@since(version = 1.N.0)`. A worker that knows 1.N links every 1.x world (missing newer imports are simply absent for older Apps). Replaces today's `sdkVersion` feature detection (`typeof chariox.events.automations === 'function'`). |
| Major | Any changed signature, removed function, or sync→async change. The worker links **two majors side by side** (one `Linker` per world) for one deprecation window; install refuses unknown majors. Maps to today's `appContractVersion`. |
| Wire | Unchanged (v1). A new WIT function that needs a new kernel method follows the normal protocol-change rule (protocol bump, snapshot, drill). |

### Sync now, WASI 0.3 async as the next major

Measured on 2026-10-22 (§8): Rust (`wit-bindgen 0.63`) and wasmtime 50 run
component-model async; componentize-js 0.23 (StarlingMonkey) supports **only
sync** imports and exports ("imported functions can only be synchronous
pending component-model-level async support", its README); componentize-qjs
0.4.5 supports async exports. A world written with `async func` today would
lock out the main JS toolchain.

So world 1.x uses sync-typed functions. The host still implements every import
as an async Rust function: wasmtime suspends the guest on a fiber while the
broker call is in flight, so a waiting App costs no thread and other Apps are
unaffected. What sync costs: one instance runs one call at a time (measured: 16
waiting calls take 830 ms instead of 60 ms). Instances are cheap (10–12 µs
warm instantiate), so the worker runs concurrent calls on a per-App instance
pool (spike: `CHARIOX_WASM_POOL`; 16 calls in 54–60 ms, like Node). Pooled
instances do not share memory, so an App keeps shared data in `state`, not in
globals; the manifest should declare the pool size (default 1). World 2.0 (`async func`, WASI 0.3 streams
for HTTP bodies and file transfer) follows when componentize-js ships async.

## 3. SDK call → IPC message

The worker owns this mapping (spike: `worker/src/host.rs`). Parameters are the
exact wire shapes in `packages/app-sdk/WIRE.md`.

| WIT (guest calls) | Wire request (worker → kernel) |
| --- | --- |
| `registry.tool/event/lifecycle` | none; collected, then `worker.ready {tools, events, lifecycle}` |
| `state.get(key)` | `state.get {key}` → `null \| {value, version}` |
| `state.transact(tx)` | `state.transaction {schemaVersion, checks, writes, occurrences?, wakes?}` |
| `schedule.set/cancel/list` | `schedule.set {id,dueAtMs,revision}` / `schedule.cancel {id}` / `schedule.list {}` |
| `events.emit/status/retry/automations` | `events.emit` / `events.status` / `events.retry` / `events.automations {}` |
| `events.occurrence-id` | none (pure: `evt1.<ms>.<sha256>`, computed in the worker) |
| `connections.list/action` | `connections.list` / `connections.action` |
| `files.atomic-replace/snapshot/import/export` | `files.atomic_replace` / `files.snapshot` / `files.import` / `files.export` |
| `http.send(req)` | `http.open` → `http.write`* → `http.headers` (until not pending) → `http.read`* → `http.cancel` (the SDK's buffered `http.request` sequence, 64 KiB chunks, 512 KiB cap) |
| `log.write` | `log.write {level, message, fields}` |
| `host.notify/open-link/write-clipboard/pick-file` | `host.notify` / `host.open_link` / `host.clipboard_write` / `host.pick_file` (+ `host.pick_file_status`) |
| `validation.request/status` | `validation.request` / `validation.status` |
| migration world: `migrate(from, to)` returns | `migration.step {to}` |

| Kernel request (kernel → worker) | WIT export the worker calls |
| --- | --- |
| `tools.invoke {name, input}` + context | `handlers.call-tool(name, input, invocation-context)` |
| `events.deliver {name, occurrence_id, payload}` | `handlers.deliver-event(...)` |
| `lifecycle.dispatch {event, data?}` | `handlers.lifecycle(event, data)` |
| `schedule.wake {id, dueAtMs, revision, overdue}` | `handlers.deliver-wake(wake, overdue)` |
| `cancel {id}` | sync world: cannot interrupt a running call; the per-call epoch deadline bounds it (§6). Async world: drop the guest task. |

Errors: a guest `app-error` becomes the wire error object unchanged; a guest
trap becomes `APP_CALL_TIMEOUT`, `APP_MEMORY_LIMIT` or `APP_TRAPPED` and the
worker replaces the instance (the bootstrap's exit codes 131–136 stay for
worker-level failures: bad config 130, load/register 131, IPC 133).

## 4. WASI, virtualized onto kernel brokers

The component never sees the host. The worker links only:

| Interface | Implementation |
| --- | --- |
| `chariox:app/*` | worker → FD3 wire → kernel brokers (above) |
| `wasi:http/outgoing-handler` (world 1.1) | worker → the **same HTTP broker stream** (`http.open/write/headers/read/cancel`). wasmtime-wasi-http lets the embedder replace `send_request`, so standard clients (Rust `waki`/`reqwest` on wasi, Go `net/http` on wasip2, StarlingMonkey's global `fetch`) go through the kernel's destination policy, approvals and connections without SDK code. Same role as SDK 0.7's fixed global `fetch`. |
| `wasi:filesystem` | two preopens only: the App's private `data` and `tmp` roots (already quota-limited private mounts). `sync`/`sync-data` are denied like the bootstrap's fsync deny; durable writes use `files.atomic-replace`. |
| `wasi:clocks`, `wasi:random` | direct (no authority). |
| `wasi:cli/stdout/stderr` | the supervisor's bounded log pipes. `environment` empty, `stdin` empty, `exit` ends the instance. |
| `wasi:sockets`, `wasi:http/incoming-handler`, anything else | **not linked.** The install check refuses a component that imports them, and the WASI context denies TCP/UDP/DNS as a second line. |

## 5. Package, manifest and install

- Manifest: `runtime: {engine: "wasm", entry: "runtime/app.wasm", world: "chariox:app/app@1.0.0"}`.
  `RuntimeEngine` (`packages/app-package/src/manifest.rs`) gains `Wasm`; the
  "runtime entry must be JavaScript" check becomes per-engine. Migrations:
  `migrations/NNN.wasm` on the `app-migrations` world. The package signs the
  `.wasm` like any file. The package never contains native code.
- Install checks (kernel, after signature verification), all cheap:
  1. parse/validate the component (`Component::new` in the precompile step);
  2. every import ⊆ the allowlist of the declared world version
     (spike: `chariox-wasm-worker check`, refuses e.g. `wasi:sockets/*`);
  3. required exports present (`handlers`; `migration` for steps);
  4. size limits (component bytes, custom sections stripped from the cache key).
- **AOT precompile + cache:** the kernel runs the worker's `precompile` with the
  worker's exact `Engine` config and stores `app.cwasm` in the release store,
  keyed by (release digest, worker build id, engine config). Loading a `.cwasm`
  is `unsafe` (it is native code), so only kernel-produced files from the
  immutable release store are ever loaded; a worker update re-precompiles.
  Measured (§8): Rust App 20–27 ms, JS App 1.0–1.5 s, once per install/update.

## 6. Resource limits

| Limit | Mechanism (spike) |
| --- | --- |
| Per-call CPU/wall | `epoch_interruption` with a 10 ms ticker thread; deadline = min(request `deadline_ms`, 30 s per-call cap). Expiry traps the guest; the worker answers `APP_CALL_TIMEOUT` and builds a fresh instance. No process kill, no cold start. |
| Memory | a `ResourceLimiter` cap per instance (spike: 128 MiB, same as the V8 heap it is compared with). A refused `memory.grow` returns -1 to the guest instead of trapping, so a GC guest can collect; a trap after a refusal is `APP_MEMORY_LIMIT`. Linear memory is the whole guest heap, so the cap is real (unlike V8 old space, §8). |
| Tables/instances | capped in the same limiter. |
| Stack | wasmtime `max_wasm_stack` (default 512 KiB) traps on overflow. |
| Fuel | not used: epochs bound time at near-zero cost; fuel (instruction counting) is only needed for deterministic metering, which Chariox does not need. |
| Process | unchanged and still required: launcher RLIMITs, cgroup memory/CPU/pids, seccomp, namespaces. wasmtime is the inner, per-call boundary; the OS sandbox stays the containment. |

## 7. What stays and what is replaced

| Stays (unchanged) | Replaced |
| --- | --- |
| Kernel brokers, authorization, JSON-schema validation, receipts, outbox/inbox, wakes | Embedded `libnode` runtime (`libchariox-app-runtime.so`, Node 24.20 source build: the 165-minute CI build) → one Rust worker binary (~20 MB) |
| Package signing, publisher trust, release store, installation | `bootstrap.cjs` and its Node patches (fsync deny, `Worker` guard, macOS `fs.watch` polyfill) → host-side WASI policy |
| Native launcher contract: FD3/FD4, clean env, bwrap namespaces, cgroup, RLIMITs, seccomp, macOS Seatbelt | JS SDK transport (`peer.js`, `transport.js`, framing) → `worker/src/ipc.rs` on the kernel's own `wire` codec |
| Wire v1 and `WorkerPeer` | `runtime.entry` JS-only → `engine: "wasm"` + `.wasm` entry |
| `@chariox/app-sdk` **API** for JS authors (`register(chariox)`, `chariox.state.get`…) | its implementation: a thin layer over the WIT imports, built into the component by componentize-js |

The launcher's seccomp list needs a review for the new binary: wasmtime maps
precompiled code from the `.cwasm` and `mprotect`s it to read+execute once (no
runtime JIT, so it is stricter than V8), runs one ticker thread, and needs no
syscalls Node does not already use.

## 8. How the spike is built (what it does and does not prove)

- `worker/` — `chariox-wasm-worker`: wasmtime 50, `wasmtime-wasi` p2, FD4
  handshake, FD3 wire via `chariox-app-runtime::wire::Channel`, epoch + memory
  limits, trap → fresh instance. Subcommands `precompile` and `check` are the
  install steps.
- `harness/` — plays the kernel: real `WorkerPeer` + `Broker` from
  `chariox-app-runtime`, spawns the worker with a clean environment, FD3/FD4
  socketpairs and optional bubblewrap with the launcher's namespace set
  (user, pid, ipc, net, uts, hostname `chariox-app`, `--cap-drop ALL`,
  read-only `/usr`, private data/tmp binds). Brokers are stand-ins: in-memory
  state with the kernel's version checks; an HTTP stream broker that admits
  one `https://` test origin served on loopback (the worker itself has no
  network: `--unshare-net`).
- `apps/rust-app` (wit-bindgen, `wasm32-wasip2`), `apps/js-app`
  (componentize-js), `apps/node-app` (today's SDK), `apps/jco-app` (the Rust
  component transpiled by jco, inside today's bootstrap).
- `async-probe/` — component-model async on wasmtime 50 (one instance,
  concurrent calls).

**Harness, not the kernel launch path, and why:** the kernel's own launch path
needs the native launcher and the enrolled `libnode` runtime bundle, which no
builder produces yet (`apps/app-worker/README.md`: "No App backend can be
started through this code yet"), and wiring a new engine into
`worker_process` is product work the spike must not do. The harness reuses
the kernel's IPC code itself (`wire::Channel`, `WorkerPeer`, `Broker`), the
launcher's FD3/FD4 contract and its bubblewrap namespace set, so the worker is
exercised on the real protocol. Not covered: the launcher's seccomp filter,
cgroup attachment, the real brokers, macOS.
