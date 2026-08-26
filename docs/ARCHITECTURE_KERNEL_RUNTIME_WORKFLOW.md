# Kernel Runtime and Workflow Architecture Details

Extracted from [ARCHITECTURE.md](ARCHITECTURE.md) to keep the main architecture overview below the line cap while preserving detailed kernel runtime and workflow responsibility notes.

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
- coarse worktree claim coordination belongs to the kernel boundary before parallel remote workflow scale-out; Workspace Live Sync managed mode now owns provider-session reads/writes, with remaining future coordination limited to port claims, optional integration checks, policy commands for unsafe mode, and post-v1 artifact-specific region models

Current M4.5 implementation status:

- `KernelCommand`, `KernelEvent`, `EventLog`, `SessionSnapshotProjection`, `CommandRouter`, bounded interactive routing, typed replay-gap handling, and command-id retry/fanout semantics have landed.
- WebSocket request admission is bounded before task spawn, and local IPC now normalizes through the same router path for compatibility requests.
- Provider launch now has a dedicated command executor seam for router admission/deferred runtime binding, while app-backed stores remain the current mutation mirror. Provider-run actors now own structured provider submit/cancel/poll execution and guard runtime slots with cleanup tombstones so in-flight provider I/O cannot resurrect cleared runtime state. Structured provider submit/abort/output-poll/selection-sync enqueue failures now propagate as daemon errors instead of being logged and swallowed, so prompt dispatch cleanup can run if the provider actor does not accept work. Structured output drained while another run is being pumped is retained per provider run for a later direct pump return, while terminal fanout still receives the output immediately.
- `KernelSessionService` now owns public session creation/default-agent bootstrap plus session attach, detach, config, alias, end, delete-by-ref, focus/cycle, and terminal resize behavior behind the current compatibility API, and the `SessionRuntime` executor calls that service directly instead of duplicating session mutation matches.
- `KernelAgentService` now owns prompt submit, kernel submit acknowledgement/dispatch preparation, cancel, runtime cancel, completion, queue advancement, cancellation finalization, and the shared agent request execution boundary behind the current compatibility API. Kernel prompt submit is split internally into admission validation, history append scheduling, prompt-owner mutation, and dispatch-effect preparation; prompt cancellation mirrors that shape with admission, remote/already-cancelling handling, local owner mutation, abort-dispatch preparation, notice, and projection steps; and prompt completion now follows the same pattern for remote leased completion, local owner completion, assistant-history/workflow/flow-control effects, queue advancement, and projection publication. These phase structs, phase methods, prompt lifecycle helpers, and the direct-submit compatibility wrapper are isolated in `app/kernel_agent/prompt_commands.rs`, so they can move to runtime-owned services without changing the mailbox API. `app/kernel_agent.rs` now stays focused on shared agent request orchestration. `DaemonApp::handle_agent_request` has been removed; local compatibility dispatch enters `KernelAgentService` directly, session-runtime agent spawn/destroy delegates to the same service boundary, and the remaining app-level prompt submit/complete/cancel methods are crate-private compatibility shims rather than public facade APIs.
- `AgentRuntime` now admits prompt submit/cancel through bounded per-agent mailboxes, and its mailbox executor uses a narrow `AgentPromptCommandService` for prompt mutation plus provider/remote dispatch side effects while the underlying stores are still being split out of `DaemonApp`. `SessionRuntime` now admits public session creation plus attach/detach/focus/cycle/resize/config/alias/end/delete through bounded mailboxes. Successful end/delete deregisters the session mailbox. Delete-by-ref and detach-by-attachment can resolve their target session from warmed projected session state before falling back to the compatibility app store. The two runtimes and router-side agent lifecycle refreshes also share a focused-agent projection, so warm untargeted prompt submit/cancel routing can resolve focus without synchronously reading the compatibility app store.
- `SessionStateProjectionStore` now serves warmed `GetSessionState`, `ListSessions`, and successful `ResolveSession` reads from a shared daemon projection. The router refreshes it from responses that already carry session/list snapshots, list responses hydrate per-session projection entries for follow-up state/resolve reads, and prompt lifecycle mutations publish prompt-state snapshots as prompts start, complete, cancel, fail dispatch, or advance from the queue.
- `PromptStateOwner` is now a cloneable kernel service shared with `AgentRuntime`, and it owns per-agent active prompts and queued prompt backlogs. Prompt submit, complete, cancel, cancellation finalization, dispatch-failure cleanup, detach cleanup, workflow prompt submission, and local/remote queue advancement mutate that owner first and then mirror the resulting agent state into `RuntimeSession`.
- Local provider prompt submit no longer acquires a whole-worktree provider-prompt claim. After owner mutation, PTY writes and provider actor enqueue work run through the spawned provider-run operation dispatch instead of inline in the per-agent mailbox response path. Kernel prompt submit also defers user-prompt history appends and remote relay prompt dispatch out of the acknowledgement path; remote relay failures cancel the active prompt, refresh projections, and publish a runtime notice after the acknowledged owner mutation.
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
- Warmed session projections now serve agent and workflow inspection reads, including `ListAgents`, `ListWorkflows`, `ResolveWorkflow`, `ListWorkflowRuns`, `GetWorkflowRun`, `ListWorkflowWatchdogs`, and `ListQueuedWorkflowPrompts`, without taking the compatibility app lock.
- Workflow runtime-tool calls now republish session and agent-runtime projections after recording tool-call state, so MCP/relay acknowledgements and output submissions keep warmed workflow inspection reads current even when they do not enter through the router workflow lane.
- `DaemonHealthProjection` now exposes session command lanes, agent command lanes, provider runtime operation lanes, session projection counts, active/queued prompt counts, provider-catalog cache status, kernel websocket transport pressure, workspace worktree-collision state, active workspace operation claims, and projection-invariant drift between warmed session prompt state and the agent-runtime prompt read model without taking the compatibility app lock.
- `WorkspaceCoordinator` now enforces scoped worktree claims for explicit file-writing capabilities (`EditFile` and `StoreTransferredFile`) and workflow node dispatch. Claims carry `read`/`write` mode metadata and are keyed by normalized real worktree where possible. Provider prompts are not workspace-wide claims, so independent sessions can run prompts in the same worktree; workflow nodes are scheduled work, so claim conflicts move them to `BlockedOnWorkspaceClaim` and they retry after claim release. Workspace Live Sync managed mode handles Chariox-managed provider-session file reads/writes through runtime/MCP tools, using fine-grained text coordination and opaque whole-file coordination for non-text artifacts.
- Relay-client daemon/workflow requests now normalize into `KernelCommandSource::RelayClient` commands and dispatch through `CommandRouter`, so proxied clients share actor admission, projection refresh, and overload behavior with local IPC/kernel transport instead of bypassing the runtime through `DaemonApp::handle_local_request`. Workflow validation and acknowledgement requests are no longer rejected by the relay transport before reaching the workflow lane.
- Runtime migration slices are gated by [IMPLEMENTATION_INVARIANTS.md](/Users/miguel/chariox/docs/ops/IMPLEMENTATION_INVARIANTS.md): ownership, projection refresh, cleanup, overload, health, and tests must be explicit for runtime and coordination work.
- `DaemonApp` now remains as bootstrap/composition scaffolding, not the command-state owner. Follow-up architecture work should keep removing stale compatibility wording and continue tightening projection correctness around provider output, workflow progression, and session lifecycle edges.

