# Arroba v1 Protocol

## Status

Draft protocol aligned with `docs/spec-v1.md`.

## 1. Scope

This document defines message classes and protocol contracts between:

- clients
- server relay
- kernel
- provider adapters

It is intentionally transport-agnostic at the message level.

Current implementation baseline:

- local daemon-client communication now defaults to a daemon-owned WebSocket transport with pushed events
- the older Unix-socket request/response IPC path still exists for harnessing/tests and compatibility
- daemon-OpenCode communication uses native local HTTP control plus SSE events

Target direction:

- one kernel-owned bidirectional transport for terminal clients
- one transport shape for both local and remote terminal members, with relay as a forwarding layer rather than a second authority
- relay is an external member that speaks the same transport contract, not a second kernel
- generic agent-facing transport remains deferred; current agent integrations continue to use native/provider-specific adapter protocols
- WebSocket is the current and recommended transport for the kernel-client path

## 2. Design Principles

- preserve native provider interaction semantics, using PTY passthrough where required and structured local provider protocols where they are stronger and officially supported
- reserve `/...` as the Arroba command namespace
- keep structured control surface intentionally small
- isolate capability/control errors from terminal stream
- ensure all user-generated in-transit payloads are session-E2E encrypted on remote transport, including prompts, workflow inputs/outputs, and transferred/attached artifacts
- this requirement applies equally to:
  - self-hosted relay deployments
  - any later managed relay deployment
- relay must only ever see opaque encrypted payloads plus the minimum metadata required for routing and liveness

Current sequencing note:

- OpenCode is the reference provider for the current development cycle
- protocol and adapter boundaries should stay future-compatible, but they should not be generalized prematurely at the expense of finishing the OpenCode-first runtime
- web/mobile clients come before multi-provider expansion in the current rollout order
- same-kernel remote clients should fit the same kernel-owned protocol rather than a separate remote-only API
- same-kernel remote agents remain part of the architecture, but their generic transport contract is intentionally deferred until Arroba has integrated more than one concrete agent family

## 2.1 Node Roles

The protocol should distinguish at least these logical roles:

- `client`
- `agent_endpoint`
- `relay_or_server`

The kernel remains the workspace/runtime authority in all cases.

## 3. Protocol Lanes

## 3.1 Terminal Lane (Provider Output Stream)

Purpose:

- user keystrokes to provider PTY
- provider stdout/stderr/control sequences to clients

Semantics:

- byte-stream-like behavior
- no requirement for structured parse by Arroba for ordinary non-command traffic
- for providers with structured event streams, Arroba MAY render provider output into the client terminal without treating PTY bytes as the source of truth for turn lifecycle
- for same-kernel remote clients, the terminal lane should still be kernel-routed; relay changes the path, not the workspace authority

Suggested events:

- `terminal.input`
- `terminal.output`
- `terminal.resize`

OpenCode-specific note:

- OpenCode should graduate from PTY-polled `terminal.output` to adapter-fed output derived from its local event stream
- incremental assistant text should come from provider message-part delta events
- terminal rendering remains kernel-owned even when the source is a structured provider event stream
- the current protocol surface should be proven against OpenCode first before new provider families drive further adapter generalization

## 3.2 Capability Lane (Structured Daemon Actions)

Purpose:

- daemon-owned operations invoked from Arroba slash-command dispatch

Suggested request envelope:

- `request_id`
- `session_id`
- `capability`
- `args`
- `sent_at`

Suggested result envelope:

- `request_id`
- `status` (`ok` | `error`)
- `result` or `error`
- `completed_at`

Capabilities in v1:

- `shell.run`
- `dir.tree`
- `file.view`
- `file.edit`
- `screenshot.capture`
- `git.info`
- `file.transfer`
- `file.attach_transferred`
- `context.compact` (mapped from `/compact`)
- `schedule.*`

Slash-command routing rules:

- `/...` is parsed by Arroba before PTY forwarding
- `/<provider> ...` is resolved against the focused provider command catalog
- ordinary non-command input continues through `terminal.input`
- unsupported provider versions MAY produce warnings, but MUST NOT disable best-effort `/<provider> ...` completions by default

## 3.3 Control Lane (Structured Daemon->Provider Adapter)

Canonical operations in v1:

- `attach_file`
- `request_memory_update`
- `request_compaction_summary`

These operations are not typed by users into ordinary terminal traffic.

Arroba MAY route `/<provider> ...` invocations into the control lane after resolving the focused provider command catalog.

OpenCode-specific structured adapter contract:

- prompt submit maps to the provider session prompt operation
- `/<provider> ...` command invoke maps to the provider session command operation
- turn abort maps to the provider session abort operation
- provider lifecycle and output state are consumed from the provider event stream rather than inferred from PTY EOF or PTY idleness
- later providers such as Claude Code and Codex should fit behind the same daemon/client contract after the OpenCode-first cycle is closed

Provider hidden-context injection contract:

- Prompt submission from the kernel to a provider adapter is conceptually a `PromptEnvelope`, not one concatenated string.
- `visible_user_prompt` is the only prompt body that may be shown in Arroba prompt blobs, terminal input history, native provider prompt boxes, or user-facing prompt echoes.
- `hidden_system_context` carries Arroba runtime/system prompt material: runtime instructions, Workspace Live Sync managed/tracked instructions, native permission rules, workflow-level prompts, node-level instructions, granted capability summaries, continuation instructions, and utility-call instructions.
- `attachments` remain structured prompt attachments and are not used to smuggle hidden system instructions.
- `manifest` records prompt template ids, template hashes or versions, assembly conditions, and the provider injection channel selected for the turn; the manifest is audit/debug metadata, not prompt UI content.
- Arroba MUST NOT implement hidden context by prepending text to `visible_user_prompt` and later redacting it from UI surfaces.
- The relay MUST treat prompt envelopes as opaque encrypted payloads and MUST NOT inspect, transform, redact, or split visible versus hidden prompt fields.

