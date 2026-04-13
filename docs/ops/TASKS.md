# Arroba Task Board

A lightweight, repo-native task board so contributors and future agents can continue work without external PM tooling.

## How to use

- Keep tasks small and implementation-oriented.
- Move tasks between `Backlog` -> `In Progress` -> `Done`.
- When a task is completed, add a short completion note and link commit hash.
- Use IDs (`M0-001`, `M1-003`, etc.) so references stay stable.
- Do not delete completed tasks; keep historical context.

## In Progress

- [ ] **M3-002** Close the OpenCode-first capability and local-runtime cycle
  - Note: Finish the remaining shell/file/git/screenshot/transfer/schedule-facing productization and local slash-command UX without adding another provider family yet.
- [ ] **M4-003** Stabilize the OpenCode-backed multi-agent runtime path
  - Note: The core daemon integration suite is green again, but the OpenCode-first path still needs further stabilization around transcript/pane UX and deeper live runtime drills.
  - Progress: queued prompts now stay bound to their target agent run, and queued work can advance onto another healthy agent run after the active run exits unexpectedly.
  - Future transport drills still pending:
    - deeper live replay/catch-up validation during active streaming output
    - long-idle heartbeat/liveness validation on the kernel WebSocket path
- [ ] **M4-005** Add multi-machine session ownership and resume behavior on the OpenCode-first path
  - Note: Close machine reassignment/resume semantics before moving to multi-platform clients or more providers.
- [ ] **M4.5-001** Implement the kernel runtime refactor
  - Note: Move hot-path daemon work from the global `DaemonApp` mutex toward the actor/event/projection kernel described in `docs/M4_5_KERNEL_RUNTIME_REFACTOR_PLAN.md`. Start with command/event contracts, replay-gap behavior, projections, and the interactive command lane before migrating actors and relay/runtime background work.
  - Progress: Kernel command/event envelopes, event replay gaps, projection metadata, command routing, bounded interactive routing, CLI replay-gap handling, command-id retry/fanout safety, inbound WebSocket request bounds, provider-run actor isolation, runtime cleanup tombstones, reserved-listener websocket tests, `KernelSessionService` public create/default-agent bootstrap plus lifecycle/focus/resize/end/delete ownership, `KernelAgentService` prompt submit/cancel/complete/queue lifecycle ownership, per-agent prompt command mailboxes including completion/cancellation routing with owner/projection active-owner resolution, session-runtime create plus per-session attach/focus/resize/end/delete command mailboxes with close cleanup and projection-backed delete/detach lane lookup, projected focus fallback for untargeted prompt routing, projection-first reads for session/list/resolve/history/provider-run/process/catalog plus agent/workflow inspection, list-hydrated session-state projection reads, direct owner-backed and agent-runtime active/queued prompt reads including queue-advance and provider-settlement inspection, shared kernel `PromptStateOwner` ownership for active/queued prompt lifecycle mutation with compatibility session mirroring, agent-mailbox prompt submit/cancel/complete projection publication from the shared projection store without a duplicate private prompt-state shadow, agent-mailbox completion consumption of lifecycle-published projections, compatibility prompt mirrors concentrated in a dedicated flattened `PromptRuntimeState` session prompt-runtime module, removal of the unused direct complete-and-auto-advance mutation API, direct compatibility complete/cancel owner resolution aligned with agent-runtime active-owner rules, session-private `RuntimeSession` prompt mutators, prompt lifecycle projection publication for complete/cancel, session response-borne projection refresh/removal, trimmed router-side snapshots for non-state terminal control commands, provider actor enqueue error propagation, canonical agent-runtime prompt-count projections and projection-invariant drift reporting in daemon health with legacy session-counter mirroring, exposed daemon health queue snapshots, implementation-invariants checklist, explicit file-writing capability worktree claims, provider prompt lifecycle worktree claims, provider prompt writes/enqueues running in spawned provider-operation dispatch after synchronous claim admission, kernel prompt-submit history append and remote relay dispatch deferral with failure cleanup, terminal stream cleanup on session end/delete, workflow-lane missing-session rejection from warmed projections, workflow response-borne projection refresh, and workflow node dispatch blocking/retry on workspace claims are now landed.
  - Next: Move session state ownership into mailbox runtimes, make reads projection-first, harden workflow/provider/terminal runtime paths, and remove remaining hot-path `Arc<Mutex<DaemonApp>>` dependencies. Near-term A+ sequence: session ownership, projection correctness, workflow runtime hardening, provider/terminal hardening, hot app-lock removal, docs/invariant lock, then final I/O coordination. Keep the current workspace claims as a coarse safety/scheduler layer for now; defer file-level claims, port claims, harness sandboxing, and transactional patch/rebase coordination until the final I/O-coordination slice.

