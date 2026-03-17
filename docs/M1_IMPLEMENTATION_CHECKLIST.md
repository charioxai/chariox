# M1 Implementation Checklist

## Status

Execution checklist for **M1 - Core Session Runtime**.

M1 is complete.

This checklist translates the M1 roadmap milestone into concrete implementation steps for the repository state after M0.

## 1. Target M1 outcomes

From `docs/ROADMAP.md`, M1 outcomes are:

- daemon process lifecycle
- session lifecycle (`create`, `attach`, `detach`, `end`)
- PTY manager for provider runs
- multi-attachment support with daemon-owned prompt queueing and canonical config propagation
- parked provider run support with one active run at a time
- workflow-compatible runtime ownership so later multi-agent graph execution can reuse session/provider/worktree foundations without redesign

Exit criteria:

- local client can run a native provider in a managed session
- multiple clients can attach without breaking terminal behavior
- current runtime design does not block future workflow definitions, node-scoped provider runs, structured handoffs, or explicit worktree assignments

## 2. M1 implementation principles

- Keep all core runtime ownership in the daemon.
- Preserve provider-native PTY behavior.
- Separate terminal streaming from structured session/control state.
- Treat local CLI and future remote/full-terminal clients as the same attachment model.
- Prefer additive protocol/domain changes and keep the server lightweight.
- Keep session/provider/worktree ownership compatible with future multi-agent workflow mode.
- Avoid topology-specific assumptions in M1 runtime APIs; future workflow execution must be able to compose on top of the same daemon-owned services.

## 3. Planned M1 deliverables

```text
apps/
  daemon/
    src/
      lib.rs
      main.rs
      app.rs                  # daemon bootstrap / lifecycle entry
      config.rs               # daemon runtime config
      session/
        mod.rs
        service.rs            # session lifecycle orchestration
        store.rs              # in-memory runtime state
        types.rs              # runtime-only session state structs
      attachment/
        mod.rs
        service.rs            # attach/detach and session membership logic
      provider/
        mod.rs
        registry.rs           # provider adapter lookup
        process.rs            # launch/park/terminate provider runs
      pty/
        mod.rs
        manager.rs            # PTY spawn/read/write/resize
      terminal/
        mod.rs
        stream.rs             # fan-out for PTY output and input ingestion
      tests/
        session_runtime.rs
        attachment_runtime.rs
        provider_runtime.rs
packages/
  domain/
    src/
      index.ts                # domain type expansions for M1 if needed
      index.test.ts
docs/
  M1_IMPLEMENTATION_CHECKLIST.md
```

File/module names may change, but these responsibilities must exist by the end of M1.

The M1 implementation is not required to ship full workflow execution, but every runtime boundary introduced here MUST remain compatible with:

- `WorkflowDefinition`, `WorkflowNode`, `WorkflowEdge`, `WorkflowRun`, `NodeRun`, `NodeMessage`, `WorktreeAssignment`, and `AggregationState`
- coordinator-owned start/stop/completion decisions
- node-scoped provider runs
- explicit worktree isolation for parallel code-writing branches
- structured handoff/completion contracts instead of transcript forwarding

## 4. Implementation checklist

## 4.1 Daemon runtime skeleton

- [x] Add a daemon runtime entry layer that can initialize config, runtime services, and shutdown handling.
- [x] Define a daemon application state object that owns the real runtime services implemented so far and can expand as new M1 responsibilities land.
- [x] Add structured error types for runtime operations instead of relying on ad hoc strings.
- [x] Decide the async runtime baseline for the daemon and document it in code/comments where the runtime is initialized.

## 4.2 Session lifecycle

- [x] Implement session creation with runtime state for:
  - workspace id
  - worktree id
  - host machine id
  - host daemon id
  - active provider run id
  - attachment set
  - prompt queue state and canonical config state hooks
- [x] Implement session lookup/listing in daemon memory.
- [x] Implement session termination/end behavior.
- [x] Define session state transitions and reject invalid transitions with explicit errors.
- [x] Add unit tests for create/get/end session behavior.

## 4.3 Attachment lifecycle and shared-session interaction model

- [ ] Implement attachment join for a session with capability level and shared participation semantics.
- [ ] Implement attachment detach and cleanup.
- [ ] Implement daemon-owned prompt queueing for prompts submitted while another prompt is running.
- [ ] Notify all other attachments in the session when a prompt is queued and expose canonical queue state.
- [ ] Implement canonical session config updates and propagation to all attachments.
- [ ] Reject config updates that are unsafe while a prompt is running with an explicit busy-state error.
- [ ] Add tests for:
  - prompt queueing from multiple attachments in one session
  - queued-message notifications to other attachments
  - config propagation after accepted updates

## 4.4 Provider adapter baseline

- [x] Define a provider adapter trait/interface for launch, park, resume, terminate, and PTY wiring.
- [x] Add one local development adapter stub to exercise the runtime without depending on a real provider integration.
- [x] Introduce provider run runtime metadata distinct from the persistent/domain shape where needed.
- [x] Implement active vs parked provider run management for a session.
- [x] Enforce that only one provider run is active per session.
- [x] Add tests for:
  - launching the first provider run
  - parking an existing run when a new one becomes active
  - rejecting inconsistent active-run state

## 4.5 PTY manager and terminal stream path

