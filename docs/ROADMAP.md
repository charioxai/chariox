# Arroba Roadmap

## Status

Roadmap derived from `docs/spec-v1.md` and updated to match the code currently in `main`.

Current milestone status:

- M0 completed on 2026-03-16
- M1 completed on 2026-03-17
- M2 completed on 2026-03-18
- M3 is now the OpenCode-first completion phase: capabilities, agent harnessing behavior, and remaining single-provider hardening work are still open
- M4 is now the OpenCode-first multi-agent and node-connectivity phase: the first manual multi-agent runtime slice has landed, while stabilization, same-node remote connectivity, and multi-machine session behavior remain open

## 1. Roadmap Goals

- deliver a stable daemon-centered v1 runtime
- evolve the daemon into an explicit node runtime that can route both local and remote members in one session domain
- ship a fully functioning local daemon + CLI path with one provider before expanding orchestration breadth
- preserve native provider UX while adding orchestration value
- keep server lightweight with strict security boundaries
- close the OpenCode-first development cycle before adding more providers
- finish agent harnessing and multi-machine session behavior before multi-platform and multi-provider expansion
- keep the runtime compatible with later multi-agent workflow execution, structured handoffs, and isolated branch worktrees
- add daemon-owned workspace coordination so multi-agent integration does not depend on a human manually resolving every conflict

## 2. Milestone Structure

- M0: Foundations
- M1: Core Session Runtime
- M2: End-to-End Local OpenCode Baseline
- M3: OpenCode Completion and Local Capability Hardening
- M4: OpenCode Agent Harnessing and Node Connectivity
- M5: CLI Polish and Multi-Platform Clients
- M6: Multi-Provider Expansion and Adapter Generalization
- M7: v1 Stabilization and Launch

## 2.1 Delivery Sequence Within v1

Multi-agent workflow execution remains part of v1, but it is no longer the immediate implementation priority.

Rollout priority:

- first deliver a single-agent local daemon + CLI path with one provider and live terminal streaming
- then close the OpenCode-first local cycle with capabilities, agent harnessing behavior, same-node remote connectivity, and multi-machine session support
- then polish the TypeScript CLI as the reference client
- then add multi-platform clients on the same daemon/protocol model
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

## M4 - OpenCode Agent Harnessing and Node Connectivity

Status:

- in progress as of 2026-03-31
- the first manual multi-agent runtime slice is now in the codebase: top-level session agents are real execution targets, direct prompts route to the focused agent, provider runs are tracked per agent, agent-scoped history/output are recorded, and the TypeScript CLI supports `individual` and `split` multi-agent response modes
- the current TypeScript CLI split view is still an initial slice, centered on the primary transcript plus up to two auxiliary panes
- same-node remote connectivity for terminals and agents is still pending as part of this same OpenCode-first completion phase
- the first transport/alignment slice has now landed:
  - kernel-facing WebSocket transport for the TypeScript CLI
  - managed vs external OpenCode endpoint binding
- kernel-CLI transport hardening is now part of the delivered slice:
  - durable pushed `event_id` values
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
- multi-machine session behavior is still pending after that, on the same node-oriented architecture
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
- same-kernel remote terminal connectivity on top of the hardened kernel-CLI WebSocket transport
- workflow runnable validation and preflight diagnostics (missing endpoints/nodes/agents, invalid graphs)
- broader automated interaction coverage:
  - deterministic kernel/CLI transcript-flow integration tests
  - PTY-driven terminal smoke tests for visible CLI behavior
- relay-backed same-kernel remote member support for terminals and agents
- generic agent transport remains deferred until after additional agent integrations beyond OpenCode
- workspace coordination baseline:
  - per-agent worktree or branch allocation
  - file/workspace claim tracking
  - mergeability/integration validation
- multi-machine session ownership, reassignment, and resume semantics on the same node-oriented one-provider baseline
- daemon-owned provider runtime process ledger:
  - tracked managed provider processes
  - tracked provider-native session ids per provider run
  - CLI `/provider processes` inspection
  - safe teardown of idle/orphaned managed provider processes without breaking attached sessions
- watchdog wakeup budgeting:
  - default bounded `max_wakeups`
  - explicit `null` for unbounded schedules
- live-drill operational cleanup:
  - every reusable drill harness should own and stop the daemon it starts
- kernel/daemon naming cleanup:
  - evaluate renaming `scheduler` to `runtime` or equivalent so ownership boundaries read correctly

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
- same-node local and relayed terminals/agents can attach through one daemon-owned protocol model without changing session authority
- workspace coordination prevents or explicitly surfaces conflicting edits before shared integration
- multi-machine session reassignment/resume behavior is coherent on the same OpenCode-first node model
- the one-provider development cycle can be considered closed without depending on a second provider for validation
- the primary kernel/CLI path has transport-contract, daemon-integration, and live smoke coverage strong enough that transport refactors do not depend on compile-only verification

## M5 - CLI Polish and Multi-Platform Clients

Outcomes:

- polished TypeScript CLI as the reference Arroba client
- refined split-pane and multi-agent transcript UX
- UX and interaction cleanup on the OpenCode-first path before surface expansion
- server relay and discovery flows
- machine registry and presence
- session-scoped E2E encryption for user-generated in-transit payloads
- operational metadata storage boundaries enforced
- web client and relay-backed remote attachment path
- iOS and Android clients on the same daemon/protocol model

Exit criteria:

- the TypeScript CLI is the polished reference client for the local/runtime model
- web and mobile surfaces consume the same daemon/protocol semantics rather than introducing surface-specific runtime logic

## M6 - Multi-Provider Expansion and Adapter Generalization

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

## M7 - v1 Stabilization and Launch

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
