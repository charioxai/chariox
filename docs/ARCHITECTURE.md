# Arroba v1 Architecture

## Status

Draft architecture aligned with `docs/spec-v1.md`.

## 1. Purpose

This document translates the v1 specification into an implementation-oriented architecture view:

- component boundaries
- runtime ownership
- trust and security boundaries
- state ownership and storage boundaries
- critical runtime flows

## 1.1 Terminology

Target architectural terms:

- `Arroba Kernel`
  - the authoritative orchestration/runtime kernel
- `arroba-daemon`
  - the process that hosts the kernel on one machine/user context
- `workspace`
  - the persistent collaboration domain
- `workflow`
  - a directed execution graph inside one workspace

Current implementation note:

- the current Rust code still uses `daemon` and `session` heavily
- the docs now use `kernel` and `workspace` as the target conceptual model
- unless stated otherwise, “workspace” in the docs maps to the current code’s `session`

## 2. System Topology

Arroba v1 is composed of five runtime components:

- Client
- Machine
- Arroba Kernel
- Relay Server
- Directory Service

High-level topology:

`Client <-> Arroba Kernel <-> Agent Endpoint`

Remote topology:

`Remote Client <-> Relay Server <-> Arroba Kernel <-> Agent Endpoint`

Discovery topology:

`Client | Kernel -> Directory Service`

Current implementation mapping:

- the kernel currently runs inside the Rust runtime in [apps/daemon](/Users/miguel/arroba/apps/daemon)
- the primary client is the TypeScript CLI in [apps/cli](/Users/miguel/arroba/apps/cli)
- the current OpenCode adapter talks to a local OpenCode HTTP + SSE endpoint
- the primary daemon-client transport is now a kernel WebSocket with pushed events; the local Unix-socket IPC surface remains for harnesses, compatibility, and local management paths
- relay, directory, and unified node transport are later implementation work, not current code
- the relay is planned as an independent app, separate from both daemon and CLI

## 2.1 Architectural Rules

- CLI is one client implementation, not the owner of business logic.
- The kernel owns workspace state, routing, workflow state, and coordination.
- Transport and discovery are separate concerns.
- Relay forwards traffic; it does not own rendezvous/discovery.
- Directory provides identity/discovery/reachability metadata; it does not own workspace state.
- Managed identity/discovery service, if introduced later, should remain outside this repository and consume the same relay/directory boundaries rather than becoming a dependency of core runtime code.
- New features should land in kernel/protocol layers first, not UI-specific code.

## 2.2 Connectivity Model

Arroba should model a kernel-hosted runtime domain that may contain both local and remote members.

Members of the same workspace may include:

- local terminals
- remote terminals attached through relay
- local agent endpoints
- remote agent endpoints attached through relay

Normative rules:

- locality is a transport property, not an authority property
- local and remote members attached to the same kernel belong to the same runtime domain
- the kernel remains the authority for workspaces, attachments, prompt queues, provider runs, workflow routing, and coordination regardless of member locality
- relay must not become the workspace or workflow authority

## 2.3 Workflow Rule

The purpose of a multi-agent workspace is to host workflows.

Normative rules:

- a workspace may contain multiple workflow definitions
- a workflow is a directed graph inside one workspace
- graph nodes are top-level Arroba-managed agents
- graph edges define allowed message flow
- each workflow definition may expose multiple endpoints
- each workflow endpoint targets exactly one entry node in that workflow
- disconnected subgraphs are allowed inside one workflow; a subgraph is only reachable if some endpoint targets an entry node within it
- the kernel owns workflow runs, routing, and turn activation

## 3. Component Responsibilities

### 3.1 Client

Responsibilities:

- render workspace state, transcript/output, and focused-agent state
- capture terminal input or structured actions and route them to the kernel
- render slash-command completion, help, warnings, and command results
- upload or reference artifacts
- remain attached or return to a no-workspace state without becoming a runtime authority

Current implementation note:

- the primary local client is the TypeScript OpenTUI app in [apps/cli](/Users/miguel/arroba/apps/cli)
- `arroba-cli` is currently a Rust launcher for that client
- the primary local transport is the kernel WebSocket event stream; local IPC remains as a lower-level compatibility and harness surface
- M4.5 session, agent, and workflow commands are routed through the kernel command router into bounded runtime lanes; local IPC, relay-proxied workflow requests, and actor workers delegate to explicit runtime request handlers while the legacy `DaemonApp` remains only as the current mutation mirror. Session runtime execution now reaches session lifecycle/config/alias mutations through `KernelSessionService` directly instead of broad app-level request helpers.

### 3.2 Machine

A machine hosts one kernel process per OS user context.

Responsibilities:

- provide execution environment for kernel, providers, artifacts, and worktrees
- host workspace runtime files
- later participate in registration and reachability metadata

Remote-machine note:

- a machine is a user-facing placement target
- users choose a machine when spawning a remote agent; they do not need to choose a kernel in the common path
- once spawned, a remote agent is bound to one selected worker kernel for its lifetime
- provider availability is advertised from the worker kernel back through relay metadata
- provider login remains local to the worker kernel; the home kernel consumes provider availability but does not proxy provider auth flows

### 3.3 Arroba Kernel

The Arroba Kernel is the runtime authority for live workspace state on one machine/user context.

Responsibilities:

- workspace lifecycle and attachment lifecycle
- workspace routing between all attached local and remote members
- workflow scheduling and workflow-run state ownership
- inter-agent message routing and structured handoff processing
- worktree allocation and isolation enforcement
- PTY lifecycle for provider runs
- provider switching and parked-run management
- capability execution
- extension/MCP/runtime ownership
- logging/root correlation metadata
- workspace coordination to reduce edit/integration conflicts across top-level agents in the same workspace

Remote-agent note:

- a home kernel remains the only session authority even when some agents execute on remote machines
- worker kernels host leased execution for those remote agents but do not become session authorities
- from the user point of view, a remote agent should behave the same way as a local agent after placement, with machine placement shown as metadata rather than as a separate runtime mode