Provider adapter hidden-context channels:

- Codex adapters MUST send hidden context through `thread/start.developerInstructions` or `thread/resume.developerInstructions` when a Codex thread is created or resumed. Codex does not accept this context through `turn/start`; for kernel-managed Codex runs, the kernel MUST hot-reload the Codex thread before a turn when the assembled hidden context fingerprint changes.
- OpenCode adapters MUST send turn-scoped hidden context through the provider session prompt request `system` field, currently `POST /session/{id}/prompt_async` body `system`.
- Claude Code adapters MUST send turn-scoped hidden context through the `UserPromptSubmit` hook response `hookSpecificOutput.additionalContext`.
- If a provider channel is unavailable, the adapter may run without hidden context for that turn or restart the provider process with an initialization-scoped system prompt only when the caller explicitly accepts that behavior; it must not silently fall back to visible prompt injection.
- Live provider drills validate direct provider hidden-context channels in current supported harnesses. Prompt assembly changes that touch these channels must keep or update `pnpm --filter @arroba/cli run provider-context-injection:drill`.
- End-to-end prompt assembly changes must also keep `pnpm --filter @arroba/cli run prompt-assembly:drill` passing. That drill edits a temporary `~/.arroba/prompts/runtime/base.md`, runs real Arroba provider turns for Codex/OpenCode/Claude, verifies the model sees the hidden registry token through the provider-native hidden channel on successive turns, and verifies Arroba user-prompt history does not contain the hidden token.

Prompt template storage:

- Arroba prompt templates are user-owned markdown files under `~/.arroba/prompts`.
- Source-controlled defaults may be materialized there for first run, but runtime assembly reads from the registry path rather than hardcoding prompt text in adapter code.
- Required templates include runtime base instructions, Workspace Live Sync managed instructions, Workspace Live Sync tracked instructions, native permission instructions, slice runtime instructions, MCP/skill continuation instructions, workflow turn/completion/intermediate-output templates, and utility-call templates.
- Cloud editing, if introduced later, edits this registry model and must not create a second prompt source of truth.

Provider-local visibility caveat:

- Arroba UI and protocol prompt blobs must hide `hidden_system_context`, but provider-local histories may still store it in provider-native form.
- Current provider harnesses expose hidden context in internal histories/transcripts: Codex history APIs, OpenCode message `info.system`, and Claude transcript `hook_additional_context`.
- The protocol guarantee is therefore “not visible in Arroba/native prompt input surfaces,” not “unrecoverable from provider-owned local state.”

## 3.3.1 Agent Endpoint Direction

Longer-term agent runtimes compatible with Arroba should speak a daemon-facing endpoint contract rather than requiring the daemon to launch only local child processes.

Required properties:

- bidirectional messaging
- explicit prompt or turn lifecycle
- explicit tool/runtime events
- health and capability advertisement

Existing providers like OpenCode may continue to be adapted through their native protocols.

## 3.3.2 Native TUI Agents

Native TUI agents let a user run a familiar provider CLI UI while the Arroba kernel remains the session authority.

Current commands:

- `arroba codex [session-ref] [--kernel-port PORT|--kernel-url URL]`
- `arroba opencode [session-ref] [--kernel-port PORT|--kernel-url URL]`
- `arroba claude [session-ref] [--kernel-port PORT|--kernel-url URL]`

Semantics:

- if no session ref is provided, Arroba creates a session and its first native TUI agent
- if a session ref is provided, Arroba attaches a new top-level native TUI agent to that Arroba session
- local native TUI launchers default to the web-dev kernel at `ws://127.0.0.1:43119/kernel`; `--kernel-port` selects another local kernel port
- a native TUI launch never attaches to an existing provider run; every native TUI agent owns its own provider run
- prompts from the provider TUI are intercepted and submitted through the same kernel prompt path as Arroba clients
- prompts from Arroba clients are forwarded through the kernel-managed provider run so the provider TUI observes the same turns
- native TUI provider runs are marked with `client_interface = native_tui`
- Arroba clients must treat model/variant controls for those runs as provider-controlled; provider-native changes may be recorded when observable, but Arroba-side parameter mutation is disabled for the active native TUI run
- daemon health reports `duplicate_arroba_agent_bindings` when more than one active Arroba provider run is bound to one session/agent, and `multi_interface_agent_bindings` when active Arroba and native TUI provider runs are bound to the same session/agent
- daemon health `provider_catalog` reports whether provider/model metadata is cached, expired, and how old it is; clients should surface stale catalog state near provider/session launch diagnostics
- daemon-tracked provider process listings include PID and best-effort current RSS (`resident_set_bytes`) when the host can read it; clients should surface this beside teardown safety so provider memory pressure is diagnosable without external process tools
- `ExportDebugBundle { session_id, bundle_label, limit }` is the shared session-scoped debug bundle request for TUI, web, and remote clients. The caller supplies only a session id, optional label, and optional record limit; the kernel filters current structured logs by `session_id`, sanitizes the label, writes `manifest.json` and `logs.ndjson` under its own debug-bundles root, and returns `DebugBundleExported { bundle_dir, manifest_path, logs_path, log_root, record_count, limit }`. Clients must display the returned paths as kernel-machine-local paths and must not send arbitrary output directories.
- Agent inspection and pane chrome should surface the session home kernel/machine alongside agent placement, worktree, provider run, extension grants, and remote extension manifest state so users can distinguish session authority from worker execution.

