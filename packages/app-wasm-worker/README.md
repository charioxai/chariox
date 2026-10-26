# Async App worker

`chariox-app-wasm-worker` is the Rust execution path for Apps v2 R3
(MP-08 / MP-09 / MP-10 / MP-11). The kernel owns the installation, verified
manifest, process, generation and admission. One process serves up to 64 calls
on one component instance. R5 supplies mandatory native containment in `serve`;
R4 owns verified installation/launch integration. R6 extends the linker with broker-backed WASI.
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
then waits for `CXAWGO01` before compiling or running guest code. Native
containment is installed before that ready reply. R4 must establish generation
authority and retain kernel resource/lifecycle supervision before sending continuation. The first
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

## Native containment (R5; MP-08 / MP-09 / MP-10 / MP-11)

`serve` reads the bounded supervisor config and component, discards every FD
above 4, then installs containment before the ready reply, compiler, epoch ticker
or Tokio threads. There is no switch or environment variable to bypass it. The
native worker has no filesystem paths at all: installation data/tmp, network and
process requests go through kernel brokers. R6 must not add ambient native WASI.
Offline `check`/`precompile` compile without instantiating; they do not assert a
contained installation or run any App export.

Linux uses a default-deny seccomp allowlist to remove path access, sockets,
exec/fork, ptrace, foreign signals, namespaces, kernel interfaces, memfd and
io_uring. This ports the v1 native worker's allowlist, thread-only clone,
clone3-to-ENOSYS and self-query-only prlimit checks from
`apps/app-worker/src/sandbox_linux.c`, narrowed for the Rust broker transport.
The v1 Node/bubblewrap/cgroup path is unchanged. No user namespace, mount,
delegated cgroup or privileged host change is required for the WASM entry.
Landlock is not required: all path syscalls are already refused by seccomp,
and inherited descriptors are closed before that boundary.
Unsupported containment fails with `APP_SANDBOX_UNAVAILABLE` before ready.

The Linux executable-memory policy is W^X: seccomp refuses simultaneous WRITE
and EXEC on mmap/mprotect and denies pkey_mprotect; inherited READ_IMPLIES_EXEC
is refused. Wasmtime's RW-to-RX code publication and trusted, already-read AOT
bytes remain admissible. This is a permission policy, not instruction-origin
authentication: native code that has already compromised the worker can still
request an ordinary RX mapping. AOT provenance remains R4's authority.

Hard OS limits cover the whole process, including compiler, runtime and all
threads: on Linux, 1 GiB address space; on both systems, 8 MiB per regular file,
64 descriptors, zero core dump size. Tighter inherited limits are preserved.
Memory mappings fail with ENOMEM; oversized regular-file output stops with
SIGXFSZ/EFBIG. These are
independent of `APP_MEMORY_LIMIT`, `APP_CPU_SLICE` and `INVALID_OUTPUT` returned
by the component/wire boundaries. A native process stop needs a fresh kernel
generation; do not replay a non-idempotent call. R4 must retain its existing
owned-process exit/reap supervision and distinguish native stop from guest trap.
macOS omits the native address-space cap: Darwin rejects `RLIMIT_AS` below
the already-mapped dyld shared region with EINVAL. `RLIMIT_DATA` has the same
VM-map-size check, and `RLIMIT_RSS` does not provide a hard allocation ceiling.
The Mac native memory probe reports this gap explicitly; it does not claim an
ENOMEM boundary. Guest linear-memory limits remain enforced by Wasmtime.
R4 must retain its kernel memory monitor; this worker alone does not bound
native compiler/runtime allocations on macOS.
The worker introduces no lifetime CPU budget: a healthy long-lived generation
must not expire after accumulated successful calls. Epoch slices and call
deadlines bound guest CPU; R4 retains supervisor monitoring for native runaways.
Inherited OS CPU limits remain in force. The native CPU signal probe lowers
only its own child's inherited limit to verify that behavior.
Wasmtime reserves only committed guest memory plus a 64 KiB guard, without
memfd-backed copy-on-write initialization. The engine configuration digest
includes these settings; old AOT entries cannot share the new key.

macOS reuses v1's Seatbelt entry and JIT entitlement, with a broker-only profile
in `src/containment/macos.sb`. It requires a valid hardened-runtime signature,
allow-jit, and no unsigned-code/page-protection/debug/library-validation escape
entitlements, then applies the same hard resource limits and a deny-default
profile. Native files, network, process creation and executable file mappings
are not granted. The production release must be signed; ad-hoc signing is only
for the drill's own disposable copies. Hardened runtime, rather than Seatbelt,
must enforce the JIT exception. The pinned engine's code publisher and the
target macOS version must pass both the native/AOT and real-component drills;
the profile or successful cross-compilation alone cannot establish that proof.

On a Mac with the pinned Rust toolchain and wasm32-unknown-unknown installed:

```sh
bash packages/app-wasm-worker/scripts/macos-containment-drill.sh "$HOME/chariox-r5-evidence"
```

The script builds in disposable state, signs only its own binaries with the
existing minimal entitlement, runs native red/green probes and every source
component subprocess drill, saves source/binary identities and failures, and
removes its scratch on exit. It does not use sudo, change system settings or
touch product accounts. Return its complete evidence directory, including
`result.json` and `worker-stderr.log`. A signing/JIT/resource refusal is a failed
platform gate; do not add broader signing entitlements to make it pass.

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

Run the native hostile corpus with `cargo test -p chariox-app-wasm-worker --test
containment` through the same cargo slot. Its uncontained native probes deliberately
bypass WASM: ordinary base components already cannot import libc, so they cannot
demonstrate a native file/socket/spawn escape on R3. Engine tests reject those
imports and malformed bytes; real source components exercise memory/CPU/huge-output
rejection, same-App recovery and unrelated-App health under the native boundary.