### 3.3.1 Internal Kernel Subsystems

The kernel should be understood as containing several internal subsystems even when they are not yet split into separate processes.

Required subsystem roles:

- `WorkspaceRouter`
  - authoritative routing/fanout of prompt lifecycle, notices, provider output, and workflow handoffs
- `TransportGateway`
  - accepts local and remote terminal connections and normalizes them into kernel-owned attachments
  - current agent integrations remain adapter-owned and do not yet share this transport
- `AgentEndpointManager`
  - connects to managed or external agent endpoints and normalizes their native/provider-specific protocols into kernel events
- `WorkspaceCoordinator`
  - manages worktree/branch allocation, current coarse workspace claims, and later integration/merge safety checks inside one workspace

Current implementation note:

- the current codebase has pieces of these responsibilities inside the daemon crate, but not yet as a unified bidirectional node transport
- the current OpenCode adapter is already a provider-endpoint integration example
- the current local CLI transport is now a long-lived WebSocket subscription with pushed kernel events
- a generic WebSocket transport for agent endpoints is still intentionally deferred
- the current `WorkspaceCoordinator` is intentionally a coarse safety/scheduling boundary: file-writing capabilities, local provider prompts, and workflow node dispatch can acquire visible worktree claims, but final I/O conflict control for arbitrary harness writes, file-level claims, port claims, sandboxing, and transactional patch application is deferred until the actor/projection kernel is further along

### 3.3.1.1 Target Kernel Runtime Implementation Model

The target daemon implementation should be an actor/event/projection kernel, not a large shared mutable application object protected by one global lock.

Implementation rules:

- transports submit commands and subscribe to events/projections; transports do not directly mutate workspace state
- every command has a stable command id plus causation and correlation metadata
- workspace mutation is owned by actors or single-owner services with mailboxes, not by ad hoc lock acquisition from request handlers
- the event log is the source of truth for ordered runtime facts that matter to clients, recovery, replay, and remote relay resume
- projections are the primary read API for clients, session lists, transcript views, workflow state, provider/process state, and health snapshots
- provider runs, capability jobs, history scans, provider catalog discovery, and relay I/O run outside the hot interactive command lane with bounded queues and explicit backpressure
- prompt submit, cancel, focus, resize, attach, detach, and subscription resume stay on an `InteractiveCommandLane` and must not wait behind slow background work
- provider/runtime control operations remain owned by the `ControlService`; the interactive command lane is separate from the provider control lane

Target kernel services:

- `DaemonKernel`
  - composition root for command routing, actors, event log, projection store, transport gateways, and background executors
- `CommandRouter`
  - validates and routes `KernelCommand` values to the owning actor/service
- `EventLog`
  - append-only ordered stream of `KernelEvent` values with sequence ids, causation ids, and replay windows
- `ProjectionStore`
  - materialized read models for client boot, session views, transcripts, workflow inspection, provider process ledgers, and health snapshots
- `SessionActor`
  - owns one workspace/session lifecycle, attachments, focused agent, prompt queues, and session-scoped routing
- `AgentActor`
  - owns one top-level agent runtime lane, active/queued prompt state, provider run binding, and output fanout
- `ProviderRunActor`
  - owns one provider endpoint/run integration and normalizes provider-native events into kernel events
- `WorkflowRunActor`
  - owns one workflow run, mailbox delivery, turn activation, output buffering, failure state, and watchdog interaction
- `CapabilityExecutor`
  - owns bounded execution of file/tree/screenshot/shell/MCP capability jobs and reports progress through events
- `RelayRuntime`
  - owns remote transport sessions and relay registration without becoming workspace authority

`DaemonApp` should become a temporary compatibility facade during the migration and then disappear from hot paths. It can remain as a bootstrap/test adapter while command handlers move to `DaemonKernel`, but request handlers should not hold a global `Arc<Mutex<DaemonApp>>` across I/O, provider work, history scans, or client fanout.

Client UX may use optimistic local projections for immediate feedback, but the kernel remains the authority. When the kernel echo or session projection arrives, the client reconciles by command id, event id, target agent id, and prompt text instead of rendering duplicates.

M4.5 implementation contract:

- `docs/M4_5_KERNEL_RUNTIME_REFACTOR_PLAN.md` is the execution plan for the kernel refactor
- every mutating hot-path request should normalize to a `KernelCommand` before actor dispatch
- every runtime fact needed for client reconciliation, projection updates, replay, or relay resume should be represented as a `KernelEvent`
- the first event-log slice may be in-memory, but replay retention and replay-gap behavior must be explicit
- events should not be described as daemon-restart durable until persisted event streams or equivalent projection checkpoints exist
- projections must expose `projection_version`, `last_event_id`, and `generated_at_ms`
- coarse worktree claim coordination belongs to the kernel boundary before parallel remote workflow scale-out; file-level, port-level, sandbox, and transactional patch coordination belong to the later I/O-coordination design after actor/projection ownership is stable

Current M4.5 implementation status:

- `KernelCommand`, `KernelEvent`, `EventLog`, `SessionSnapshotProjection`, `CommandRouter`, bounded interactive routing, typed replay-gap handling, and command-id retry/fanout semantics have landed.
- WebSocket request admission is bounded before task spawn, and local IPC now normalizes through the same router path for compatibility requests.
- Provider-run actors now own structured provider submit/cancel/poll execution and guard runtime slots with cleanup tombstones so in-flight provider I/O cannot resurrect cleared runtime state. Structured provider submit/abort/output-poll/selection-sync enqueue failures now propagate as daemon errors instead of being logged and swallowed, so prompt dispatch cleanup can run if the provider actor does not accept work. Structured output drained while another run is being pumped is retained per provider run for a later direct pump return, while terminal fanout still receives the output immediately.
- `KernelSessionService` now owns session attach, detach, end, delete-by-ref, focus/cycle, and terminal resize behavior behind the current compatibility API.
- `KernelAgentService` now owns prompt submit, kernel submit acknowledgement/dispatch preparation, cancel, runtime cancel, completion, queue advancement, and cancellation finalization behind the current compatibility API.
- `AgentRuntime` now admits prompt submit/cancel through bounded per-agent mailboxes, and `SessionRuntime` now admits public session creation plus attach/detach/focus/cycle/resize/end/delete through bounded mailboxes. Successful end/delete deregisters the session mailbox. Delete-by-ref and detach-by-attachment can resolve their target session from warmed projected session state before falling back to the compatibility app store. The two runtimes and router-side agent lifecycle refreshes also share a focused-agent projection, so warm untargeted prompt submit/cancel routing can resolve focus without synchronously reading the compatibility app store.
- `SessionStateProjectionStore` now serves warmed `GetSessionState`, `ListSessions`, and successful `ResolveSession` reads from a shared daemon projection. The router refreshes it from responses that already carry session/list snapshots, list responses hydrate per-session projection entries for follow-up state/resolve reads, and prompt lifecycle mutations publish prompt-state snapshots as prompts start, complete, cancel, fail dispatch, or advance from the queue.
- `PromptStateOwner` is now a cloneable kernel service shared with `AgentRuntime`, and it owns per-agent active prompts and queued prompt backlogs. Prompt submit, complete, cancel, cancellation finalization, dispatch-failure cleanup, detach cleanup, workflow prompt submission, and local/remote queue advancement mutate that owner first and then mirror the resulting agent state into `RuntimeSession`.
- Local provider prompt submit keeps provider-prompt worktree claim admission synchronous, so conflicting cross-session same-worktree prompts still fail before acknowledgement. After claim admission and owner mutation, PTY writes and provider actor enqueue work run through the spawned provider-run operation dispatch instead of inline in the per-agent mailbox response path. Kernel prompt submit also defers user-prompt history appends and remote relay prompt dispatch out of the acknowledgement path; remote relay failures cancel the active prompt, refresh projections, and publish a runtime notice after the acknowledged owner mutation.
- `AgentRuntimeProjectionStore` now materializes per-agent active/queued prompt read models from the compatibility session mirror after owner mutations. Daemon health treats that agent-runtime projection as the canonical prompt-count read model and mirrors the counts into legacy session-projection health fields for compatibility.
- Compatibility session prompt state now lives in `PromptRuntimeState` in `session/prompt_runtime.rs` as a mirror/projection boundary. It maintains the legacy per-agent and flattened session-level active prompt, queued prompt, and scheduler fields for the existing wire shape, but it is no longer the hot prompt lifecycle authority.
- The agent-runtime projection now carries the front queued prompt per agent for warm routing previews, while `AgentRuntime` can consult `PromptStateOwner` directly for active-owner and queue-front decisions when the session projection is warm. Stale projections and stale session mirrors cannot force prompt lifecycle decisions.
- Detach now republishes session and agent-runtime projections after removing active or queued prompts, keeping projected queue-front state aligned for follow-up advancement.
- Provider idle settlement now checks the prompt owner for per-agent active prompt state when deciding whether to complete, finalize cancellation, or clear activity.
- `AgentRuntime` now carries the shared agent-runtime projection store into per-agent mailbox workers and publishes submit/cancel/complete prompt state from the mailbox execution path. The temporary private prompt-state shadow has been removed; `AgentRuntimeProjectionStore` is the single warm prompt-state read model while `PromptStateOwner` is the mutation authority and compatibility session state remains the mirror.
- Kernel `CompletePrompt` requests now route through the same per-agent mailbox, resolve the active prompt owner from warmed session projection before falling back to compatibility session state, and publish completion state into the agent-runtime projection from the mailbox worker.
- `CompletePrompt` owner routing now consults the agent-runtime active-prompt projection before session snapshots, so a stale session projection does not force completion routing back through the compatibility app lock.
- Kernel `CancelActivePrompt` now shares the same agent-runtime active-owner projection resolver before entering the per-agent mailbox, while mailbox execution still validates attachment/session ownership and active prompt state.
- Router response refresh now skips the duplicate agent-runtime projection update for `PromptSubmitted`; the per-agent mailbox worker owns that projection publication while the router still mirrors the returned session into the compatibility session projection.
- `AgentRuntime` now falls back to the warmed session projection for focused-agent lookup before touching the compatibility app store, so normal state warm-up can keep untargeted prompt routing off the hot app lock even when the dedicated focus projection is cold.
- Prompt complete and cancel paths now rely on prompt lifecycle projection publication instead of a redundant router-side session snapshot after the response. Warmed `GetSessionState` reads can observe completed or cancelling prompt state from projection without the router reacquiring the compatibility app store.
- Runtime notice polling and terminal resize no longer trigger router-side session projection snapshots because they do not mutate projected session state. Terminal output pumping still refreshes because provider output polling can settle prompt lifecycle state.
- Session end/delete clears terminal stream input, output, notice, and completion buffers from the `SessionRuntime` boundary, so terminal health cannot retain stale backlog for removed sessions.
- `SessionHistoryProjectionStore` now serves warmed `GetSessionHistory` transcript pages. A warmed session snapshot lets the router load disk history without taking the compatibility app lock, and repeated warmed reads return from memory while successful history appends keep the projection current.
- `ProviderRunProjectionStore` now serves warmed `GetProviderRun` reads. The router refreshes it from provider-run responses, and the daemon updates the shared projection as provider runs start, finish launch, fail launch, park, resume, or end.
- `ProviderProcessProjectionStore` now serves warmed `ListProviderProcesses` reads. Provider-run and session lifecycle changes invalidate it so teardown-safety metadata is recomputed before the next warmed reuse.
- `ProviderCatalogProjectionStore` now serves TTL-bound warmed `GetProviderCatalog` reads. Provider logout and relay/provider configuration changes invalidate the projection.
- Warmed session projections now serve agent and workflow inspection reads, including `ListAgents`, `ListWorkflows`, `ResolveWorkflow`, `ListWorkflowRuns`, `GetWorkflowRun`, `ListWorkflowWatchdogs`, and `ListQueuedWorkflowLaunches`, without taking the compatibility app lock.
- Workflow runtime-tool calls now republish session and agent-runtime projections after recording tool-call state, so MCP/relay acknowledgements and output submissions keep warmed workflow inspection reads current even when they do not enter through the router workflow lane.
- `DaemonHealthProjection` now exposes session command lanes, agent command lanes, provider runtime operation lanes, session projection counts, active/queued prompt counts, provider-catalog cache status, kernel websocket transport pressure, workspace worktree-collision state, active workspace operation claims, and projection-invariant drift between warmed session prompt state and the agent-runtime prompt read model without taking the compatibility app lock.
- `WorkspaceCoordinator` now enforces scoped worktree claims for explicit file-writing capabilities (`EditFile` and `StoreTransferredFile`), active local provider prompt lifecycles, and workflow node dispatch. Claims carry `read`/`write` mode metadata and are keyed by normalized real worktree where possible. Provider prompts fail fast on cross-session same-worktree conflicts; workflow nodes are scheduled work, so claim conflicts move them to `BlockedOnWorkspaceClaim` and they retry after claim release. File-level scopes, port claims, sandbox enforcement, and transactional patch application are deferred to the final I/O-coordination design.
- Relay-client daemon/workflow requests now normalize into `KernelCommandSource::RelayClient` commands and dispatch through `CommandRouter`, so proxied clients share actor admission, projection refresh, and overload behavior with local IPC/kernel transport instead of bypassing the runtime through `DaemonApp::handle_local_request`. Workflow validation and acknowledgement requests are no longer rejected by the relay transport before reaching the workflow lane.
- Runtime migration slices are gated by [IMPLEMENTATION_INVARIANTS.md](/Users/miguel/arroba/docs/ops/IMPLEMENTATION_INVARIANTS.md): ownership, projection refresh, cleanup, overload, health, and tests must be explicit before final I/O coordination starts.
- `DaemonApp` still remains the compatibility facade for many paths. The next major implementation step is to move session/workflow state into mailbox-owned stores, expand actor-owned projections beyond focused-agent routing and warmed session/list/history/provider-run/process/prompt-state/provider-catalog snapshots, and continue removing hot-path `Arc<Mutex<DaemonApp>>` dependencies before final I/O coordination.