Remote native TUI composition:

- remote native TUI mode MUST compose existing protocol paths rather than create a second prompt/runtime protocol
- provider-native TUIs and Arroba TUIs attach to the home kernel session through the same client/session attachment semantics used locally
- provider-native TUI prompts MUST enter the home kernel through the same `SubmitPrompt` path as Arroba prompts
- the home kernel MUST dispatch remote execution through the existing leased-agent relay path (`SubmitLeasedPrompt`, remote prompt attachments, remote MCP/skill checks, and related completion/cancel paths)
- `ExecutionLeaseCreated` MUST include `relay_peer_protocol_version`; the home kernel must reject a worker that omits it or advertises a lower version before `SpawnLeasedAgent`, so stale remote kernels fail with an upgrade/restart action instead of breaking during provider tool calls
- the worker kernel MUST talk to the provider through the same kernel-provider adapter/server path used by ordinary worker-owned provider runs
- worker output, notices, completions, and permission interactions MUST return to the home kernel through existing leased runtime projection and native interaction relay paths
- relay peer protocol v3 extends leased runtime projection with an optional worker `provider_run` snapshot. The home kernel MUST project that worker-owned run onto the home session/agent, including any resolved `provider_session_id` and resume state, because some providers such as Codex only expose the durable thread id after the first turn rather than at launch time.
- relay peer protocol v7 correlates projected completions with `home_prompt_id`. Workers retain the latest settled completion for pull-based replay, and home kernels MUST ignore a stale replay when that prompt is no longer active. This makes a completion recoverable when a fire-and-forget worker projection is lost without allowing the replay to settle a later queued turn.
- the provider-native proxy/launcher MAY translate home-kernel session output back into provider-native UI protocol or PTY rendering, but it must not become a session authority or bypass the home kernel prompt queue
- the relay remains transport-only and must not inspect or transform provider-native prompts, outputs, attachments, permissions, or history
- slice-backed native TUI mode follows the same contract: provider TUIs and Arroba TUIs attach to the home kernel session, `slice_ref` selects a home-managed worker execution environment, and the slice worker uses the same worker-owned provider adapter/server path as remote leased agents

Native TUI MCP and skill placement:

- local native TUI provider runs use the same agent-scoped grant filtering as ordinary local provider runs, so only MCPs and skills granted to that agent are injected or rendered for that run
- standard home-worker native TUI may expose home-authorized remote extension manifests to the worker. Home-owned active extensions remain grant/revoke authoritative on the home kernel and execute on home through relay peer calls; the worker only advertises the manifest and forwards calls. Each forwarded call carries `invocation_id`, optional `provider_tool_call_id`, `attempt`, and optional `idempotency_key`; home reconstructs the current tool definition before execution and rejects stale or forged worker metadata, including calls from a worker provider run that is not the current remote binding.
- when an extension is explicitly worker-local, the home kernel may still compute grant-derived remote MCP requirements and pass those requirements to the worker launch/prompt path so the worker can fail fast on missing or mismatched local worker definitions before provider execution
- slice-backed native TUI may synchronize Arroba skill packages from the home kernel to the child worker because the slice is home-managed; this is not a general remote-machine install mechanism
- slice-backed native TUI still executes worker-local MCP commands on the worker side, so worker-local MCP commands and environment must be available in the slice image or injected slice environment; Arroba vault credentials remain home-owned and are exposed to slice workers only through home-authorized credential proxy calls and one-operation secret injection
- capability grants remain agent-scoped in all modes; native TUI launch must not expose ungranted local/user MCPs or skills just because the provider CLI can see them natively

Native TUI permissions:

- provider-native permission requests MUST be represented as one agent-scoped, kernel-owned `RuntimeInteraction`
- that interaction MUST be projected to every Arroba TUI attached to the session, regardless of whether the current turn was submitted from an Arroba TUI or provider-native TUI
- answering from an Arroba TUI resolves the kernel interaction and the provider adapter/proxy forwards the resulting decision to the provider
- where a provider-native TUI can submit an approval response through a stable proxy or hook seam, the native response MUST resolve the same kernel interaction rather than bypassing it; first valid resolution wins
- if the provider only exposes the approval through a rendered PTY, Arroba may detect the rendered prompt and create the kernel interaction, then inject the resulting decision back into the PTY using the provider's native selection semantics

Native TUI hidden context:

- granted skill prompt context and other Arroba-only prompt injections MUST be delivered on the provider-facing path without becoming visible provider-TUI text
- Codex native TUI hidden context MUST use the same Codex turn-scoped `developer_instructions` channel as ordinary Codex provider runs
- OpenCode native TUI hidden context MUST use the same OpenCode prompt request `system` field as ordinary OpenCode provider runs
- Claude Code native TUI MUST use the `UserPromptSubmit` hook `additionalContext` path for hidden context; the hook emits a scoped context request id, and the Arroba CLI bridge or worker kernel writes the matching context response before the hook returns
- Claude hook context responses are scoped to the session, agent, and provider run; they must not expose broad kernel authority or accept arbitrary provider-origin file paths
- if a Claude hook context response is unavailable before timeout, the provider-facing hidden context is empty and the native TUI remains coherent; Arroba MUST NOT fall back to visible PTY prompt injection for skill bodies or system prompt blocks
- local Claude native TUI can answer hook context requests through the launcher bridge and home kernel; remote/slice Claude native TUI answers them on the provider-execution side so worker-local or slice-isolated skill material is used

