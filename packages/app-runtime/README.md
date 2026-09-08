# Chariox App runtime services

This crate belongs to the trusted kernel App supervisor. It currently supplies
worker messaging and durable installation metadata. It does not yet launch an
App, establish an OS sandbox, mount quota volumes, or perform migrations.

`wire::Channel` uses the SDK's [versioned wire contract](../app-sdk/WIRE.md).
Split it into a dedicated reader and writer for concurrent tool dispatch and
worker broker calls. Both halves share terminal failure state; cancellation or
failure during frame I/O closes admission on both and wakes blocked operations.
Messages have bounded frames and JSON depth, exact installation generations,
and direction-specific caller-context validation. The supervisor still owns
bounded mailboxes, request correlation, caller authorization and worker death.
A `worker.ready` request is application readiness, never proof of containment.

`installation::InstallationRegistry` borrows the kernel-owned SQLite connection.
The caller supplies durability settings and verified release/approval metadata.
It allocates non-reusable candidate generations, records staged approvals and
preparation, and commits release/schema/capability/catalog/view metadata in one
transaction. Admission pauses before preparation. Only a precommit candidate
can abort; committed data cannot be reverted through this API. The supervisor
must stop existing writers, prepare snapshots, run restricted migrations, prove
worker health and enforce the registry's generation at every operation boundary.
Uninstall deactivates installation metadata without deleting user-owned assets.

## Focused verification

```sh
CARGO_BUILD_JOBS=1 cargo test -p chariox-app-runtime -- --test-threads=1
```

Tests exercise SQLite reopen and injected transaction failures, stale independent
connections, generation exhaustion, approval ordering and postcommit write
preservation. Transport tests cover partial frames, backpressure, cancellation,
concurrent duplex traffic and shared failure wakeups. The same malformed-message
corpus runs in Rust and Node; an actual Node SDK process round-trips Rust frames.
These are component checks, not the full Phase 1 release or sandbox gates.