## Backlog

- [ ] **M4-002** Add daemon-scheduled multi-agent workflow runtime
  - Note: The first management slice is now in place: kernel-backed workflow definitions, workflow/endpoint alias resolution, endpoint binding, and node/edge editing all exist. Runtime phase 1 is landed too: daemon-owned `WorkflowRun` state plus local API invoke/list/get/cancel flow. The scheduler now covers entry-node execution, daemon-owned downstream handoffs, target-side buffering, and default `all_inputs` join-node gating. Workflow-owned prompts request explicit `summary` plus downstream `output.message`, the daemon persists `summary + output + optional workflow-owned artifact refs`, and the CLI wires `/workflow run`, `/workflow runs`, and `/workflow cancel` with basic canvas run visibility. Remaining work is explicit per-node policy overrides, runnable validation/preflight reporting, daemon-managed node instruction artifacts, output schema validation tooling, richer run inspection/history UI, and only then recurring schedules. See `docs/M4_WORKFLOW_RUNTIME_PLAN.md`.
- [ ] **M5-001** Polish the TypeScript CLI as the reference Arroba client
  - Note: Finish the local UX, pane behavior, and command flow on the OpenCode-first path before adding more client surfaces.
- [ ] **M6-001** Add multi-platform clients on the same daemon/protocol model
  - Note: Web comes first, then iOS/Android, all reusing the semantics proven by the polished CLI.
- [ ] **M7-001** Add Claude Code, Codex, and generalized provider-adapter/protocol support
  - Note: Multi-provider expansion is intentionally the last major breadth step after OpenCode, harnessing, multi-machine behavior, and multi-platform clients are settled.

## Done

- [x] **M3-CLI-001** Improve TypeScript CLI transcript rendering with markdown-aware output and syntax-highlighted fenced code blocks
  - Note: Added markdown-aware assistant/reasoning rendering, parser bootstrap, fence-language normalization, and syntax-highlighted fenced code blocks without depending on LSP semantic coloring.
  - Commit: _pending next commit_
- [x] **M3-001** Implement persistent session management and no-session CLI state
  - Note: Added explicit session deletion, resumable detached sessions, unattached landing state, and commit-like ids plus optional aliases.
  - Commit: _pending next commit_

- [x] **M4-001** Deliver manual multi-agent session runtime and split-pane CLI UX
  - Note: Landed real session-agent execution targets, focused-agent prompt routing, per-agent provider-run ownership/history metadata, `/view split|individual`, and the initial multi-agent split-pane TypeScript CLI surface.
  - Commit: `23829c2`
- [x] **M4-004** Expand the TypeScript CLI split-pane model beyond the current first slice
  - Note: Completed on a parallel branch by another agent and now treated as landed roadmap state.
  - Commit: `unknown`

- [x] **M2-001** Deliver end-to-end local OpenCode daemon + CLI baseline
  - Note: Shipped one working local flow: launch OpenCode through the daemon, submit prompts from a CLI input field, and stream output live back into the terminal.
  - Commit: `727a97f`
- [x] **M2-002** Add real local daemon transport
  - Note: Replaced the in-process-only local harness assumption with a real local IPC path that a CLI can connect to.
  - Commit: `727a97f`
- [x] **M2-003** Add minimal local CLI client
  - Note: Added a usable CLI that connects to the daemon, submits prompts, and renders live output.
  - Commit: `727a97f`
- [x] **M2-004** Add real OpenCode provider adapter
  - Note: Replaced `dev-stub` for the main M2 path with a PTY-launched OpenCode adapter.
  - Commit: `727a97f`