### 3.3.2 Workflow Model

The kernel should treat workflows as general directed graphs.

Each workflow definition contains:

- graph nodes that reference top-level agents
- edges that define allowed message flow
- one or more endpoints, each bound to an entry node
- per-agent instructions/system prompts and optional capabilities/repo scopes
- daemon-managed node instruction artifacts with stable references plus per-edge handoff schema constraints

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
- another internal Chariox component
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

### 3.3.5 Workflow Triggers and Deployments

A trigger routes input into the existing workflow in its existing session. A
deployment is the separate operation that captures an immutable workflow
revision and materializes it on another kernel. Neither an HTTP gateway nor an
event producer becomes a workflow authority.

Target concepts:

- `trigger`
  - an HTTP, schedule, or notification input attached to one workflow endpoint
    and one workflow queue
- `publication package`
  - an immutable, portable deployment directory containing
    `publication.json`, `workflow.snapshot.json`, `requirements.json`, app
    assets, packaged scripts, and a launcher
- `deployed workflow session`
  - a hidden, non-editable kernel-owned session materialized from a publication
    package on a destination kernel

Rules:

- adding, enabling, disabling, or exposing a trigger MUST NOT clone the source
  workflow, agents, queues, or session
- every source-kernel trigger invocation enters the same workflow queue system
  as manual input and is serialized by the source session arbiter
- deployed workflow sessions MUST NOT appear in normal local/web CLI session
  lists
- deployed workflow sessions MUST NOT be editable through ordinary workflow,
  session, or agent authoring commands
- the kernel remains the authority for source and deployed workflow
  sessions, workflow queues, workflow runs, provider runs, artifacts, and
  outputs
- the HTTP gateway owns HTTP transport handling,
  static/editable HTML/assets, request parsing, internal progress streaming, and
  response forwarding only
- one workflow may have multiple triggers; those triggers share the source
  workflow's agents and configured queue namespace
- output fanout and trace fanout are distinct publication surfaces. Outputs are
  the endpoint consumer result stream. Traces are an explicitly configured
  observability stream filtered before they leave the publication runtime.
- trace exposure is configured per workflow node. One node may expose only
  output summaries while another exposes assistant messages and tool use.
  Thinking traces are disabled unless a publication policy explicitly enables
  them for that node.
- the `human_http` transport owns a self-contained split viewer. The left side
  shows invocation status, partial outputs, and final output. The right side
  shows exposed traces tagged by the responsible node or agent alias.
- if a final output is a renderable HTML payload, the `human_http` viewer
  replaces the left output region with a sandboxed iframe containing the
  generated HTML while the trace pane remains visible in the parent viewer.
- building or deploying a workflow captures extension requirements but does not
  export secrets
