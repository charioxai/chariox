# Protocol Capability, Session, Workflow, Security, and Versioning Details

Extracted from [PROTOCOL.md](PROTOCOL.md) to keep the main protocol overview below the line cap while preserving detailed capability, session, workflow, security, compatibility, versioning, and terminal-conformance notes.

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

## 5.0.1 Workspace Live Sync Coordination

The kernel owns Workspace Live Sync for Arroba-launched provider sessions. The global `providers.workspace_live_sync` config is a launch policy with values `off`, `managed`, or `tracked`. `off` is the default and leaves new launches unmanaged unless the session asks otherwise. The concrete sync mode is session-scoped and is changed with `SetWorkspaceLiveSyncMode { session_id, mode }`; successful changes return `WorkspaceLiveSyncModeUpdated { session }` and update the session projection. The wire enum may still carry internal `unrestricted` for Off.

Workspace Live Sync has two coordinated modes:

- **managed**: supported providers are configured so coordinated workspace files can only be changed through Arroba MCP/runtime tools; direct provider-native writes are denied inside synced roots, while repositories outside those roots remain editable through normal provider-native paths when Arroba has an active write fence for the provider process.
- **tracked**: provider-native file writes are allowed, but the kernel snapshots allowed workspace files during an Arroba-managed turn, computes changed-file diffs at turn end, and fans those changes out to attached Workspace Live Sync targets.

Provider hidden context is mode-specific: managed runs receive instructions to use Arroba runtime/MCP write tools inside synced roots, while tracked runs receive instructions that provider-native edits inside synced roots are allowed and observed at turn end. Both modes must tell the provider that unrelated repositories outside synced roots remain normal provider-native edit targets.

macOS hardening moves this from provider-specific policy to an Arroba-owned process launch boundary. Arroba-managed provider processes are launched behind a macOS workspace write fence that denies filesystem writes under selected protected roots only: the canonical selected worktree Git root plus explicitly attached local workspace-link roots. Provider state/cache/temp writes and writes to unrelated sibling repositories outside those roots remain allowed. Codex provider-native sandboxing remains enabled as defense in depth. OpenCode native tools that can write are enabled for managed mode only when this Arroba fence is active. Linux and Windows write-fence backends are deferred.

External provider endpoints are not a managed-runtime mode. A provider process must be launched by Arroba before Arroba can apply the workspace write fence or claim workspace live sync enforcement. Native TUI agents that bind an externally launched provider app-server are therefore not workspace live sync runs unless that process was launched behind the Arroba runtime boundary.

The current contract is:

- `arroba.read_artifact` returns content plus snapshot/version metadata.
- `arroba.write_artifact`, `arroba.edit_artifact`, `arroba.apply_patch`, `arroba.move_artifact`, and `arroba.delete_artifact` are synchronous managed writes.
- Runtime MCP also exposes short aliases `read_artifact`, `write_artifact`, `edit_artifact`, `apply_patch`, `move_artifact`, and `delete_artifact` because providers such as Codex qualify/sanitize MCP tool names (for example `mcp__arroba__read_artifact`).
- Runtime MCP exposes extension control-plane tools `arroba.list_extensions` / `list_extensions` and `arroba.request_extension` / `request_extension`. These list Arroba-managed MCPs, skills, scripts, and connectors visible to the current workspace and let the current agent request one by `kind` and `name`. For remote agents, valid home-owned grants are projected to the worker as a `RemoteExtensionManifest` with explicit `authority`, `definition_origin`, and `execution_location`. Home-proxy MCPs, scripts, and connectors execute on the home kernel via `InvokeHomeMcpProxy` or `InvokeHomeExtensionTool`; the worker cannot grant, revoke, widen safety, execute locally, or inspect home credentials. Each forwarded call carries `RemoteExtensionInvocationMetadata` (`invocation_id`, optional `provider_tool_call_id`, `attempt`, optional `idempotency_key`, and `started_at_ms`). `InvokeHomeMcpProxy` also carries the projected `RemoteExtensionTool` so home can reject stale MCP version/timeout hints before proxying the MCP request. Home validates the current grant, lease, session, agent, worker identity, active worker provider run, tool identity, placement, safety, timeout, and current version hash from home state before every invocation; worker-sent tool fields are hints only. Duplicate idempotent calls may replay a cached result, while duplicate non-idempotent calls are rejected. `CancelHomeExtensionInvocation` lets a worker best-effort cancel in-flight home execution when the leased prompt is cancelled; home audits the request, suppresses late successful completion for marked invocations, and reports whether the invocation was still in flight. Skills remain passive projected snapshots/materialized content; executable helpers must be exposed separately as MCPs, scripts, or connectors. Remote/slice credential tools use `InvokeHomeCredentialTool` for home vault operations and `ResolveHomeCredentialSecret` for scoped browser or PTY injection; the worker validates local slice/PTY targets, home validates the current leased-agent binding and credential policy, and secret material is returned only to the worker injection path, not to the provider transcript. Agent state includes `remote_extension_manifest_sync` (`pending`, `syncing`, `synced`, `failed`, or `stale`) plus manifest hash, timestamps, pending-revoke flag, and last error. `SyncRemoteExtensionManifest` retries projection, while `ListHomeExtensionAudit` returns recent durable home-extension grant/revoke/manifest/invocation events for the authorized home owner.
- Runtime MCP exposes slice saved-state tools `arroba.save_slice_state` / `save_slice_state` and `arroba.create_slice_backup` / `create_slice_backup` for slice agents with full user authority. If `slice_ref` is omitted, the home kernel infers the current agent's slice from the leased-agent binding. The worker never receives Docker socket authority; it sends `InvokeHomeSliceStateTool`, the home kernel validates the leased agent/session/worker/provider-run binding, and the home kernel performs Docker commit/archive work. Tool results contain only slice/state/backup metadata such as ids, image refs, archive paths, and status.
- Daemon health `remote_execution.issues` identifies malformed remote-agent bindings and actively working remote agents that do not have an `active_worker_provider_run_id`. Idle remote agents may legitimately have no worker run. Clients should surface the affected agent, worker kernel/machine, lease, leased agent, state, processing flag, and worktree so users can reconnect/relaunch the right remote or slice worker.
- Daemon health `remote_extension_sync.issues` identifies each home-proxy remote agent whose manifest is missing, failed, stale, or pending revoke by session, agent, worker kernel/machine, lease, leased agent, active worker provider run, state, hash, error, grants, and worktree so clients can direct the user to the exact `/extension sync-status` / retry target.
- text artifacts coordinate edited ranges, serialize same-area writes, rebase stale non-overlapping external changes, and reject stale overlapping external changes.
- non-text artifacts use `domain: "opaque"` and `content_base64`; they are coordinated as whole-file writes/moves/deletes.
- tracked mode syncs only at turn end. Changes made outside an Arroba-managed agent turn are ignored as origins, though they can still cause a target-side rebase or conflict.
- Workspace Live Sync never creates commits. Git history remains user/agent-owned.
- Daemon health `workspace_live_sync.workspace_identity.issues` identifies each managed/tracked provider run whose observed workspace identity changed by provider run id, root, generation, validity, baseline/current fingerprint, branch, head commit, and repo URL so clients can tell the user exactly which provider run needs relaunch.
- Daemon health `workspace_live_sync.external_changes.issues` identifies each tracked artifact changed outside Arroba after its last managed observation by artifact key, provider run id when still tracked, workspace fingerprint, workspace root, and path so clients can point the user at the exact file/turn to reconcile.
- `.arrobaignore` controls user exclusions and is initialized from `.gitignore` when present, otherwise empty. The kernel always force-excludes runtime/private paths such as `.git/**`, `.arroba/**`, `.arrobaignore`, `.env*`, provider state directories, sockets, session/history stores, dependency caches, and build outputs.
- Workspace links are the session sync-group primitive. `GetWorkspaceLiveSyncStatus` returns `sync_groups` derived from the session's workspace links plus flattened `targets`, `conflicts`, and ignore state for rendering.
- remote agents working in the same repo/branch or in worktrees attached to the same session workspace link forward workspace live sync through the home kernel. The home kernel owns snapshots, reservations, and conflict decisions, and the worker applies accepted final states only if its local artifact still matches the forwarded pre-apply state.
- cross-branch, cross-worktree, and cross-user/fork collaboration requires explicit workspace-link attachment. Session membership gates workspace-link creation, attachment, and status visibility.
- relay peers use `ApplyWorkspaceLiveSyncChange` / `WorkspaceLiveSyncChangeApplied` for tracked fanout. Per-path results are `applied`, `rebased`, `skipped_conflict`, or `failed_io`.
- conflicts are recorded and surfaced with source agent, target user/worktree, path, and next action. Resolver edits become new journal entries and continue fanout until all targets converge or a conflict remains unresolved.
- if the workspace identity changes during a managed or tracked run, the kernel rejects workspace live sync until the run rejoins a valid coordinated workspace.

Future coordination work still includes port claims and type-specific non-text artifact region models beyond opaque whole-file locking.

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
- workflow runtime tools such as `ack_workflow_turn` and `validate_workflow_handoff` SHOULD be exposed through an Arroba-managed MCP surface rather than direct daemon/kernel APIs
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
- nodes with indegree `> 1` MAY run once per incoming message by default
- nodes that must combine parallel branch outputs MUST opt into explicit barrier/fan-in state
- barrier/fan-in state MUST synchronize handoffs by source node iteration so faster loop branches do not get paired with slower branch outputs from a different iteration
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
- the daemon MUST expose a kernel-owned handoff validation tool to workflow nodes
- the runtime MUST expose a dedicated workflow-turn acknowledgment operation separate from output validation
- node-to-node handoff payloads SHOULD be validated against per-edge schema constraints before routing
- invalid handoffs MUST be rejected or flagged and surfaced back to the node as a validation error
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
- `handoff_schema_ref`
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
