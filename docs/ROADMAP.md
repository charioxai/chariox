# Arroba Roadmap

## Status

Roadmap derived from `docs/spec-v1.md` and updated to match the code currently in `main`.

Current milestone status:

- M0 completed on 2026-03-16
- M1 completed on 2026-03-17
- M2 completed on 2026-03-18
- M3 is now the OpenCode-first completion phase: capabilities, agent harnessing behavior, and remaining single-provider hardening work are still open
- M4 is now the OpenCode-first local-runtime completion phase: the first manual multi-agent runtime slice and workflow runtime are landed, while local stabilization, immediate CLI responsiveness, and workflow polish remain open
- M4.5 is now the kernel runtime refactor phase: the daemon implementation is actively moving from a global `DaemonApp` lock toward an actor/event/projection kernel before relay scale-out
- M5 is now the relay and remote-transport phase: relay infrastructure, remote terminal attachment, and daemon identity are the next major delivery target

## 1. Roadmap Goals

- deliver a stable daemon-centered v1 runtime
- evolve the daemon into an explicit node runtime that can route both local and remote members in one session domain
- ship a fully functioning local daemon + CLI path with one provider before expanding orchestration breadth
- preserve native provider UX while adding orchestration value
- keep server lightweight with strict security boundaries
- close the OpenCode-first development cycle before adding more providers
- finish agent harnessing and multi-machine session behavior before multi-platform and multi-provider expansion
- keep the runtime compatible with later multi-agent workflow execution, structured handoffs, and isolated branch worktrees
- add daemon-owned workspace coordination so multi-agent integration does not depend on a human manually resolving every conflict, while keeping deep I/O conflict control as a later design after the actor/projection kernel is stable

## 2. Milestone Structure

- M0: Foundations
- M1: Core Session Runtime
- M2: End-to-End Local OpenCode Baseline
- M3: OpenCode Completion and Local Capability Hardening
- M4: OpenCode Local Runtime Completion
- M4.5: Kernel Runtime Refactor
- M5: Relay and Remote Transport
- M6: Remote Agents and Machine Membership
- M7: Additional Clients
- M8: Workflow Interconnection
- M9: Multi-Provider Expansion and Adapter Generalization
- M10: v1 Stabilization and Launch

## 2.1 Delivery Sequence Within v1

Multi-agent workflow execution remains part of v1, but it is no longer the immediate implementation priority.

Rollout priority:

- first deliver a single-agent local daemon + CLI path with one provider and live terminal streaming
- then close the OpenCode-first local cycle with capabilities, agent harnessing behavior, workflow runtime behavior, and local hardening
- then remove the daemon hot-path dependency on one shared `DaemonApp` lock by introducing actor-owned mutation, command routing, ordered kernel events, and query projections
- then add relay-backed remote transport
- then add remote agents and machine membership on top of relay
- then add additional clients on the same daemon/protocol model
- then add workflow interconnection on top of remote agent connectivity
- then add additional providers such as Claude Code and Codex plus the more generic provider-adapter/protocol work they require
- workflow scheduling remains in scope, but it should follow the OpenCode-first runtime/harnessing/node-connectivity completion and fit inside the same long-term daemon architecture
- the workflow model is now explicitly multi-definition per workspace and multi-endpoint per workflow; the first implementation slice should start with kernel-backed workflow definitions/endpoints before graph scheduling
- that first slice is now implemented in the local runtime: workflow definitions are kernel-backed, workflows/endpoints resolve by id or alias, and node/edge/endpoint management exists before any run scheduling

## 3. Milestones

## M0 - Foundations

Status:

- completed on 2026-03-16
- delivered workspace scaffolding, baseline CI, shared domain types, Prisma schema baseline, and Rust daemon/server bootstrap packages

Outcomes:

- monorepo/workspace setup and baseline CI
- initial shared domain model for user/machine/daemon/session/provider run
- developer docs baseline (`spec-v1`, architecture, protocol, roadmap)

Exit criteria:

- repository can build/lint/test baseline packages
- docs provide a coherent implementation target

