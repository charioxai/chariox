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

Remote native TUI composition:

- remote native TUI mode MUST compose existing protocol paths rather than create a second prompt/runtime protocol
- provider-native TUIs and Arroba TUIs attach to the home kernel session through the same client/session attachment semantics used locally
- provider-native TUI prompts MUST enter the home kernel through the same `SubmitPrompt` path as Arroba prompts
- the home kernel MUST dispatch remote execution through the existing leased-agent relay path (`SubmitLeasedPrompt`, remote prompt attachments, remote MCP/skill checks, and related completion/cancel paths)
- the worker kernel MUST talk to the provider through the same kernel-provider adapter/server path used by ordinary worker-owned provider runs
- worker output, notices, completions, and permission interactions MUST return to the home kernel through existing leased runtime projection and native interaction relay paths
- the provider-native proxy/launcher MAY translate home-kernel session output back into provider-native UI protocol or PTY rendering, but it must not become a session authority or bypass the home kernel prompt queue
- the relay remains transport-only and must not inspect or transform provider-native prompts, outputs, attachments, permissions, or history
- slice-backed native TUI mode follows the same contract: provider TUIs and Arroba TUIs attach to the home kernel session, `slice_ref` selects a home-managed worker execution environment, and the slice worker uses the same worker-owned provider adapter/server path as remote leased agents

Native TUI MCP and skill placement:

- local native TUI provider runs use the same agent-scoped grant filtering as ordinary local provider runs, so only MCPs and skills granted to that agent are injected or rendered for that run
- standard home-worker native TUI does not install, copy, or otherwise coordinate MCPs/skills between the home and worker machines; the worker must already have matching MCP definitions, commands, environment, provider credentials, and any provider/Arroba skill material needed for the run
- when the home kernel can compute grant-derived remote MCP requirements, it MAY pass those requirements to the worker launch/prompt path so the worker can fail fast on missing or mismatched local worker definitions before provider execution
- slice-backed native TUI may synchronize Arroba skill packages from the home kernel to the child worker because the slice is home-managed; this is not a general remote-machine install mechanism
- slice-backed native TUI still executes MCP commands on the worker side, so MCP commands, environment, and credentials must be available in the slice image or injected slice environment
- capability grants remain agent-scoped in all modes; native TUI launch must not expose ungranted local/user MCPs or skills just because the provider CLI can see them natively

Native TUI permissions:

- provider-native permission requests MUST be represented as one agent-scoped, kernel-owned `RuntimeInteraction`
- that interaction MUST be projected to every Arroba TUI attached to the session, regardless of whether the current turn was submitted from an Arroba TUI or provider-native TUI
- answering from an Arroba TUI resolves the kernel interaction and the provider adapter/proxy forwards the resulting decision to the provider
- where a provider-native TUI can submit an approval response through a stable proxy or hook seam, the native response MUST resolve the same kernel interaction rather than bypassing it; first valid resolution wins
- if the provider only exposes the approval through a rendered PTY, Arroba may detect the rendered prompt and create the kernel interaction, then inject the resulting decision back into the PTY using the provider's native selection semantics

Native TUI hidden context:

- granted skill prompt context and other Arroba-only prompt injections MUST be delivered on the provider-facing path without becoming visible provider-TUI text
- Codex/OpenCode proxies redact Arroba hidden blocks from provider-TUI-facing protocol traffic while forwarding them to the provider server
- Claude Code native TUI MUST use the `UserPromptSubmit` hook `additionalContext` path for hidden context; the hook emits a scoped context request id, and the Arroba CLI bridge or worker kernel writes the matching context response before the hook returns
- Claude hook context responses are scoped to the session, agent, and provider run; they must not expose broad kernel authority or accept arbitrary provider-origin file paths
- if a Claude hook context response is unavailable before timeout, the provider-facing hidden context is empty and the native TUI remains coherent; Arroba MUST NOT fall back to visible PTY prompt injection for skill bodies or system prompt blocks
- local Claude native TUI can answer hook context requests through the launcher bridge and home kernel; remote/slice Claude native TUI answers them on the provider-execution side so worker-local or slice-isolated skill material is used

Provider-specific transport:

- Codex uses a native WebSocket proxy in front of a Codex app-server endpoint and binds the observed Codex thread to the Arroba provider run.
- OpenCode uses a native HTTP proxy in front of a launcher-managed `opencode serve` endpoint. The kernel binds its provider run to the proxy endpoint, while the provider TUI attaches to the same proxy/provider session.
- Claude Code has no stable provider UI/server split. Local and remote native TUI mode therefore use a kernel-owned PTY: the provider process runs where execution belongs, and the launcher streams/render-controls that PTY while the kernel projects prompts, output, attachments, status, and supported interactions back into the Arroba session.

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

## 5.0 Capability API Baseline

After M2, the local daemon API continues to expose structured capability requests in addition to session-runtime requests.

Current baseline capability request:

- `capability.shell.run`
- `capability.tree.read`
- `capability.file.read`
- `capability.file.edit`
- `capability.git.inspect`
- `capability.screenshot.capture`
- `capability.transfer.store`

Minimum shell request fields:

- `session_id`
- `attachment_id`
- `command`
- `args`
- optional `working_directory`
- optional `timeout_ms`

Minimum shell response fields:

- `session_id`
- `command`
- `args`
- `working_directory`
- `exit_code`
- `stdout`
- `stderr`

Capability failures MUST remain structured and MUST NOT corrupt PTY/session runtime state.
Shell capability requests MUST be validated against the requesting attachment and the session worktree boundary before execution.

Tree/file/git capability requests MUST remain scoped to the session worktree boundary and return structured results rather than raw transcript fragments.
Screenshot capture MUST write only into daemon-chosen session artifact locations.

## 5.0.1 Managed I/O Coordination

The kernel owns managed artifact I/O for Arroba-launched provider sessions. Supported providers are configured so coordinated workspace files can only be changed through Arroba MCP/runtime tools; direct provider-native shell/edit paths are denied for managed sessions.

macOS hardening moves this from provider-specific policy to an Arroba-owned process launch boundary. Arroba-managed provider processes are launched behind a macOS workspace write fence that denies filesystem writes under the canonical worktree path while still allowing provider state/cache/temp writes outside the worktree. Codex provider-native sandboxing remains enabled as defense in depth. OpenCode native shell may be enabled only when this Arroba fence is active. Linux and Windows write-fence backends are deferred.

External provider endpoints are not a managed-runtime mode. A provider process must be launched by Arroba before Arroba can apply the workspace write fence or claim managed-I/O enforcement. Native TUI agents that bind an externally launched provider app-server are therefore not managed-I/O runs unless that process was launched behind the Arroba runtime boundary.

The v1 contract is:

- `arroba.read_artifact` returns content plus snapshot/version metadata.
- `arroba.write_artifact`, `arroba.edit_artifact`, `arroba.apply_patch`, `arroba.move_artifact`, and `arroba.delete_artifact` are synchronous managed writes.
- Runtime MCP also exposes short aliases `read_artifact`, `write_artifact`, `edit_artifact`, `apply_patch`, `move_artifact`, and `delete_artifact` because providers such as Codex qualify/sanitize MCP tool names (for example `mcp__arroba__read_artifact`).
- Runtime MCP exposes extension control-plane tools `arroba.list_extensions` / `list_extensions` and `arroba.request_extension` / `request_extension`. These list Arroba-managed MCPs, skills, and scripts visible to the current workspace and let the current agent request one by `kind` and `name`. Script requests also require an already registered local `environment`. V1 auto-grants valid requests; newly requested MCPs trigger Arroba-managed provider conversation activation, and agent-triggered MCP requests continue via an automatic continuation prompt after the current turn. Newly requested skills are effective immediately because the request returns the full `SKILL.md` body by default. Script extensions expose a registered script as a runtime MCP tool; the script must live on the machine hosting the agent and runs in the registered external environment. Remote worker agents forward these extension calls to the home kernel where applicable; standard home-worker mode does not copy scripts or environments across machines.
- text artifacts coordinate edited ranges, serialize same-area writes, rebase stale non-overlapping external changes, and reject stale overlapping external changes.
- non-text artifacts use `domain: "opaque"` and `content_base64`; they are coordinated as whole-file writes/moves/deletes.
- remote agents working in the same repo and branch forward managed I/O through the home kernel. The home kernel owns snapshots, reservations, and conflict decisions, and the worker applies accepted final states only if its local artifact still matches the forwarded pre-apply state.
- if the workspace identity changes during a managed run, the kernel rejects managed I/O until the run rejoins a valid coordinated workspace.

Future coordination work still includes port claims, an explicit user command for unsafe/uncoordinated mode, and type-specific non-text artifact region models beyond opaque whole-file locking.