Provider-specific transport:

- Codex uses a native WebSocket proxy in front of a Codex app-server endpoint and binds the observed Codex thread to the Arroba provider run.
- OpenCode uses a native HTTP proxy in front of a launcher-managed `opencode serve` endpoint. The kernel binds its provider run to the proxy endpoint, while the provider TUI attaches to the same proxy/provider session.
- Claude Code has no stable provider UI/server split. Local and remote native TUI mode therefore use a kernel-owned PTY: the provider process runs where execution belongs, and the launcher streams/render-controls that PTY while the kernel projects prompts, output, attachments, status, and supported interactions back into the Arroba session.

## 3.3.3 Metaagent Event Prompts

Metaagent event notifications are Arroba runtime-origin prompts. They are not
hidden provider context, and adapters MUST NOT deliver them through hidden
system/developer channels. The visible prompt text should identify the message
as an Arroba runtime event, summarize what happened, and point the metaagent to
`arroba.meta.read_event`, `arroba.meta.turn_overview`, or
`arroba.meta.turn_blob` for detail payloads that are too large for the prompt.

Each recorded event carries prompt-delivery state so a reconnecting metaagent
can reconstruct what happened:

- `recorded`: the event exists in the kernel inbox but has not reached a provider prompt path
- `submitted`: the kernel submitted a visible event prompt to the provider path
- `steered`: the event was attached to an already-active metaagent turn
- `queued`: the event prompt is queued behind an active turn
- `delivered`: the provider accepted or completed the corresponding event prompt
- `failed`: delivery failed and the event should be visible as a liveness fault until retried or superseded

Provider-specific delivery behavior:

- Codex: event prompts use the ordinary visible user-prompt path for the bound
  Codex thread. If the Codex run is active and supports same-turn steering,
  Arroba may mark the event `steered`; otherwise the prompt remains queued and
  visible in Arroba prompt history.
- OpenCode: event prompts use the provider session prompt API as visible
  prompt content. Hidden `system` context remains reserved for runtime
  instructions and MUST NOT carry event notifications. OpenCode event-stream
  completion should update delivery status without relying on PTY idleness.
- Claude Code: event prompts are submitted through the same visible prompt path
  used for user turns. `UserPromptSubmit.additionalContext` remains reserved for
  hidden context such as skill bodies and MUST NOT carry event notifications.
  When Claude is exposed through a kernel-owned PTY, Arroba may render or steer
  the visible event prompt through that PTY only as provider-visible prompt
  input, not as a hidden hook response.

Required metaagent events, including owned-agent turn completion, owned-agent
turn failure, and owned regular-agent runtime interactions, must preserve
ordering per metaagent. Optional workflow subscriptions may share the same
visible prompt mechanism, but filtering and durable inbox state remain
kernel-owned. A missing provider run or delivery failure must be surfaced in
the event status and retry path rather than being silently dropped.

## 3.4 Workflow Coordination Semantics

Multi-agent workflow coordination is a daemon-owned structured protocol concern.

Delivery priority inside v1:

- circular topology is the earlier implementation target
- hierarchical topology remains in scope for v1, but is expected to land later in v1 after lower-level runtime and protocol foundations are stable

Required rules:

- node-to-node communication MUST use structured handoff payloads
- workflow progression MUST be driven by node completion reports, not raw provider turns
- workflow routing, barrier/fan-in handling, and termination decisions MUST NOT depend on forwarding raw terminal transcript output between agents

## 4. Common Message Envelope

All structured messages should carry a minimum common envelope. Some fields are lane-specific or message-class-specific.

Common fields:

- `version` (protocol version, currently `v2` for the shared local daemon protocol)
- `lane` when applicable (`capability` | `control`)
- `type` (event/action identifier)
- `request_id` when request/response matching is needed
- `command_id` when the message represents a kernel command or a command-caused event
- `session_id`
- `agent_id` when agent-scoped
- `provider_run_id` when provider-scoped
- `workflow_run_id` when workflow-scoped
- `node_run_id` when node-scoped
- `target_node_id` or `target_node_run_id` when routing workflow handoffs
- `payload`
- `meta` (timestamps, source attachment id, causation id, correlation id, trace id)

Future unified node-transport fields should also allow:

- `connection_id`
- `attachment_id`
- `member_role`
- `event_id`
- `resume_from_event_id`

## 4.1 Current Kernel Transport Baseline

For the current local baseline, the kernel exposes a request/response plus pushed-event surface over a daemon-owned WebSocket transport.

Transport scope (current definition):

- connects clients (CLI, relay, agent adapters)
- maintains live session state subscriptions
- emits output/notices/config updates to attachments
- enforces prompt flow control policies (queue advancement, idle/timeout completion, cancellation transitions)
- provides request/response dispatch for the local transport
- bridges the transport contract across local and remote transports

Current implementation notes:

- the TypeScript CLI now defaults to `ws://127.0.0.1:${ARROBA_KERNEL_PORT:-43118}/kernel`
- the Rust daemon process hosts that WebSocket listener directly
- the older Unix-socket local IPC path still exists for daemon harnessing/tests and compatibility shims, but it is no longer the primary CLI transport
- the current wire shape now supports request/response plus pushed kernel events over one long-lived connection
- subscriptions carry optional `resume_from_event_id`
- the kernel emits monotonic in-process `event_id` values on pushed events
- heartbeat events are part of the current transport so the CLI can detect and recover stale connections
- reconnect/resubscribe is now part of the intended local CLI behavior
- current replay is bounded by the daemon's retained recent-event window; if a resume cursor falls outside that window, the M4.5 contract requires an explicit replay-gap response plus a fresh projection snapshot
- event ids should not be treated as daemon-restart durable until a persisted event log or equivalent projection checkpoint/tail-event store lands

