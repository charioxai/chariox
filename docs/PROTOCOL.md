# Arroba v1 Protocol

## Status

Draft protocol aligned with `docs/spec-v1.md`.

## 1. Scope

This document defines message classes and protocol contracts between:

- clients
- server relay
- daemon
- provider adapters

It is intentionally transport-agnostic at the message level (WebSocket recommended for remote relay).

## 2. Design Principles

- preserve native provider interaction semantics, using PTY passthrough where required and structured local provider protocols where they are stronger and officially supported
- reserve `/...` as the Arroba command namespace
- keep structured control surface intentionally small
- isolate capability/control errors from terminal stream
- ensure user-generated in-transit payloads are session-E2E encrypted on remote transport

## 3. Protocol Lanes

## 3.1 Terminal Lane (Provider Output Stream)

Purpose:

- user keystrokes to provider PTY
- provider stdout/stderr/control sequences to clients

Semantics:

- byte-stream-like behavior
- no requirement for structured parse by Arroba for ordinary non-command traffic
- for providers with structured event streams, Arroba MAY render provider output into the client terminal without treating PTY bytes as the source of truth for turn lifecycle

Suggested events:

- `terminal.input`
- `terminal.output`
- `terminal.resize`

OpenCode-specific note:

- OpenCode should graduate from PTY-polled `terminal.output` to adapter-fed output derived from its local event stream
- incremental assistant text should come from provider message-part delta events
- terminal rendering remains daemon-owned even when the source is a structured provider event stream

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
- `/agent ...` is resolved against the active provider command catalog
- ordinary non-command input continues through `terminal.input`
- unsupported provider versions MAY produce warnings, but MUST NOT disable best-effort `/agent` completions by default

## 3.3 Control Lane (Structured Daemon->Provider Adapter)

Canonical operations in v1:

- `attach_file`
- `request_memory_update`
- `request_compaction_summary`

These operations are not typed by users into ordinary terminal traffic.

Arroba MAY route `/agent ...` invocations into the control lane after resolving the active provider command catalog.

OpenCode-specific structured adapter contract:

- prompt submit maps to the provider session prompt operation
- `/agent ...` command invoke maps to the provider session command operation
- turn abort maps to the provider session abort operation
- provider lifecycle and output state are consumed from the provider event stream rather than inferred from PTY EOF or PTY idleness

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

- `version` (protocol version, e.g. `v1`)
- `lane` when applicable (`capability` | `control`)
- `type` (event/action identifier)
- `request_id` when request/response matching is needed
- `session_id`
- `provider_run_id` when provider-scoped
- `workflow_run_id` when workflow-scoped
- `node_run_id` when node-scoped
- `target_node_id` or `target_node_run_id` when routing workflow handoffs
- `payload`
- `meta` (timestamps, source attachment id, trace id)

## 4.1 Local Daemon API Baseline

For the current local baseline, the daemon exposes a local-first request/response surface over local IPC.

Minimum request set:

- `session.create`
- `session.attach`
- `session.detach`
- `provider_run.launch`
- `session.state.get`
- `session.notice.poll`
- `prompt.submit`
- `prompt.complete`
- `session.config.update`
- `terminal.output.poll`
- `terminal.resize`
- `session.end`

Minimum response/result shapes:

- session creation returns structured session metadata
- attach/detach returns structured attachment metadata
- provider launch returns structured provider-run metadata
- session state reads return canonical queue and config state
- notice polling returns structured daemon notices scoped to the requesting attachment within the session
- prompt submission returns structured prompt status (`started` or `queued`) plus canonical session state
- prompt completion returns structured completion details and the next started prompt when relevant
- config update returns canonical session config state, version, and updated session state
- terminal output polling returns structured terminal-output fan-out records
- end-session returns structured final session metadata

This local API MUST remain daemon-owned, local-first, and compatible with later workflow-mode runtime surfaces.

Current M2 runtime note:

- the local daemon transport is a daemon-owned Unix-socket IPC path on Unix-like systems
- the local CLI is a transport client layered on top of this request/response surface rather than owning runtime logic directly
- the in-process harness remains useful for daemon smoke coverage, but it is no longer the primary local user path

OpenCode-specific next-step note:

- the current M2 baseline still uses PTY-backed OpenCode prompt/output flow
- the next OpenCode adapter revision should switch to provider-native session and event APIs while preserving the same daemon-owned local request/response surface for Arroba clients

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

Planned extension metadata fields:

- `extension_id`
- `type` (`skill` | `mcp_server` | `command_pack` | `instruction_pack` | `hook`)
- `source`
- `version`
- `provider_support`
- `visibility_policy`
- `install_state`

## 5. Control Operations

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
- if the detected version is unsupported, Arroba warns but keeps best-effort `/agent` completions enabled

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

## 7.1 Workflow Semantics

When a session runs in multi-agent workflow mode:

- the daemon MUST treat the workflow as a general directed graph
- v1 validators may accept only `circular` and `hierarchical` topologies
- every workflow MUST declare a coordinator node
- the coordinator MUST receive the initial user prompt
- the coordinator MUST decide final continue, stop, or completion behavior

Required runtime entities:

- `WorkflowDefinition`
- `WorkflowNode`
- `WorkflowEdge`
- `WorkflowRun`
- `NodeRun`
- `NodeMessage`
- `WorktreeAssignment`
- `AggregationState` or equivalent barrier/fan-in state

## 7.2 Node Completion Contract

Each workflow node MUST emit a structured completion report that the daemon can parse.

Minimum fields:

- `workflow_run_id`
- `node_run_id`
- `status`
- `summary`
- `artifacts` or changed files
- `handoff_payload`
- `stop_recommendation`

Suggested event:

- `workflow.node.completed`

The daemon scheduler MUST advance workflow execution from these completion reports.

## 7.3 Handoff Contract

Outputs from one node MUST be transformed by the daemon into a structured handoff payload before delivery to the next node.

Suggested payload fields:

- `workflow_run_id`
- `source_node_run_id`
- `target_node_id` or `target_node_run_id`
- `message_type`
- `summary`
- `artifacts`
- `handoff_payload`
- `meta`

Suggested event:

- `workflow.node.handoff`

## 7.4 Topology and Barrier Semantics

Circular topology rules in v1:

- each node has one incoming edge and one outgoing edge
- the final node routes back to the coordinator
- execution is serialized
- the workflow uses bounded iteration or round limits

Hierarchical topology rules in v1:

- the workflow forms a rooted tree
- child branches may run in parallel
- parent fan-in waits for all children by default
- results propagate upward through structured aggregation

Implementation priority note:

- circular topology should be implemented and stabilized first
- hierarchical topology should follow later in v1 on top of the same generic workflow engine

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