### 3.3.2 Workflow Model

The kernel should treat workflows as general directed graphs.

Each workflow definition contains:

- graph nodes that reference top-level agents
- edges that define allowed message flow
- one or more endpoints, each bound to an entry node
- per-agent instructions/system prompts and optional capabilities/repo scopes
- daemon-managed node instruction artifacts with stable references plus per-edge output schema constraints

Each workflow run contains:

- the workflow endpoint that started the run
- one inbound queue per agent
- one active turn at most per agent
- kernel-owned routing and delivery state
- zero or more run outputs emitted by output-producing nodes

### 3.3.3 Workflow Messaging and Turns

Workflows should use a minimal, general message model.

Logical message shape:

- `message`
- `recipients`
- `artifacts`

Rules:

- artifacts are intentionally open-ended and may include text, JSON, files, paths, URLs, images, or lists of those
- the kernel should not impose workflow-specific semantic fields by default
- at most one message per recipient may be emitted by a sender in a single turn

Turn/input rules:

- each agent has an inbound queue within a workflow run
- when an agent is idle and eligible, the kernel may start a turn
- at turn start, the agent must use a kernel-owned input-consumption tool to fetch and consume the current queued inputs for that turn
- once a turn has started, that turn cannot inspect newly arriving queue items; new arrivals remain queued for a later turn
- the kernel should support both:
  - `sync` delivery mode: validated outputs are delivered when the turn ends
  - `async` delivery mode: validated outputs are delivered as soon as they are produced during the turn

Required kernel tools:

- `consume_input_messages`
- `validate_output_messages`

Validation policy:

- invalid output payloads SHOULD follow daemon-owned policy (warn-and-continue vs halt-run), with per-edge overrides allowed
- warnings SHOULD be visible to the emitting node and any downstream recipients

### 3.3.4 Workflow Endpoints and Run Outputs

Workflow entry should not be limited to a human prompt from a terminal, and a workflow may expose more than one entry surface.

The kernel should support logical `workflow endpoints` that can be invoked by:

- a terminal user in the workspace
- another internal Arroba component
- an external system through a published API surface

Rules:

- each endpoint maps to one entry node in its workflow
- multiple endpoints may target the same entry node
- the workflow itself should remain agnostic to whether the initial input came from a terminal or an external system
- workflow invocations should fail fast when endpoints, nodes, or agent bindings are invalid

Workflow output should start as a run-level concept rather than a strict graph object.

Rules:

- a workflow run may emit zero or more outputs
- output-producing nodes may be the same nodes used as workflow entry targets
- explicit first-class output endpoints may be added later if publishing/integration needs them

### 3.3.5 Workflow/Agent Binding and Missing Agents

Workflow definitions are user-authored artifacts and should not silently mutate when workspace agents change.

Rules:

- creating a new agent MUST NOT automatically add that agent to existing workflows
- deleting an agent MUST NOT automatically delete workflow nodes or edges
- a workflow node whose referenced agent no longer exists should remain in the workflow and be marked missing/unavailable
- workflows with missing endpoint targets or required nodes should remain listable/editable but be considered invalid for execution until repaired

### 3.3.6 Observability and Debug Logging Baseline

Arroba should treat debug logging as a shared local-runtime subsystem rather than as ad hoc per-process stderr output.

Required baseline rules:

- There should be one machine-local Arroba log root per OS user account.
- The daemon should own discovery of that root and expose or propagate it to local Arroba-managed processes.
- Each process should write its own append-only structured log file under that shared root instead of multiple processes appending to one shared file directly.
- Structured log records should include enough correlation metadata to reconstruct one session/provider-run/client flow across processes.

Minimum correlation fields:

- timestamp
- level
- component
- process kind
- pid
- session id when known
- provider run id when known
- attachment id or client id when known
- request id or trace id when known

Recommended layout:

- one root such as `XDG_STATE_HOME/arroba/logs` or a daemon-configured equivalent
- per-process files grouped by date and process role
- session/provider-run correlation handled in record fields rather than by requiring one file per session

Current local baseline:

- `ARROBA_LOG_DIR` overrides the log root when set
- otherwise Arroba resolves `XDG_STATE_HOME/arroba/logs`, then `~/.local/state/arroba/logs`, then a local `./.arroba/logs` fallback
- the daemon, the Rust `arroba-cli` launcher, and the TypeScript CLI all write per-process NDJSON log files under that root
- the local Fastify server now uses the same root and record shape
- local inspection can happen either with standard tools (`tail`, `jq`) or through the built-in `arroba-cli logs` command

Default privacy posture:

- metadata, warnings, errors, lifecycle events, and structured diagnostics should be loggable by default
- prompt content, provider output, and other user-generated content should be treated as debug-only capture and should not be enabled silently

### 3.4 Relay Server

The relay server is a lightweight transport-forwarding layer.

Responsibilities:

- websocket relay
- daemon connection registry and liveness
- client-to-daemon request/response/event forwarding
- minimal routing metadata needed to target a connected daemon

Current architectural interpretation:

- the relay should forward transport, not own discovery/rendezvous
- local and remote connections should ideally share one daemon-owned application protocol even if they arrive through different physical paths
- the relay must not become the workspace or workflow authority
- the relay should be implemented as an independent Rust app
- daemon connections should be outbound from daemon to relay so the model works cleanly through NAT/firewall boundaries
- one daemon should use one active relay connection at a time in v1, even if multiple relay endpoints can be configured
- self-hosted relay mode must work without any external managed identity/discovery service
- all user-generated payloads that cross relay boundaries must be session-scoped end-to-end encrypted, including prompts, workflow payloads, and transferred artifacts
- this encryption requirement applies equally to self-hosted relay deployments; self-hosting does not relax the transport privacy model
- the same CLI should support local direct daemon operation and relay-mediated remote operation without becoming two apps
- the CLI should always open the waiting room first; local sessions remain available even when relay is not configured or disconnected
- relay connection is configured from slash commands or the waiting-room relay section, then auto-connects in the background
- `/relay use <ws-url>` may read the token from `ARROBA_RELAY_TOKEN`; passing the token as a visible slash-command argument remains supported for self-hosted/manual testing but should not be the preferred documented path for shared terminals or screenshots
- the waiting room groups relay status and relay actions together under `Relay`; it also groups machines and pending machine counts together under `Machines`
- once relay connects, machine/provider availability updates automatically; if the user is already in a session, remote capability can become available silently with at most a small informational footer
- the home kernel maintains local machine trust state: live unknown machines are pending, `/machine approve` makes them spawn targets, `/machine rename` stores a user alias, and `/machine forget` hides them from normal machine/provider availability
- relay kernel display names are relay-scoped live labels, not durable user aliases: each registered kernel reports its OS name and kernel start time, and the relay exposes addressable names such as `machine 1 (macOS)` for discovery and routing
- relay machine lists remain grouped by stable machine identity; when several kernels are online from the same machine, `/machine kernels <machine-ref>` shows each addressable relay kernel alias separately
- user-facing stable names still come from local home-kernel rename/approval state; relay aliases are plain live metadata and do not become relay-owned user preferences
- relay-visible machine metadata remains plain routing/liveness metadata; trust decisions and aliases are local home-kernel state, not relay authority


### Docker Remote-Machine Lab

The Docker lab models containers as ordinary Arroba machines. The base image should include every Arroba app: CLI, daemon/kernel, and relay. It should not include provider credentials or manage provider login. Users install and authenticate provider CLIs inside each persistent container. This intentionally supports multiple concurrent accounts for the same provider by isolating each container's home directory, Arroba identity, provider config, and provider credentials.

Required container properties:

- outbound internet for provider APIs, package installation, hosted relay access, and auth flows
- persistent `/home/arroba` or equivalent so machine identity and provider credentials survive restart
- separate machine identity per container, derived from persisted config rather than baked into the image
- optional URL-printing browser/`xdg-open` shims for provider login flows that request a browser
- documented provider compatibility for the small launch-provider set, including login method, callback-port behavior, and tested CLI versions

Normal provider runtime ports do not need host mapping when the provider process and worker kernel run in the same container. Login callback ports are provider-specific and must be tested/documented per provider.

### 3.5 Directory Service

The directory service is a later, intentionally simple control-plane component for:

- identity registration
- discovery
- reachability metadata
- rendezvous/bootstrap information

It is distinct from relay:

- directory answers where/how a kernel or published endpoint can be reached
- relay forwards traffic after that decision is made
- a later managed service may provide identity/discovery on top of these boundaries, but that service remains outside this repository

### 3.6 Agent Endpoints

An agent endpoint is the kernel-facing runtime interface implemented by a provider integration or Arroba-native agent runtime.

Required endpoint modes:

- `managed`
  - kernel launches the endpoint/runtime itself
- `external`
  - the endpoint already exists and the kernel discovers/configures/connects to it

Normative rules:

- the kernel should depend on an endpoint contract, not only on child-process ownership
- existing providers like OpenCode may keep native transport adapters
- Arroba-native or third-party agent runtimes should eventually target a canonical daemon-facing agent protocol directly
- transport unification should happen at the kernel protocol/event model level, not by forcing every provider to mimic the same wire transport internally

