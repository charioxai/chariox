# Arroba Progress Log

Chronological notes to preserve execution context between contributors/agents.

## 2026-04-15

### M4.5 session alias ownership update

- Moved session alias updates onto the owned `CompatibilityRuntimeState` session store when runtime-owned state is available, with a no-app-lock regression covering alias mutation and projection refresh while the compatibility app mutex is held.
- Verified the slice with `cargo test` in `apps/daemon`: 332 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 session-runtime ownership update

- Moved session create/default-agent bootstrap and session config updates onto the owned `CompatibilityRuntimeState` session, agent, attachment, terminal, history, and projection stores when runtime-owned state is available. These session-runtime paths no longer wait for the compatibility app lock in the normal router-owned configuration, with app-lock fallback kept only for no-owned-state legacy tests.
- Fixed session creation responses to return the refreshed focused-agent session after default-agent bootstrap, so session/focus projections are populated from the authoritative post-bootstrap state.
- Verified the slice with `cargo test` in `apps/daemon`: 331 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 runtime integration bridge update

- Restored the daemon integration suite after direct facade retirement by routing the stale test-facing session, prompt, terminal, provider-output, and structured-runtime-state helpers through the explicit `KernelSessionService`, `KernelAgentService`, provider-output pump, provider terminal-input, and provider-run actor boundaries instead of the deleted generic local request dispatcher.
- Verified the slice with `cargo test` in `apps/daemon`: 329 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

## 2026-04-12

### M4.5 kernel runtime refactor progress

- Landed the first substantial kernel responsiveness slices:
  - `KernelCommand` / `KernelEvent` envelopes and command routing
  - bounded `EventLog` replay with explicit replay-gap behavior
  - session snapshot projection metadata
  - bounded interactive routing and inbound WebSocket admission
  - safe command-id retry handling with in-flight fanout and conflict rejection
  - typed CLI replay-gap handling and user-visible refresh notice
  - local IPC compatibility routing through the same command normalization path
