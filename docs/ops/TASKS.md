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
  - Progress: Kernel command/event envelopes, event replay gaps, projection metadata, command routing, bounded interactive routing, CLI replay-gap handling, command-id retry/fanout safety, inbound WebSocket request bounds, provider-run actor isolation, runtime cleanup tombstones, reserved-listener websocket tests, `KernelSessionService` public create/default-agent bootstrap plus lifecycle/focus/resize/end/delete ownership, `KernelAgentService` prompt submit/cancel/complete/queue lifecycle ownership, per-agent prompt command mailboxes including completion/cancellation routing with owner/projection active-owner resolution, session-runtime create plus per-session attach/focus/resize/end/delete command mailboxes with close cleanup and projection-backed delete/detach lane lookup, session-scoped agent lifecycle routing through `SessionRuntime`, projected focus fallback for untargeted prompt routing, projection-first reads for session/list/resolve/history/provider-run/process/catalog plus agent/workflow inspection, list-hydrated session-state projection reads, direct owner-backed and agent-runtime active/queued prompt reads including queue-advance and provider-settlement inspection, shared kernel `PromptStateOwner` ownership for active/queued prompt lifecycle mutation with compatibility session mirroring, agent-mailbox prompt submit/cancel/complete projection publication from the shared projection store without a duplicate private prompt-state shadow, agent-mailbox completion consumption of lifecycle-published projections, compatibility prompt mirrors concentrated in a dedicated flattened `PromptRuntimeState` session prompt-runtime module, removal of the unused direct complete-and-auto-advance mutation API, direct compatibility complete/cancel owner resolution aligned with agent-runtime active-owner rules, session-private `RuntimeSession` prompt mutators, prompt lifecycle projection publication for complete/cancel, session response-borne projection refresh/removal, dedicated SessionRuntimeCommandExecutor seam with named `SessionRuntimeStore` execution for session commands and no generic session service request facade, dedicated WorkflowRuntimeCommandExecutor seam with no generic workflow store request method, trimmed router-side snapshots for non-state terminal control commands, provider actor enqueue error propagation, canonical agent-runtime prompt-count projections and projection-invariant drift reporting in daemon health with legacy session-counter mirroring, exposed daemon health queue snapshots, implementation-invariants checklist, explicit file-writing capability worktree claims, provider prompt lifecycle worktree claims, provider prompt writes/enqueues running in spawned provider-operation dispatch after synchronous claim admission, provider launch command executor seam plus isolated provider-launch store boundary, kernel prompt-submit history append and remote relay dispatch deferral with failure cleanup, terminal stream cleanup on session end/delete, explicit session request handling through the session-runtime boundary, session worker removal from compatibility session/agent dispatch helpers, explicit agent request handling through the agent-runtime boundary, legacy generic interactive fallback rejection and generic interactive mailbox removal, workflow-lane missing-session rejection from warmed projections, workflow worker removal from the compatibility workflow dispatcher, explicit agent prompt command executor seam with a narrow prompt command service dependency, split kernel prompt submit/cancel/complete admission and effect preparation plus remaining queue/cancel/claim helpers and direct-submit compatibility isolated in a prompt command phase module, deleted generic `KernelAgentService::execute_request` facade after session-runtime agent lifecycle moved to named store operations, cloneable prompt id allocation shared by session and agent runtime, dedicated terminal output executor seam, compatibility-backed provider output pump context seam, structured provider-output polling inside the provider-output context boundary, terminal output executor direct use of the provider-output boundary, cloneable structured output record store, removal of broad provider-output and terminal-output app helpers, provider-output recipient resolver seam, provider-output fanout seam, structured batch application inside provider-output context, provider-output liveness seam, provider-run exit reconciliation moved into ProviderRunLivenessRuntime, provider-output liveness wired directly to ProviderRunLivenessRuntime, provider-run liveness process and recipient seams, removal of the provider-run liveness app helper, explicit ProviderProcessTracker ownership for managed process registration/listing/teardown/cleanup, provider-run liveness notice sink, provider/session liveness-state seam, provider-run liveness outcome split from session/prompt effects, provider-only liveness reconciliation with provider-service wrapper retired and session-side active-run sync, ProviderRunEndedOutcome for provider-only run endings with the session-mutating mark-ended wrapper retired and termination with the session-mutating terminate wrapper retired, ProviderSessionRunsTerminatedOutcome for session-wide provider termination, ProviderRunParkedOutcome for provider-only parking, ProviderRunResumedOutcome for provider-only resume, ProviderRunStartedOutcome for provider-only start with provider-service launch/start wrappers retired and detached launch reusing provider-only start, workflow response-borne projection refresh, runtime-tool workflow projection refresh, explicit workflow request handling through the workflow-runtime boundary, workflow node dispatch blocking/retry on workspace claims, relay-client daemon/workflow requests through `CommandRouter`, local/forwarded runtime MCP tool calls through `CommandRouter`, explicit router handling for relay config/remote-machine registry mutations, exhaustive normal/background router dispatch without the generic local compatibility fallback, direct named router helpers instead of production `handle_local_request` calls, retirement of `DaemonApp::handle_local_request` as a public facade API, crate-private narrowing of app-level prompt submit/complete/cancel shims, transport-surface output pump helpers for external runtime integration tests, deterministic provider-actor output-poll cleanup race coverage, and an explicit provider terminal input boundary, and removal of the app-level session/workflow request facade shims, initial router-backed local API coverage, direct provider terminal-input boundary use from prompt dispatch/cancel paths, an explicit TerminalOutputStore boundary isolating remaining app-lock provider-output access, additional router-backed local API session/agent lifecycle coverage, router-backed local API workflow graph-management coverage, router-backed local API workflow-run coverage, router-backed local API prompt/provider coverage, router-backed local API capability coverage for shell/file/tree/git/transfer/workspace-claim paths, router-backed local API relay/remote-machine coverage, shared router-backed unit-test fixture extraction, initial non-local-api prompt-owner setup migration, full router-backed `local/api/tests.rs` request coverage, blocked workflow-claim retry projection refresh, screenshot capability env-lock deadlock fix, removal of the broad app-level provider-input shim, and non-local-api actor/projection/scheduler/router/flow-control/relay-client/MCP/runtime-tool test setup migrated off `handle_local_request`, and deletion of the catch-all `DaemonApp::handle_local_request` dispatcher, and initial `SessionRuntimeStore`/`AgentRuntimeStore`/`WorkflowRuntimeStore` boundaries for session/agent/workflow worker fallback ownership, an explicit `TerminalOutputStore` boundary for compatibility-backed provider-output pumping, and an explicit `CapabilityRuntimeStore` boundary for compatibility-backed capability context lookup, and removal of the generic `KernelSessionService::execute_request` facade in favor of named `SessionRuntimeStore` operations, and direct named session-runtime spawn/destroy agent lifecycle operations, and deletion of the unused generic `KernelAgentService::execute_request` facade, removal of the generic `WorkflowRuntimeStore::execute_request` method, named `WorkflowRuntimeStore` methods for workflow command dispatch, `KernelWorkflowService` ownership for workflow command mutations, a narrow `KernelWorkflowContext` compatibility dependency boundary for workflow command ownership, removal of the app-level `kernel_workflows()` helper, removal of the app-level `kernel_agents()`/`kernel_sessions()` helper facades, narrow `SessionRuntimeContext`, `AgentRuntimeContext`, `AgentPromptCommandContext`, `AgentPromptDispatchContext`, and `WorkflowRuntimeContext`, `TerminalOutputContext`, and `CapabilityRuntimeContext` compatibility dependency boundaries plus session-service-owned reference/attachment, session-read, and agent lifecycle ports for session, agent, workflow, terminal-output, and capability execution, and cleanup of remaining app-backed provider/relay `handle_*_request` names, app-level prompt spawn helpers, the app-level provider runtime-binding initialization shim, and production/test session snapshot consumers routed through `KernelSessionReadService`, removal of the `DaemonApp::local_api_session_snapshot` shim, removal of the `DaemonApp::publish_session_projection` shim, and removal of app-level attachment/provider-run authorization shims, removal of the app-level provider prompt dispatch shim, removal of the app-level remote workflow-turn context shim, and removal of app-level direct capability execution helpers with capability authorization moved to `KernelSessionReadService::capability_context`, and removal of app-level session-history, agent-list, terminal-input, and terminal-resize convenience helpers, and removal of app-level session/agent lifecycle convenience helpers for create/spawn/destroy/focus/cycle/config/resolve paths, and `RemoteLeaseRuntime` ownership for remote execution lease, leased-agent, leased-prompt, runtime-tool binding, and leased projection handling, plus the explicit `CompatibilityRuntimeState` quarantine boundary and named session/agent/workflow/prompt-command compatibility ports for the remaining app-backed runtime workers, are now landed.
  - Next: Compatibility-facade retirement outside I/O coordination. Keep router independence locked and the deleted catch-all dispatcher from returning, move `SessionRuntime`/`AgentRuntime`/`WorkflowRuntime` workers off `Arc<Mutex<DaemonApp>>`, split runtime-owned services out of `DaemonApp`, and then retire/rename the remaining facade-shaped helper seams. Keep the current workspace claims as a coarse safety/scheduler layer, and defer file-level claims, port claims, harness sandboxing, coordinator-owned patch application, and transactional mutation/rebase coordination to the final I/O slice.

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