- [x] Choose and integrate the Rust PTY library baseline for M1.
- [x] Implement PTY spawn/read/write/resize operations behind a dedicated manager.
- [x] Keep PTY byte stream handling isolated from structured daemon control state.
- [x] Implement terminal output fan-out so multiple attachments can observe the same session stream.
- [x] Align prompt submission semantics with the shared-attachment queue model and keep raw terminal input outside the public local daemon API contract.
- [x] Add tests or harness coverage for:
  - PTY spawn
  - input/write path
  - resize path
  - multi-attachment output fan-out

## 4.6 Daemon APIs for local clients

- [x] Define the minimum daemon API surface needed for local M1 flows:
  - create session
  - attach to session
  - detach from session
  - get session state
  - poll runtime notices
  - submit prompt
  - complete prompt
  - update session config
  - receive terminal output
  - resize terminal
  - end session
- [x] Keep the M1 API local-first; do not overbuild remote/server behavior yet.
- [x] Document request/response or event shapes in `docs/PROTOCOL.md` if reusable protocol contracts become concrete during implementation.

## 4.7 Domain and schema alignment

- [x] Review `packages/domain` and add any M1 fields/enums needed for attachment/shared-session/provider runtime coherence.
- [x] Ensure domain and runtime naming remain compatible with future workflow entities (`WorkflowDefinition`, `WorkflowNode`, `WorkflowEdge`, `WorkflowRun`, `NodeRun`, `NodeMessage`, `WorktreeAssignment`, `AggregationState`).
- [x] Ensure current session/provider/worktree fields do not assume single-agent execution as the only long-term runtime shape.
- [x] Keep Prisma changes minimal unless M1 code truly requires persisted runtime metadata.
- [ ] If schema/domain names change, update all affected docs in the same change.

## 4.8 Local client/dev harness

- [x] Add a minimal local harness that proves a client can create a session and attach to a running provider stub.
- [x] The harness may be a CLI command, integration test harness, or daemon smoke binary, but it must exercise the real runtime path.
- [x] Prefer a deterministic stub provider process for tests over a dependency on external provider CLIs.

## 4.9 Testing and verification

- [x] Add Rust unit tests for session lifecycle logic.
- [x] Add Rust integration tests for shared-attachment queue/config behavior.
- [x] Add Rust integration tests for provider run activation/parking behavior.
- [x] Add PTY/terminal conformance-oriented tests or smoke tests for local runtime behavior.
- [x] Ensure documented JS workspace verification still passes after M1 work lands, and keep daemon verification passing through the dedicated Rust commands.

## 4.10 Documentation updates required in the same PR set

- [x] Update `docs/PROTOCOL.md` if concrete M1 session or attachment events are introduced.
- [x] Update `docs/ARCHITECTURE.md` if implementation choices for daemon runtime, PTY handling, or attachment semantics become concrete.
- [x] Update `docs/CONTRIBUTING.md` with any new daemon test commands.
- [x] Update `agents/AGENTS.md` current status when usable M1 runtime behavior lands.
- [x] Update `docs/ops/TASKS.md` and `docs/ops/PROGRESS_LOG.md` as work progresses.
- [x] Keep `docs/spec-v1.md` and `docs/ARCHITECTURE.md` aligned with workflow-compatibility constraints introduced during M1 implementation.

## 4.11 Workflow-Compatibility Guardrails

- [x] Keep session APIs compatible with future single-agent and multi-agent workflow modes.
- [x] Do not assume raw terminal transcript forwarding as a valid future inter-agent communication mechanism.
- [x] Keep provider-run ownership flexible enough for future node-scoped runs in workflow mode.
- [x] Keep worktree handling compatible with future explicit worktree assignment and branch isolation for parallel code-writing nodes.
- [x] Keep scheduler-related runtime decisions daemon-owned so a generic workflow engine can later enforce runnable/waiting/completed node state, barriers, retries, and resource limits.

## 5. Suggested execution order

1. M1-001 daemon runtime skeleton and module layout
2. M1-002 session service and in-memory runtime store
3. M1-003 attachment lifecycle and shared-session interaction model
4. M1-004 provider adapter baseline and provider run service
5. M1-005 PTY manager and terminal stream fan-out
6. M1-006 local daemon API/harness
7. M1-007 runtime and integration tests
8. M1-008 docs/protocol alignment

## 6. Verification commands for claiming M1 complete

Run and pass locally before claiming M1 complete:

```bash
pnpm lint
pnpm build
pnpm test
cargo test --manifest-path apps/daemon/Cargo.toml
```

Recommended additional daemon checks:

```bash
cargo fmt --manifest-path apps/daemon/Cargo.toml --check
cargo clippy --manifest-path apps/daemon/Cargo.toml --all-targets --all-features -- -D warnings
```

If a dedicated daemon integration test target is introduced during M1, add the exact command here in the same change.

## 7. Definition of Done for M1

M1 is complete when all are true:

- [x] A local daemon runtime can create and end sessions.
- [x] A provider process can be launched under daemon ownership through a PTY abstraction.
- [x] A session supports multiple attachments with daemon-owned prompt queueing and canonical config propagation.
- [x] Terminal input/output flows through the daemon without breaking provider-native PTY behavior.
- [x] Parked provider run behavior exists with one active run at a time.
- [x] A deterministic local harness or integration test proves the managed-session flow end to end.
- [x] Documentation and protocol references are updated to match the implemented runtime behavior.
- [x] The resulting runtime remains compatible with the documented future workflow graph model, structured handoff contract, and explicit worktree isolation requirements.