- Moved structured provider submit, abort, and output polling through provider-run actors.
- Added runtime slot tombstones/generations so cleanup racing with slow provider I/O cannot restore stale runtime state.
- Removed provider-family global locks from structured output polling while provider I/O is in progress.
- Fixed the websocket integration harness port race by reserving listeners through server startup.
- Added responsiveness/race coverage for slow history, provider catalog, shell capability, provider launch, structured submit/cancel/poll, slow consumers, replay gaps, duplicate command ids, and runtime cleanup races.
- Introduced `KernelSessionService` and moved session attach, detach, end, delete-by-ref, focus/cycle, and terminal resize behavior behind that service while keeping `DaemonApp` as a compatibility facade.
- Introduced `KernelAgentService` and moved prompt submit, kernel submit acknowledgement/dispatch preparation, cancel, runtime cancel, completion, queue advancement, and cancellation finalization behind that service while keeping `DaemonApp` as a compatibility facade.
- Added `AgentRuntime` per-agent mailboxes for prompt submit/cancel admission so agent prompt commands no longer wait behind the generic interactive queue.
- Added `SessionRuntime` per-session mailboxes for attach, detach, focus/cycle, resize, end, and delete admission so session UI/lifecycle commands are isolated from the generic interactive queue and from unrelated sessions.
- Added session mailbox deregistration after successful end/delete so closed sessions do not leave stale mailbox registrations behind.
- Added projected session lane-key lookup for session delete and detach admission, so delete-by-ref and detach-by-attachment can route to the right session mailbox from warmed projection state before falling back to the compatibility app lock.
- Added `DaemonHealthProjection` snapshots for session/agent command mailboxes, provider runtime operation lanes, projected session counts, active/queued prompt counts, and provider-catalog cache state exposed through `GetDaemonHealth`.
- Added a session-owned focused-agent projection shared by `SessionRuntime`, `AgentRuntime`, and the router's agent-lifecycle response path, so untargeted prompt submit/cancel routing can resolve the focused agent without taking the compatibility `DaemonApp` lock once focus is warmed by session commands or local agent spawn/destroy responses.
- Added the first shared session projection store. `GetSessionState`, successful `ResolveSession`, and warmed `ListSessions` now serve projected data without taking the compatibility `DaemonApp` lock; the router refreshes the projection from session-bearing responses and list responses. List responses also hydrate the per-session projection map, so follow-up state and successful resolve reads can return from projection without a separate compatibility-store read.
- Added the first agent runtime projection store. Session projection refreshes now materialize per-agent active and queued prompt counts, direct agent/session-scoped reads are available, and `GetDaemonHealth` uses those counts as the canonical prompt-work health source while mirroring them into the legacy session-projection counters for compatibility.
- Extended the agent runtime projection with each agent's front queued prompt and made local/remote queue advancement inspect that projected candidate before falling back to compatibility session queue reads.
- Updated detach prompt cleanup to republish session and agent-runtime projections after active/queued prompt removal, preventing stale queue-front reads during follow-up advancement.
- Moved provider idle settlement's active-prompt status check to the agent-runtime projection first, with compatibility session inspection retained as fallback when no projected active prompt is warm.
- Moved agent-runtime prompt projection publication into the per-agent mailbox execution path for prompt submit/cancel, so the mailbox runtime now updates its own active/queued prompt read model while compatibility session projections remain mirrored.
- Routed kernel `CompletePrompt` through the per-agent mailbox, resolving the active prompt owner from warmed session projection before falling back to compatibility session state and publishing completion state into the agent-runtime projection from that mailbox worker.
- Moved `CompletePrompt` owner lookup to the agent-runtime active-prompt projection first, so completion routing can stay off the compatibility app lock even when the warmed session projection is stale.
- Moved kernel `CancelActivePrompt` owner lookup to the same agent-runtime active-prompt projection resolver before per-agent mailbox dispatch, with mailbox execution still enforcing attachment/session and active prompt validation.
- Removed the duplicate router-side agent-runtime projection refresh for `PromptSubmitted`; the router still refreshes the session projection from the response while the agent mailbox owns the agent-runtime prompt projection.
- Extended untargeted prompt routing to use warmed session projection focused-agent state before falling back to the compatibility app lock when the dedicated focused-agent projection is cold.
- Added a shared session-history projection. `GetSessionHistory` can now load from disk using a warmed session snapshot without taking the compatibility `DaemonApp` lock, and repeated warmed transcript reads are served from memory while successful history appends keep the warmed projection current.
- Added a shared provider-run projection. Warmed `GetProviderRun` reads now return without taking the compatibility `DaemonApp` lock, and launch/start/finish/fail/park/resume/ended lifecycle updates refresh the warmed projection.
- Added a warmed provider-process projection. Repeated `ListProviderProcesses` reads now return without taking the compatibility `DaemonApp` lock, while provider-run and session lifecycle changes invalidate the projection so teardown-safety metadata is not served stale.
- Added prompt lifecycle publication into the shared session projection. Prompt submit, complete, cancel, dispatch failure, and queue advancement now update warmed prompt-state snapshots so `GetSessionState` can reflect those transitions without taking the compatibility app lock.
- Removed redundant router-side session snapshots for prompt complete/cancel. Those paths now rely on prompt lifecycle projection publication, keeping follow-up warmed `GetSessionState` reads projection-first without an extra compatibility-store refresh.
- Trimmed router-side session snapshots for non-state terminal control commands: `PollRuntimeNotices` and `ResizeTerminal` no longer perform post-response session projection snapshots, while `PumpTerminalOutput` still does because provider output pumping can settle prompt state.
- Added a TTL-bound provider-catalog projection. Warmed `GetProviderCatalog` reads now return without taking the compatibility app lock, and provider logout/configuration changes invalidate the projection.
- Fixed projection correctness gaps: provider-process projection now stores a canonical unfiltered snapshot and refreshes after teardown, warmed OpenCode `GetProviderRun` no longer bypasses selection-sync side effects, relay reconfiguration invalidates provider catalog projection state, and agent lanes are removed on agent/session cleanup.
- Added warmed session-projection reads for agent and workflow inspection: `ListAgents`, `ListWorkflows`, `ResolveWorkflow`, `ListWorkflowRuns`, `GetWorkflowRun`, `ListWorkflowWatchdogs`, and `ListQueuedWorkflowLaunches` can now return without taking the compatibility app lock once the session projection is warm.
- Added transport health projection counters for kernel websocket pressure: active connections/subscriptions, incoming requests, emitted events, replay gaps, inbound overload rejections, outgoing queue overflows, and slow-consumer closes are now exposed through `GetDaemonHealth`.
- Added a workspace coordination health baseline: active worktree claims and same-workspace worktree collisions are now reported from the warmed session projection through `GetDaemonHealth`.
- Added initial `WorkspaceCoordinator` enforcement for explicit file-writing capabilities. `EditFile` and `StoreTransferredFile` now acquire scoped worktree write claims, reject overlapping same-workspace/worktree writes with a retryable workspace-claim conflict, publish active operation claims through daemon health, and release claims on operation completion.
- Added provider prompt lifecycle worktree claims. Active local provider prompts now acquire provider-prompt operation claims before dispatch, reject cross-session same-workspace/worktree prompt conflicts, publish those claims through daemon health, and release them through the existing prompt cleanup path on completion, cancellation, dispatch failure, and session cleanup.
- Promoted workspace claims into the workflow scheduler. Claims now expose `read`/`write` mode metadata, workflow node dispatch acquires an exclusive `workflow_node_dispatch` write claim before provider submission, blocked nodes move to `BlockedOnWorkspaceClaim`, and claim release retries blocked workflow nodes instead of failing temporary contention.
- Clarified the claim strategy after review: current claims should remain a coarse safety/scheduler layer while M4.5 finishes actor/projection ownership. Deeper I/O coordination, including file-level claims, port claims, harness sandboxing, coordinator-owned patch application, and automatic patch rebase loops, is intentionally deferred to the final coordination slice.
- Removed the duplicate `AgentRuntimePromptStateStore` shadow after review. `AgentRuntimeProjectionStore` is now the single warm prompt-state read model for active-owner routing, queue-front preview, daemon health, and projection-first reads while `PromptStateOwner` is the mutation authority and compatibility session state remains the mirror.
- Changed structured provider submit/abort/output-poll/selection-sync enqueue failures to propagate as daemon errors instead of being logged and swallowed. This keeps prompt dispatch cleanup, claim release, notices, and retryable failures on the normal error path when a provider actor does not accept work.
- Added a per-provider-run structured output return buffer so globally drained background output still comes back from the later direct pump for that provider run, without delaying terminal fanout.
- Introduced `PromptRuntimeState` inside the compatibility session mirror. It is now the only writer for per-agent active/queued prompt state and the legacy session-level prompt/scheduler projections, while serialization remains flattened to the existing wire fields.