Current pushed event contract:

- all pushed events use the `KernelOutgoingFrame::Event` envelope with monotonic `event_id` plus an `event` payload tagged by its `event` string
- `terminal_output` carries terminal records and should be used for terminal append/update rendering without forcing `session.state.get`
- `runtime_notices` carries runtime notices for the subscribed attachment/session
- `assistant_message_completed` carries `session_id`, `provider_run_id`, optional `agent_id`, `message_id`, and `completed_at_ms`
- `session_snapshot` is the full subscribed-session projection and remains the fallback after attach, replay gaps, explicit recovery, and structural changes
- `agent_activity_changed` carries `session_id` and the complete agent activity map for activity-only projection updates
- `provider_run_changed` carries `session_id` and the current provider run, or `null` when no provider run is active
- `session_metadata_changed` carries `session_id` and a `metadata` patch with alias, last-used timestamps, hidden state, focused agent, and workspace live-sync mode
- `runtime_interactions_changed` carries `session_id` and the current active runtime interactions for permission/choice prompts
- `waiting_room_inventory_changed` carries only `inventory_version` and is retained as a lightweight compatibility/fallback signal
- `waiting_room_rows_changed` carries `inventory_version`, `schema_version`, `generated_at_ms`, optional `launch_target`, changed session rows, and `removed_session_ids`; clients should apply it as a row patch instead of refetching the full waiting-room snapshot
- `provider_catalog_changed` carries `generated_at_ms` and the current provider catalog
- `slices_changed` carries `generated_at_ms` and the current slice list
- `workflow_run_updated` carries `session_id` and the updated workflow run for workflow-run-only updates
- `heartbeat`, `transport_resumed`, `replay_gap`, `session_unavailable`, and `transport_closed` are transport/recovery signals; heartbeat and successful resume should not force full session, waiting-room, or prompt-history reads, while replay gaps require clients to discard optimistic deltas and request a fresh projection

Minimum request set:

- `session.create`
- `session.list`
- `session.resolve`
- `session.attach`
- `session.detach`
- `session.delete`
- `agent.spawn`
- `agent.destroy`
- `agent.focus`
- `agent.cycle`
- `agent.list`
- `provider_run.launch`
- `session.state.get`
- `session.notice.poll`
- `prompt.submit`
- `prompt.complete`
- `prompt.cancel`
- `session.config.update`
- `terminal.output.poll`
- `terminal.resize`
- `session.end`

Minimum response/result shapes:

- session creation returns structured session metadata
- session resolution returns structured session metadata
- attach/detach returns structured attachment metadata
- agent lifecycle/focus operations return structured agent metadata plus updated focused-agent state where relevant
- provider launch returns structured provider-run metadata
- session state reads return canonical queue and config state
- notice polling returns structured daemon notices scoped to the requesting attachment within the session
- prompt submission returns structured prompt status (`started` or `queued`) plus canonical session state
- prompt completion returns structured completion details and the next started prompt when relevant
- prompt cancellation returns the updated prompt state; for provider-backed turns the daemon advances queued work only after the provider confirms the stop
- config update returns canonical session config state, version, and updated session state
- terminal output polling returns structured terminal-output fan-out records, including distinct provider text, reasoning, tool, error, and status output kinds
- end-session returns structured final session metadata

Current session-management semantics:

- user-facing clients should prefer `session.delete` over an implicit "end on exit" model
- `session.resolve` and `session.delete` accept a `session_ref` that may be:
  - full session id
  - unique session-id prefix
  - alias
  - unique alias prefix
- `session.create` accepts an optional alias
- deleting the currently attached session invalidates the attachment and the client should transition to an unattached "no session" state instead of forcing process exit
- `session.delete` is a real delete operation: after runtime teardown the session is removed from the daemon registry and can no longer be listed, resolved, or reattached
- if a session reference is ambiguous, the daemon rejects it with a structured ambiguity error

Current agent-management semantics:

- the local daemon API now includes top-level session-agent management operations (`spawn`, `destroy`, `focus`, `cycle`, `list`)
- focused-agent state is part of canonical session state and is intended to determine which top-level agent receives direct user interaction
- direct prompt submission now targets the focused top-level agent in the local runtime
- provider runs are now tracked per top-level agent and the daemon can park/resume them as focus changes or the session returns to idle
- session history and terminal-derived structured output records are now agent-scoped for the local multi-agent path
- pane-capable clients can now render per-agent transcript surfaces from daemon-owned state, although the current TypeScript CLI split-pane surface is still an initial slice rather than the final generalized layout

Local cancellation policy:

- any currently attached client in a session may request cancellation of that session's active prompt
- cancellation is session-scoped rather than attachment-owned because the active provider turn is shared session state

This local API MUST remain daemon-owned, local-first, and compatible with later workflow-mode runtime surfaces.

Architectural note:

- the WebSocket request/event transport is the primary CLI path
- the request/response IPC surface remains a bootstrap, harness, and compatibility transport
- both transports should normalize mutating requests into `KernelCommand` values during the M4.5 refactor
- future kernel-client and kernel-agent communication should converge on one long-lived event-capable connection model

Current local runtime note:

- the primary local CLI implementation is now a TypeScript OpenTUI client
- `arroba-cli` currently launches that TypeScript client through a small Rust compatibility wrapper
- the Unix-socket local transport remains useful for daemon smoke coverage and compatibility shims, but it is no longer the primary local user path

Current slice-management surface:

- `slice.list`, `slice.create`, `slice.get`, `slice.start`, `slice.stop`, `slice.delete`, `slice.display_endpoint.get`, `slice.logs.get`, `slice.state.save`, `slice.state.status`, `slice.state.reset`, and `slice.backup.create` are daemon-owned local requests.
- Local Docker slice records persist their assigned host port set in `local_docker_ports`; clients may display these diagnostics, but launch, relay, display, and log behavior must use the kernel-owned values rather than reconstructing ports from the slice id.
- Local Docker slices keep Docker's default seccomp profile unless the home user explicitly sets `slices.linux.allow_unconfined_seccomp = true`; clients must present this as an advanced security compatibility option.
- Slice lifecycle status is `stopped`, `starting`, `stopping`, `running`, or `unhealthy`. Start must only report `running` after the worker kernel has been discovered; otherwise the slice remains `unhealthy` and diagnostics are available through `slice.logs.get`.
- Slice records also carry display-only operation diagnostics: `last_operation`, `last_operation_status`, `last_error`, and `last_operation_at_ms`. The kernel updates these on lifecycle operations and restart reconciliation; clients may render them in status/doctor views, but must continue to treat `status` as the lifecycle state and audit/log records as the detailed diagnostic source.
- Daemon health `slice_lifecycle.issues` identifies each unhealthy slice or failed slice operation by slice id/name, status, last operation/status/error, sessions, agents, and worktree so clients can point users directly to the affected slice before they open logs/audit or restart/delete it. `slice_lifecycle.provider_auth_issues` separately identifies attached-agent slices with no provider account summaries or with `unknown`/`not_configured` provider auth, including provider, alias/identity, sessions, agents, and worktree. Clients should surface this from kernel health and point users to `/slice doctor`, `/slice audit`, and slice auth login/import before they send more provider prompts.
- Local Docker slices use the kernel-configured relay when it has a token and a non-loopback `ws://` or `wss://` URL, so hosted Cloud and self-hosted relay deployments expose the slice worker on the same relay fabric as other remote workers. The slice receives only the scoped runtime relay URL/token, not the home Cloud refresh profile. Loopback or incomplete relay configuration falls back to a private per-slice relay owned by the home kernel; clients should render the projected `relay_endpoint.private` flag rather than guessing from the URL.
- Kernel restart reconciliation must not leave runtime-only states active. Local Docker reconciliation inspects the host container: missing/stopped previously running slices become `stopped`, still-running or unverifiable runtime state becomes `unhealthy`, and interrupted `starting`/`stopping` transitions become `unhealthy`.
- `slice.logs.get` returns structured log entries for local Docker slice provisioner actions and recent container logs. Clients should render these as diagnostics only and must not treat log text as control data.
- Slice provider auth import/login/alias/remove requests are scoped by provider and the kernel owns displayed provider auth summaries. Removal purges the slice-side provider credential files and clears matching auth summaries from kernel state; `opencode` removes all `opencode:*` account summaries for that slice.
- Slice saved state is a kernel-owned product concept, not a Docker-management UX. `slice.state.save` overwrites the active state for the slice, `slice.state.status` returns the active saved-state metadata, `slice.state.reset` removes the active state so future starts use the base slice image, and `slice.backup.create` creates a separate backup plus manual swap instructions. Saved state is composite: a Docker image tag and a `/home/slice` archive under the Arroba slice state root. Slice records expose only metadata (`saved_state_ref`, `saved_state_status`, `saved_state_updated_at_ms`); clients must not inspect archive contents or expose them to provider transcripts.
- `slice.create.from_saved_state` may reference an existing saved-state id/name. Local Docker restore uses the saved image tag instead of the configured base image and extracts the saved home archive into the fresh slice home volume before normal provisioning continues. Restore still allocates fresh ports, relay identity, and worker identity through the normal slice start path.

Current session-lifecycle note:

- the local implementation still exposes `session.end` as an internal/runtime operation, but the intended user-facing local client contract is persistent detached sessions plus explicit `session.delete`
- `session.end` and `session.delete` are intentionally distinct:
  - `session.end` is an internal/runtime operation and may still be reused for resumable daemon-owned transitions
  - `session.delete` is the user-facing destructive operation and removes the session from the daemon registry after teardown
- the current local implementation now uses 16-character lowercase hexadecimal session ids with optional aliases and unique-prefix resolution

OpenCode current runtime note:

- the daemon already routes OpenCode prompt submit through the provider-native local HTTP session APIs
- the daemon already consumes OpenCode output and completion through the provider event stream
- provider-native TUI mode can supply an external OpenCode structured endpoint so the native launcher can proxy both kernel and provider-TUI traffic before forwarding to `opencode serve`
- active-turn cancellation is routed through the OpenCode abort API and reconciled from provider events before queued prompts advance
- PTY remains a liveness/process-management surface for the OpenCode server process, not the primary prompt/output transport
- the same daemon-owned local request/response surface remains the client contract while the adapter becomes more provider-specific internally

## 4.1.1 Unified Node-Transport Direction

The intended node architecture now assumes that the kernel should eventually act as a general router for:

- local clients
- remote clients connected through relay
- local agent endpoints
- remote agent endpoints connected through relay

Recommended direction:

- one long-lived kernel-owned bidirectional protocol
- request/response messages for control
- pushed daemon events for prompt/session/provider updates
- relay forwarding without changing daemon authority

