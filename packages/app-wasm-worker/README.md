# Async App worker

`chariox-app-wasm-worker` is the Rust execution path for Apps v2 R3
(MP-08 / MP-09 / MP-10 / MP-11). The kernel owns the installation, verified
manifest, process, generation and admission. One process serves up to 64 calls
on one component instance. This crate does not install Apps or supply OS
containment: those are R4/R5. R6 extends its linker with broker-backed WASI.
There are no ambient filesystem, network, subprocess or native-library imports.

## Entry points

- `check WORLD COMPONENT`: R1 structural admission and real Wasmtime compilation,
  without instantiation or executing App code. This is engine preflight, not
  a containment, trust or complete installation compatibility verdict.
- `precompile WORLD COMPONENT OUTPUT`: the same admission followed by local
  native compilation. Output must not already exist. R4 owns protected cache
  publication, full AOT provenance and any subsequent unsafe deserialization.
  This worker never deserializes caller-supplied native code.
- `serve SUPERVISOR_CONFIG`: run a supervisor-owned JSON launch configuration.
  Fields are `component`, `generation`, `installation_id`, `release_digest`,
  `manifest` (the verified v2 manifest), and optional `limits`. This file is
  supplied by the kernel, never an App-controlled path or retry policy.

`WorkerEngine::{compile,precompile}` are the library entry points. The engine
configuration digest is `engine::configuration_digest()`; R4 must also key its
cache by exact worker build, engine version, target and CPU-feature policy.
`AppWorker::linker` / `with_linker` are the R6 extension seam. Unknown imports
fail linking; production does not install trapping placeholders or expose
Wasmtime's ambient WASI linker.

FD3 carries `chariox_app_runtime::wire::Channel`. FD4 writes `CXAWR001` and the
existing length-prefixed generation, installation and release-digest identity,
then waits for `CXAWGO01` before loading guest code. R4/R5 must establish
containment and generation authority before sending continuation. The first
broker request is `worker.ready`; restart registration must match it exactly.

## Ownership and failure

Every accepted call owns its broker requests, HTTP bodies and subscriptions.
Pending broker requests cancel when dropped. Stream queues have four 64 KiB
response chunks; request bodies forward one bounded chunk per broker write.
Dropping a body or ending a call releases its broker stream. There are at most
128 broker requests and 128 open stream bridges; transport overflow closes the
channel so the kernel can release worker-owned resources.

Deadlines and explicit cancellation signal the guest and close its host
resources. An uncooperative guest gets a bounded grace period, then the shared
instance is replaced. Normal completion closes resources without resolving the
guest cancellation future. Linear memory is capped across all core memories;
refused growth traps with `APP_MEMORY_LIMIT`. Epoch interruption bounds a CPU
slice. Task-group hooks identify the culprit even after a trap unwinds.

An instance loss reports the culprit's stop reason and
`APP_INSTANCE_RESTARTED` for affected calls. Only a tool in the verified
manifest's optional `idempotentTools` list can replay, once, before its original
deadline. The culprit, cancelled calls and non-idempotent operations never
replay. The kernel broker must not add a second automatic replay layer.
Native process death and OS quotas remain the kernel supervisor/R5 boundary.

## Engine compatibility and checks

Wasmtime is pinned to 50.0.0. R1's 0.262 dummy generator imports unused
`stream.forward` / `future.forward`, which this engine rejects. Test-only
reencoding removes those unused imports and adjusts indices; it rejects any
actual use. Production bytes are never rewritten. A real component requiring
those built-ins fails engine preflight. The source drill App uses wit-bindgen
0.63.0, whose unused imports are eliminated by its normal WASM link.

Run the focused unit tests with `cargo test -p chariox-app-wasm-worker`.
Build the source App and run the real subprocess drills through the lane's
`cargo-slot.sh sh packages/app-wasm-worker/scripts/drill.sh`, with
`CHARIOX_WORKER_DRILL_DIR` set outside the repository and the
`wasm32-unknown-unknown` Rust target installed. The script uses the slot's target
and copies the exact guest before releasing it. Full kernel/client App install,
containment and real provider acceptance require R2/R4/R5/R6 integration; these
process drills do not close Stage 1 or MP acceptance rows.

## R4 installed launch and AOT cache

The kernel uses `serve-supervised`, with inherited FD5/6/7/8 as documented in
`app-runtime/src/worker_process/README.md`. `identity` reports the exact running
worker build, engine, target and CPU policy; `precompile-stdio WORLD` compiles
retained verified component bytes for the supervisor's protected cache. The
worker verifies the supervisor's provenance signature, exact runtime key and
artifact digest before native deserialization. Path-based `check`, `precompile`
and `serve` remain the R3 offline development/process-drill interfaces; the
installed kernel launch accepts none of those caller-chosen paths.

Run `scripts/install-drill.sh` through the compile slot for real signed-package
installation, native echo/restart/cache-hit, eviction, damaged/stale input and
offline rebuild process drills. These supplement client/kernel/provider drills
and do not establish R5 containment, R6 WASI/migrations or Stage 1 acceptance.

The installed kernel keeps the `worker.ready` response pending until activation.
The worker serves lifecycle calls while awaiting that response, so precommit
`health_check` can finish without granting the App broker effects early.
The installation process drill preserves this ordering before its first echo.