## 2026-04-13

### M4.5 kernel runtime refactor progress

- Added `PromptStateOwner` as the kernel write owner for per-agent active prompts and queued prompt backlogs. Prompt submit, complete, cancel, cancellation finalization, dispatch-failure cleanup, queue advancement, detach cleanup, provider settlement, and workflow prompt submission now mutate the owner first and then mirror into compatibility `RuntimeSession` prompt fields.
- Demoted `PromptRuntimeState` to the flattened compatibility mirror/projection boundary. It still preserves the existing wire shape for active prompt, queued prompts, per-agent prompt states, and scheduler state, but it is no longer the hot prompt lifecycle authority.
- Removed projection-based prompt submit admission as an authority. Agent-runtime projections still provide warm queue-front previews and health/read models, but stale projection state cannot force an otherwise idle prompt owner to queue.
- Added regression coverage that deliberately corrupts the compatibility session mirror and verifies completion still succeeds from the prompt owner.
- Promoted `PromptStateOwner` from a private compatibility-app sidecar to a cloneable kernel service shared with `AgentRuntime`. Active-prompt owner resolution and complete queue-front preview can now consult the owner without taking the app lock when a session projection is warm, and regression coverage locks the stale-mirror/no-app-lock path.
- Moved `PromptRuntimeState` into `session/prompt_runtime.rs` as a dedicated compatibility prompt mirror boundary. `RuntimeSession` still flattens and forwards it for wire compatibility, but scattered prompt mutation is no longer embedded in the shared session type.
- Hardened `WorkflowRuntime` lane admission and projection refresh. Warmed missing-session workflow mutations now fail from `SessionStateProjectionStore` without creating a workflow lane or waiting on the compatibility app lock, and workflow workers refresh session/agent-runtime projections directly from session-bearing workflow responses before falling back to a compatibility snapshot.
- Hardened `SessionRuntime` projection publication. Session mailbox workers now publish session-bearing create/config/alias/end responses directly, remove deleted-session projections at the mailbox boundary, and only fall back to compatibility snapshots for responses that do not carry enough session state.
- Trimmed prompt-completion app-lock work in `AgentRuntime`. The agent mailbox now consumes the session projection published by the prompt lifecycle service instead of taking a second compatibility session snapshot after completing a prompt.
- Deferred kernel prompt-submit side effects out of the agent-mailbox acknowledgement path. User-prompt history appends now run through spawned blocking persistence with projection refresh after success, and remote relay prompt submit now returns a dispatch object that is spawned after owner mutation; remote dispatch failure cancels the active prompt, refreshes projections, and records a notice.
- Added projection publication after workflow runtime-tool calls. Direct MCP/relay runtime-tool mutations now republish the session and agent-runtime projections after recording the tool call, so workflow turn acknowledgements and output submissions do not leave warmed workflow inspection reads stale when they bypass the router workflow lane.
- Routed relay-client daemon/workflow requests through `CommandRouter` as `KernelCommandSource::RelayClient`. Proxied relay clients now share the same actor admission, projection refresh, and overload behavior as local IPC/kernel transport requests, regression coverage verifies relay list requests warm the shared session projection, and workflow validation requests no longer die at the relay transport gate.
- Consolidated workflow command handling behind the workflow-runtime boundary. `WorkflowRuntime` workers now invoke the explicit workflow request handler instead of the generic local compatibility handler, and local IPC delegates workflow requests to that same handler so workflow mutation logic is not doubled while `DaemonApp` remains the compatibility mirror.
- Consolidated session command handling behind the session-runtime boundary. `SessionRuntime` workers and local IPC now delegate lifecycle, focus, resize, notice, config, alias, end, and delete commands to one explicit session request handler, removing the duplicate session mutation implementations from the actor and local API paths.
- Consolidated agent prompt command handling behind the agent-runtime boundary. Local IPC, the legacy interactive fallback, and `AgentRuntime` now share one explicit agent request handler for prompt submit, completion, and cancellation while prompt mutation still goes through `KernelAgentService` and `PromptStateOwner`.
- Closed the legacy generic interactive fallback. The fallback lane now accepts only explicit session/agent commands and rejects unsupported commands immediately instead of re-entering the full local API or waiting on the compatibility app lock.
- Routed agent lifecycle mutations through the session runtime lane. `SpawnAgent` and `DestroyAgent` now normalize as interactive session-scoped commands, share the explicit agent request handler with local IPC, and publish session/focus projections from the runtime path instead of entering the normal local fallback.
- Routed runtime MCP through `CommandRouter`. The MCP HTTP server now binds from the router-owned config projection, authenticated local workflow runtime tools dispatch through the router, and forwarded relay workflow runtime tools also enter through the router instead of locking `DaemonApp` directly from transport handlers.
- Added explicit router handlers for relay configuration and remote-machine registry mutations. `ConfigureRelay`, `ApproveRemoteMachine`, `ForgetRemoteMachine`, and `RenameRemoteMachine` now invalidate provider-catalog projections from the router path instead of falling through the generic local compatibility request handler.
- Removed the normal/background generic local compatibility fallback from `CommandRouter`. Every non-interactive request now has an explicit router branch: warmed projection reads return early, cold session/provider/capability paths are named, workflow requests enter `WorkflowRuntime`, and provider auth/login/logout no longer wait on the app lock before running provider-side work.
- Split the remaining router cold-read/provider-sync paths away from the public local API facade. `CommandRouter` now calls named session/list/resolve/state/agent-list and provider-run helpers directly, leaving `DaemonApp::handle_local_request` out of production router dispatch.
- Ran the post-slice live drill set: daemon smoke harness, focused-agent multi-agent prompt routing, shared-endpoint multi-agent prompt routing, workflow progression without terminal pumps, downstream workflow scheduling, join-node scheduling, workflow workspace-claim retry, and CLI workflow graph/outline drill catalogs all passed.
- Removed the unused direct complete-and-auto-advance prompt mutation API from `RuntimeSession` and `PromptRuntimeState`, leaving completion on the kernel lifecycle path that reconciles against the agent-runtime queue-front preview before explicit queue advancement.
- Aligned direct compatibility complete/cancel owner resolution with the agent runtime rule: prefer the focused agent only when it is active, otherwise resolve the single active agent and reject ambiguous multi-active ownership.
- Narrowed compatibility prompt mutation visibility. `RuntimeSession` prompt mutators are now private to the session module tree, and provider dispatch failure cleanup now calls back into `KernelAgentService` instead of reaching into `SessionService` directly.
- Moved public session creation and default-agent bootstrap behind `KernelSessionService`, leaving `DaemonApp::create_session` as a compatibility facade instead of a direct lifecycle owner.
- Added daemon-health projection invariant reporting for session/agent-runtime prompt drift, with regression coverage that detects stale agent-runtime queue-front and queued-count projections.
- Routed public `CreateSession` through the session runtime mailbox boundary, including projection and focused-agent publication for the created session, so creation is not rejected behind the generic interactive lane.
- Collapsed `CreateSession` response construction into one compatibility helper shared by local API and session runtime dispatch, avoiding duplicate create/logging components while the runtime migration is still in progress.
- Added `docs/ops/IMPLEMENTATION_INVARIANTS.md` as the explicit M4.5 gate for ownership, projection refresh, cleanup, overload, health, and tests before final I/O coordination starts.
- Recorded the full A+ sequence for the rest of M4.5: prompt ownership, session ownership, projection correctness, workflow hardening, provider/terminal hardening, hot app-lock removal, docs/invariant lock, and final I/O coordination last.