## M1 - Core Session Runtime

Outcomes:

- daemon process lifecycle
- session lifecycle (create/attach/detach/end)
- PTY manager for provider runs
- multi-attachment support with daemon-owned prompt queueing and canonical config propagation
- parked provider run support with one active run at a time
- workflow-compatible runtime foundations so later multi-agent graph execution can be added without redesigning session/provider/worktree ownership

Exit criteria:

- local client can run native provider in managed session
- multiple clients can attach without breaking terminal behavior
- implemented runtime surfaces do not block later workflow graph execution, node-scoped provider runs, or explicit worktree assignment

## M2 - End-to-End Local OpenCode Baseline

Status:

- completed on 2026-03-18
- delivered a real local daemon IPC path, a minimal local CLI, a real `opencode` adapter, and end-to-end prompt/output streaming through the daemon
- first iteration continues to exclude provider login flows because OpenCode can run without login by default

Outcomes:

- OpenCode provider adapter wired through the daemon
- local CLI client attached to daemon-managed sessions
- single-agent prompt submission from an input field
- live terminal/output streaming from the active OpenCode run back into the CLI as output appears
- stable session creation/attach/launch/prompt/output loop for one local user on one machine
- no workflow mode, no remote relay, no web app, and no provider switching in this milestone

Exit criteria:

- local CLI can create or attach to a session, launch OpenCode, submit a prompt, and stream output in real time
- daemon remains the authority for session and PTY/provider-run lifecycle during that flow

## M3 - OpenCode Completion and Local Capability Hardening

Status:

- M3 remains focused on closing the one-provider OpenCode cycle rather than expanding provider breadth
- a large part of the local baseline hardening work has already landed: shared logging, persistent session management, richer transcript rendering, command-center work, and the early agent-management controls the M4 runtime now builds on
- remaining M3 work is now the narrower OpenCode-first follow-on set: broader capability wiring, local UX hardening, and adapter/runtime stabilization without adding new provider families yet

Outcomes:

- upgrade OpenCode from the M2 PTY bootstrap path to a structured local server/session/event adapter
- add a project-wide logging/debugging system with one machine-local log root and correlated logs across daemon and client processes
- add explicit persistent session management with daemon-owned delete semantics and a reusable unattached CLI state
- add syntax-highlighted markdown/code rendering in the TypeScript CLI transcript for provider responses
- replace numeric session ids with commit-like ids and optional aliases, including unique-prefix resolution
- harden the primary TypeScript local CLI after the Rust-to-TypeScript client migration
- shell command capability
- directory tree + file view/edit capabilities
- screenshot capture capability
- git/worktree inspection capability
- file transfer + attach-transferred workflow
- daemon-owned slash-command dispatch for Arroba capabilities
- workflow-compatible local capability design so later multi-agent execution can reuse the same surfaces
- keep provider breadth intentionally fixed to OpenCode while this cycle is being closed

Exit criteria:

- capability failures remain isolated from the terminal lane
- the OpenCode-first local daemon + CLI model is complete enough that new providers are no longer needed to shake out the primary runtime contract
- local slash-command UX is usable enough to drive capabilities without a web surface
- OpenCode prompt lifecycle no longer depends on PTY-idle heuristics
- detached sessions remain resumable until explicit deletion, and deleting the current session returns the CLI to a no-session state instead of forcing process exit

## M4 - OpenCode Local Runtime Completion

Status:

- in progress as of 2026-04-10
- the first manual multi-agent runtime slice is now in the codebase: top-level session agents are real execution targets, direct prompts route to the focused agent, provider runs are tracked per agent, agent-scoped history/output are recorded, and the TypeScript CLI supports `individual` and `split` multi-agent response modes
- the current TypeScript CLI split view is still an initial slice, centered on the primary transcript plus up to two auxiliary panes
- relay-backed remote connectivity has been moved out of this milestone and is now the primary goal of M5
- the first transport/alignment slice has now landed:
  - kernel-facing WebSocket transport for the TypeScript CLI
  - managed vs external OpenCode endpoint binding