- serving a source HTTP trigger or deploying a package MUST verify required providers, models,
  extensions, and credentials before accepting traffic
- if the captured provider/model is unavailable, `chariox serve` may prompt for
  a replacement provider/model from the kernel's available catalog and persist
  the choice in local publication bindings

V1 ingress:

- HTTP GET: browser prompt entry and generated output page
- HTTP POST: form and API invocation
- internal HTTP event streaming for viewer progress

Execution and deployment modes:

- local trigger: HTTP gateway bound to `127.0.0.1` by default; schedules and
  notifications require no HTTP listener
- public ingress to current kernel: a Chariox ingress forwards
  requests over an outbound publication connector to the user's local
  trigger. Workflow execution, provider credentials, provider
  processes, artifacts, queues, traces, and outputs remain local.
- hosted deployment: one Docker container per deployment runs an independent
  kernel, hidden deployed session, selected triggers, gateway when HTTP is
  selected, snapshot, requirements, scripts, and assets on a runner host

For v1 Cloud deployment, OpenShip-managed Chariox Cloud remains the control plane
only. It owns account auth, deployment records, runner registration, deployment
commands, status/log metadata, and the web UI. Runtime publication traffic MUST
NOT be proxied through the Chariox Cloud API/web process. A dedicated Chariox
publication ingress exposes public workflow URLs and routes to either a
local-runtime connector or a hosted publication container on an eligible
publication runner.

The public URL is represented as `public_base_url`. In staging this may be a
path under the publication ingress host; later product DNS may map the
same contract to `https://<slug>.chariox.run/`. Callers should not need to know
whether the backend is local-runtime ingress or a hosted container.

Agent Apps generalize workflow publication from "workflow returns HTML or data"
to "selected app routes are mediated by workflow endpoints." The concept covers
base app assets, route wrapping, workflow-produced response effects, overlays,
app actions, endpoint manipulation policy, replica pools, and external web/mobile
integration. See `docs/AGENT_APPS_CONCEPT.md`.

Cloud-hosted workflow deployments should behave as independent web apps. Callers
do not need Chariox accounts unless the owner configures Chariox-managed access.
Deployment records and runner/container tokens are scoped execution identities
and MUST NOT carry a general Chariox user account session.

Images and publication packages MUST NOT include provider credentials or Chariox
Cloud account credentials. Hosted-container validation may use an explicit
staging credential profile mounted by the runner, but product credential
onboarding for arbitrary users is a later phase after the deployment pipeline is
validated end to end with real providers.

The web CLI MUST keep one workflow identity and expose its operational surfaces
inside the workflow view: Design, Runs, Live, and Code. Creating or enabling a
local HTTP, schedule, or event trigger MUST NOT clone the workflow, its agents,
its queues, or its session. A separate session is created only when the workflow
is exported or deployed into another kernel.

For `human_http`, the preferred web CLI action is central-panel embedding:
selecting/opening a publication embeds the publication display URL in the main
terminal stage. Cloud does not independently render output or traces; the
embedded publication HTML owns the split viewer so the same surface works
locally, over relay display, over Cloud local-runtime ingress, and in hosted
container mode.

Detailed Cloud deployment implementation and validation are tracked in
`docs/M9_WORKFLOW_PUBLICATION_CLOUD_DEPLOYMENT_PLAN.md`.

### 3.3.6 Workflow/Agent Binding and Missing Agents

Workflow definitions are user-authored artifacts and should not silently mutate when workspace agents change.

Rules:

- creating a new agent MUST NOT automatically add that agent to existing workflows
- deleting an agent MUST NOT automatically delete workflow nodes or edges
- a workflow node whose referenced agent no longer exists should remain in the workflow and be marked missing/unavailable
- workflows with missing endpoint targets or required nodes should remain listable/editable but be considered invalid for execution until repaired

### 3.3.7 Observability and Debug Logging Baseline

Chariox should treat debug logging as a shared local-runtime subsystem rather than as ad hoc per-process stderr output.

Required baseline rules:

- There should be one machine-local Chariox log root per OS user account.
- The daemon should own discovery of that root and expose or propagate it to local Chariox-managed processes.
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

- one root such as `XDG_STATE_HOME/chariox/logs` or a daemon-configured equivalent
- per-process files grouped by date and process role
- session/provider-run correlation handled in record fields rather than by requiring one file per session

Current local baseline:

- `CHARIOX_LOG_DIR` overrides the log root when set
- otherwise Chariox resolves `CHARIOX_HOME/logs`, `XDG_STATE_HOME/chariox/logs`, `~/.local/state/chariox/logs`, then an operating-system temporary-directory fallback; it never creates logs in the workspace automatically
- the daemon, the Rust `chariox-cli` launcher, and the TypeScript CLI all write per-process NDJSON log files under that root
- the local Fastify server now uses the same root and record shape
- local inspection can happen either with standard tools (`tail`, `jq`) or through the built-in `chariox-cli logs` command

Default privacy posture:

- metadata, warnings, errors, lifecycle events, and structured diagnostics should be loggable by default
- prompt content, provider output, and other user-generated content should be treated as debug-only capture and should not be enabled silently