This does not require all provider adapters to use the same wire transport internally.

## 4.2 Planned Command-Dispatch Surface

The current local API baseline does not yet expose slash-command discovery/invocation, but the protocol should reserve room for it.

Planned request types:

- `command.list`
- `command.invoke`
- `agent.command.list`
- `agent.command.invoke`
- `provider.auth.status.get`
- `provider.event.subscribe`
- `extension.install`
- `extension.list`
- `extension.bind`
- `extension.unbind`
- `mcp.runtime.list`

Planned command metadata fields:

- `command_path`
- `description`
- `source` (`builtin` | `custom` | `best_effort_catalog`)
- `provider`
- `provider_version`
- `catalog_version`
- optional `warning`

OpenCode adapter metadata additions:

- optional `provider_session_id`
- optional `provider_event_capabilities`
- optional `provider_command_source` (`catalog` | `provider_api` | `custom_files` | `merged`)

Planned provider auth status fields:

- `provider`
- `account_profile`
- `auth_state` (`authenticated` | `not_logged_in` | `expired` | `unknown` | `provider_not_installed`)
- optional `login_hint`
- optional `detected_version`

Current kernel-client metadata fields:

- `member_role` (`client`)
- `connection_mode` (`local_direct` | `relayed`)
- `protocol_version`
- optional `resume_from_event_id`

Deferred agent-endpoint note:

- OpenCode remains adapter-owned and continues to use native local HTTP control plus SSE events
- managed vs external OpenCode endpoint binding is the current agent-endpoint abstraction boundary in code
- a generic WebSocket transport for agent endpoints is explicitly deferred until after Arroba has integrated more than one agent family and can derive a better common denominator from real integrations

Planned extension metadata fields:

- `extension_id`
- `type` (`skill` | `mcp_server` | `command_pack` | `instruction_pack` | `hook`)
- `source`
- `version`
- `provider_support`
- `visibility_policy`
- `install_state`

## 5. Control Operations

## 4.3 Workflow Message and Endpoint Direction

The workflow model should use a minimal, general message envelope rather than predefined domain-specific fields.

Logical workflow message fields:

- `message`
- `recipients`
- `artifacts`

Rules:

- the workflow graph defines which recipients are valid from a given sender
- artifacts are intentionally open-ended
- the kernel validates message structure and routing before delivery
- each sender may emit at most one message per recipient in a single turn

Workflow endpoint direction:

- a workspace may contain multiple workflow definitions
- each workflow definition may expose multiple logical endpoints
- each workflow endpoint maps to one entry node in that workflow
- an endpoint may be invoked by a terminal user or by an external published API
- once accepted by the kernel, the workflow should treat the resulting input message the same way regardless of source
- disconnected subgraphs are allowed; a subgraph is reachable only if some endpoint points into it

Published workflow direction:

- a published workflow is a durable package plus a materialized publication
  runtime session, not a live pointer to an editable workspace session
- a publication package contains `publication.json`, `workflow.snapshot.json`,
  `requirements.json`, optional generated app assets, and packaged scripts
- a publication runtime session is kernel-owned, hidden from ordinary session
  lists, and non-editable through normal workspace commands
- a hook binds one transport to one workflow endpoint and one workflow queue
  inside a published workflow runtime
- multiple hooks MAY feed one published workflow runtime and therefore share
  the same queue namespace and agents
- serving a publication MUST validate provider/model bindings, extension
  requirements, and credential requirements before it accepts traffic
- if the captured provider/model is unavailable, a local binding may substitute
  another available provider/model without mutating the exported package
- Cloud publication deployment is a control-plane record plus a runtime backend.
  It is not a new workflow authority and does not replace the kernel-owned
  publication runtime session.
- v1 Cloud deployment supports two backend modes:
  - `local_runtime`: a public publication ingress routes to a user's local
    `arroba serve` process over an outbound connector
  - `hosted_container`: a public publication ingress routes to one Docker
    container per deployment on the publication runner
- Scalingo-hosted Arroba Cloud APIs own deployment records and control commands
  only. Runtime publication traffic should terminate at the dedicated
  publication ingress and route from there to the active backend.

Publication deployment record:

- `deployment_id`
- `account_id`
- `mode` (`local_runtime` | `hosted_container`)
- `slug`
- `public_base_url`
- `status`
- `publication_id`
- `publication_alias`
- `workflow_id`
- `endpoint_id`
- `hook_id`
- `transport`
- `package_digest`
- `runner_id`
- `backend_target`
- `runtime_session_id`
- `credential_profile` or credential state
- `last_health_at`
- `last_error`

Deployment records are operational metadata. They must not contain provider
auth secrets, Arroba Cloud user session credentials, workflow prompt payloads,
artifacts, outputs, or traces.

Public deployment URL contract:

- all transports are rooted at `public_base_url`
- `GET /` opens the human/browser-compatible viewer or form
- `GET /<prompt>` invokes `human_http` with an address-bar prompt path
- `POST /invoke` invokes `api_sse_json`
- `/.well-known/arroba/publication/ws` invokes `websocket_json`
- `POST /mcp` invokes the MCP publication endpoint
- `GET /.well-known/arroba/publication/status` returns publication status

The external contract is the same for `local_runtime` and `hosted_container`.
The caller should not infer execution location from the URL.

Publication invocation envelope:

- `publication_id`
- `hook_id`
- `invocation_id`
- `transport`
- `endpoint_id`
- `queue_ref`
- `input`
- `artifacts`
- `mode`