### 3.7 Workspace Coordination

If Arroba is to orchestrate multiple top-level agents without relying on a human to manually clean up merge conflicts, workspace coordination must be kernel-owned.

Baseline responsibilities:

- allocate worktrees and branches per top-level agent
- record edit intent or claim information at least at workspace/file granularity
- prevent or warn about obviously conflicting edits
- run integration and mergeability checks before changes are combined

Near-term practical rule:

- keep current worktree-level coordination as the near-term guardrail while the kernel refactor completes
- defer file-level, port-level, sandbox, transactional patch, and advanced region-level locking decisions to the final I/O-coordination design
- the kernel should own integration policy rather than delegating all conflict discovery to late Git merges or human PR review

Scope rule:

- coordination is workspace-scoped, not machine-wide or repo-wide across all workspaces
- different workspaces may still collide at integration time in the same way independent PRs can conflict

Non-responsibility:

- should not require plaintext access to user-generated session content

## 4. Runtime Ownership and State Authority

### 4.1 Authority Model

- **Daemon**: source of truth for active runtime state
- **Server**: source of truth for shared operational metadata
- **Provider process**: source of truth for provider-native behavior

### 4.2 Session Ownership

A session is bound to:

- one workspace
- one primary worktree in single-agent mode
- one active provider run in single-agent mode
- an eligible set of machines with one active host at a time

A session may include:

- many top-level Arroba-managed agents
- many client attachments
- parked provider runs
- agent-scoped provider runs when multi-agent session mode or workflow mode is active
- agent-scoped history/runtime metadata and worktree assignments when multi-agent session mode or workflow mode is active
- prompt queue state and canonical session config state
- a workflow definition and zero or one active workflow run
- node-scoped provider runs when workflow mode is active
- worktree assignments for isolated workflow branches
- schedules
- artifacts
- extension bindings resolved for top-level provider runs

### 4.2.1 Shared Attachment and Queue Ownership

In single-agent mode, attachments are shared session participants rather than exclusive control roles.

Required daemon-owned responsibilities:

- serializing prompt execution per session
- maintaining explicit scheduler state boundaries (`idle`, `runnable`, `running`, `waiting`) for queued work
- maintaining canonical queued-prompt state
- exposing canonical session state and runtime notices to attachments
- applying accepted config changes to canonical session config state
- rejecting unsafe config changes while a prompt is running
- notifying all other attachments when a prompt is queued
- propagating canonical config updates to all attachments after acceptance

Current M1 runtime note:

- the daemon now keeps explicit primary worktree assignment metadata for each session even in single-agent mode so later branch/worktree isolation can extend the same runtime shape

The daemon MUST treat prompt scheduling and config state as structured runtime state, not terminal-local behavior.

### 4.2.2 Multi-Agent Session Ownership

Manual multi-agent session behavior is still daemon-owned runtime behavior, not just client chrome.

Required daemon-owned responsibilities:

- maintain the canonical top-level agent list for each session
- maintain focused-agent state for direct user interaction
- route prompt submission to the selected agent's runtime context
- keep agent-scoped provider-run, history, and worktree-assignment metadata authoritative in daemon state
- expose enough agent-scoped state for pane-based clients to render one visible sub-area per active agent

Current implementation note:

- the local runtime already has session agent records, focused-agent metadata, and Arroba-owned `/agent ...` management commands
- the current CLI footer/chrome reflects that state, but transcript routing and provider execution are still effectively single-agent
- the next implementation step is to make focused-agent changes affect both prompt routing and visible per-agent panes/history

## Workflow Console

Kernel components now include a workflow-scoped shared console service.

Responsibilities:

- one append-only console per workflow definition
- shared human-facing output stream separate from provider traces
- readable/writable/clearable by workflow nodes through Arroba MCP tools
- rendered by the CLI in the workflow right-side panel via `/workflow terminal`

Ownership split:

- transport exposes MCP tools and authenticates/scopes calls
- scheduler/runtime owns workflow-console state and semantics
- CLI renders the live console stream without rewriting content

Boundary:

- the workflow console is not mailbox state
- the workflow console is not handoff state
- the workflow console is not audit state
- it is a shared presentation/output surface for one workflow

### 4.2.3 Persistent Session and Deletion Ownership

Arroba session lifetime should be explicit and daemon-owned.

Required rules:

- the daemon MUST treat detach and delete as distinct operations
- detaching the last client MUST NOT delete the session by default
- idle sessions SHOULD remain discoverable and reattachable until explicit deletion
- deleting a session MUST:
  - terminate or clear active provider/runtime state
  - remove the session from the daemon registry
  - invalidate further attach attempts
  - notify attached clients that the session no longer exists
- attached clients SHOULD transition to an unattached "no session" state when their current session is deleted, rather than being forced to terminate the whole client process

Planned client behavior:

- `/exit` detaches from the current session
- explicit session deletion is handled through a dedicated session-management command or external control command
- when the currently attached session is deleted, the client clears transcript/session chrome, renders an Arroba ASCII-art landing state, and returns to a reusable unattached shell state

Current local baseline:

- the TypeScript CLI now supports an unattached no-session state after explicit session deletion
- temporary session-management commands exist ahead of the general slash-command system:
  - `/session create [alias]`
  - `/session attach <ref>`
  - `/session delete [ref]`

### 4.3 Workflow Ownership

When a session runs in multi-agent workflow mode, the daemon MUST treat the workflow as a generic directed graph execution problem.

Required runtime concepts:

- `WorkflowDefinition`
- `WorkflowNode`
- `WorkflowEdge`
- `WorkflowRun`
- `NodeRun`
- `NodeMessage`
- `WorktreeAssignment`
- `AggregationState` or equivalent barrier/fan-in state

Required rules:

- Execution policy MUST be derived from the graph the user created, not from a separate user-declared topology flag.
- Nodes with indegree `<= 1` are serial with respect to input gating by default.
- Nodes with indegree `> 1` require explicit barrier/fan-in handling.
- Nodes with outdegree `> 1` are branching points and may release outputs to multiple children.
- Cycles are a separate graph property and MUST be handled independently from input/output synchronization policy.
- The runtime SHOULD support per-node execution policy rather than a workflow-wide sync/async switch.

Implementation priority note:

- graph-derived serial execution is the earlier implementation target
- graph-derived barrier/fan-in and bounded-cycle handling should follow on top of the same generic workflow engine

### 4.4 Inter-Agent Communication Ownership

Inter-agent communication MUST be daemon-orchestrated.

Required rules:

- Agents MUST NOT communicate directly.
- The daemon MUST own all routing and delivery between workflow nodes.
- Output from one node MUST be transformed into a standardized structured handoff payload before it is delivered to the next node.
- Inter-agent communication MUST NOT be modeled as raw terminal transcript forwarding.
- Workflow scheduling MUST advance from structured node completion reports, not arbitrary provider turns.

Each node completion artifact/report MUST include at least:

- `status`
- `summary`
- optional explicit `output`
- optional `artifacts` or changed files
- `stop_recommendation`

Rules:

- `summary` is human-facing and audit-oriented; it is not the downstream workflow payload by default
- downstream workflow delivery should use explicit output messages plus optional artifact refs
- transcript history remains audit state, not workflow output

### 4.5 Worktree Isolation

Parallel code-writing branches MUST NOT share the same active worktree.

Required rules:

- Worktree assignment MUST be explicit in runtime state and the data model.
- In hierarchical workflows, each active code-writing branch or subtree SHOULD receive an isolated worktree and git branch.
- The daemon MUST reject or prevent concurrent mutation of the same active worktree by parallel code-writing nodes.

## 5. Interaction Lanes

### 5.1 Terminal Lane

- Carries raw PTY output and user keystrokes.
- Must preserve provider-native semantics.
- Must not be transformed into structured command traffic by default.
- Must not be used as the source of truth for prompt queue ordering or session config state.
- For providers with stable structured local protocols, daemon-rendered output derived from provider events is acceptable and preferred over PTY-idle heuristics.

### 5.2 Capability Lane

- Carries daemon capability requests/results.
- Used for shell, file ops, screenshot, git/worktree, schedules, transfers, and other Arroba-owned slash commands.

### 5.3 Control Lane

Structured daemon-to-provider adapter control boundary.

Canonical control operations in v1:

- `attach_file`
- `request_memory_update`
- `request_compaction_summary`

`request_memory_update` and `request_compaction_summary` are daemon-owned and distinct from normal user prompt/response traffic.

`/<provider> ...` commands are resolved by Arroba first, then dispatched into adapter-owned behavior through the control lane or adapter-specific execution hooks.

Provider authentication is not part of the control lane in v1; adapters probe and report auth state, but login itself remains a provider-native local CLI flow on the host machine.

Provider-facing extension projection is also adapter-owned: the daemon resolves the authoritative extension bindings, and the adapter materializes the provider-specific runtime view.

### 5.3.1 OpenCode Structured Adapter

OpenCode is the first provider where Arroba intentionally prefers a structured local provider protocol over PTY-only inference.

Target runtime flow:

- daemon launches `opencode serve` in the assigned worktree or workspace context
- daemon waits for the local OpenCode server health endpoint
- daemon creates or binds an OpenCode session for the Arroba provider run
- daemon submits prompts through the OpenCode session API
- daemon subscribes to the OpenCode SSE event stream
- daemon maps OpenCode session/message events into Arroba prompt lifecycle, notices, and client-facing output

Target signal mapping:

- prompt submit: OpenCode session prompt API
- `/<provider> ...`: OpenCode command list plus session command API
- turn abort: OpenCode session abort API
- turn busy/idle: OpenCode session status events
- incremental text: OpenCode message-part delta and part-update events
- assistant completion: OpenCode assistant message updates with completion timestamps
- provider errors: OpenCode session error events plus adapter protocol errors when the event/session transport itself fails

Implication:

- PTY process exit remains a provider-run liveness signal
- PTY idleness is not the completion signal for OpenCode once the structured adapter path exists

### 5.4 Workflow Lane Semantics

Workflow scheduling and node-to-node handoffs belong to the daemon's structured state/control surfaces, not the terminal lane.

Implications:

- node handoffs MUST use structured daemon-owned payloads
- node completion reports MUST be machine-parseable
- workflow barriers, fan-in, aggregation, and termination decisions MUST operate on structured runtime state
- PTY traffic MAY be observed by the daemon for runtime coordination when needed, but MUST NOT be reused as the inter-agent contract

## 6. Security and Trust Boundaries

### 6.1 E2E Encryption Scope

User-generated content in transit must use session-scoped end-to-end encryption when crossing remote transport boundaries, including:

- terminal-entered prompts/content
- capability payloads (edit instructions, prompt templates)
- uploaded file payloads
- memory transfer package payloads
- compaction summary payloads

### 6.2 Relay Trust Model

Server acts as relay/registry and should not require content plaintext to perform core duties.

### 6.3 Session Key Isolation

Cryptographic context is session-scoped. A key compromise in one session must not imply compromise of other sessions.

## 7. Memory Architecture

## 7.1 Dual Memory Model

Arroba memory has two scopes:

- short-term memory: recent transcript/task continuity
- long-term memory: durable user/project guidance

## 7.2 Memory Update Mechanism

Daemon may call `request_memory_update` to refresh memory-relevant signals after provider compaction/reset or before transfer.
Daemon may call `request_compaction_summary` during user-triggered Arroba compaction before starting a fresh warmed run.

Fallback:

- if unsupported, daemon continues with Arroba-managed memory sources
- memory refresh failure must not terminate provider run

## 7.3 Context Transfer Package

