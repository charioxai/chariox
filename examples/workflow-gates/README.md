# Pure workflow entry gates (MP-08 / MP-09 / MP-10 / MP-11)

`advisory` is an import-free Rust gate. It routes public critical/high facts to
its first allowed entry node; other facts join a fixed summary window and the
flush selects its second entry node. `batch_window_ms` is revisioned configuration.
The example keeps an evaluation counter in gate-owned workflow state. It neither
reads a clock nor imports the App host. Its borrowed JSON view preserves complete
fact bytes without introducing floating point instructions into the component.

With Rust 1.99.1 and `wasm32-unknown-unknown` installed, build outside a repository
(`TMPDIR` and Cargo's target directory must also be outside):

```sh
cargo build --manifest-path examples/workflow-gates/advisory/Cargo.toml --locked --release --target wasm32-unknown-unknown
cargo run -p chariox-app-sdk-tools --bin chariox-gate-component -- \
  "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/advisory_entry_gate.wasm" "$TMPDIR/advisory-gate.wasm"
```

On a shared builder run every Cargo command through its build-slot wrapper.
The encoder refuses imports, floating point, threads, starts and other features
outside the gate profile. It never adds a WASI adapter.

The normal shell/TUI commands are:

```text
/workflow new gate-upload
/workflow gate put gate-upload /absolute/path/advisory-gate.wasm
/workflow code apply /absolute/path/advisory-workflow.js
/workflow gate entries advisory
/workflow gate entry advisory <entry-id>
/workflow gate state advisory [binding-id]
/workflow gate reset-state advisory <reason>
/workflow gate cancel advisory <entry-id> <reason>
```

The upload response supplies the module digest. Define the workflow with
`w.define({alias:"advisory",gate:{abi:"gate_v1",module_sha256,config:{batch_window_ms:60000},config_sha256}})`.
`config_sha256` covers canonical JSON (keys sorted, integer literals only).
Declare two nodes and `w.endpoint([triage,summary],{alias:"entry"})`. The gate uses
those kernel-resolved IDs from `allowed_entry_nodes`, rather than inventing IDs.
A module is scoped to the owner and may be reused across that owner's workflows.
A source export preserves gate/config digests; transfer the portable WASM alongside
it and upload it for the destination owner before applying.

`workflow gate test <workflow> <module-digest> <evaluation.json>` runs the same
contained evaluator without changing persistent state or creating a provider run.
Inspect the returned report and fuel. Entry inspection returns recorded evaluations,
state revisions, decisions, module/config hashes and failures. An explicit state
reset cancels pending work and records its actor/reason; it does not undo a queue
handoff or provider run.

`passthrough-app` is the Rust WASM App source used by Upwatch's bridge bundle.
It validates through the signed incoming declaration, preserves `data`, and emits
one deterministic occurrence to each active `public_advisory` automation. Routing
and batching policy live in the workflow. Build it with the ordinary App Rust
builder (`packages/app-sdk-tools/build-rust.py`), then package it with the official
App packer and enroll/install through the normal publisher and capability decisions.
No fixture installation metadata or Node backend is needed.

Gate admission is kernel-owned for explicit workflow manual/Room, HTTP, schedule,
App automation and notification entries. Generated agent-trigger workflows reject
this gate setting. The kernel persists an entry before evaluation; a deferred or
batched entry has no provider run until its selected routes reach the ordinary
workflow queue. Room supervision follows the eventual run, and reports a drop or
failure without inventing a run. Batch aggregates always use queued delivery.
They preserve every member's provenance and earliest source deadline; cancellation
of any member cancels the pending aggregate, including an active flush claim.

Build `chariox-app-wasm-worker` with the release profile for actual use. The same
R5 containment, fuel, memory, CPU and elapsed-time limits apply to upload, Test and
live evaluation. A debug worker can exceed those limits during compilation.

The journal refuses new admission at its fixed pending/history/body limits.
Replay bodies compact after seven days or earlier source expiry; inspection then
returns `replay_compacted_at_ms` and retained decision summaries instead of a
replayable evaluation. Terminal metadata and source dedupe survive through the
original deadline plus 30 days. Pending entries are never removed to reclaim
history space. Module reclamation keeps live workflow and replay references.

This backend slice exposes gate history through the gate entry requests and
commands. Ordinary workflow run history lists actual runs. Inspector UI and
utility-agent authoring are separate work; a passing native example does not
establish the full cross-platform, managed-placement or long-soak gate matrix.