## 5.1 `attach_file`

Request payload:

- `session_id`
- `provider_run_id`
- `absolute_path`
- optional `display_name`
- optional `mime_type`

Response payload:

- on success: `attachment_ref` (provider-specific), optional metadata
- on unsupported: `unsupported: true`
- on error: structured error object

## 5.2 `request_memory_update`

Purpose:

- daemon-driven memory-refresh inquiry (out-of-band from terminal prompts)

Request payload:

- `session_id`
- `provider_run_id`
- `reason_code` (`compaction_detected` | `user_requested_refresh` | `before_provider_switch` | custom)
- optional `policy_hint` (`recency_only` | `full_update` | custom)

Response payload:

- on success: structured memory update payload (recency summary, continuity markers, provider-supported hints)
- on unsupported: `unsupported: true`
- on error: structured error object

Failure contract:

- errors must not terminate provider run by default
- daemon falls back to Arroba-managed memory data

## 5.3 `request_compaction_summary`

Purpose:

- daemon-driven compaction-summary inquiry for user-triggered Arroba compaction

Request payload:

- `session_id`
- `provider_run_id`
- `reason_code` (`user_triggered_compact` | custom)
- optional `policy_hint` (`brief` | `detailed` | custom)

Response payload:

- on success: structured compaction summary payload
- on unsupported: `unsupported: true`
- on error: structured error object

Failure contract:

- errors must not terminate provider run by default
- daemon may fallback to Arroba-managed memory summary for warm-start

## 5.4 Provider Command Compatibility

Provider adapters SHOULD report command compatibility state to the daemon.

Minimum fields:

- `provider`
- `detected_version`
- `catalog_version`
- `support_status` (`supported` | `best_effort` | `unsupported_not_installed`)
- optional `warning`

Behavior rules:

- Arroba ships built-in command catalogs for supported provider version families
- Arroba MAY augment those catalogs by reading supported custom-command files/config
- Arroba SHOULD NOT rely on scraping human-oriented slash help output as the primary discovery mechanism
- if the detected version is unsupported, Arroba warns but keeps best-effort `/<provider> ...` completions enabled

## 5.5 Provider Authentication

Provider authentication remains provider-native and local to the execution host.

Behavior rules:

- Arroba reuses the local login/session state of the native provider CLI when available
- Arroba MUST NOT store or relay provider credentials in v1
- `account_profile` selects a provider-native local profile or config context
- if the provider is not logged in, the daemon reports structured auth state instead of attempting to silently fall back to API credentials
- remote clients may view provider-auth status, but provider login itself remains a host-local provider-native flow

## 5.6 Extensions and MCP Runtime

Arroba manages extension installation and per-agent binding through structured daemon-owned APIs.

Behavior rules:

- installation is machine-scoped; binding is agent-scoped
- provider-facing extension files or config are generated projections, not the canonical source of truth
- MCP servers are daemon-managed runtime components and SHOULD be exposed only to the top-level provider runs they are bound to
- provider-native subagents are not separate extension-binding targets in v1
- workflow runtime tools such as `ack_workflow_turn` and `validate_workflow_output` SHOULD be exposed through an Arroba-managed MCP surface rather than direct daemon/kernel APIs
- for managed provider runs, MCP attachment SHOULD be automated by Arroba at launch time rather than delegated to end-user provider setup
- day-1 implementation MAY statically advertise the workflow runtime tools and enforce turn/schema scope at call time; dynamic per-turn tool advertisement is a later hardening step
- workflow-console MCP tools SHOULD use the same split:
  - transport owns the MCP surface, authentication, and provider-run scoping
  - scheduler/runtime owns workflow-scoped console semantics and state

Workflow console tool family:

- `workflow_console_read`
- `workflow_console_write`
- `workflow_console_clear`

Behavior rules:

- one append-only console exists per workflow definition
- nodes MAY read, write, and clear only the console for their workflow
- console content is rendered as-is for the workflow terminal view and is not kernel-curated
- console content is not a control channel and does not replace mailbox, handoff, or audit state

## 6. Error Model

Structured error shape:

- `code`
- `message`
- optional `retryable`
- optional `details`

Error isolation rules:

- capability/control errors are reported separately from PTY terminal output
- unsupported control operations are valid compatibility outcomes

## 7. Session and Attachment Semantics