Transfer package composes:

- selected short-term snapshot
- relevant long-term entries
- workspace state

Requirements:

- deterministic and auditable composition
- user control over long-term entry inclusion
- encrypted in transit

## 7.4 Extension Architecture

Arroba manages extensions in two phases:

- install: register an extension on the machine
- bind: make that extension available to a top-level Arroba-managed agent or provider run

The daemon owns:

- extension installation metadata
- compatibility and validation checks
- per-agent binding resolution
- provider-view materialization inputs
- MCP runtime lifecycle for bound MCP servers

Provider-native subagents are not separate extension targets; they inherit whatever their parent top-level provider run can access.

### 7.5 Arroba-Driven Context Compaction

Arroba provides a user-triggered compaction command: `/compact`.

Compaction sequence:

1. daemon requests compaction summary via `request_compaction_summary`
2. daemon stores summary as a session artifact/memory input
3. daemon launches fresh provider run with empty context window
4. daemon warms new run with compaction summary + selected Arroba memory/workspace context

This flow is daemon-orchestrated and separate from ordinary user prompt traffic.

## 8. Failure and Degradation

Mandatory behavior:

- adapter lacking `attach_file`, `request_memory_update`, and/or `request_compaction_summary` does not break core PTY usage
- control operation failures are isolated and user-visible
- remote client disconnect does not terminate session by default
- workflow node failure propagation and retry policy MUST remain daemon-owned and explicit
- workflow concurrency/resource limits MUST be centrally enforced by the daemon runtime
- unsupported provider versions emit compatibility warnings but retain best-effort `/<provider> ...` completions
- provider-auth failures are surfaced as structured local host warnings and MUST NOT cause Arroba to take ownership of provider credentials
- relay-mediated remote attachment must not change daemon authority over sessions, provider runs, or workflow state

## 9. Deployment and Evolution Notes

v1 is local-first and single-active-host-per-session. Architecture should remain forward-compatible with:

- relay-backed remote terminal/client attachment
- daemon identity and machine identity for remote registration
- richer multi-machine scheduling/migration
- expanded provider adapter capabilities
- optional content persistence policies
- more advanced workflow topologies
- bounded loops and other cycle policies
- multi-user and team workflows
- richer aggregation policies and barrier behavior
- explicit merge or reconciliation stages
- per-node provider, model, and account selection

## 10. Implementation Choices (v1 baseline)

Contributor workflow conventions (coding style, testing, PR hygiene) are documented in `docs/CONTRIBUTING.md`.

This section captures current implementation choices for v1 so engineering work has a stable baseline. These are implementation defaults, not product invariants, and may evolve with explicit architecture updates.

### 10.1 Monorepo and Package Management

- monorepo layout
- pnpm workspaces

### 10.2 Client Stack

- React
- TypeScript
- xterm.js for terminal rendering

### 10.3 Daemon Stack

- Rust (required for v1 daemon implementation baseline)

### 10.4 Backend Stack

- Fastify

### 10.4.1 Relay Stack

- Rust for the relay implementation baseline
- independent app/process, separate from both daemon and CLI
- shared protocol/domain model should be reused where practical, while keeping relay transport-only

### 10.5 Data Layer

- Prisma
- SQLite for early/local phases
- Postgres as scale-up target

### 10.6 Transport and Local IPC

- WebSockets for kernel-facing client transport
- relay transport later forwards the same logical client/kernel protocol
- Unix socket on Unix-like systems remains as a local compatibility and harness path
- named pipe on Windows remains a later local compatibility follow-up

Current local runtime note:

- the daemon now hosts a kernel WebSocket listener directly and the TypeScript CLI uses that path by default
- the Unix-socket local transport remains implemented for local harnessing/tests and backward-compatible tooling
- Windows local compatibility transport remains a later follow-up
- kernel-client transport hardening now includes event ids, resumable subscribe, heartbeat events, and reconnect-friendly client behavior

### 10.6.1 OpenCode Integration Strategy

- M2 baseline: PTY-launched OpenCode wrapper path
- current M3 direction: daemon-launched `opencode serve` plus local HTTP/SSE adapter
- current implementation also supports an external OpenCode endpoint via `ARROBA_OPENCODE_ENDPOINT`, which is the first concrete `external agent endpoint` path in the codebase
- adapter-owned OpenCode session/event handling should remain behind daemon/provider abstractions so later providers can still use PTY or their own structured surfaces without changing client contracts
- OpenCode remains the only agent-side structured transport that Arroba is currently tightening closely against; a generic agent WebSocket protocol is intentionally deferred until more agent integrations exist

### 10.7 Governance

Implementation choices should be revised when they materially change runtime architecture, protocol assumptions, security posture, or operational behavior.

### 10.8 Cross-Platform Terminal Consistency Strategy

Arroba should use a shared terminal behavior contract while allowing platform-native implementation languages.

Approach:

- define canonical terminal behavior in protocol/conformance terms (PTY byte stream handling, resize semantics, key mapping expectations, control-sequence fidelity)
- use xterm.js as the web/remote reference implementation and golden-behavior baseline
- keep slash-command parsing, completion semantics, and warning behavior consistent across clients

Platform framework options for xterm.js-consistent rendering:

- Web: browser-hosted xterm.js
- iOS: `WKWebView` hosting xterm.js plus native shell for platform integration
- Android: `android.webkit.WebView` hosting xterm.js plus native shell for platform integration
- macOS desktop: `WKWebView` (AppKit/SwiftUI host) with xterm.js
- Windows desktop: WebView2 (`Microsoft.Web.WebView2`) with xterm.js
- Linux desktop: embedded Chromium/WebKit host (for example Electron or GTK WebKit) with xterm.js
- CLI/TUI clients: native terminal stack is allowed, but must pass the same conformance profile for input/output/resize semantics

Result:

- consistent remote terminal behavior across platforms
- freedom to use standard language/tooling per target platform