### Remaining M4.5 work

- Move `KernelSessionService` session state into the new `SessionRuntime` mailbox owner, then finish removing the remaining prompt claim/mirror side effects from the shared app lock.
- Expand actor-owned projections beyond focused-agent routing and warmed session/list/history/provider-run/process/prompt-state/provider-catalog snapshots so remaining provider/read models no longer require synchronous compatibility-store access.
- Keep current workspace claims bounded until actor/projection ownership is complete; return to file-level scopes, port claims, harness enforcement, and transactional mutation/rebase semantics in the final I/O-coordination slice.
- Retire remaining hot request paths that depend on `Arc<Mutex<DaemonApp>>`.
- Run live multi-agent and workflow drills only after all non-I/O ownership/runtime slices above are complete.

## 2026-03-31

### Kernel transport hardening follow-up

- Landed resumable kernel WebSocket transport hardening for the TypeScript CLI: durable event ids, resumable subscribe, reconnect/resubscribe, heartbeat events, and bounded slow-consumer handling.
- Added layered coverage for the hardened transport:
  - TypeScript client transport contract tests
  - daemon kernel-WebSocket integration tests
  - live forced-disconnect and slow-consumer drills
- The remaining transport drills are narrower now:
  - deeper live replay/catch-up validation during active streaming output
  - long-idle heartbeat/liveness validation