- multiple attachments can participate in the same session concurrently
- any attachment may submit prompts or request supported config changes
- the daemon is the source of truth for prompt queue state and session config state
- queued prompts MUST be surfaced to all other attachments in the session through structured events or equivalent state sync
- config changes accepted by the daemon MUST be propagated to all attachments in the session

Suggested events:

- `session.attachment.joined`
- `session.attachment.left`
- `session.prompt.queued`
- `session.prompt.started`
- `session.prompt.completed`
- `session.config.updated`
- `session.notice`
- `session.provider_run.changed`

## 7.1 Multi-Agent Session Semantics

When a session runs in multi-agent session mode:

- the daemon MUST maintain a canonical list of top-level session agents
- one top-level agent MAY be marked focused for direct user interaction
- prompt submission, runtime notices, and provider output intended for direct interaction SHOULD be agent-scoped
- the daemon SHOULD treat the focused agent as the direct prompt target for user-submitted prompts
- provider runs SHOULD be associated with specific top-level agents rather than only with the session at large
- pane-capable clients SHOULD be able to render one visible sub-area per top-level agent using daemon-owned state and agent-scoped events

Suggested events:

- `session.agent.spawned`
- `session.agent.destroyed`
- `session.agent.focused`
- `session.agent.cycled`

## 7.2 Workflow Semantics

When a session runs in multi-agent workflow mode:

- the daemon MUST treat the workflow as a general directed graph
- execution policy MUST be derived from the graph rather than from a separate user-declared topology label
- nodes with indegree `> 1` require explicit barrier/fan-in state
- nodes with outdegree `> 1` are branching points and may release outputs to multiple children
- cycles are a separate graph property and require bounded-cycle handling independent of input/output synchronization policy

Required runtime entities:

- `WorkflowDefinition`
- `WorkflowNode`
- `WorkflowEdge`
- `WorkflowRun`
- `NodeRun`
- `NodeMessage`
- `WorktreeAssignment`
- `AggregationState` or equivalent barrier/fan-in state

## 7.2.1 Workflow Runnable Validation

Before starting a workflow run:

- the daemon MUST validate that the endpoint exists and targets a valid entry node
- the daemon MUST reject invocations that reference missing nodes or missing agents
- the daemon SHOULD return a structured preflight report that enumerates blocking issues

## 7.3 Node Completion Contract

Each workflow node MUST emit a structured completion report that the daemon can parse.

Minimum fields:

- `workflow_run_id`
- `node_run_id`
- `status`
- `summary`
- optional explicit `output`
- optional artifact references or changed files
- `stop_recommendation`

Suggested event:

- `workflow.node.completed`

The daemon scheduler MUST advance workflow execution from these completion reports.

## 7.3.1 Node Instructions and Output Validation

Workflow nodes require daemon-owned instruction and validation surfaces.

Required rules:

- the daemon MUST maintain per-node instruction content used as system or preamble context
- the daemon MUST maintain an optional workflow-level prompt used as shared context for all nodes
- the daemon MUST provide a stable reference so nodes can reload instructions after compaction
- workflow prompt injection MUST be rendered by the scheduler-owned prompt injection layer, not by provider adapters, transport handlers, CLI scripts, or per-dispatch ad hoc string assembly
- local workflow dispatch, remote workflow dispatch, retry/replay dispatch, and tests MUST enter the same prompt renderer so turn index, last-turn guidance, final-output tool guidance, runtime-tool instructions, handoff payloads, edge contracts, and control mailbox content stay consistent
- the daemon MUST expose a kernel-owned output validation tool to workflow nodes
- the runtime MUST expose a dedicated workflow-turn acknowledgment operation separate from output validation
- node completion output SHOULD be validated against per-edge schema constraints before routing
- invalid output MUST be rejected or flagged and surfaced back to the node as a validation error
- validation failures SHOULD follow daemon policy (warn-and-continue vs halt-run) and MAY be configured per edge, with `warn` as the default

Workflow turn durability rules:

- the runtime MUST persist the rendered workflow turn envelope, including workflow-level prompt text, mailbox content, and handoff payloads, before dispatch
- dispatch success alone MUST NOT make transient workflow inputs eligible for deletion
- transient workflow inputs MUST remain retained until the turn reaches a validated terminal state
- the validated terminal state is reached only after:
  - the node has acknowledged the turn, and
  - the node turn has completed, and
  - final output validation has passed
- if the provider disconnects or becomes unreachable before that state, the runtime MUST retain enough dispatch state to retry or reconcile the turn safely