- kernel-CLI transport hardening is now part of the delivered slice:
  - monotonic pushed `event_id` values with bounded in-process replay
  - resumable subscribe via `resume_from_event_id`
  - heartbeat/liveness events
  - client reconnect/resubscribe behavior
- layered interaction testing is now part of the M4 baseline:
  - CLI transport contract tests
  - daemon kernel-WebSocket integration coverage
  - live daemon + CLI smoke validation when transport/runtime behavior changes
- transport resiliency drills are now partly complete:
  - forced live disconnect with CLI reconnect/resubscribe validation is done
  - slow-consumer/backpressure validation is done
  - missed-event replay/catch-up validation after reconnect still needs a deeper streaming-focused live drill
  - long-idle heartbeat/liveness validation is still pending
- workflow runtime/state coverage inside the current M4 slice now also includes:
  - daemon-owned workflow MCP tools for managed runs
  - stop/resume for workflow runs with preserved turn envelopes
  - structured workflow failure events and mailbox routing
  - CLI workflow inspection of failure/audit state
  - live mixed-provider workflow drills across Codex and OpenCode
  - workflow-scoped shared console
  - endpoint-scoped watchdog scheduling with interval triggers
  - watchdog `skip` and `queue` scheduling policies
- recent OpenCode multi-agent stabilization now covers:
  - queued prompts preserving their target agent run even while another agent is actively working
  - queued backlog advancing onto another healthy agent run after an unexpected active-run exit
- OpenCode-backed multi-agent runtime stabilization is still needed

Outcomes:

Delivered in the current M4 slice:

- multiple top-level Arroba-managed agents inside one session, each with its own focused-agent targeting, provider-run ownership, and agent-scoped history/runtime metadata
- `Tab` and `/agent cycle` switch the active agent for direct interaction, not only footer metadata
- prompts and provider output route through the focused agent's runtime context for the local daemon + CLI path
- TypeScript CLI `individual` and `split` response modes with visible per-agent panes/previews for the current local path
- explicit distinction between Arroba-managed top-level agents and provider-native subagents in the local runtime/data model
- kernel-hosted WebSocket transport now exists for the primary TypeScript CLI path, while the older local IPC path remains for harnessing/tests
- the first OpenCode-specific endpoint abstraction now exists in code through managed vs external endpoint binding, with OpenCode already supporting both modes
- layered interaction testing now exists for the kernel/CLI path:
  - TypeScript transport contract tests for request, subscribe, unsubscribe, and close behavior
  - daemon-side kernel-WebSocket integration coverage for request/subscribe/session-unavailable flows
  - manual live-program smoke runs are now an expected part of transport/runtime validation

Still pending in M4:

- OpenCode-path multi-agent stabilization
- broader agent interactions for harnessing on top of the current focused-agent runtime
- workflow runnable validation and preflight diagnostics (missing endpoints/nodes/agents, invalid graphs)
- broader automated interaction coverage:
  - deterministic kernel/CLI transcript-flow integration tests
  - PTY-driven terminal smoke tests for visible CLI behavior
- generic agent transport remains deferred until after additional agent integrations beyond OpenCode
- workspace coordination baseline:
  - per-agent worktree or branch allocation
  - file/workspace claim tracking
  - mergeability/integration validation
- daemon-owned provider runtime process ledger is now landed:
  - tracked managed provider processes
  - tracked provider-native session ids per provider run
  - CLI `/provider processes` inspection
  - safe teardown of idle/orphaned managed provider processes without breaking attached sessions
  - blocker-aware safe-teardown reporting in the CLI
  - late-stage daemon identity and manager semantics:
    - persisted daemon runtime record under `.arroba/runtime/daemon`
    - single-active-daemon lock per workspace/worktree
    - CLI attach-or-start behavior against the workspace daemon record
    - managed child stamping with daemon instance id
    - optional stale managed-child reap on daemon restart
- watchdog wakeup budgeting:
  - default bounded `max_wakeups`
  - explicit `null` for unbounded schedules
