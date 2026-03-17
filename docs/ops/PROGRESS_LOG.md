# Arroba Progress Log

Chronological notes to preserve execution context between contributors/agents.

## 2026-03-16

### Context

- M0 assessed as not complete yet.
- M0 implementation direction clarified and accepted:
  - include Rust daemon bootstrap
  - use GitHub Actions now
  - Option A baseline structure
  - smoke tests + minimal domain contract tests
  - include Prisma schema now

### Changes made in this update

- Added `docs/M0_IMPLEMENTATION_CHECKLIST.md` with concrete M0 task breakdown and DoD.
- Added `docs/ops/TASKS.md` lightweight board for backlog/in-progress/done tracking.
- Added this `docs/ops/PROGRESS_LOG.md` for chronological handoff notes.

### Next recommended execution order

1. M0-001 workspace root and scripts
2. M0-004 server stub
3. M0-002 + M0-003 domain + contract tests
4. M0-005 daemon rust crate
5. M0-006 prisma schema
6. M0-007 CI workflow
7. M0-008 docs alignment and status update

### 3.1 implementation progress

- Completed workspace/package bootstrapping (`M0-001`, `M0-002`, `M0-004`, `M0-005`).
- Added root workspace scripts for `build`, `lint`, `test`, and daemon test invocation.

### M0 completion update

- Expanded `packages/domain` to cover the full M0 entity baseline and added contract tests.
- Added `prisma/schema.prisma` for the initial persistence model.
- Added `.github/workflows/ci.yml` for pnpm and Rust verification.
- Updated `README.md`, `docs/CONTRIBUTING.md`, `docs/ROADMAP.md`, and `docs/M0_IMPLEMENTATION_CHECKLIST.md`.
- M0 verification now consists of `pnpm lint`, `pnpm build`, `pnpm test`, and `cargo test --manifest-path apps/daemon/Cargo.toml`.
- M0 is considered complete once those commands pass on the repository state produced in this update.

### M1 planning update

- Added `docs/M1_IMPLEMENTATION_CHECKLIST.md` to break M1 into concrete runtime, PTY, attachment, provider, and test workstreams.
- Seeded `docs/ops/TASKS.md` with `M1-001` through `M1-008`.
- Recommended M1 execution order:
  1. daemon runtime skeleton
  2. session lifecycle service
  3. attachment and shared-session interaction logic
  4. provider adapter baseline
  5. PTY manager and terminal fan-out
  6. local harness/API
  7. runtime tests
  8. docs/protocol alignment

### M1-001 implementation update

- Added the daemon runtime skeleton in `apps/daemon` with:
  - `app.rs` for bootstrap and shutdown handling
  - `config.rs` for daemon configuration loading/validation
  - `error.rs` for structured daemon runtime errors
  - a lean application container that owns only real runtime services
- Switched the daemon binary to a Tokio-based async entrypoint and documented Tokio as the M1 async runtime baseline.
- Added crate tests to verify config validation and top-level runtime wiring.

### M1-002 implementation update

- Implemented an in-memory session lifecycle service in `apps/daemon/src/session/`.
- Added runtime session records for workspace/worktree/host ownership, active provider run, and attachment membership state. This was later extended to prompt-queue and config-state ownership.
- Added explicit session transition validation for `created`, `active`, `parked`, and `ended` states.
- Added Rust unit tests for create/get/list/end flows, invalid transitions, and unknown-session lookup behavior.
- Refined the session model to remove duplicated derived state, keep host metadata out of the in-memory store, and encapsulate session mutation behind methods.

### M1-003 implementation update

- Initial implementation added a real `attachment` runtime module with in-memory attachment records and daemon-facing event recording.
- The original implementation used controller-style semantics, which were later superseded by the shared-attachment prompt-queue/config-state model.
- Current runtime behavior is governed by the later shared-attachment refactor notes in this log.

## 2026-03-17

### Scope clarification update

- Multi-agent workflow execution is explicitly in scope for v1.
- Circular topology is the earlier implementation priority inside v1.
- Hierarchical topology remains in scope for v1, but is planned for a later stage of v1 after lower-level runtime, capability, control, and protocol foundations stabilize.