- Extracted CLI live-event application and transcript-history seams so incremental pushed-event behavior and reattach catch-up can be tested directly instead of only through manual PTY runs.

### Manual multi-agent session runtime slice

- Landed the first real M4 runtime slice instead of keeping agent handling as footer/chrome-only plumbing.
- Added daemon-owned top-level agent runtime services under `apps/daemon/src/agent/`.
- Direct prompt submission now targets `focused_agent_id`.
- Provider runs are now associated with top-level agents, and the daemon parks/resumes runs as focus changes or the session returns to idle.
- Session history entries now carry `agent_id`, so provider output, notices, and user prompts can be partitioned by agent in the local runtime.
- The TypeScript CLI now supports `individual` and `split` multi-agent response modes plus visible per-agent transcript panes/previews.
- Added shared domain and Prisma updates for focused agents, agent-owned provider runs, and prompt queue targeting.

### Docs alignment update

- Updated roadmap/status docs to reflect that manual multi-agent session runtime is now in progress and no longer just planned plumbing.
- Updated local-running/protocol notes so they describe focused-agent prompt routing, agent-scoped history/provider-run ownership, and the current split-pane CLI behavior.
- Re-sequenced the spec/roadmap around an OpenCode-first development cycle: close one provider deeply first, then polish the CLI, then add multi-platform clients, and only after that expand provider support.