Workflow failure rules:

- failures MUST be represented as structured workflow failure events, not just free-form notices
- each workflow failure event MUST include:
  - `kind`
  - `source_node_run_id`
  - `edge_ids`
  - `message`
  - `timestamp_ms`
- the runtime MUST support at least two failure policy modes:
  - `none`
  - `notify`
- the default mode SHOULD be `notify`
- under `notify`, edge-related failures MUST be added to the mailbox of:
  - the source node
  - sink-side nodes on the affected edges
- node-local failures without affected edges SHOULD notify the source node only
- structured workflow failure events MUST be retrievable through runtime APIs

Resume rules:

- when a workflow run stops mid-execution, active node runs with preserved turn envelopes MUST be resumable
- resuming a workflow run MUST preserve mailbox content, handoff payloads, and rendered prompt text for those node runs
- the runtime does not need to reconstruct broader conversational history for this feature; agents continue from preserved workflow-turn context

Suggested workflow turn runtime states:

- `prepared`
- `dispatched`
- `acknowledged`
- `validated_completed`
- `cancelled`
- `failed`

## 7.4 Handoff Contract

Outputs from one node MUST be transformed by the daemon into a structured handoff payload before delivery to the next node.

Suggested payload fields:

- `workflow_run_id`
- `source_node_run_id`
- `target_node_id` or `target_node_run_id`
- `message_type`
- `summary`
- `output`
- `artifacts`
- `output_schema_ref`
- `handoff_payload`
- `meta`

Suggested event:

- `workflow.node.handoff`

## 7.5 Graph-Derived Barrier and Release Semantics

Default v1 model:

- synchronization behavior is a per-node concern, not a workflow-wide topology label
- `input_gate` controls when a node becomes runnable:
  - `first_input`
  - `all_inputs`
- `output_release` controls when validated outputs are released downstream:
  - `on_completion`
  - `immediate`

Default derivation rules:

- if a node has indegree `<= 1`, the default `input_gate` is `first_input`
- if a node has indegree `> 1`, the default `input_gate` is `all_inputs`
- the default `output_release` is `on_completion`

Required rules:

- barrier/fan-in state MUST be tracked by the daemon on the receiving side
- output release decisions MUST be daemon-owned even when a node emits output earlier
- outputs may be placed into the target node's inbound queue before that node becomes runnable
- a node with `all_inputs` gating MUST NOT start until the required parent outputs for that activation are present
- cycles require bounded-cycle policy and MUST NOT be conflated with barrier semantics

Suggested events:

- `workflow.run.started`
- `workflow.run.completed`
- `workflow.node.started`
- `workflow.node.waiting`
- `workflow.node.failed`
- `workflow.aggregation.updated`

## 8. Security Semantics

## 8.1 Encryption

For remote transport, user-generated payloads (terminal/capability/memory transfer artifacts) should be session-E2E encrypted.

## 8.2 Relay behavior

Server should relay opaque encrypted payloads and avoid plaintext dependency.

## 8.3 Metadata minimization

Only operational metadata required for discovery/presence/scheduling should be stored server-side in v1.

## 9. Compatibility Rules

- provider adapters may support PTY-only operation
- control-lane unsupported responses are expected and non-fatal
- protocol evolution should be additive within `v1` where possible

## 10. Versioning Strategy

- every structured message carries `version`
- breaking changes require a new major protocol version
- new optional fields/capabilities may be introduced as backward-compatible extensions

## 11. Cross-Platform Terminal Conformance Profile

To keep behavior consistent across CLI, web, desktop, and mobile clients:

- terminal behavior is specified by protocol semantics, not by one UI framework
- clients may be implemented in platform-native languages and toolkits
- clients should conform to shared expectations for:
  - PTY byte-stream fidelity
  - terminal resize behavior (`terminal.resize`)
  - key/input mapping (`terminal.input`)
  - output rendering/control-sequence handling (`terminal.output`)

Reference model:

- xterm.js serves as the reference behavior baseline for web/remote surfaces
- recommended platform hosts for xterm.js-based clients:
  - Web: browser runtime
  - iOS: `WKWebView`
  - Android: `android.webkit.WebView`
  - macOS: `WKWebView` in AppKit/SwiftUI container
  - Windows: WebView2
  - Linux desktop: embedded Chromium/WebKit container
- non-web clients should be validated against the same conformance suite and snapshot expectations