- live-drill operational cleanup:
  - every reusable drill harness should own and stop the daemon it starts
  - reusable drill harnesses should end the sessions they create on exit
- buffered endpoint-facing workflow outputs:
  - intermediate workflow run outputs emitted by designated nodes
  - workflow-level intermediate output schema with optional node-level override
  - final and intermediate workflow outputs buffered at turn scope and only committed/forwarded when the turn completes
- kernel/daemon naming cleanup:
  - evaluate renaming `scheduler` to `runtime` or equivalent so ownership boundaries read correctly
- daemon internal modularization is now landed:
  - `SessionService` split into focused internal modules
  - local API split into module + request/response type/test files
- local completion still pending:
  - richer workflow inspection/history UI
  - watchdog cron syntax
  - current CLI syntax/structure stabilization where needed

The following workflow-runtime items are no longer pending in M4:

- daemon-managed workflow instruction artifacts per node
- per-edge workflow output schema registry plus runtime validation tooling
- managed-run MCP tool exposure for workflow ACK and validation
- workflow stop/resume with preserved turn-envelope context
- structured workflow failure/audit state surfaced through runtime and CLI

Additional M4 workflow-runtime item now planned:

- kernel-owned workflow console service:
  - one append-only console per workflow definition
  - MCP `read` / `write` / `clear` for workflow nodes
  - CLI `/workflow terminal` surface in the right-side panel
- kernel/CLI drill reliability and timeout diagnostics:
  - request-level local IPC tracing with request ids and duration logging
  - clearer timeout attribution between transport, handler, and provider wait states
  - a cleaner dedicated live-drill harness with isolated daemon/provider lifecycle
  - daemon health snapshot surfaces for active requests, prompts, provider runs, and scheduler state
- watchdog follow-up:
  - cron syntax support on top of the shipped interval scheduler
  - architecture naming cleanup: reconsider whether `scheduler` should be renamed to `runtime` now that endpoint watchdogs sit beside, not inside, workflow dispatch

Exit criteria:

- manual multi-agent session execution works through the daemon node, with visible per-agent panes/history and correct focused-agent prompt routing, and the OpenCode-backed runtime path is stable under integration coverage
- workspace coordination prevents or explicitly surfaces conflicting edits before shared integration
- the one-provider development cycle can be considered closed without depending on a second provider for validation
- the primary kernel/CLI path has transport-contract, daemon-integration, and live smoke coverage strong enough that transport refactors do not depend on compile-only verification
- local workflow runtime behavior is stable enough that remote transport work does not need to also solve local runtime ambiguity

## M4.5 - Kernel Runtime Refactor

Status:

- in progress as of 2026-04-13
- architectural target is documented in [ARCHITECTURE.md](/Users/miguel/arroba/docs/ARCHITECTURE.md)
- implementation plan is documented in [M4_5_KERNEL_RUNTIME_REFACTOR_PLAN.md](/Users/miguel/arroba/docs/M4_5_KERNEL_RUNTIME_REFACTOR_PLAN.md)
- first implementation slices are landed: kernel command/event envelopes, event replay gaps, projection metadata, command routing, bounded interactive routing, typed CLI replay-gap handling, command-id retry/fanout safety, inbound WebSocket admission bounds, provider-run actor runtime isolation, provider actor enqueue error propagation, reserved-listener websocket tests, `KernelSessionService` ownership for public session creation/default-agent bootstrap plus session lifecycle/focus/resize/end/delete operations, `KernelAgentService` ownership for prompt submit/cancel/complete/queue-advance lifecycle operations, per-agent prompt command mailboxes, session-runtime create plus per-session attach/focus/resize/end/delete command mailboxes with cleanup on session close, a focused-agent projection refreshed by session mailboxes and agent lifecycle responses for untargeted prompt routing, warmed projections for `GetSessionState`, `ListSessions`, `GetSessionHistory`, `GetProviderRun`, `ListProviderProcesses`, `GetProviderCatalog`, and agent/workflow inspection reads, shared kernel `PromptStateOwner` ownership for active/queued prompt mutation with compatibility session mirroring, owner-backed prompt routing and queue-front reads from `AgentRuntime`, prompt lifecycle publication into the shared session projection, one shared agent-runtime active/queued prompt projection read model, session response-borne projection refresh/removal from the session mailbox, workflow-lane missing-session rejection from warmed projections, workflow response-borne projection refresh, daemon-health projection invariant drift reporting between session and agent-runtime prompt projections, a dedicated flattened `PromptRuntimeState` session prompt-runtime module for compatibility prompt mirrors/projections, daemon health projection snapshots for actor queues, provider runtime lanes, prompt counts, provider-catalog cache state, kernel websocket transport pressure, workspace worktree-collision state, `WorkspaceCoordinator` write-claim enforcement for explicit file-writing capabilities, provider prompt lifecycle worktree claims for cross-session same-worktree conflicts, and workflow node dispatch blocking/retry on workspace claims
- remaining work is still substantial: moving session/workflow state ownership behind mailbox runtimes, broadening actor-owned projections beyond focused-agent routing and warmed session/list/history/provider-run/process/prompt-state/provider-catalog snapshots, and removing remaining hot-path `Arc<Mutex<DaemonApp>>` dependencies; workspace coordination should stay at its current coarse claim scope until the final I/O-coordination slice