### Known follow-up from current code state

- The OpenCode-backed multi-agent runtime path is not fully stable yet.
- The current daemon integration suite still reports failures around:
  - provider-run launch health checks in the OpenCode event-stream path
  - delayed local-response handling through the local transport
- The current split-pane CLI is still a first slice centered on the primary transcript plus up to two auxiliary panes.

## 2026-03-30

### CLI TUI repaint skill

- Added `docs/CLI_TUI_REPAINT_SKILL.md` as a repo-native repaint playbook for future agents working on OpenTUI/JVX visual update bugs.
- Captured the main lesson from split-pane focus bugs: proactive multi-pass repainting and child-renderable rebuilds matter more than only changing parent pane colors.

## 2026-03-29

### Multi-agent docs alignment update

- Reviewed the current daemon and TypeScript CLI agent plumbing after reproducing that focused-agent changes currently affect footer/chrome state more than actual runtime routing.
- Updated `README.md`, `docs/spec-v1.md`, `docs/ARCHITECTURE.md`, `docs/PROTOCOL.md`, `docs/RUNNING_LOCAL.md`, `docs/ROADMAP.md`, and `docs/ops/TASKS.md` so they distinguish three things clearly:
  - current single-agent-effective runtime behavior
  - already-landed session-agent metadata/focus plumbing (`/agent ...`, `Ctrl+A`, focused-agent state)
  - the intended next milestone: manual multi-agent sessions with per-agent context/history and split-pane CLI rendering before workflow automation
- Reframed the roadmap so manual multi-agent session execution is the next step ahead of daemon-scheduled workflow topology work.

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

### Workflow runtime phase 1 update

- Added daemon-owned workflow runtime entities for `WorkflowRun`, `WorkflowNodeRun`, and `WorkflowMessage` on the current session runtime path.
- Added local API invoke/list/get/cancel flow for workflow runs, keyed off existing workflow endpoints.
- Added workflow-run daemon tests plus a local IPC socket round-trip covering create -> list -> get -> cancel on the new transport surface.
- Kept this slice intentionally narrow: endpoint invocation now persists runnable workflow state, but it does not yet schedule provider turns or execute graph handoffs.
- New immediate priority: local daemon + CLI + OpenCode integration with prompt submission and live output streaming.
- Deferred broader local capabilities, additional providers, multi-agent workflows, relay/web surfaces, provider switching, memory, compaction, and per-agent extension management to later milestones after that baseline is proven.

### Workflow runtime scheduler slice update

- Added daemon-owned entry-node scheduling for endpoint-triggered workflow runs on top of the existing prompt queue and provider runtime.
- Workflow-owned prompts now carry workflow run/node run context so prompt start, completion, cancellation, and unexpected provider exits reconcile back into `WorkflowRun` and `WorkflowNodeRun` state.
- Entry-node scheduling can auto-launch a provider run for the bound agent when one is not already active, then dispatch the workflow prompt through the same top-level agent runtime.
- Kept this slice intentionally narrow: there is still no CLI `/workflow run` surface yet, and downstream node handoffs are not executed. Runs currently become `Completed` when the entry node has no outgoing edges, or `Waiting` when downstream edges exist.

### Workflow runtime handoff slice update