The invocation envelope is created after hook transport parsing. It should be a
kernel-native structured value, not only a JSON string submitted through the
ordinary prompt compatibility path.

Publication event direction:

- every accepted publication invocation should have a stable `invocation_id`
- events should cover at least `queued`, `started`, `partial`, `final`, and
  `error`
- events MAY also include `trace` when the publication explicitly exposes
  workflow traces for the node and trace level that produced the record
- trace fanout is governed by a per-node publication policy, not by transport
  defaults; if no policy is present, trace events are not exposed
- trace levels are `output_summary`, `assistant_messages`, `thinking`, and
  `tool_use`
- `thinking` trace events are sourced from provider reasoning chunks persisted
  on the active `WorkflowNodeRun.thinking_traces` list while the workflow node
  prompt is running
- each `trace` event must include `invocation_id`, `workflow_run_id`,
  `node_id`, `node_label`, `agent_id`, `agent_alias`, `level`, `sequence`,
  `timestamp_ms`, and a structured `payload`
- trace filtering is part of the publication runtime contract: clients and
  publication gateways must not infer or expose hidden workflow internals
  beyond the policy
- browser-compatible transports share one publication viewer HTML app. The
  viewer renders output/status on the left and exposed traces on the right, and
  selects a small client-side adapter for the configured transport
- `human_http` can invoke from an address-bar GET path or from the shared viewer
  form; GET result pages subscribe to publication events by SSE
- `api_sse_json` streams publication events directly from `POST /invoke`
- `websocket_json` sends publication events over the WebSocket connection
- the shared viewer can drive `api_sse_json` with browser `fetch` streaming and
  can drive `websocket_json` over the publication WebSocket path
- `mcp` maps publication progress/final output to MCP tool progress and result
  concepts

Publication trace exposure policy:

```json
{
  "trace_exposure": {
    "nodes": {
      "node-a": ["output_summary", "assistant_messages", "thinking"],
      "node-b": ["output_summary", "tool_use"]
    }
  }
}
```

Trace exposure policy is evaluated per workflow node. Nodes without an explicit
entry expose no traces. Unknown node ids or trace levels fail publication or
serve-time validation before a server accepts traffic. Trace policy is fixed by
the publication artifact; changing exposure requires republishing or creating a
new publication.

Human HTTP renderable output:

- a final workflow output whose message parses as `{ "kind": "html", "html":
  "..." }` is renderable HTML for `human_http`
- the split viewer must render that HTML in a sandboxed `iframe srcdoc` in the
  left pane, replacing the textual output/status region
- generated HTML must not be injected directly into the publication viewer DOM
- the right trace pane remains visible and continues to show exposed traces
  after the generated HTML is rendered
- Agent Apps generalize this renderable-output model. A future generalized
  response output can represent serving an app route, returning JSON, redirecting,
  applying overlays, invoking app actions, or emitting persistent patches while
  still remaining a workflow output interpreted by the publication server. See
  `docs/AGENT_APPS_CONCEPT.md`.

Remote terminal and Cloud invocation:

- remote Arroba terminals must invoke a published workflow through its published
  transport, not by directly calling the workflow endpoint in the kernel
- when a local-only published workflow is invoked remotely, the kernel/relay may
  tunnel the transport request and response between the remote terminal and the
  local publication server
- the relay remains transport-only and must not inspect workflow prompts,
  artifacts, outputs, or published transport payloads
- Cloud publication ingress is the public runtime ingress for deployed
  publications. It forwards HTTP, SSE, WebSocket, and MCP traffic to the active
  backend target and must preserve streaming semantics.
- Scalingo Cloud should not proxy runtime publication streams. It may create,
  list, start, stop, and observe deployment metadata, and the web terminal may
  embed `public_base_url` in the central panel.
- If the active backend is unavailable, transports return transport-appropriate
  unavailable responses: human HTTP unavailable page, API SSE structured
  unavailable event/error, WebSocket close reason, and MCP structured error.
- Hosted containers receive scoped deployment/runtime identity only. They must
  not receive a general Arroba Cloud user account token.
- Publication images and packages must not include provider credentials. Real
  provider hosted-container validation may use a staging credential profile
  mounted by the runner; arbitrary-user provider login and credential onboarding
  are a separate product phase.

Workflow output direction:

- a workflow run may emit zero or more outputs
- outputs are a run-level concept first; strict graph-level exit points are deferred
- entry and output may be handled by the same node when the workflow design requires it

Workflow/agent binding direction:

- creating a new agent MUST NOT implicitly add it to existing workflows
- deleting an agent MUST NOT implicitly remove workflow nodes or edges
- workflows should preserve nodes whose agents are missing and mark them unavailable until repaired

Queue and turn direction:

- each workflow agent should have an inbound queue
- turn start should use a kernel-owned `consume_input_messages` tool
- output validation should use a kernel-owned `validate_output_messages` tool
- workflow turn delivery acknowledgment should use a runtime-owned `ack_workflow_turn` operation
- a running turn should not re-open its input set mid-turn; newly arrived messages remain queued for a later turn

## 5.0 Capability, Session, Workflow, Security, and Versioning Details

Detailed capability API baseline, Workspace Live Sync coordination, provider control operations, session/attachment semantics, workflow contracts, security semantics, compatibility rules, versioning strategy, and cross-platform terminal conformance now live in [PROTOCOL_CAPABILITY_SESSION_WORKFLOW.md](PROTOCOL_CAPABILITY_SESSION_WORKFLOW.md). Keep this main protocol document focused on scope, lanes, native provider behavior, envelope shape, current transport baseline, and command/workflow message direction.