Problem statement:

- the daemon is already the correct authority boundary, but too much live implementation still flows through a coarse shared application object
- a global `DaemonApp` lock is incompatible with an operating-system-like agent orchestration runtime where prompt submit, cancel, focus, resize, relay resume, provider output, workflow scheduling, history scans, capability jobs, and health inspection all run concurrently
- splitting the lock into many smaller locks is not enough; the runtime needs clear ownership, ordered events, and read projections

Outcomes:

- introduce `DaemonKernel` as the composition root for command routing, event logging, projection updates, actor lifecycle, transport gateways, and background executors
- define first-class `KernelCommand`, `KernelEvent`, command ids, event sequence ids, causation ids, and correlation ids for local and future relay paths
- move prompt submit, cancel, focus, attach, detach, resize, and subscription resume onto an `InteractiveCommandLane`
- create actor or mailbox ownership for sessions, agents, provider runs, workflow runs, capability execution, and relay runtime state
- introduce a projection store for session lists, session snapshots, transcript pages, workflow inspection, provider process ledgers, and daemon health snapshots
- retire `Arc<Mutex<DaemonApp>>` from hot request paths; keep a compatibility facade only while handlers migrate
- keep provider catalog discovery, history hydration, capability jobs, provider process inspection, and relay bookkeeping off the interactive command path
- add explicit bounded queues and backpressure policies for slow consumers and background work
- make replay-window and replay-gap behavior explicit before relay scale-out
- introduce daemon-owned worktree/collision coordination as part of the kernel boundary

Exit criteria:

- prompt submit, cancel, focus, terminal resize, attach, detach, and event subscription resume do not wait behind history reads, capability jobs, provider discovery, provider output fanout, or relay background work
- the primary CLI reads projections and reconciles pushed events without requiring synchronous whole-session refreshes on the hot path
- daemon tests cover command ordering, idempotent command retry, actor isolation, projection consistency, reconnect/replay gaps, slow consumers, worktree collision, and backpressure
- live CLI drills show immediate local feedback and stable daemon responsiveness while background work is intentionally slowed
- the relay milestone can build on the same command/event/projection model without turning relay into workspace authority

## M5 - Relay and Remote Transport

Planning reference:

- `docs/M5_RELAY_PLAN.md`

Outcomes:

- standalone relay service as an independent Rust app
- daemon-owned stable identity for remote registration:
  - persisted `daemon_id`
  - persisted `machine_id`
  - optional human-friendly daemon alias