- Added a daemon-owned structured handoff payload for downstream routing, including workflow run id, workflow id, source node run id, source node id, source agent id, target node id, and the root invocation prompt.
- Node completion now creates one workflow message per outgoing edge, creates one downstream node run per routed message, and schedules those downstream node prompts through the same prompt/provider runtime.
- Queued workflow prompts can now auto-launch the target agent's provider run when they reach the front of the session queue, so chained workflow execution no longer depends on pre-launched runs.
- Added daemon tests plus a local IPC socket round-trip covering entry execution -> downstream routing -> downstream completion for a simple chained workflow.

### Workflow join gating slice update

- Workflow handoffs are now buffered on the target side instead of immediately creating one node run per incoming edge.
- Join nodes default to `all_inputs` gating when their indegree is greater than one, so one downstream node run starts only after all required parent messages are present.
- Workflow messages now record which node run consumed them, making aggregated activations and later audit/replay possible without forwarding transcript history.
- Fixed a queue-advancement bug where completing one workflow prompt could overwrite an already-started downstream prompt instead of only advancing when no active prompt remained.
- Added daemon coverage for service-level join gating and a local API round-trip proving that join nodes do not start early and do start exactly once after the final parent completes.

### Workflow runtime CLI slice update

- Wired `/workflow run`, `/workflow runs`, and `/workflow cancel` into the TypeScript CLI on top of the existing daemon workflow-run API.
- Added command-center entries and CLI help text for the new workflow runtime commands.
- Updated the workflow canvas to show the selected workflow's display run id/status and per-node status derived from the newest active run, falling back to the newest run overall.
- Added CLI tests covering workflow runtime commands plus graph-layout tests for runtime status rendering.

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
- Removed the previous Rust-only CLI after the TypeScript client became the only supported local CLI implementation.

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

### Workflow runtime completion snapshot update

- Extended workflow node completion so the daemon now derives a summary-only completion payload from persisted provider output for the exact provider run that settled the node.
- Persisted that summary-only payload on completed `WorkflowNodeRun` records and forwarded it in downstream workflow handoff payloads, while keeping the full transcript only in session history for audit rather than as workflow output.
- Added daemon coverage proving that downstream handoffs retain the upstream node summary when provider output exists before prompt completion.

### Workflow runtime artifact reference update

- Added optional artifact refs to the workflow completion payload so a completed node can forward `summary + artifacts` without forwarding transcript data.
- Namespaced session artifacts by attachment/workflow source under the daemon artifact root so workflow-owned artifacts can be discovered without sweeping unrelated session files.
- Added daemon coverage proving that a workflow-owned artifact appears on the completed node run and in the downstream handoff payload.

### Workflow explicit output contract update

- Changed the workflow runtime contract so `summary` remains human-facing while downstream routing uses an explicit `output.message` plus optional artifact refs.
- Updated workflow-owned prompts to request a structured JSON completion envelope with separate `summary` and `output`.
- Reframed the docs around graph-derived execution and per-node gating/release policy instead of user-declared circular vs hierarchical workflow modes.

### M4.5 prompt dispatch and terminal cleanup update

- Kept provider-prompt worktree claim admission synchronous, so cross-session same-worktree prompt conflicts still fail before `PromptSubmitted`.
- Moved local provider prompt PTY writes and provider actor enqueue work into the spawned provider-run operation dispatch after owner-backed prompt mutation, reducing work done inline by the per-agent mailbox response path.
- Added session-runtime terminal cleanup on session end/delete so terminal input, pending output, notices, completions, and terminal backlog health do not retain stale records for removed sessions.
- Added CLI request helpers for `CompletePrompt` and `AckWorkflowTurn` so deterministic live drills can use the public kernel API once the non-I/O runtime slices are complete.

### M4.5 compatibility facade retirement update

- Added an explicit facade-retirement checklist to the M4.5 plan, separating public facade retirement, router independence, actor ownership, runtime service extraction, and final compatibility handler deletion.
- Added a router-backed in-process local daemon client for tests and smoke harnesses that need to send `LocalDaemonRequest` without calling `DaemonApp::handle_local_request` directly.
- Moved the local smoke harness and external daemon integration test off direct `handle_local_request` calls.
- Demoted `DaemonApp::handle_local_request` to crate-private compatibility surface. It remains only for internal compatibility tests and transitional service code until later ownership slices remove the remaining facade-only handlers.
