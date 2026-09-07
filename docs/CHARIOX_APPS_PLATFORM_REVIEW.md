# Chariox Apps platform review

Reviewed 2026-09-05 against `CHARIOX_APPS_IMPLEMENTATION_PLAN.html` and `CHARIOX_APP_BACKEND_SANDBOX_RESEARCH.md`. Line references below refer to the plan before the review was incorporated and are historical. The updated implementation plan includes these findings and phase-specific validation; it remains the implementation authority. This is a design review, not evidence that an App worker implementation has passed its release gates.

## Recommendation

Keep the Rust kernel, a separate sandboxed Node App worker, private filesystem access, mediated service access, and the managed Chromium App view. macOS and Linux remain Phase 1; Windows remains Phase 2. Node embedding is a supported integration path, although Node explicitly excludes the embedder API from normal semver guarantees. Pin an LTS runtime and maintain its embedding and sandbox policies together. A return to Deno would not resolve the storage, resource, or network-contract issues below. [Node embedder API](https://nodejs.org/api/embedding.html)

Chromium validates the general trusted-process and sandboxed-process architecture. It does not supply a drop-in sandbox for full Node or automatically validate Chariox's additional file and service permissions. Chromium's macOS policy integration uses Seatbelt APIs and must evolve with platform changes. [Chromium macOS sandbox](https://chromium.googlesource.com/chromium/src/sandbox/+/refs/heads/main/mac/), [Chromium Seatbelt implementation](https://chromium.googlesource.com/chromium/src/+/refs/tags/140.0.7263.0/sandbox/mac/seatbelt.cc)

## Findings

### P1: The reviewed-code-only claim is not enforced

Plan line 1106 says installed Apps cannot download code. That does not follow from immutable packages or disabled npm installation. Under the specified capabilities an App can fetch text from an allowed service, evaluate it, or write it to private data and import it. Portable WebAssembly is another executable data format. Node explicitly supports compiling and executing supplied code. [Node VM API](https://nodejs.org/api/vm.html)

State that Chariox installs immutable reviewed packages and prohibits runtime dependency installation as a distribution policy. Do not claim that the OS sandbox distinguishes downloaded code from downloaded data. If immutable executable provenance is a separate requirement, it needs a deliberately narrower runtime contract and dedicated loading tests. Keep the capability boundary effective even when the package behaves maliciously or interprets downloaded instructions.

### P1: macOS memory enforcement is unspecified

V-RUN-02 at line 1312 promises an OS limit that kills only the offending App on both initial platforms. The macOS row at line 943 names process limits and deadlines, but no process-memory mechanism. Seatbelt supplies access restrictions; Chromium's FAQ explicitly allows sandboxed processes to consume CPU and memory. [Chromium sandbox FAQ](https://chromium.googlesource.com/chromium/src/+/main/docs/design/sandbox_faq.md)

Node's worker heap limits exclude external allocations such as ArrayBuffers and cannot establish this guarantee. [Node worker resource limits](https://nodejs.org/api/worker_threads.html#new-workerfilename-options)

The plan needs an explicit macOS enforcement implementation, its required privileges, supported versions, and measured behavior under pressure. Apple XNU's memory-status control path has root or entitlement checks and some hard-limit operations are conditionally compiled. It is not enough to name a Jetsam function. This review has not established a supported unprivileged equivalent to Linux `memory.max`. [Apple XNU memory-status implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_memorystatus.c)

Keep resource protection as a product requirement. Specify how the implementation meets it, including native buffers, WASM memory, JIT memory, thread stacks, concurrent Apps, and broker-side buffers. If using supervisor monitoring, document and test the response bound and overshoot instead of calling it a hard OS memory ceiling.

### P1: A namespace list is not the Linux installation contract

Line 944 chooses user, mount, PID and network namespaces, seccomp, and cgroup v2. This is a reasonable foundation. However, access to delegated cgroups and enabled controllers is an installation requirement; simply starting an unprivileged kernel process does not create it. Linux documents explicit delegation permissions. `memory.max` can also be exceeded temporarily, and `memory.high` is throttling rather than an OOM limit. [Linux cgroup v2 documentation](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html)

Define the supported Linux floor, installer-provisioned helper or service, cgroup delegation, handling of distributions that restrict unprivileged user namespaces, and clean removal. Test fresh supported installations as ordinary users. The installer must establish these facilities so conforming Apps work; do not make every App developer solve platform setup.

### P1: Private Node files need a concrete quota and snapshot implementation

Lines 832 and 1079-1083 combine direct `node:fs`, hard byte limits, multi-file transactions, and consistent snapshots. These are separate storage mechanisms. APFS quota applies to a volume, not an arbitrary directory. The implementation must select and provision a quota-capable private volume or another mechanism that constrains direct writes. It must account for temporary storage, snapshots, metadata, and aggregate host capacity. [Apple APFS volume quota documentation](https://support.apple.com/guide/disk-utility/add-delete-or-erase-apfs-volumes-dskua9e6a110/mac)

SDK transactions cannot isolate against concurrent ordinary file writes that ignore the SDK's coordination. A filesystem snapshot taken at an arbitrary instant provides a filesystem image, not necessarily an application's logical commit point. SQLite's separate online backup machinery illustrates this distinction. [SQLite backup API](https://www.sqlite.org/backup.html)

Define a quiesce protocol that stops new callbacks, drains or stops background writers, closes relevant handles, and checkpoints data before migration or clone. State separately the guarantees for raw Node writes, SDK-managed transactional data, and crash recovery. Test a writer that ignores shutdown, open descriptors during rollback, ENOSPC during snapshot, and mixed raw/SDK access. A migration worker must receive a staged data generation rather than live data still accessible by the old worker.

### P1: The HTTP adapter needs a supported compatibility contract

Global Fetch replacement handles clients that call that function, but it does not cover imported `undici`, custom dispatchers, `node:http`, HTTP/2, or WebSocket automatically. Node's Fetch uses Undici and exposes dispatcher customization. [Node Fetch and dispatcher documentation](https://nodejs.org/api/globals.html#fetch)

Keep the broker, but publish a tested list of transports and adapters with the pinned runtime. The Phase 1 matrix should explicitly cover abort propagation, streamed upload/download, SSE, multipart, backpressure, body-size limits, compression expansion, timeout phases, and broker memory accounting. Decide whether WebSocket is supported; it is common for otherwise ordinary API integrations.

Destination checks must operate on the actual connected address, including redirects, IPv4-mapped IPv6, DNS rebinding, proxy environment, and connection reuse. A malicious worker's own dispatcher must still fail to obtain a raw socket. Credential handles must be bound to installation, connection, and permitted service requests. Opaque injection avoids placing a token in App memory, but does not by itself prove that every permitted remote response cannot echo it.

### P1: Test the OS policy independently of Node's permission layer

V-RUN-07 at line 1317 can pass while a broken OS sandbox remains hidden behind Node denials. Node says its permission model is not a malicious-code boundary and documents existing descriptor and cross-process signaling bypasses. [Node permissions](https://nodejs.org/api/permissions.html)

Use a test-only native probe target through the same production launcher and OS policy, with the runtime permission checks bypassed. Probe filesystem traversal, process signaling, networking, inherited handles, devices, and policy inheritance. Keep the ordinary malicious JavaScript package as a separate compatibility test.

Also change the promise that OS denials always produce stable App errors. Seccomp can terminate the process with SIGSYS, so the supervisor should report a stable failure classification when no JavaScript exception can be returned. [Linux seccomp filter actions](https://docs.kernel.org/userspace-api/seccomp_filter.html)

### P2: Native add-on wording is broader than the technical requirement

Line 1104 correctly treats native add-ons as a separate compatibility class. Per-platform and architecture packaging remains necessary. However, Node-API deliberately provides ABI stability across Node versions when used exclusively; legacy V8 or Node C++ bindings do not have that guarantee. Say that Chariox checks the declared Node-API level or exact native ABI as applicable. Per-artifact signing and review are Chariox policy choices, rather than a universal Node requirement. [Node-API ABI guarantees](https://nodejs.org/api/n-api.html#implications-of-abi-stability)

Do not expand native dependency support before the portable App contract works. It adds packaging, loader, and syscall coverage without helping typical service API integration.

## Additional release gates

1. Run production startup with the final signed and notarized macOS artifact and production Linux installer. Verify sandbox setup precedes any package preload, migration, or handler.
2. Add a native OS-policy probe alongside the JavaScript hostile package. Confirm a deliberately broken policy causes the test to fail.
3. Test memory beyond the JS heap, many simultaneous Apps, broker response buffering, thread pools, startup storms, and host low-memory conditions. Assign numerical latency and resource budgets before declaring the resilience tests passing.
4. Exercise quota exhaustion during snapshots, migration, import, log output, and rollback. Include filesystem metadata and snapshot retention in accounting.
5. Verify snapshot and rollback behavior with persistent background writers and open handles. Read resulting data with the old App version after every injected failure.
6. Run actual service SDK fixtures against the approved HTTP implementation. Cover Fetch and Undici clients, a `node:http` adapter, streaming/SSE, cancellation, reconnect, and the chosen WebSocket policy.
7. Attempt code loading from writable data, fetched strings, data URLs, workers, and WASM. Assert the documented code-loading policy and continued sandbox confinement.
8. Repeat the relevant corpus on each pinned Node security update and supported OS release. Keep a rollback procedure that cannot re-enable a runtime with a known critical sandbox issue.

These are implementation and release checks. They do not require a separate cross-platform feasibility project or move Windows into Phase 1.