- [x] **M2-005** Add end-to-end smoke coverage for local daemon + CLI + OpenCode
  - Note: Milestone completion is now covered through the real transport and CLI path rather than only the in-process harness.
  - Commit: `727a97f`
- [x] **M2-006** Create and later reshape the M2 implementation checklist and execution plan
  - Note: Added `docs/M2_IMPLEMENTATION_CHECKLIST.md` to break M2 into concrete capability workstreams, testing, and documentation steps.
  - Commit: `727a97f`
- [x] **M1-001** Add daemon runtime skeleton and module layout
  - Note: Added a lean daemon bootstrap around config, structured runtime errors, shutdown handling, and expandable real services instead of placeholder scaffolding.
  - Commit: _pending next commit_
- [x] **M1-002** Implement in-memory session lifecycle service
  - Note: Added an in-memory Rust session store with create/get/list/end flows, explicit session transitions, and encapsulated runtime session state.
  - Commit: _pending next commit_
- [x] **M1-003** Implement attachment lifecycle and shared-session interaction model
  - Note: Refactored attachments to shared participation, added daemon-owned prompt queueing/config propagation semantics, and kept in-memory attachment event recording.
  - Commit: _pending next commit_
- [x] **M1-004** Add provider adapter baseline and provider run service
  - Note: Added a provider adapter trait, a deterministic dev-stub adapter, and in-memory provider run management with active/parked transitions.
  - Commit: _pending next commit_
- [x] **M1-005** Add PTY manager and terminal stream fan-out
  - Note: Added a `portable-pty` backed PTY manager, terminal input/output routing, and multi-attachment fan-out through daemon-owned terminal records.
  - Commit: _pending next commit_
- [x] **M1-006** Add local daemon API or harness for managed-session flows
  - Note: Added a local request/response daemon API, a smoke harness binary, and runtime tests proving managed-session PTY flow through the daemon.
  - Commit: _pending next commit_
- [x] **M1-007** Add session runtime and PTY-oriented integration tests
  - Note: Added daemon integration coverage for lifecycle cleanup, prompt queue notifications, active run switching, and local managed-session PTY/config flow.
  - Commit: _pending next commit_
- [x] **M1-008** Align protocol and architecture docs with concrete M1 runtime behavior
  - Note: Updated protocol, architecture, checklist, and status docs to match shared attachments, prompt queueing, session-state reads, notice polling, and config propagation.
  - Commit: _pending next commit_
- [x] **M0-001** Bootstrap workspace root (`pnpm-workspace`, root scripts, TS base config)
  - Note: Added workspace root `package.json`, `pnpm-workspace.yaml`, and `tsconfig.base.json`.
  - Commit: _pending next commit_
- [x] **M0-002** Create `packages/domain` with initial shared model types
  - Note: Expanded the domain package to cover the full M0 entity baseline and runtime status constants.
  - Commit: _pending next commit_
- [x] **M0-003** Add minimal domain contract tests (shape/enum invariants)
  - Note: Added TypeScript contract tests for shared runtime constants and core entity serialization.
  - Commit: _pending next commit_
- [x] **M0-004** Create `apps/server` Fastify stub with strict TypeScript
  - Note: Added a smoke test for the server health endpoint.
  - Commit: _pending next commit_
- [x] **M0-005** Bootstrap `apps/daemon` Rust crate and crate tests
  - Note: Baseline daemon crate remains in place and passes cargo tests.
  - Commit: _pending next commit_
- [x] **M0-006** Add Prisma core schema for M0 entities
  - Note: Added `prisma/schema.prisma` for the baseline M0 entity set and relationships.
  - Commit: _pending next commit_
- [x] **M0-007** Add GitHub Actions CI for lint/build/test (+ Rust checks)
  - Note: Added `.github/workflows/ci.yml` for pnpm and Rust verification.
  - Commit: _pending next commit_
- [x] **M0-008** Align docs after scaffolding (`CONTRIBUTING`, `ROADMAP`, `AGENTS` status)
  - Note: Updated root README and M0-related docs to reflect the completed foundation state.
  - Commit: _pending next commit_
- [x] **OPS-001** Establish repo-native task tracking docs for cross-agent continuity
  - Note: Added this board and progress log in `docs/ops/`.
  - Commit: _pending next commit_