- daemon outbound registration to one active relay endpoint at a time, configured from the CLI or waiting-room relay panel
- the CLI always starts in the normal waiting room; local sessions are never blocked by relay setup or relay failures
- relay auto-connect happens in the background after a relay is configured; the waiting room updates when connected, and active sessions may surface a small informational footer when remote capability becomes available
- the waiting room keeps relay status and relay actions together under a `Relay` section, including the currently configured relay and the option to configure another one while connected
- the same CLI supports both local direct operation and relay-mediated remote operation without a separate remote CLI app
- remote terminal attachment through relay using the same daemon-owned request/event semantics
- remote event subscription, heartbeat, reconnect, and resume behavior
- self-hosted relay mode for the open-source project:
  - static/shared credential configuration
  - explicit daemon targeting by id or alias
- mandatory session-scoped end-to-end encryption for all user-generated remote payloads, including prompts, workflow payloads, and transferred artifacts
- self-hosted relay deployments do not relax the end-to-end encryption requirement
- narrow roadmap note only:
  - a separate service may later integrate with the relay for managed identity/discovery, but that service remains outside this repository and roadmap scope

Exit criteria:

- a daemon can register to a configured relay in the background and remain connected through an outbound connection
- the waiting room shows local status, relay status/actions, and machine status in one surface
- a CLI can connect through the relay to a selected daemon by id or alias
- remote prompt submission and output streaming work without changing daemon session authority
- the relay remains a transport broker, not a workspace/workflow authority
- open-source self-hosted relay usage does not depend on any external managed service


## M5 Docker Remote-Machine Lab

The Docker lab is part of validating M5/M6 locally before requiring real separate machines. The repository should provide an Arroba base image that includes all Arroba apps (`arroba` CLI, daemon/kernel, and relay) plus required runtime dependencies. Provider installation and provider login remain the user's responsibility inside each container. Containers must have outbound internet for provider model calls, provider installation, hosted relay access, and provider login. Browser-based provider login is handled as a provider compatibility concern: the base image may include URL-printing browser shims, and each launch provider is tested/documented individually for Linux/container login behavior, callback ports, credential persistence, and normal prompt execution after restart.

The lab should support multiple persistent worker containers so users can keep separate accounts for the same provider active at the same time and select them in Arroba as machine-qualified providers such as `Codex (work-container)` and `Codex (personal-container)`. It includes a smoke runner, Codex/OpenCode installation helper, and a remote-CLI-to-host drill; provider account login remains manual.

## M6 - Remote Agents and Machine Membership

Outcomes:

- remote machine registration on top of relay connectivity, with machine status displayed in the waiting room; pending machines are shown inline in the machine section rather than as a separate mode
- all machine management operations are available as in-CLI slash commands: `/machine list`, `/machine kernels <machine-ref>`, `/machine approve <machine-ref>`, `/machine forget <machine-ref>`, and `/machine rename <machine-ref> <alias>`
- remote daemons can host top-level Arroba-managed agents for the same logical collaboration model
- session membership spanning multiple machines
- remote agent routing and lifecycle ownership
- provider-run ownership and state reporting for remote nodes
- remote member resume/reassignment semantics
- worker-kernel provider availability is advertised back to the home kernel and surfaced in the CLI as machine-qualified provider availability
- remote agents are placed by machine, then bound to one selected worker kernel for their lifetime
- provider login remains local to the worker kernel; the home kernel only consumes advertised provider availability

Exit criteria:

- a session can include remote machine-hosted agents without creating a second session authority
- daemon and machine identity are strong enough for remote resume/reassignment
- remote agent lifecycle is observable and debuggable through the same kernel-owned model
- remote agents feel the same as local agents in the CLI once spawned, aside from explicit machine placement metadata

## M7 - Additional Clients

Outcomes:

- polished TypeScript CLI as the reference Arroba client for both local and relay-backed use
- refined split-pane and multi-agent transcript UX
- session-scoped E2E encryption for user-generated in-transit payloads
- operational metadata storage boundaries enforced
- iOS terminal client on the same daemon/protocol model
- later web and Android clients if still in v1 scope

Exit criteria:

- the TypeScript CLI is the polished reference client for the local and relay-backed runtime model
- additional client surfaces consume the same daemon/protocol semantics rather than introducing surface-specific runtime logic

## M8 - Workflow Interconnection

Outcomes:

- workflow nodes bound to remote agents and machines
- remote workflow message routing and handoff delivery
- cross-machine workflow-run progression on the same logical runtime model
- remote workflow failure/cancellation semantics
- endpoint-facing workflow outputs preserved across inter-machine execution

Exit criteria:

- workflow runs can cross machine boundaries without moving workflow authority out of the daemon/kernel model
- routing, failure, and output delivery semantics remain machine-parseable and observable

## M9 - Multi-Provider Expansion and Adapter Generalization

Outcomes:

- provider adapter abstraction hardened after the OpenCode-first cycle closes
- Claude Code and Codex support
- generic protocol and adapter design hardened for later provider families
- canonical control operations:
  - `attach_file`
  - `request_memory_update`
  - `request_compaction_summary`
- provider installation/auth-state probing with native CLI login reuse
- provider version probing plus shipped command catalogs for supported provider versions
- best-effort `/<provider>` completion on unsupported provider versions with explicit warnings
- transfer package generation for provider switch/machine reassignment/resume
- dual memory model implementation:
  - short-term memory
  - long-term memory
- user-triggered Arroba context compaction flow (`/compact`)
- agent-scoped extension registry for skills, MCPs, command packs, and related provider assets
- daemon-managed MCP runtime with per-agent binding and visibility

Exit criteria:

- additional providers can fit the daemon/client contract without reshaping the OpenCode-first baseline
- provider switching works without depending on provider-private hidden state
- extension binding and MCP visibility are enforced per top-level agent

## M10 - v1 Stabilization and Launch

Outcomes:

- reliability hardening and compatibility testing across providers
- operational telemetry and failure diagnostics
- user/admin docs for setup, security expectations, and limitations
- release process and upgrade guidance
- completion of the v1 workflow rollout, with circular topology delivered earlier and hierarchical topology completed later in v1
- MCP runtime-tool hardening before release:
  - one daemon-owned Arroba MCP server
  - automatic MCP attachment for managed provider runs
  - dynamic per-turn tool scoping and stronger per-run isolation
  - reconnect/health monitoring for MCP-bound provider runs
  - removal of transitional prompt-only ACK/tool guidance once MCP execution is proven

Exit criteria:

- v1 release checklist complete
- known non-goals and follow-up items documented

## 4. Cross-Cutting Workstreams

- Project-wide observability and debugging pipeline
- Unified node transport for local and relayed members
- Provider compatibility matrix and adapter conformance
- Extension compatibility matrix and projection rules per provider
- UX quality for slash-command completion, status, and transfer transparency
- Security/privacy review and threat modeling
- Performance targets for PTY throughput and capability latency
- Workspace coordination and integration safety
- Generic workflow-engine compatibility: directed-graph scheduling model, structured handoff/completion contracts, worktree isolation, and aggregation/barrier semantics

## 5. Risks and Mitigations

- **Risk:** Provider behavior variance across CLIs.
  - **Mitigation:** strict adapter contracts, shipped versioned command catalogs, custom-command discovery where supported, and explicit best-effort warnings on unsupported versions.
- **Risk:** Provider login drift or expired local sessions.
  - **Mitigation:** reuse provider-native login flows, probe structured auth state before launch, and surface local reauthentication guidance without storing provider credentials.
- **Risk:** Provider-local extension files leak across agents or workflows.
  - **Mitigation:** keep a daemon-owned extension registry, bind per top-level agent, and use provider config-root/worktree isolation when materializing provider views.
- **Risk:** Memory drift or stale long-term memory.
  - **Mitigation:** user review/edit/remove controls + explicit refresh reasons.
- **Risk:** Overgrowth of control surface in v1.
  - **Mitigation:** keep canonical control operations minimal and versioned.
- **Risk:** Relay trust expansion.
  - **Mitigation:** maintain session-E2E transport and metadata minimization.

## 6. Post-v1 Candidates

- richer workflow automation and policy hooks
- broader messaging-client integrations
- provider command driver registration so future adapters can install command catalogs into Arroba without hardcoding them in the CLI
- advanced machine migration orchestration
- optional content persistence with explicit governance
