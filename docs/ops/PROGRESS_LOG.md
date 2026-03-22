# Arroba Progress Log

Chronological notes to preserve execution context between contributors/agents.

## 2026-03-22

### CLI transcript highlighting update

- Added `docs/CLI_TRANSCRIPT_HIGHLIGHTING_PLAN.md` to define transcript syntax highlighting as an M3 TypeScript CLI subphase separate from LSP.
- Implemented markdown-aware assistant/reasoning transcript rendering in the TypeScript CLI.
- Implemented syntax-highlighted fenced code blocks in the TypeScript CLI transcript using OpenTUI parser/code rendering infrastructure.

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

### M1-008 documentation alignment update

- Aligned `docs/PROTOCOL.md` with the current local daemon API: session state reads, notice polling, prompt submit/complete, and config update responses now match the implemented runtime surface.
- Aligned `docs/ARCHITECTURE.md`, `docs/spec-v1.md`, `agents/AGENTS.md`, and `docs/CONTRIBUTING.md` with the shared-attachment prompt/config model and the current client/daemon responsibilities.
- Reconciled the M1 checklist and task board with the now-complete runtime, integration coverage, and documentation work for M1-001 through M1-008.

### M1 closure update

- Added explicit scheduler-state ownership and primary worktree-assignment-compatible session state so the remaining workflow-compatibility guardrails are satisfied without redesigning the current runtime.
- Closed the remaining M1 checklist items and marked M1 complete in the project status docs.

### M2 planning update

- Added `docs/M2_IMPLEMENTATION_CHECKLIST.md` to break M2 into concrete capability workstreams, local API alignment, testing, and documentation requirements.
- Seeded the task board with initial M2 planning and implementation tasks.

### M2 shell capability baseline update

- Added a new `capability` module in the daemon runtime and implemented a structured shell command capability service.
- Exposed shell command execution through the local daemon API with structured stdout/stderr/exit-code results.
- Added daemon tests and local API tests covering successful shell execution, non-zero exits, and working-directory scoping.

### M2 shell hardening update

- Added timeout bounds and worktree-boundary validation to the shell capability so long-running or escaped commands do not silently bypass daemon safety expectations.
- Added attachment-aware authorization for shell execution through the local daemon API.
- Tightened prompt lifecycle UX by emitting notices when queued prompts are dropped because an attachment detached.

### M2 filesystem and git capability update

- Added structured directory tree capability support scoped to the session worktree.
- Added file read and file edit capabilities with structured results and worktree-boundary validation.
- Added structured git/worktree inspection capability for branch and status reporting.
- Exposed the new capabilities through the local daemon API and added daemon/local API tests for each baseline capability.

### M2 screenshot baseline update

- Added a screenshot capability contract and local runtime baseline with structured unavailable fallback when no capture backend is available.
- Exposed screenshot capture through the local daemon API and added daemon/local API tests for the baseline unavailable path.

### M2 transfer baseline update

- Added a daemon-owned file transfer storage baseline that copies source files from the session worktree into a session artifact root.
- Exposed transfer storage through the local daemon API and added daemon/local API tests for the stored-artifact path.

### Roadmap reprioritization update

- Reordered the near-term roadmap around one end-to-end local success path before broader platform scope.
- New immediate priority: local daemon + CLI + OpenCode integration with prompt submission and live output streaming.
- Deferred broader local capabilities, additional providers, multi-agent workflows, relay/web surfaces, provider switching, memory, compaction, and per-agent extension management to later milestones after that baseline is proven.

### M2 checklist realignment update

- Rewrote `docs/M2_IMPLEMENTATION_CHECKLIST.md` so it now matches the new M2 milestone instead of the earlier capability-first ordering.
- Broke the M2 task board into concrete sub-workstreams: daemon transport, CLI app, OpenCode adapter, and end-to-end smoke coverage.
- Explicitly marked the already-implemented local capability work as preserved but deferred relative to the new OpenCode-first critical path.

### M2 closure update

- Closed M2 formally after landing the real local daemon IPC transport, minimal local CLI, real `opencode` adapter, and end-to-end delayed-output smoke coverage through the daemon.
- Updated `README.md`, `docs/ROADMAP.md`, `docs/M2_IMPLEMENTATION_CHECKLIST.md`, `docs/PROTOCOL.md`, `docs/ARCHITECTURE.md`, and `docs/ops/TASKS.md` so repository status now reflects M2 as complete and M3 as the next milestone.
- Recorded shipped M2 implementation work against commit `727a97f`.

### TypeScript CLI migration update

- Promoted `apps/cli` to the primary local CLI implementation using TypeScript + OpenTUI.
- Kept `arroba-cli` as a Rust compatibility launcher that builds and starts the TypeScript client.
- Retained the previous Rust-only CLI as `arroba-cli-rust`, but marked it as phased out rather than the default local client surface.

### TypeScript CLI hardening update

- Added retry/backoff policy for transient local IPC polling failures in the TypeScript CLI instead of treating the first poll error as immediately fatal.
- Changed TypeScript CLI exit semantics so cleanup failures remain visible and require a second explicit exit attempt before forcing shutdown.
- Added initial TypeScript CLI behavior tests around retry/exit policy helpers and updated the roadmap/checklist so M3 now explicitly calls out TypeScript CLI hardening before slash-command expansion.

### M3 observability priority update

- Raised a project-wide logging/debugging system ahead of the remaining M3 tasks after the TypeScript CLI migration.
- Documented the intended baseline as one shared machine-local log root with per-process structured log files and shared session/provider/client correlation fields.
- Reprioritized the next M3 slice toward persistent session management: detached sessions should remain resumable, deletion should be explicit, the CLI should support a no-session state after deletion, and session references should move toward commit-like ids plus optional aliases.
- Marked privacy policy, retention, and content-capture scope as explicit design decisions to resolve before implementation.

### M3 logging foundation update

- Added a shared NDJSON logging baseline for the daemon, the Rust `arroba-cli` launcher, and the primary TypeScript CLI.
- Standardized log-root resolution around `ARROBA_LOG_DIR`, `XDG_STATE_HOME/arroba/logs`, `~/.local/state/arroba/logs`, then `./.arroba/logs`.
- Added built-in local log inspection through `arroba-cli logs`.
- Removed the previous ad hoc CLI debug-file hook and daemon IPC debug stderr hook in favor of the shared logger.
- Updated contributor and agent guidance so future debug work must extend the shared logging system instead of introducing separate mechanisms.
