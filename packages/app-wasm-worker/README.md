# Async App worker

`chariox-app-wasm-worker` is the Rust execution path for Apps v2 R3
(MP-08 / MP-09 / MP-10 / MP-11). The kernel owns the installation, verified
manifest, process, generation and admission. One process serves up to 64 calls
on one component instance. This crate does not install Apps or supply OS
containment: those are R4/R5. R6 links the enumerated WASI imports through
scoped filesystem capabilities and the existing kernel HTTP broker. There are
no ambient filesystem, network, subprocess or native-library imports.

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
  `manifest` (the verified v2 manifest), optional `limits`, and optional
  `installation_root` (an absolute supervisor-owned path). Only its existing real
  `data` and `tmp` children are preopened; missing/symlinked children are refused.
  Omission exposes no filesystem preopens. This file is
  supplied by the kernel, never an App-controlled path or retry policy.

`WorkerEngine::{compile,precompile}` are the library entry points. The engine
configuration digest is `engine::configuration_digest()`; R4 must also key its
cache by exact worker build, engine version, target and CPU-feature policy.
`AppWorker::linker` is the production allowlisted linker, also used by `new`.
Unknown imports fail linking; production does not install trapping placeholders
or call the upstream all-interfaces WASI linkers. `with_linker` remains available
for explicit supervisor/test extensions, never for App-supplied linker policy.

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

## WASI brokers and installation directories (R6)

The linker implements precisely R1's WASI 0.3 surface and its versioned 0.2.0
compatibility imports. Both HTTP versions use `http.open/write/headers/read/cancel`
on the kernel wire. The upstream ambient HTTP transport feature is disabled;
there is no socket, DNS, incoming HTTP, process, environment or terminal linker.
Standard HTTP cannot select opaque connection/effect handles: use `chariox:app/http`
when an operation needs those explicit authorities. HTTP destinations and effects
remain subject to the kernel's normal manifest/grant/approval checks.
Wasmtime's synthesized `Host` header is disabled for this broker; the kernel
derives HTTP authority from the approved URL and still rejects App-supplied Host.

Responses and uploads use bounded chunks and backpressure. Dropping a response,
including an early response during an unfinished upload, cancels both pumps and
releases the broker stream independently of invocation completion. Invocation
cancellation/deadlines also release both. Explicit WASI request timeouts bound the
response phases. The broker does not expose connection-phase timing, so setting
an explicit connect timeout returns WASI `not-supported` (0.3) / an error (0.2);
clearing it succeeds. It never pretends local stream admission measures TCP/TLS.
Nonempty request trailers are rejected because the
broker has no trailers operation; they are never silently discarded.

The supervisor opens `InstallationDirectories::open(root)` after establishing
installation/generation authority and OS containment, then passes it to
`AppWorker::with_directories`. `root/data` and `root/tmp` must already be real
directories. Only those two names are preopened; no host root, cwd, environment
or inherited descriptors are exposed. Without this explicit configuration there
are no preopens. WASI 0.2 and 0.3 share the same installation files. Absolute
paths, traversal outside a preopen and escaping symlinks return WASI
`not-permitted`; broker-forbidden HTTP returns `http-request-denied`. Other
broker failures retain stable WASI errors or an internal-error containing only
the broker reason code. Stdout/stderr go to bounded `log.write` frames, not the
worker's native stdout/stderr.

The optional supervisor-owned `installation_root` in the CLI launch JSON
connects this same library seam to R4's verified installation directories.
The kernel and worker are built/shipped together; this internal launch field
changes no daemon, relay or client protocol version (coordinator authorization,
2026-10-26). R5 must apply filesystem quotas and OS containment. Installation snapshots, generation fencing and
private-file rollback remain supervisor responsibilities.

## Restricted migration world (R6)

`MigrationWorker` executes `chariox:app/migration@1.0.0` in a fresh one-shot
instance. Its linker contains only transactional state, with no WASI, HTTP,
registry, filesystem, agents, workflows or host imports. The existing kernel
staged-generation state broker must continue to refuse occurrences/wakes and
other effects during migration. Memory, CPU-slice, deadline and cancellation
grace limits apply; a failed migration is never replayed by this worker.
Cancelled or expired scopes are rejected before guest entry and checked again
before accepting a result, so guest success cannot override a stop reason.

R4 must verify the signed migration artifact, fence old writers, checkpoint
structured and private-file state, invoke the migration under the staged broker,
and record `migration.step` only after success. This API does not activate a
release, advance the kernel migration journal or claim that SQL rollback restores
private files. No serialized migration-run request or launch mode is added here.

## Additional real-engine drills

`scripts/wasi-policy-drill.sh` builds the source WASI guest and runs both HTTP
versions through the real Wasmtime adapters, wire decoder and kernel HTTP policy.
Run it through `cargo-slot.sh` with an external `CHARIOX_WORKER_DRILL_DIR`.
It requires no controlled HTTP broker reply or network service, and checks that
URL authority survives while App-supplied Host remains forbidden. This is a
policy-boundary regression drill, not installed-App or live network acceptance.

`scripts/sdk-drill.sh` builds the actual C/C++ templates and behavioral components
from `CHARIOX_SDK_CHECKOUT`, using `CHARIOX_C_TOOLCHAIN`, then runs them in the
real worker subprocess. Set `CHARIOX_WORKER_SDK_DIR` and
`CHARIOX_WORKER_DRILL_DIR` outside the checkout, and run through `cargo-slot.sh`.
The SDK fixture checks stream bytes at EOF, yield cancellation, cancellation
of a pending import, and exactly-once invocation/coroutine destruction. A
composed caller writes the canonical cancellation future before SDK entry and
checks that no App code starts. The exact `STARTED_CANCELLED` import status is
covered by the frozen SDK native tests; this real-host drill does not force it.

`scripts/wasi-drill.sh` builds the source WASI and migration guests, then runs
real Wasmtime worker-library checks for both HTTP versions, broker denials,
scoped data/tmp, traversal/symlink rejection, clocks/random/output and restricted
migration execution/cancellation/deadlines/traps. Its supervisor replies are
controlled broker test inputs. These source/process checks are supplementary;
real signed install/launch, kernel broker, OS containment, relay/client/provider
and ordinary/managed acceptance still require R4/R5 integration.