### Documentation alignment update

- Updated `README.md` to distinguish current implementation status from planned v1 scope.
- Updated `agents/AGENTS.md`, `docs/spec-v1.md`, `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, and `docs/PROTOCOL.md` so workflow execution is clearly in v1 scope, with circular earlier and hierarchical later within v1.
- Corrected planning/status drift by marking `M1-004` as pending again in `docs/M1_IMPLEMENTATION_CHECKLIST.md` and `docs/ops/TASKS.md`.
- Updated `docs/CONTRIBUTING.md` so testing guidance and baseline-command wording match the current repository state and the new workflow-oriented scope.

### Runtime review update

- Reviewed the current daemon runtime against the new workflow-oriented specifications before continuing M1 work.
- Added explicit session execution mode metadata so the session model now distinguishes current single-agent behavior from future multi-agent workflow mode.
- Kept PTY ownership and terminal stream handling keyed by provider run so future node-scoped provider runs can reuse the same runtime surfaces.

### M1-004 implementation update

- Added a provider adapter trait, registry, and deterministic `dev-stub` adapter for local runtime tests without depending on external provider CLIs.
- Added in-memory provider run lifecycle management for launch, park, resume, terminate, and session active-run ownership.
- Added provider runtime tests covering first launch, automatic parking on active-run replacement, and inconsistent active-run rejection.

### M1-005 implementation update

- Integrated `portable-pty` as the PTY baseline for the daemon runtime.
- Added a PTY manager for spawn, write, resize, output draining, and process cleanup keyed by provider run.
- Added terminal stream records for attachment-driven input routing and multi-attachment output fan-out.
- Added daemon-level tests covering PTY spawn, terminal input/write path, resize behavior, and output fan-out to multiple attachments.

### Runtime hardening update

- Hardened PTY lifecycle ownership so the daemon now retains child-process handles and performs explicit PTY cleanup on provider/session teardown instead of only dropping in-memory records.
- Updated failed provider-switch handling to resume the previously active run automatically when the replacement PTY cannot be established, and record a user-facing runtime notice for that recovery path.
- Expanded `packages/domain` with workflow-oriented v1 entities and enums so shared contracts no longer stop at the earlier single-agent baseline.

### M1-006 implementation update

- Added a local daemon request/response API in `apps/daemon/src/local/` covering create, attach, detach, provider launch, session state reads, notice polling, prompt submit/complete, config updates, terminal output polling, terminal resize, and session end flows.
- Added a local smoke harness binary in `apps/daemon/src/bin/arroba-daemon-harness.rs` plus runtime tests proving a managed-session path through the PTY and terminal fan-out surfaces.
- Updated `docs/PROTOCOL.md` to record the local-first daemon API baseline for M1 flows.

### Domain and schema alignment update

- Expanded `packages/domain/src/index.ts` and `packages/domain/src/index.test.ts` to reflect workflow-oriented runtime naming, richer workflow entities, handoff/completion fields, worktree-isolation modes, and delivery statuses.
- Updated `prisma/schema.prisma` to add workflow-oriented enums, execution-mode/session fields, and baseline models for workflow definitions, runs, nodes, edges, node messages, worktree assignments, and aggregation state.

### M1-007 implementation update

- Added daemon integration tests in `apps/daemon/tests/runtime_integration.rs`.
- Covered session lifecycle cleanup, prompt queue/notification behavior, provider run switching with PTY-backed terminal flow, and the local managed-session smoke harness path.
- Marked the M1 testing/verification checklist items complete now that daemon integration coverage passes and the documented JS workspace verification plus dedicated daemon verification commands both pass.

### Shared-attachment refactor update

- Replaced the earlier controller/observer runtime model with shared attachment participation in the daemon runtime.
- Added daemon-owned prompt queue state, active-prompt completion/advancement, and queued-message notices for the other attachments in a session.
- Added canonical session config state with versioned updates plus propagation notices to the rest of the session attachments.
- Updated local daemon APIs, domain types, Prisma schema, and daemon tests to match the shared-attachment queue/config model.
