# Chariox v1 Protocol

### Hosted terminal renewal (local protocol 472)

`RelayStatus.capabilities` advertises `terminal_relay_authorization_renewal_v1`.
Key-bound Cloud terminals probe this capability before relying on background
renewal through `IssueCloudRelayClientToken`. This contract preserves the
requested terminal subject, recipient key and exact target. Kernels without the
capability require an explicit kernel upgrade; their login-client tokens must
never substitute for a terminal grant. Transient target loss reconnects and
retries within the admitted grant lifetime. The existing relay peer protocol
and `client_connect` frames remain unchanged.

Client grants retain the existing 30-minute lifetime, including keyed issuance
and renewal. Initial `/relay cloud client-token` commands issued by a local
account-linked kernel carry `--relay-token-issuer LOCAL_ENDPOINT ISSUER_DAEMON_ID`.
The shared client authenticates that local endpoint from the same private CLI
profile, checks the issuing kernel ID and renewal capability, and renews the
admitted subject, key, session and target through that kernel. Runtime requests
and events still travel directly to the target over the encrypted relay lanes.
A machine-only managed target must never replace the account issuer.

If the original issuer is unavailable, the client displays a notice and retries
within the existing grant lifetime without changing authority or dropping the
admission. Recovery resumes renewal; reaching the original expiry ends the
session with an issuer-unavailable message. Authorization refusal still ends
admission promptly. The issuer route is public endpoint/ID metadata, not a
credential, and accepts only local Unix or loopback WebSocket endpoints. Moving
the launch command to another machine does not transfer the issuing profile or
make its local endpoint reachable. Legacy launch commands without issuer metadata
cannot automatically discover an account issuer from a machine-only target;
they retain their original lifetime with the existing warning/until-expiry path.


### MP-11 F7 public provider-run boundary (local protocol 435)

All client-facing provider-run responses (single/batch launch, read, selection,
external import and fork) and terminal snapshot/change events use
`PublicProviderRun`. This allowlist carries identity, selection, usage, state,
control capabilities, public import metadata and native session binding. It
omits provider launch environment/arguments, MCP authorization/configuration,
private resume payloads, launch manifests and free-form diagnostics. Native TUI
structured endpoints are projected only when the URL has no userinfo, query or
fragment. Internal persistence and authenticated worker launch/projection
contracts retain `RuntimeProviderRun`; no relay-peer shape changes in this
revision (peer protocol remains 70 on this branch).

The focused synthetic protocol drill is
`apps/cli/scripts/public-provider-run-protocol-drill.mjs`. It runs the Rust DTO,
response/event and private persistence regressions with a prebuilt kernel test
binary; it neither invokes provider accounts nor establishes live parity.


## Status

Draft protocol aligned with `docs/spec-v1.md`.

Apps Phase 1 protocol numbers were renumbered above release F on 2026-10-03 (local
N → N + 9 for 368–406, relay 58 → 69); see [PROTOCOL_PHASE1_RENUMBERING.md](PROTOCOL_PHASE1_RENUMBERING.md).

## MP-08 / MP-11: user-domain grant contract (local 432 / relay 78)

The owner-approved multidomain access contract is specified in
[MULTIDOMAIN_USER_DOMAIN_ACCESS.md](MULTIDOMAIN_USER_DOMAIN_ACCESS.md).
MP-11: authenticated human-terminal focus grants resource-scoped authority.
Agent-originated focus, attach, cycle and alias-routed prompts preserve session
orchestration without creating or refreshing browser/Notes grants. Changing
human focus does not revoke prior
holders. Existing kernel turn/wake state retains grants until fully idle expiry,
session/agent end or explicit revocation. Authenticated owner terminals use
`KernelBrowser` operations `list_grants`, `subscribe_grants` and `revoke_grants`;
agent tools cannot invoke these owner controls. Snapshots carry the live cursor,
holders and non-focused-use notice. Revocation cancels grant epochs and idle
subscriptions. MP-11: retained holders have the same input and mutations as
focused agents on granted resources, including typing, keys, Tab and clicks.
Explicit start/open is allowed; open grants its newly created tab. Claiming an
unrelated existing resource or loading a capability requires focus. Vault,
protected regions, sensitive approvals and App/passkey validation keep their
shared protections. Ordinary input has no retained/focused classification;
revoke and idle lapse still cancel authority immediately. Observation reads,
including state, never start/restart Chromium or its controller for any caller;
stopped/unavailable reads require explicit start/open (`browser_unavailable`).
Browser/App window projections identify the owning kernel and focused-agent
reachability; cross-kernel control is refused. Consumers of these new fields and
commands require local 432. Existing multidomain feature minima remain 427.
The focused MP-10 drill and protocol snapshots cover these changes; they do not
alone establish ordinary/managed parity or official-provider wait behavior.

MP-08 / MP-11: ordinary text and text-producing key events use the same
Vault-only protected-target check, including password/OTP fields, focused
frames and open nested shadow fields, for both focused and retained holders.
Successful browser results are bound to the exact admission epoch under the
grant lock before resource/subscription registration or inventory projection;
revocation followed by refocus cannot adopt an old call's result into a fresh
grant. A final live cancellation/provider-run check fences returned results.

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
- reserve `/...` as the Chariox command namespace
- keep structured control surface intentionally small
- isolate capability/control errors from terminal stream
- ensure all user-generated in-transit payloads are session-E2E encrypted on remote transport, including prompts, workflow inputs/outputs, and transferred/attached artifacts
- this requirement applies equally to:
  - self-hosted relay deployments
  - any later managed relay deployment
- relay must only ever see opaque encrypted payloads plus the minimum metadata required for routing and liveness
- serialized instants must be absolute UTC values: RFC 3339 strings use a `Z` suffix and numeric
  timestamps are Unix epoch values; timezone-free wall-clock strings are not protocol instants
- an IANA timezone is carried only when the timezone is part of the operation's semantics, such as
  a recurring cron schedule; fixed UTC offsets are not timezone identities

Current sequencing note:

- OpenCode is the reference provider for the current development cycle
- protocol and adapter boundaries should stay future-compatible, but they should not be generalized prematurely at the expense of finishing the OpenCode-first runtime
- web/mobile clients come before multi-provider expansion in the current rollout order
- same-kernel remote clients should fit the same kernel-owned protocol rather than a separate remote-only API
- same-kernel remote agents remain part of the architecture, but their generic transport contract is intentionally deferred until Chariox has integrated more than one concrete agent family

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
- no requirement for structured parse by Chariox for ordinary non-command traffic
- for providers with structured event streams, Chariox MAY render provider output into the client terminal without treating PTY bytes as the source of truth for turn lifecycle
- for same-kernel remote clients, the terminal lane should still be kernel-routed; relay changes the path, not the workspace authority

Suggested events:

- `terminal.input`
- `terminal.output`
- `terminal.resize`

`terminal.resize` carries the session id, dimensions, and an optional provider-run id. General
Chariox clients may omit the provider-run id to resize the session's active run. Provider-native
clients must target their own provider run so concurrent native terminals cannot resize each
other. When that run is projected from a leased worker, the home kernel forwards the resize to
the worker PTY and reports success only after the worker applies the requested dimensions.
Clients retry the latest dimensions after a transient transport failure.

OpenCode-specific note:

- OpenCode should graduate from PTY-polled `terminal.output` to adapter-fed output derived from its local event stream
- incremental assistant text should come from provider message-part delta events
- terminal rendering remains kernel-owned even when the source is a structured provider event stream
- the current protocol surface should be proven against OpenCode first before new provider families drive further adapter generalization

## 3.2 Capability Lane (Structured Daemon Actions)

Purpose:

- daemon-owned operations invoked from Chariox slash-command dispatch

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

- `/...` is parsed by Chariox before PTY forwarding
- `/<provider> ...` is resolved against the focused provider command catalog
- ordinary non-command input continues through `terminal.input`
- unsupported provider versions MAY produce warnings, but MUST NOT disable best-effort `/<provider> ...` completions by default

## 3.3 Control Lane (Structured Daemon->Provider Adapter)

Canonical operations in v1:

- `attach_file`
- `request_memory_update`
- `request_compaction_summary`

These operations are not typed by users into ordinary terminal traffic.

Chariox MAY route `/<provider> ...` invocations into the control lane after resolving the focused provider command catalog.

OpenCode-specific structured adapter contract:

- prompt submit maps to the provider session prompt operation
- `/<provider> ...` command invoke maps to the provider session command operation
- turn abort maps to the provider session abort operation and retains the provider session
- after abort, unscoped session errors settle a follow-up immediately only with matching current-prompt assistant evidence; if the accepted current user has no assistant and remains observably idle for five seconds, the kernel closes the stalled turn with an uncorrelated-error diagnostic
- provider lifecycle and output state are consumed from the provider event stream rather than inferred from PTY EOF or PTY idleness
- later providers such as Claude Code and Codex should fit behind the same daemon/client contract after the OpenCode-first cycle is closed

Provider hidden-context injection contract:

- Prompt submission from the kernel to a provider adapter is conceptually a `PromptEnvelope`, not one concatenated string.
- `visible_user_prompt` is the only prompt body that may be shown in Chariox prompt blobs, terminal input history, native provider prompt boxes, or user-facing prompt echoes.
- `hidden_system_context` carries Chariox runtime/system prompt material: runtime instructions, Workspace Live Sync managed/tracked instructions, native permission rules, workflow-level prompts, node-level instructions, granted capability summaries, continuation instructions, and utility-call instructions.
- `attachments` remain structured prompt attachments and are not used to smuggle hidden system instructions.
- `manifest` records prompt template ids, template hashes or versions, assembly conditions, and the provider injection channel selected for the turn; the manifest is audit/debug metadata, not prompt UI content.
- Chariox MUST NOT implement hidden context by prepending text to `visible_user_prompt` and later redacting it from UI surfaces.
- The relay MUST treat prompt envelopes as opaque encrypted payloads and MUST NOT inspect, transform, redact, or split visible versus hidden prompt fields.

Provider adapter hidden-context channels:

- Codex adapters MUST send hidden context through `thread/start.developerInstructions` or `thread/resume.developerInstructions` when a Codex thread is created or resumed. Codex does not accept this context through `turn/start`; for kernel-managed Codex runs, the kernel MUST wait for the managed thread to become idle, unsubscribe, and resume the same Codex thread before a turn when the assembled hidden context fingerprint changes, preserving its conversation.
- OpenCode adapters MUST send turn-scoped hidden context through the provider session prompt request `system` field, currently `POST /session/{id}/prompt_async` body `system`.
- Claude Code adapters MUST send turn-scoped hidden context through the `UserPromptSubmit` hook response `hookSpecificOutput.additionalContext`.
- If a provider channel is unavailable, the adapter may run without hidden context for that turn or restart the provider process with an initialization-scoped system prompt only when the caller explicitly accepts that behavior; it must not silently fall back to visible prompt injection.
- Live provider drills validate direct provider hidden-context channels in current supported harnesses. Prompt assembly changes that touch these channels must keep or update `pnpm --filter @chariox/cli run provider-context-injection:drill`.
- End-to-end prompt assembly changes must also keep `pnpm --filter @chariox/cli run prompt-assembly:drill` passing. That drill edits a temporary `~/.chariox/prompts/runtime/base.md`, runs real Chariox provider turns for Codex/OpenCode/Claude, verifies the model sees the hidden registry token through the provider-native hidden channel on successive turns, and verifies Chariox user-prompt history does not contain the hidden token.

Failed requests (protocol 393):

- A turn that fails before completing (provider error, rate or usage limit, crash, or a dispatch the provider did not accept) is not retried: its request is dropped. A user cancel keeps its own semantics and adds nothing.
- The failed turn gets a provider-error transcript entry, `Request not carried out: <reason>. It was dropped; send it again to retry.`, on every surface; provider rate, usage and billing limits read `usage limit reached`.
- The request stays on the agent as `AgentInstance.failed_requests` (`{prompt_id, excerpt, reason}`, omitted when empty, durable across kernel restarts). The next turn delivered to the agent's provider, which may resume a session still holding the request (Claude `--resume`, a Codex thread, an OpenCode session), carries a one-time `hidden_system_context` note: `Your previous request ("<excerpt>") failed (<reason>) and was not carried out. Do not act on it unless the user asks again; answer only the current request.` The field clears once the provider accepted that turn.

Prompt template storage:

- Chariox prompt templates are user-owned markdown files under `~/.chariox/prompts`.
- Source-controlled defaults may be materialized there for first run, but runtime assembly reads from the registry path rather than hardcoding prompt text in adapter code.
- Required templates include runtime base instructions, Workspace Live Sync managed instructions, Workspace Live Sync tracked instructions, native permission instructions, slice runtime instructions, MCP/skill continuation instructions, workflow turn/completion/intermediate-output templates, and utility-call templates.
- Cloud editing, if introduced later, edits this registry model and must not create a second prompt source of truth.

Provider-local visibility caveat:

- Chariox UI and protocol prompt blobs must hide `hidden_system_context`, but provider-local histories may still store it in provider-native form.
- Current provider harnesses expose hidden context in internal histories/transcripts: Codex history APIs, OpenCode message `info.system`, and Claude transcript `hook_additional_context`.
- The protocol guarantee is therefore “not visible in Chariox/native prompt input surfaces,” not “unrecoverable from provider-owned local state.”

## 3.3.1 Agent Endpoint Direction

Longer-term agent runtimes compatible with Chariox should speak a daemon-facing endpoint contract rather than requiring the daemon to launch only local child processes.

Required properties:

- bidirectional messaging
- explicit prompt or turn lifecycle
- explicit tool/runtime events
- health and capability advertisement

Existing providers like OpenCode may continue to be adapted through their native protocols.

## 3.3.2 Native TUI Agents

MP-08 / MP-10 / MP-11 (owner decision 2026-10-02): credential copying remains
unchanged across leased workers, managed-context imports and slices, including
profile-scoped Claude Keychain export and the Vault setup-token remote path.
Receiving kernels publish one non-blocking notice per renewable account and
Machine about possible refresh-token invalidation. API keys and Claude setup
tokens do not receive it. There is no credential sync or shared refresh authority.
A failed native renewal raises one kernel-owned `Log in to <Provider> on this
machine` RuntimeInteraction. Accepting invokes the existing official login on
the execution kernel; login success reloads the official harness and resumes the admitted Chariox turn.
Protocol v376 adds optional ephemeral `RuntimeInteraction.provider_login`
(receiving Kernel ID, official `ProviderLoginStart`, bounded terminal output).
Challenges and masked native responses are human-only, excluded from model
context, history and logs. Relay peer v68 updates/dismisses that projection
through the existing lease-authorized native-interaction bridge. Standalone
kernels use the same local path; no external auth coordinator is required.
Existing clients retain their minimum versions; rendering this optional login
projection requires v376. Older peer kernels reject the changed bridge version.

Native TUI agents let a user run a familiar provider CLI UI while the Chariox kernel remains the session authority.

Current commands:

- `chariox codex [session-ref] [--kernel-port PORT|--kernel-url URL]`
- `chariox opencode [session-ref] [--kernel-port PORT|--kernel-url URL]`
- `chariox claude [session-ref] [--kernel-port PORT|--kernel-url URL]`

Semantics:

- if no session ref is provided, Chariox creates a session and its first native TUI agent
- if a session ref is provided, Chariox attaches a new top-level native TUI agent to that Chariox session
- local native TUI launchers default to the web-dev kernel at `ws://127.0.0.1:43119/kernel`; `--kernel-port` selects another local kernel port
- a native TUI launch never attaches to an existing provider run; every native TUI agent owns its own provider run
- prompts from the provider TUI are intercepted and submitted through the same kernel prompt path as Chariox clients
- prompts from Chariox clients are forwarded through the kernel-managed provider run so the provider TUI observes the same turns
- native TUI provider runs are marked with `client_interface = native_tui`
- Chariox clients must treat model/variant controls for those runs as provider-controlled; provider-native changes may be recorded when observable, but Chariox-side parameter mutation is disabled for the active native TUI run
- daemon health reports `duplicate_chariox_agent_bindings` when more than one active Chariox provider run is bound to one session/agent, and `multi_interface_agent_bindings` when active Chariox and native TUI provider runs are bound to the same session/agent
- daemon health `provider_catalog` reports whether provider/model metadata is cached, expired, and how old it is; clients should surface stale catalog state near provider/session launch diagnostics
- daemon-tracked provider process listings include PID and best-effort current RSS (`resident_set_bytes`) when the host can read it; clients should surface this beside teardown safety so provider memory pressure is diagnosable without external process tools
- `ExportDebugBundle { session_id, bundle_label, limit }` is the shared session-scoped debug bundle request for TUI, web, and remote clients. The caller supplies only a session id, optional label, and optional record limit; the kernel filters current structured logs by `session_id`, sanitizes the label, writes `manifest.json` and `logs.ndjson` under its own debug-bundles root, and returns `DebugBundleExported { bundle_dir, manifest_path, logs_path, log_root, record_count, limit }`. Clients must display the returned paths as kernel-machine-local paths and must not send arbitrary output directories.
- Agent inspection and pane chrome should surface the session home kernel/machine alongside agent placement, worktree, provider run, extension grants, and remote extension manifest state so users can distinguish session authority from worker execution.

Remote native TUI composition:

- remote native TUI mode MUST compose existing protocol paths rather than create a second prompt/runtime protocol
- provider-native TUIs and Chariox TUIs attach to the home kernel session through the same client/session attachment semantics used locally
- provider-native TUI prompts MUST enter the home kernel through the same `SubmitPrompt` path as Chariox prompts
- the home kernel MUST dispatch remote execution through the existing leased-agent relay path (`SubmitLeasedPrompt`, remote prompt attachments, remote MCP/skill checks, and related completion/cancel paths)
- `ExecutionLeaseCreated` MUST include `relay_peer_protocol_version`; the home kernel must reject a worker that omits it or advertises a lower version before `SpawnLeasedAgent`, and persist the negotiated version on the binding. Restored bindings with a missing or mismatched version MUST be rejected before any leased prompt/native-provider dispatch and require a rebind, so stale remote kernels fail with an upgrade/restart action instead of breaking during provider tool calls
- the worker kernel MUST talk to the provider through the same kernel-provider adapter/server path used by ordinary worker-owned provider runs
- relay peer protocol v43 adds an optional profile-bound launch credential to leased prompt and remote native-provider launch requests. Home resolves this field only from the selected Claude profile's Chariox-vault setup token and sends it inside the end-to-end-encrypted peer packet. The worker rejects provider/profile mismatches, moves the value into the existing non-serializable zeroizing launch environment, injects only `CLAUDE_CODE_OAUTH_TOKEN` into the official CLI child, and never writes `.credentials.json` or durable kernel state. Missing fields remain compatible for providers and already-running worker runs; a new unattended Claude run without the field fails closed.
- worker output, notices, completions, and permission interactions MUST return to the home kernel through existing leased runtime projection and native interaction relay paths
- relay peer protocol v3 extends leased runtime projection with an optional worker `provider_run` snapshot. The home kernel MUST project that worker-owned run onto the home session/agent, including any resolved `provider_session_id` and resume state, because some providers such as Codex only expose the durable thread id after the first turn rather than at launch time.
- relay peer protocol v7 correlates projected completions with `home_prompt_id`. Workers retain the latest settled completion for pull-based replay, and home kernels MUST ignore a stale replay when that prompt is no longer active. This makes a completion recoverable when a fire-and-forget worker projection is lost without allowing the replay to settle a later queued turn.
- relay peer protocol v8 carries an optional exact worker-local skill requirement set for native TUI launch and prompt submission. Standard workers validate matching local package hashes without receiving package payloads; home-managed slices may materialize packages first. Workers project the validated set onto the leased backing agent so per-prompt hidden skill context stays worker-local and grant revocations cannot leave stale skills active.
- relay peer protocol v9 adds idempotent queued-prompt steering for leased agents. The home kernel keeps queue/history authority and removes the queued prompt only after the worker acknowledges provider delivery; the worker keys delivery by the home queued prompt ID, rejects stale active-turn targets, and replays acknowledgements without injecting duplicate provider input.
- relay peer protocol v10 forwards provider-targeted terminal resizes to leased worker PTYs. The worker validates that the requested provider run belongs to the leased agent and acknowledges the applied dimensions; the home kernel does not report a remote resize as successful before that acknowledgement.
- the provider-native proxy/launcher MAY translate home-kernel session output back into provider-native UI protocol or PTY rendering, but it must not become a session authority or bypass the home kernel prompt queue
- the relay remains transport-only and must not inspect or transform provider-native prompts, outputs, attachments, permissions, or history
- slice-backed native TUI mode follows the same contract: provider TUIs and Chariox TUIs attach to the home kernel session, `slice_ref` selects a home-managed worker execution environment, and the slice worker uses the same worker-owned provider adapter/server path as remote leased agents

Native TUI MCP and skill placement:

- local native TUI provider runs use the same agent-scoped grant filtering as ordinary local provider runs, so only MCPs and skills granted to that agent are injected or rendered for that run
- standard home-worker native TUI may expose home-authorized remote extension manifests to the worker. Home-owned active extensions remain grant/revoke authoritative on the home kernel and execute on home through relay peer calls; the worker only advertises the manifest and forwards calls. Each forwarded call carries `invocation_id`, optional `provider_tool_call_id`, `attempt`, and optional `idempotency_key`; home reconstructs the current tool definition before execution and rejects stale or forged worker metadata, including calls from a worker provider run that is not the current remote binding.
- when an extension is explicitly worker-local, the home kernel may still compute grant-derived remote MCP requirements and pass those requirements to the worker launch/prompt path so the worker can fail fast on missing or mismatched local worker definitions before provider execution
- slice-backed native TUI may synchronize Chariox skill packages from the home kernel to the child worker because the slice is home-managed; this is not a general remote-machine install mechanism
- slice-backed native TUI still executes worker-local MCP commands on the worker side, so worker-local MCP commands and environment must be available in the slice image or injected slice environment; Chariox vault credentials remain home-owned and are exposed to slice workers only through home-authorized credential proxy calls and one-operation secret injection
- capability grants remain agent-scoped in all modes; native TUI launch must not expose ungranted local/user MCPs or skills just because the provider CLI can see them natively

Native TUI permissions:

- provider-native permission requests MUST be represented as one agent-scoped, kernel-owned `RuntimeInteraction`
- that interaction MUST be projected to every Chariox TUI attached to the session, regardless of whether the current turn was submitted from a Chariox TUI or provider-native TUI
- answering from a Chariox TUI resolves the kernel interaction and the provider adapter/proxy forwards the resulting decision to the provider
- where a provider-native TUI can submit an approval response through a stable proxy or hook seam, the native response MUST resolve the same kernel interaction rather than bypassing it; first valid resolution wins
- if the provider only exposes the approval through a rendered PTY, Chariox may detect the rendered prompt and create the kernel interaction, then inject the resulting decision back into the PTY using the provider's native selection semantics

Provider-native credential enrollment callback bridge (local daemon protocol 241):

- an attached client first sends `ArmDeploymentCredentialEnrollment` to its home kernel. The arm is bound to the authenticated user and relay realm plus the exact enrollment, profile, target version, session, focused agent, and attachment ownership
- arms are kernel-memory-only, one-time, bounded by TTL and capacity, and shared by local and relay command routers for that kernel process. Expired, mismatched, consumed, or wrong-kernel arms fail closed
- the expected hosted helper subject is `deployment-credential-enrollment:<enrollment_id>`. `RequestCredentialEnrollmentInteraction` is accepted only from the encrypted relay client lane when relay-authenticated caller metadata identifies that exact `Service` subject and the arm's user and realm. Request-body identity is not accepted as authorization
- a relay-authenticated `Service` caller is request-scoped: the kernel rejects every local-daemon request other than `RequestCredentialEnrollmentInteraction`, even when the token's user is a session member
- the helper sends the provider authorization URL inside the encrypted kernel payload. Cloud and the transport-only relay receive no provider URL or callback content
- the kernel creates one ordinary agent-scoped `RuntimeInteraction` containing the authorization URL, a `Cancel` choice, and a custom choice whose `input_kind` is `secret`. Every attached web or TUI client receives the same interaction, and the first valid response wins
- cancel and timeout return terminal status without a callback. A submitted callback is returned only in the awaiting helper response; it is not stored in session state, projected as an event, included in command payload logging, or entered in the in-memory or persistent command-result cache
- this bridge drives the official Claude CLI callback seam only. Chariox does not implement OAuth authorization, token exchange, PKCE generation, or provider credential storage

Native TUI hidden context:

- granted skill prompt context and other Chariox-only prompt injections MUST be delivered on the provider-facing path without becoming visible provider-TUI text
- Codex native TUI hidden context MUST use Codex `thread/inject_items` developer messages before `turn/start` or `turn/steer`, preserving the attached thread and keeping `input` limited to visible user text and attachments. Resumed threads use the same bridge; newly created ordinary threads retain `thread/start.developerInstructions`. Injection failures MUST fail the prompt rather than fall back to visible user input.
- OpenCode native TUI hidden context MUST use the same OpenCode prompt request `system` field as ordinary OpenCode provider runs
- Claude Code native TUI MUST use the `UserPromptSubmit` hook `additionalContext` path for hidden context; the hook emits a scoped context request id, and the Chariox CLI bridge or worker kernel writes the matching context response before the hook returns
- Claude hook context responses are scoped to the session, agent, and provider run; they must not expose broad kernel authority or accept arbitrary provider-origin file paths
- if a Claude hook context response is unavailable before timeout, the provider-facing hidden context is empty and the native TUI remains coherent; Chariox MUST NOT fall back to visible PTY prompt injection for skill bodies or system prompt blocks
- local Claude native TUI can answer hook context requests through the launcher bridge and home kernel; remote/slice Claude native TUI answers them on the provider-execution side so worker-local or slice-isolated skill material is used

Provider-specific transport:

- Codex uses a native WebSocket proxy in front of a Codex app-server endpoint and binds the observed Codex thread to the Chariox provider run.
- OpenCode uses a native HTTP proxy in front of a launcher-managed `opencode serve` endpoint. The kernel binds its provider run to the proxy endpoint, while the provider TUI attaches to the same proxy/provider session.
- Claude Code has no stable provider UI/server split. Local and remote native TUI mode therefore use a kernel-owned PTY: the provider process runs where execution belongs, and the launcher streams/render-controls that PTY while the kernel projects prompts, output, attachments, status, and supported interactions back into the Chariox session.

## 3.3.3 Metaagent Event Prompts

Metaagent event notifications are Chariox runtime-origin prompts. They are not
hidden provider context, and adapters MUST NOT deliver them through hidden
system/developer channels. The visible prompt text should identify the message
as a Chariox runtime event, summarize what happened, and point the metaagent to
`chariox.meta.read_event`, `chariox.meta.turn_overview`, or
`chariox.meta.turn_blob` for detail payloads that are too large for the prompt.

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
  Chariox may mark the event `steered`; otherwise the prompt remains queued and
  visible in Chariox prompt history.
- OpenCode: event prompts use the provider session prompt API as visible
  prompt content. Hidden `system` context remains reserved for runtime
  instructions and MUST NOT carry event notifications. OpenCode event-stream
  completion should update delivery status without relying on PTY idleness.
- Claude Code: event prompts are submitted through the same visible prompt path
  used for user turns. `UserPromptSubmit.additionalContext` remains reserved for
  hidden context such as skill bodies and MUST NOT carry event notifications.
  When Claude is exposed through a kernel-owned PTY, Chariox may render or steer
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

Relay peer protocol v15 carries the home kernel's private hidden prompt context in
`SubmitLeasedPrompt`. This preserves catalog manifest markers and other kernel-owned
context when a queued prompt is dispatched to a leased worker; older workers are
rejected by the existing relay peer version check.

Relay peer protocol v25 requires `SubmitLeasedPrompt.expected_profile`, containing
the home-selected provider, stable provider-account ID, optional model, and optional
effort. It carries no credentials. The worker reconciles this profile before replay
or admission, using its lease owner's existing account registry. An unavailable
account blocks admission rather than selecting a different account. Profile changes
and prompt admission are serialized per leased agent through provider launch and
prompt ownership assignment. Confirming an unchanged profile is safe during a
replayed active turn; changing it requires an idle agent. This makes the next prompt
use the profile committed at home even if an earlier worker update acknowledgement
was lost. Incompatible peer versions must be rebound before dispatch.

Version 25 also removes the separate `ForwardWorkflowProviderFailure` request and
its acknowledgement. Workers settle failed leased turns locally without waiting
for a home RPC. The existing runtime projection carries the terminal diagnostic
and correlated, replayable completion. The home settles only the matching active
turn and releases the failed workflow's workspace claim; queued work is preserved.
Since protocol 397 a leased turn is not rerun on a substitute.
Rejected profile acknowledgements preserve the queue. Delayed managed projections
cannot re-establish a cleared worker-run binding after a profile change.

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

- the TypeScript CLI now defaults to `ws://127.0.0.1:${CHARIOX_KERNEL_PORT:-43118}/kernel`
- the Rust daemon process hosts that WebSocket listener directly
- local credentials travel on the upgrade's `Authorization: Bearer <token>` header, not in any frame. A kernel started with `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN(_FILE)` (managed and hosted workers) refuses upgrades without that token with HTTP 401. Any other kernel generates a fresh `chx_kat_`-prefixed token at each start and writes it to the owner-only file `<state dir>/kernel-local-auth/<port>.token`, where the state dir is `$CHARIOX_HOME/state`, `$XDG_STATE_HOME/chariox` or `$HOME/.local/state/chariox`. Local clients read that file on every connection to a loopback endpoint and present the token. Since protocol 412, upgrades without the token, or with a wrong one, receive HTTP 401 naming the token file and the Unix socket; refusals are logged (rate-limited, never the token itself)
- the Unix socket serves the same websocket protocol with OS peer identity and process-bound access grants; an ungranted peer can only request access
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
- Local daemon protocol v370 keeps recipient-scoped bounded output drains scheduled while records remain, even after the producer stops. Local and relay subscriptions preserve byte order and heartbeat scheduling; an empty drain does not schedule more work. Event shapes, relay peer protocol, and client minimum versions are unchanged (MP-08 / MP-10).
- `runtime_notices` carries runtime notices for the subscribed attachment/session
- `assistant_message_completed` carries `session_id`, `provider_run_id`, optional `agent_id`, `message_id`, and `completed_at_ms`
- `session_snapshot` is the full subscribed-session projection and remains the fallback after attach, replay gaps, explicit recovery, and structural changes
- `agent_activity_changed` carries `session_id` and the complete agent activity map for activity-only projection updates
- `provider_run_changed` carries `session_id` and the current provider run, or `null` when no provider run is active
- `session_metadata_changed` carries `session_id` and a `metadata` patch with alias, last-used timestamps, hidden state, focused agent, and workspace live-sync mode
- `runtime_interactions_changed` carries `session_id` and the current active runtime interactions for permission/choice prompts
  - protocol 293 gives each interaction exactly one subject: existing `agent_id`
    or `kernel_operation_id`. Kernel decisions work in sessions with zero agents;
    they neither focus an agent nor create another prompt area.
  - both subjects use the existing `RespondToInteraction` request and terminal
    projection. Only the authenticated terminal user who owns a kernel operation
    may answer its decision. Provider, agent-tool and remote-kernel paths cannot
    approve it; the subject is assigned by the kernel, not accepted from an App.
  - kernel decisions have explicit choices, no custom reply and no automatic
    choice. Timeout, abandoned operation and shutdown cancel the pending decision.
    The same monotonic deadline applies when resolving after a queued session lock.
    Decisions are single-use and bounded to eight per owner and 32 per kernel.
    Terminal presentation remains outside App content and App-controlled input.
- `waiting_room_inventory_changed` carries only `inventory_version` and requires clients to refetch the full waiting-room snapshot when fields outside the row patch change, including provider accounts, Git credentials, external provider sessions, relay inventory, remote kernels, and terminals
- `waiting_room_rows_changed` carries `inventory_version`, `schema_version`, `generated_at_ms`, optional `launch_target`, changed session rows, and `removed_session_ids`; clients should apply it as a row patch instead of refetching the full waiting-room snapshot
- `provider_catalog_changed` carries `generated_at_ms` and the current provider catalog
- `slices_changed` carries `generated_at_ms` and the current slice list
- `workflow_run_updated` carries `session_id` and the updated workflow run for workflow-run-only updates
- `heartbeat`, `transport_resumed`, `replay_gap`, `session_unavailable`, and `transport_closed` are transport/recovery signals; heartbeat and successful resume should not force full session, waiting-room, or prompt-history reads, while replay gaps require clients to discard optimistic deltas and request a fresh projection

Managed remote-kernel control uses local daemon protocol version 280. The kernel
client surface includes:

- Waiting Room inventory fields for provider accounts and safe Git credential
  summaries
- managed-environment create, list, detail, lifecycle, keep-running, and transfer
  preparation requests
- direct managed-context transfer start and status requests plus an explicit
  launch-target request
- multi-Workspace Project updates and exact slice repository selections

Every kernel that implements the direct source side of this contract advertises
`managed_context_source_protocol_version: 1` in its Cloud relay-presence
metadata. Cloud lists a kernel under `Kernel context from` only while that
marker, the kernel relay public key, an active owner-bound machine identity, and
a fresh authenticated heartbeat are all present. The capability marker is
independent of the local daemon protocol version so Cloud never infers transfer
support from an unrelated client protocol bump.

These messages coordinate a launch but do not move runtime authority into Cloud or
the client. A source-backed launch is bound to one source target, one target
kernel, one context id, and one plan digest. Package chunks travel through the
encrypted relay peer lane directly between those kernels. The target durably
commits each offset before acknowledgement and returns an idempotent consumed
receipt after completion. Retryable disconnects resume from the committed target
offset. Clients must surface failure and must not substitute Empty context.

Kernel-context schema v2 makes a stdio MCP portable only when its executable,
working directory, and referenced runtime files are contained by the dedicated
package directory `<user-mcp-root>/<mcp-name>/`. Export snapshots that directory,
hashes every file, preserves executable bits, and normalizes command paths before
transfer. Host-path arguments, ambient or literal credential environment values,
and paths outside the package are rejected. Import verifies the package before
materializing it below the target user's MCP root and rewriting command paths.
HTTP MCP definitions carry configuration only and never a local runtime package.

An imported managed Project reports target-owned Workspace and worktree paths.
The target kernel persists and publishes that Project before it returns the launch
target, so a retry or restart cannot expose a launch target that session creation
cannot resolve.

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
- terminal output polling returns structured terminal-output fan-out records, including distinct provider text, reasoning, tool, error, status, and transient `provider_terminal` output kinds
- `provider_terminal` carries fullscreen native-renderer bytes only; clients must not treat it as semantic transcript, turn activity, history, or completion evidence
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
- teardown removes the session's kernel agents. Provider-owned conversations remain saved in the provider profile and return to the external provider session inventory for import into a new session; waiting rooms should label these as saved provider conversations, not unattached live agents
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
- user cancellation settles the active turn while retaining the agent's provider conversation and resume identity for the next prompt

This local API MUST remain daemon-owned, local-first, and compatible with later workflow-mode runtime surfaces.

Architectural note:

- the WebSocket request/event transport is the primary CLI path
- the request/response IPC surface remains a bootstrap, harness, and compatibility transport
- both transports should normalize mutating requests into `KernelCommand` values during the M4.5 refactor
- future kernel-client and kernel-agent communication should converge on one long-lived event-capable connection model

Current local runtime note:

- the primary local CLI implementation is now a TypeScript OpenTUI client
- `chariox-cli` currently launches that TypeScript client through a small Rust compatibility wrapper
- the Unix-socket local transport remains useful for daemon smoke coverage and compatibility shims, but it is no longer the primary local user path

Current slice-management surface:

- `slice.list`, `slice.create`, `slice.get`, `slice.start`, `slice.stop`, `slice.delete`, `slice.display_endpoint.get`, `slice.logs.get`, `slice.state.save`, `slice.state.status`, `slice.state.reset`, `slice.backup.create`, and `slice.backup.restore` are daemon-owned local requests.
- Local Docker slice records persist their assigned host port set in `local_docker_ports`; clients may display these diagnostics, but launch, relay, display, and log behavior must use the kernel-owned values rather than reconstructing ports from the slice id.
- Protocol v280 adds `slice.create.display_backend`, accepting `novnc` or `selkies`. Unknown values fail decoding. Headed slices persist the selection in `display_endpoint.kind`; provisioning and later desktop lifecycle commands derive the backend from that record, not a client's process environment. Headless slices have no display endpoint. Protocol v322 makes omitted headed backends select Selkies for new slices; `novnc` remains an explicit rollback selection and durable slices continue to use their recorded endpoint kind. Clients that create headed slices using the product default require v322.
- A Selkies endpoint advertises `view`, `websocket`, `h264`, and `software_encoding`. It does not grant keyboard, pointer, clipboard, or resize authority. Those remain kernel-owned Environment operations. Its local URL identifies the private slice process, not a publicly usable viewer link. Selkies never uses the legacy raw HTTP display tunnel. Clients that select or interpret this backend require v280; clients that do not use it keep their existing minimum version.
- Protocol v281 defines encrypted display fragments with `protocol: "chariox-display-v1"`, a connection-specific `stream_id`, sender direction (`kernel` or `viewer`), a zero-based per-direction `sequence`, message `kind` (`text` or `binary`), `final_fragment`, and `data_base64`. The entire fragment is encrypted with the existing relay payload encryption to the admitted peer key. Each fragment carries at most 64 KiB of raw data, and a reassembled message is at most 4 MiB. Wrong peer, stream, direction, version, replay, out-of-order, malformed, and oversized packets permanently close that channel. Reconnect requires a fresh stream identity. This fragment layer grants neither admission nor input authority and has no history persistence. Protocol v293 supplies its runtime admission path; plaintext Selkies bytes must never enter the legacy tunnel. Clients using these encrypted fragments require v281.
- Protocol v293 and relay peer protocol v30 add kernel-owned Room viewer admission. A Selkies `slice.display_endpoint.get` request must carry `session_id`, `attachment_id`, and `viewer_public_key`; their optional wire representation preserves the legacy one-argument noVNC request. The home kernel accepts a local client or a remote client whose authenticated relay-key thumbprint matches `viewer_public_key`. It rejects service, metaagent, and kernel callers, and verifies attachment membership, attachment owner, persisted Room-to-slice placement, running slice state, and the Selkies backend before contacting the bound worker. The worker independently verifies the authenticated home kernel key plus exact Room and slice provisioner binding before registering a display target. A successful response adds `stream_protocol`, `stream_id`, and `peer_public_key` to the endpoint.
- Protocol v294 and relay peer protocol v31 add the first human Computer Action, `pointer_click`. The local request derives the human Actor from the authenticated caller, requires that Actor's explicit desktop takeover, validates the current runtime generation and canonical viewport revision, checks coordinates and click count, and admits the mutation through the Room Action ledger. The Action snapshot and history retain typed redacted arguments containing the pointer coordinates, button, click count, and viewport revision. That same argument record and the authenticated Actor define the opaque idempotency key's operation identity. An exact retry returns the original Action state after viewport change or input release and never repeats physical input; a different Actor or payload conflicts. The home sends an accepted Action to the Room's bound worker over the existing authenticated controller route. The worker revalidates the authority envelope and coordinate bounds, then executes the pointer command against the headed desktop without forcing browser focus. Physical Computer input has no transport replay command. If delivery succeeds but its response is lost, the Action fails without automatically clicking again.
- Each admitted target is an expiring 60-second, key-bound, single-use opening grant. Claiming it creates a separate short active-view lease that only the worker kernel renews while the admitted WebSocket remains attached. Viewer close, relay loss, stream failure, or kernel shutdown drops that lease and reaps the private adapter; reconnect obtains a fresh endpoint and stream identity. Remote display URLs require `wss://`; only an explicitly loopback relay may use `ws://` for local drills. Each encrypted direction has at most 16 queued fragments. Ingress saturation closes the channel instead of silently dropping a sequence number; downstream input waits at most two seconds under backpressure, and cleanup is bounded. The relay may route the outer tunnel and encrypted packet envelopes but cannot decode video or viewer controls. The display channel accepts only read-only video controls; keyboard, pointer, clipboard, settings, resize, and Environment authority remain on normal kernel operations. Released Web decoding and TUI viewer-launch UX remain separate client work and are not implied by this transport checkpoint.
- Ordinary local Docker slices keep Docker's default container security profiles unless the home user explicitly opts into a narrower exception. `slices.linux.allow_unconfined_seccomp = true` disables only Docker's seccomp profile. `slices.linux.allow_provider_sandbox_compatibility = true` separately disables seccomp, unmasks Docker's system paths, and selects the host-installed AppArmor profile named by `CHARIOX_SLICE_APPARMOR_PROFILE`. Clients must describe the complete grant before enabling it. Hosts that restrict unprivileged user namespaces must load the shipped `chariox-slice-provider.apparmor` policy and select `chariox-slice-provider`; Docker's built-in `unconfined` profile does not override that Ubuntu restriction. Before starting the worker runtime, provider compatibility mode runs a real Bubblewrap namespace probe and fails if the selected Docker security boundary cannot create it. Broker-backed managed hosts may force compatibility mode inside their dedicated rootless Docker daemon for those Docker slices. Explicit local DEV enrollment enables compatibility in its rootful, unmapped Docker engine only when its root-owned enrollment records `providerSandboxCompatibility: true` after installer acknowledgement. Older enrollments without that field retain default security profiles. The complete grant adds SYS_ADMIN, NET_ADMIN and SYS_PTRACE, disables seccomp, unmasks system paths and uses the selected AppArmor profile (default unconfined); container uid 0 maps to host uid 0, including the setuid Bubblewrap helper. Ordinary local slices still require the user configuration opt-in. This slice-only inner boundary does not apply to a provider launched directly by a Path-1 disposable worker VM, which uses the ordinary kernel provider path with no Bubblewrap requirement. The managed broker joins only the rootless daemon's user and mount namespaces, pins each validated publication directory by device and inode, and publishes it as a stable broker-owned bind mount. Docker never receives the original publication path or a `/proc` magic link. Durable handle records let the broker reuse mounts after its own restart and recreate them after a daemon restart; slice destruction removes the records and mountpoints.
- Slice lifecycle status is `stopped`, `starting`, `stopping`, `running`, or `unhealthy`. Start must only report `running` after the worker kernel has been discovered; otherwise the slice remains `unhealthy` and diagnostics are available through `slice.logs.get`.
- Slice records also carry display-only operation diagnostics: `last_operation`, `last_operation_status`, `last_error`, and `last_operation_at_ms`. The kernel updates these on lifecycle operations and restart reconciliation; clients may render them in status/doctor views, but must continue to treat `status` as the lifecycle state and audit/log records as the detailed diagnostic source.
- Daemon health `slice_lifecycle.issues` identifies each unhealthy slice or failed slice operation by slice id/name, status, last operation/status/error, sessions, agents, and worktree so clients can point users directly to the affected slice before they open logs/audit or restart/delete it. `slice_lifecycle.provider_auth_issues` separately identifies attached-agent slices with no provider account summaries or with `unknown`/`not_configured` provider auth, including provider, alias/identity, sessions, agents, and worktree. Clients should surface this from kernel health and point users to `/slice doctor`, `/slice audit`, and slice auth login/import before they send more provider prompts.
- Local daemon protocol v371 adds optional `runtime_process_identity` to `RelayStatus` for MP-10. Linux kernels report their own PID, Linux boot ID and process start ticks from native metadata, without argv, environment or account data. Legacy and non-Linux kernels omit it. The collector uses the normal kernel public API or authenticated TLS relay route, independently checks the live process executable against the signed release and requires that kernel to be a provider-child ancestor. Relay peer protocol remains v64.
- Local daemon protocol v370 projects nullable `runtimeStartedAt` from the Cloud managed-environment summary through the kernel to clients. Shutdown observation requires v370 so the three-hour minimum is measured from the authoritative runtime start; older/pre-bootstrap summaries may omit it and decode as null. Ordinary runtime behavior and other client minimums are unchanged.
- Local daemon protocol v369 and relay peer protocol v64 coordinate canonical signed daemon admission. A signed `Kernel` token registers only its canonical kernel subject, including production-shaped temporary peer IDs derived from that subject; display aliases do not authorize another daemon ID. Signed `Machine` tokens do not register daemons. Trusted `Service` admission and legacy self-host tokens retain their existing behavior. New slice kernels use their unique `slice:<id>` worker reference as both canonical daemon ID and display alias, so their existing bootstrap, key-bound runtime and recovery tokens identify the actual registered worker. The owner-only `allowed_targets` transport scope remains separate from the worker registration identity. Existing client minimum protocol versions remain unchanged.
- Local Docker slices use the kernel-configured relay when it has a token and a non-loopback `ws://` or `wss://` URL, so hosted Cloud and self-hosted relay deployments expose the slice worker on the same relay fabric as other remote workers. A hosted slice initially receives a short bootstrap token limited to registration and heartbeat. After discovering its relay key, the home kernel obtains a key-bound token that targets only that owner kernel and installs it through the encrypted peer lane with a fresh activation nonce. The worker queues the encrypted install acknowledgement before closing its bootstrap relay socket. After reconnecting, the worker must return that nonce in a worker-originated confirmation whose relay caller identity is bound to the installed key. The owner matches the slice, worker id, relay subject, full relay key, and nonce, then requires same-key live presence plus an encrypted ping before reporting the slice as running. The slice receives no Cloud session or machine credential. Loopback or incomplete relay configuration falls back to a private per-slice relay owned by the home kernel; clients should render the projected `relay_endpoint.private` flag rather than guessing from the URL.
- Kernel restart reconciliation must not leave runtime-only states active. Local Docker reconciliation inspects the host container: missing/stopped previously running slices become `stopped`, still-running or unverifiable runtime state becomes `unhealthy`, and interrupted `starting`/`stopping` transitions become `unhealthy`.
- `slice.logs.get` returns structured log entries for local Docker slice provisioner actions and recent container logs. Clients should render these as diagnostics only and must not treat log text as control data.
- Slice provider auth import/login/alias/remove requests are scoped by provider and the kernel owns displayed provider auth summaries. Removal purges the slice-side provider credential files and clears matching auth summaries from kernel state; `opencode` removes all `opencode:*` account summaries for that slice.
- Local daemon protocol v267 adds kernel-owned provider-account CRUD, profile-scoped auth/login/logout/status, provider-neutral usage meters, and account-aware catalog requests. Waiting-room projections expose safe profile metadata and materialization state, never profile paths or credential payloads.
- `GetProviderCatalog` carries provider/profile overrides plus `local`, `worker`, or `slice` execution location. Cache identity includes owner, request/profile selection, and location. Worker/slice requests require a matching kernel-projected materialization record.
- Provider account selection is immutable within one provider run. Updating an existing agent's provider/account/model uses the normal bounded context-handoff and run replacement lifecycle; it does not mutate provider auth in place.
- Relay peer protocol v18 carries the selected encrypted provider-account materialization before the first leased-agent spawn that needs it. The relay routes the opaque packet only. The worker validates lease ownership and installs a distinct profile root only when that profile is absent. Once installed, the worker kernel owns the profile and its provider-native state: later agent launches resolve the existing profile without exporting, retransmitting, refreshing, or replacing its files from the home kernel. A profile already installed manually on the worker is reused under the same rule. Credential replacement is an explicit target-kernel operation, not a side effect of agent launch.
- Relay peer protocol v22 adds managed-slice relay credential activation and renewal. Relay tokens remain redacted from debug output and travel only inside encrypted peer payloads. The slice validates the token subject, owner target, machine, action set, expiry, and worker-key thumbprint before installing it. A token change forces a new relay socket; the relay independently closes active scoped connections at expiry. Before the refresh window closes, the slice requests a replacement from its recorded owner over the same encrypted owner-targeted lane.
- Relay peer protocol v23 adds an offline-recovery credential beside the short-lived active credential. The recovery token is bound to the same slice relay key and exactly one owner target, permits only `daemon.register`, `daemon.heartbeat`, and `peer.request`, and is capped at 30 days. It cannot route packets or receive peer events. The slice persists it atomically with the relay owner and key, uses it only when the active credential is missing or expired, then asks that owner for a replacement active/recovery pair over the encrypted peer lane. Each successful refresh rotates the recovery credential. A restart after an outage spanning active-token expiry must retain this same-key recovery path; a key or owner mismatch fails closed.
- Relay peer protocol v24 adds the nonce-bound `ConfirmManagedSliceRelayToken` request and `ManagedSliceRelayTokenActivated` response. The bootstrap credential cannot send a valid confirmation because it lacks the worker-key-bound kernel identity required by the owner. Owner expectations and worker retries are short-lived and bounded; a matching confirmation remains idempotently acknowledgeable for a short window so response loss does not strand the worker. A v24 owner rejects a worker that returns the older install response or cannot perform the confirmation handshake, so mixed-version activation fails closed and requires the owner and slice worker to run the same release.
- Slice saved state is a kernel-owned product concept, not a Docker-management UX. `slice.state.save` overwrites the active state for the slice, `slice.state.status` returns the active saved-state metadata, `slice.state.reset` removes the active state so future starts use the base slice image, `slice.backup.create` creates an immutable named backup, and `slice.backup.restore` transactionally restores a stopped, agent-free slice. Protocol v301 adds backup archive digests and Docker image identities: restore rejects legacy, corrupt, mismatched, cross-slice, or ambiguous backups before mutation, captures an internal rollback generation, and durably journals that recovery intent before it recreates the container or home volume. The restored state and journal resolution are published in one durable event; only then may the kernel reclaim the previous active generation and rollback backup. If replacement, active-state capture, or durable publication fails, the kernel restores the rollback generation and republishes it as active. After a process or host interruption, durable replay finds any unresolved journal and performs the same rollback before normal runtime reconciliation; failed rollback remains journaled and retains its artifacts so startup fails closed instead of accepting an uncertain machine state. While that journal remains unresolved, the kernel quarantines the slice and rejects start, stop, delete, state, backup, authentication, and Environment operations; it also refuses to journal a second restore for the slice. Durable commit or rollback resolution clears the quarantine. On managed slices, that same resolution event records the broker acknowledgement still owed for the published home generation; the kernel retries it at startup and before the slice's next restore, refuses a new restore while it is owed, retains any rollback archive it names until acknowledged, and never rolls the committed resolution back. A successful restore leaves the slice stopped. Saved state is composite: a Docker image tag and a `/home/slice` archive under the Chariox slice state root. Before save or backup mutates either generation, every local kernel sharing the Docker engine contends on one host-wide disk-admission lock. Chariox first quiesces the source; live capture stops the desktop and pauses the remaining container processes. While the source remains quiesced and the admission lock remains held, Chariox measures the real home volume, target writable layer, Docker capacity, and state-root capacity, budgets archive overhead, retains 2 GiB on both filesystems, and completes commit and archive publication. Missing or unsafe measurements fail closed, the read-only measurement helper is removed, and a paused live source is resumed even when admission rejects the snapshot. Slice records expose only metadata (`saved_state_ref`, `saved_state_status`, `saved_state_updated_at_ms`); clients must not inspect archive contents or expose them to provider transcripts.

The saved-state archive budget includes apparent home bytes, compression slack,
and a conservative per-entry tar-metadata bound. When Docker and state storage
share a filesystem—or Chariox cannot prove they are separate—the peak budget
includes the committed layer plus both the helper and destination archive
copies in that one pool.

Local Docker slices also start with finite process and file-descriptor limits.
The defaults are 1024 processes/threads for the container and a soft/hard
`nofile` limit of 8192 for each inherited process. Chariox reapplies the mutable
process cap before starting an existing container and verifies its immutable
`nofile` setting before any service or authentication command runs. A legacy or
differently configured container fails closed with recreate guidance; stop and
destroy remain available even when the configured limits are invalid.

The local resource-exhaustion fault drill lowers limits only inside disposable
child probes. It exhausts open files and process creation separately, requires
the operating system to return an actionable `EMFILE`/`ENFILE` or `EAGAIN`
diagnostic, and proves an established terminal socket still completes a round
trip at the boundary. Probe descriptors, sockets, and child processes must be
gone before its external evidence report can pass.

- `slice.create.from_saved_state` may reference an existing saved-state id/name. Local Docker restore uses the saved image tag instead of the configured base image and extracts the saved home archive into the fresh slice home volume before normal provisioning continues. Restore still allocates fresh ports, relay identity, and worker identity through the normal slice start path.

Current session-lifecycle note:

- the local implementation still exposes `session.end` as an internal/runtime operation, but the intended user-facing local client contract is persistent detached sessions plus explicit `session.delete`
- `session.end` and `session.delete` are intentionally distinct:
  - `session.end` is an internal/runtime operation and may still be reused for resumable daemon-owned transitions
  - `session.delete` is the user-facing destructive operation and removes the session from the daemon registry after teardown
- the current local implementation now uses 16-character lowercase hexadecimal session ids with optional aliases and unique-prefix resolution
- detaching the last terminal does not cancel an active provider turn or queued
  prompt backlog. The kernel keeps source attribution in private durable prompt
  state, advances queued prompts without a live attachment, writes output and
  completion to durable agent-scoped history, and retains unresolved runtime
  interactions for later attachments
- bounded terminal fanout records are recipient-scoped and are not the recovery
  source for a long disconnection. Reattachment uses the session snapshot and
  durable history before it accepts live tail output

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

## 4.1.2 Shared Room environment protocol direction

Local daemon protocol v304 and relay peer protocol v40 add the typed
`CancelDownload` controller command and `DownloadCancellation` result. The
existing `slice_browser_downloads` runtime tool accepts either `{}` to configure
downloads or `{"cancel":{"browser_generation":2,"guid":"observed-guid"}}`.
The generation and GUID must come from `slice_browser_events`. The home kernel
derives the Room and agent from the authenticated runtime call, admits an
attributed Browser `download_cancel` Action against the Room desktop, and routes
the command through the existing bound-worker lease. It does not use the focused
tab as the cancellation target or require the originating tab to remain open.
The worker rejects malformed, stale-generation, unobserved and terminal download
identities. Home and worker validate the acknowledged generation and GUID.
`cancellation_requested: true` acknowledges the command; only the subsequent
download progress event with `state: "canceled"` proves terminal cancellation.
Connection failure must not be reported as successful cancellation. This adds
no client-local execution path. Existing client minimums remain unchanged and
the empty-argument download configuration remains compatible. Both home and
worker kernels need relay peer v40 for cancellation; an older worker cannot
decode the new command and must not be treated as having canceled the download.

The slice browser controller derives its download free-space reserve from the
same `CHARIOX_SLICE_MIN_FREE_MB` value used by slice provisioning (256 MiB by
default). Every runtime launch forwards the current value explicitly, including
when provisioning reuses an existing container, so an old container environment
cannot weaken an updated reserve. It measures the configured download filesystem
before enabling
downloads and fails closed when capacity is unavailable or below that reserve.
It rechecks capacity when a download starts and while progress is active. If
the reserve is crossed, the controller cancels every active download in that
browser generation; its terminal `download_progress` event carries
`cancellation_reason: "disk_pressure"`. Download safety is keyed by the CDP
download GUID and does not wait for optional frame-to-tab attribution, so an
immediate download from a newly created frame is still checked and canceled.
Concurrent download starts request a follow-up check, so one in-flight
measurement cannot cause a later download to escape admission. This extends the
controller's open event payload rather than the local-daemon or relay envelope,
so it does not change either protocol version.

Relay peer protocol v41 adds document-bound `Tab` controller commands and
results for `activate` and `close`. The public `slice_browser_tab` runtime tool
accepts a stable opaque `tab_id` returned by `slice_browser_status`; the home
kernel resolves that ID to the current controller target and document, admits
an attributed Browser Action against the authoritative Room tab, and sends only
the controller-private identities to the bound worker. The worker rejects stale
documents and unsupported actions. After the physical operation, home
reconciles the complete tab registry before returning the new focused tab and
tab list. This makes popup and new-window tabs controllable and closable without
exposing CDP target IDs or creating a second tab authority. Local daemon
protocol remains v304 because no local request, response, or client projection
changes; home and worker kernels both require relay peer protocol v41 for this
runtime tool.

Relay peer protocol v42 adds document-bound `History` controller commands and
results for `back`, `forward`, and `reload`. The public
`slice_browser_history` runtime tool accepts a stable opaque `tab_id` returned
by `slice_browser_status`. Home resolves the tab to its current controller
target and document, admits an attributed Browser mutation against that Room
tab, and sends only controller-private identities to the bound worker. The
worker rejects stale documents, unsupported actions, and unavailable history
directions. A successful operation preserves the stable Room tab identity and
home reconciles the complete tab registry before returning its new URL and
document revision. Same-document history may preserve that revision; a new
document advances it. Local daemon protocol remains v304 because no local
request, response, event, or client projection changes. Home and worker kernels
must both support relay peer protocol v42 before this runtime tool is used.

Local daemon protocol v305 adds `SubmitRoomEnvironmentBrowserAction` for
authenticated human Browser mutations. The first action is document-bound
history navigation: `back`, `forward`, or `reload` against a stable Room
`tab_id`. The request carries no Actor or controller-private target identity;
the session lane derives the human Actor, requires that Actor to own the
Browser-tab input target, resolves the current controller document, and uses
the same Room Action ledger and relay command as provider agents. The existing
`RoomEnvironmentActionSubmitted` response returns the attributed Action and
the fully reconciled Environment. Idempotency keys are scoped to the shared
ledger. An exact retry returns its original Action without repeating physical
navigation, even after ownership is released. A new request with a stale
runtime generation, unknown tab, missing takeover, or changed document fails
closed.

Local daemon protocol v306 extends `SubmitRoomEnvironmentBrowserAction` with
document-bound tab activation and closure against a stable Room `tab_id`.
Both operations use the existing controller tab command and shared Action
ledger. They require the authenticated human Actor to own the target Browser
tab, and reserve both that tab and the desktop while the physical operation is
in flight because focus and tab closure can change the shared graphical view.
The kernel reconciles the full tab registry before returning. An exact
idempotent retry returns the original Action even after closure removes the tab
or ownership is released; stale generations, unknown tabs, missing takeover,
and conflicting desktop ownership fail closed.

Local daemon protocol v307 adds the `provider` credential use and injection
policy. Clients may list the non-secret credential metadata, but the kernel
resolves the value only for a provider launch. The resolved value is excluded
from launch serialization, provider-run persistence, relay payloads,
projections, and debug output. Existing clients do not depend on the new enum
value, so their minimum supported protocol version does not change.

Local daemon protocol v308 adds `SetProviderAccountCredential`. An attached
client sends a provider, stable account profile, hidden credential value, and
an explicit overwrite flag. The kernel verifies account ownership, requests a
Chariox Vault unlock through the existing runtime interaction when needed, and
atomically writes the secret plus provider-only credential policy. Command
history and debug output redact the value. No client currently requires this
operation during normal session attachment, so minimum supported protocol
versions do not change.

Local daemon protocol v309 and relay peer protocol v43 add transient remote
provider-launch credential delivery. The home kernel may resolve only the
selected Claude account's vaulted setup token after validating the Room agent,
profile, and worker lease. The secret crosses only the encrypted peer packet,
is redacted from debug output, and is zeroized after the worker moves it into
the official CLI child's `CLAUDE_CODE_OAUTH_TOKEN` environment. Claude
`.credentials.json` files are excluded from ordinary account replication and
rejected by worker and managed-context import paths. Existing clients do not
depend on this internal kernel-to-kernel field, so minimum client versions do
not change.

This section defines the logical contract for the Room-owned browser and graphical Environment. Local daemon protocol v269 introduces the membership-scoped `GetRoomEnvironmentState` request and `RoomEnvironmentState` response carrying the complete snapshot below. Protocol v270 adds membership-scoped `StartRoomEnvironment`, `StopRoomEnvironment`, and `RetryRoomEnvironment` requests plus the shared `RoomEnvironmentUpdated` response. Start creates the Room's default Environment on first use, keeps its identity on repeated start, and accepts only initial viewport dimensions; later start requests retain the kernel-owned viewport without validating their ignored viewport fields. Stop preserves Environment identity and runtime generation. Until the Milestone 2 managed controller reports process completion, stop records the `stopping` transition and synchronously returns the Environment as `stopped`, so start-after-stop remains available. Retry preserves Environment identity, invalidates failed runtime handles, increments runtime generation, and returns the lifecycle to `starting`. Protocol v271 adds `UpdateRoomEnvironmentViewport`. The request carries dimensions and the revision observed by the caller. It does not accept client-supplied Environment, Actor, owner, or new revision values. The session lane derives the namespaced `user:<user_id>` Actor from the authenticated caller and the kernel assigns the next revision. Protocol v272 adds membership-scoped `RequestRoomEnvironmentInputTakeover`, the `RoomEnvironmentTakeoverUpdated` response, and pending-takeover state in the shared snapshot. Protocol v273 adds membership-scoped `ReleaseRoomEnvironmentInput` and the authoritative `RoomEnvironmentInputReleased` response. Both input requests carry only the Room and target; the session lane derives the human Actor from the authenticated caller. Protocol v274 adds stable Action sequence numbers and the `queued` Action state to the shared snapshot. Protocol v275 adds membership-scoped `GetRoomEnvironmentEvents`; a client sends its last observed cursor and receives ordered events plus the next cursor, or an authoritative snapshot when the bounded replay window has a gap. Protocol v276 adds the Action `cancellation_requested` projection. Human takeover cancels queued agent work immediately and marks every blocking running Action for controller cancellation without falsely declaring it terminal. Protocol v277 adds membership-scoped `CancelRoomEnvironmentAction`; the session lane derives the human Actor, queued Actions become terminal immediately, and running Actions remain reserved until controller confirmation. Protocol v278 adds submission, start, and finish timestamps plus closed redacted terminal outcomes to every Action projection. Protocol v279 adds membership-scoped `ListRoomEnvironmentActionHistory`; pages are newest-first, use an exclusive Action sequence cursor, and remain complete when the bounded snapshot compacts terminal Actions. Rejections use stable `environment_*` error codes on the relay surface and include that code in local IPC error text. The remaining mutation and pushed-event surfaces are still design contracts. Adding any request, response, event, or serialized field below requires the normal protocol version bump, snapshot update, minimum-client decision, and focused cross-boundary drill.

Protocol v295 adds stable Actor presentation colors, pointer presence in the Environment snapshot, the `PointersChanged` event, and membership-scoped `UpdateRoomEnvironmentPointer`. The request carries the runtime generation, viewport revision, and either desktop-pixel coordinates or null to clear the pointer. It never accepts an Actor identity. The session lane derives the human Actor from the authenticated caller. Clearing an absent pointer is idempotent and does not register Actor presence. Pointer presence creates no Action, reservation, takeover, or input ownership. The kernel clears stale pointers when an Actor disconnects, the viewport changes, the runtime is invalidated, or the Environment stops or fails. Consecutive pointer changes supersede one another in the bounded replay log while still advancing the event cursor. Motion therefore does not evict unrelated Room events, and clients that observed the prior cursor still receive a later change.

Protocol v379 adds the Room browser bar. Ordinary Tabs' windows (every window without an App view) cover the desktop fullscreen by default, like App views, so the stream shows the same page the canonical viewport lays out for agents. Membership-scoped `SetRoomBrowserBar` (`{session_id, visible}`) shows the bar instead: those windows are maximized with Chromium's tab strip and address bar, and the bottom of a page taller than the remaining area is clipped for viewers (agents still see the canonical viewport). The session lane derives the human Actor; the change is refused while another Actor holds desktop input, like a viewport change. The snapshot carries `browser_bar_visible` (omitted while false), and a change emits `TabsChanged` (no new event kind, so existing clients' replay keeps working and refreshes the snapshot); the kernel passes the flag with every controller `browser.reconcile` (the worker `Reconcile` command's `browser_bar_visible`, omitted while false), so a restarted controller or a new window takes the current state. A worker controller older than the flag leaves windows as they are.

Protocol v380 lets Apps place their agent panel. A manifest's `ui.agentPanel` (`{placement: right|bottom|none, size?}`, 120–1200 CSS px, `minKernelProtocol` ≥ 380) is the App's default; its page may ask for another through the bridge (`window.chariox.panel.set({placement, size?})` / `get()`, a `chariox.panel` view call the kernel answers with `{placement, minimized}`, never the App). Membership-scoped `SetAppViewPanel` (`{session_id, installation_id, placement?, minimized?, reset?}` → `AppViewPanelSet {installation_id, placement, minimized}`) records the user's choice for that App's views in the session; it wins over the App's, and `reset: true` first drops it, handing the panel back to the App. `EnvironmentAppPanel` carries `placement` (`right`/`bottom`) and `minimized` (a bar at the bottom); an App Tab with no panel has `app` without `panel`. The kernel sends each App page its CSS size beside the panel (`browser.app.open` `page`, `browser.app.layout {target_id, page}`); every change emits `TabsChanged`.

Protocol v296 and relay peer protocol v32 add bounded Room Environment screenshot transfer for TUI clients. `CaptureRoomEnvironmentScreenshot` carries only the Room and attachment. The home kernel accepts local or remote clients, validates Room membership and attachment ownership, resolves the running bound slice, and asks that worker to capture the shared desktop. The worker independently validates the authenticated home kernel key and exact Room/slice provisioner binding. It stores the PNG as an operational-only artifact and returns only its opaque ID, SHA-256, size, media type, and safe display name. Worker paths never cross the relay and screenshot artifacts do not enter the archive outbox. `ReadRoomEnvironmentScreenshotChunk` repeats the caller, attachment, Room, slice, and artifact-scope checks for every offset and limits each response to 131072 bytes. Clients must enforce a total-size limit, require ordered nonempty chunks, verify the final SHA-256 and EOF position, and publish the file atomically on the client host.

Protocol v298 and relay peer protocol v34 bind a browser secret fill to the exact document URL inspected before vault resolution. The worker controller rechecks that URL inside the same document-scoped operation that focuses and fills the opaque element reference; a same-document URL change and a target that cannot receive focus fail with distinct stable errors before secret insertion, and the secret is never sent through global keyboard input. Clients that do not invoke browser secret insertion need no new behavior, but home and worker kernels must use the same relay peer version.

Protocol v299 and relay peer protocol v35 add the owning `document_index` to every browser DOM snapshot node. The home kernel uses this internal association to authorize a vault credential against the exact top-level or iframe document that owns the target element, while explicit `expected_url` and `expected_host` guards continue to describe the visible top-level page. Missing or invalid document metadata fails before vault resolution. The frame URL is passed back only as the action's document-bound insertion guard and is not added to MCP browser field projections. MP-08/MP-11 amendment (Miguel 2026-10-09): secret paste accepts an editable input, textarea or contenteditable field. The kernel binds the Vault fill to its frame/backend node/document/generation. Each capture masks only its plain-text box with small device-pixel padding; password fields already render dots. Show-password toggles are rechecked each capture. Removal, document/generation change or user clearing/replacement retires tracking. No echo, container/order/bidi/budget, iframe or media mask remains.

Protocol v300 and relay peer protocol v36 add approval-gated Computer credential input. A Computer credential must declare both `allowed_uses = ["computer"]` and `injection = { kind = "computer" }`. The home kernel validates that policy and obtains an explicit user confirmation before resolving the secret. A leased worker forwards only the credential handle and its authenticated active-run context to home through the existing credential-tool request; it must not resolve the Computer secret or admit an Action against its private provider session. Home admits the redacted Action against the authoritative Room and sends the one-operation secret through the existing encrypted Room controller command. The physical worker types the value from process stdin into the already-focused desktop control; it does not focus Chromium or use the clipboard. The tool result may expose the credential handle, actor, target, action ID, and outcome. Action history records the actor, target, lifecycle, and outcome without the credential handle or secret. Debug and helper output are also secret-free. Because X11 cannot universally prove that an arbitrary native control masks its contents, the confirmation explicitly requires the user to verify masking; Browser input binds the exact filled field and masks it on capture only while it is plain text. This correction reuses the existing v300/v36 serialized shapes and therefore requires no version bump.

MP-08 / MP-11: local protocol v374 and relay peer v67 bind Computer credential approval to the native display target. The home queries `computer_secret_target` through the ordinary Room controller command and projects the window identity, native focused-control identity and geometry in one RuntimeInteraction. `SecretText.expected_target` is required; unbound input fails closed. Home rechecks after approval/unlock and rejects a changed Room generation. The worker checks the target before typing and between single-keystroke batches; an X server grab prevents another display client changing native focus between the check and delivery. A changed observable focus, window or geometry aborts with an actionable error. Already delivered keystrokes cannot be rolled back. Computer mode observes X11 native focus/geometry, not DOM field identity or masking within a shared native surface. User-confirmed masking remains the guarantee boundary; the prompt says that approving an unmasked field can expose the credential. Browser mode retains automatic expected-host and editable-field validation with fill-target capture protection.

MP-08 / MP-11: the worker excludes agent screenshot, OCR, text-from-frame and generic screenshot capability capture while Computer insertion executes. Capture already in progress settles before typing begins. Capture and insertion permits stay with the blocking helper through completion or cancellation, including dropped async callers. Human display transport remains available. The legacy raw Computer-secret resolution request is rejected in favor of home-owned Room insertion. These changes apply to ordinary and managed placement through the same kernel paths; client feature minimums are unchanged.


Protocol v302 and relay peer protocol v37 complete the shared human Computer mouse and keyboard input surface. `SubmitRoomEnvironmentAction` adds `pointer_move`, `pointer_drag`, `pointer_scroll`, `keyboard_text`, and `keyboard_key` beside the v294 `pointer_click`. Every action uses the same authenticated human Actor, explicit desktop takeover, current runtime generation, canonical viewport revision, opaque idempotency key, Room Action ledger, and bound-worker controller route. Pointer coordinates are canonical desktop pixels and must remain inside the current desktop bounds. Drag identifies both endpoints and the left, middle, or right button. Scroll uses signed discrete wheel steps: negative horizontal means left, positive horizontal means right, negative vertical means up, and positive vertical means down. At least one axis must be nonzero and each axis is bounded to 120 steps per Action. Keyboard text is nonempty UTF-8 bounded to 64 KiB. Keyboard key input is a nonempty ASCII xdotool key or chord name, bounded to 128 bytes, with a repeat count from 1 through 32. Human Computer input targets whichever desktop application already owns focus; it never activates Chromium implicitly. Text and chord payloads travel to the worker helper over stdin and are redacted from Debug output. The durable Action record keeps only text byte/character counts or a key repeat count, never keyboard contents. The in-memory idempotency ledger compares a domain-separated HMAC of keyboard contents, keyed by the home kernel identity, so a reused key with different same-length input conflicts without exposing a guessable content digest. As with v294 clicks, physical input is at-most-once and has no replay command after an ambiguous delivery failure.

MP-08/MP-10/MP-11, allocated local protocol426: `SubmitRoomEnvironmentAction` adds `keyboard_hold` (`key`, `duration_ms`) and `pointer_hold` (`x`, `y`, `button`, `duration_ms`). Provider `slice_keyboard` and `slice_mouse` expose `action=hold` through that same Room admission, bound Environment route and Action cancellation. A hold presses, waits 1–10000 ms, then releases before completing the Action; it never leaves native input held between Actions. The desktop remains reserved while holding. Human takeover cancels through the existing Action and process-group/reset path; release settles before a cancelled acknowledgement. Native interruption also releases in the helper's finalizer. Holds never refocus a desktop application. Keyboard holds use existing base-layout key names and explicit modifier chords (for example `shift+Left`); unmapped/duplicate keys, invalid durations and pre-existing native holds fail closed. Ordinary text and Unicode continue through the text helper. Keyboard history stores duration only, and HMAC idempotency binds the redacted key contents while Action arguments bind duration. Pointer history retains canonical coordinates, button, duration and viewport revision. Existing clients which do not request holds retain their supported minimums; hold callers require426. The relay envelope remains unchanged; a protocol411 worker rejects the new inner enum discriminant instead of silently applying a tap. Separate persistent down/up calls are not part of this bounded contract. Physical input retains the at-most-once and no-replay rule.


Relay peer protocol v38 adds the typed `ObserveRoomComputer` request and `RoomComputerObserved` response for provider-facing screen status, OCR, and text lookup. The home kernel derives the Room and agent from the authenticated provider run, requires the Room's running bound slice, holds its Environment-use guard, and sends the request only to that physical worker. The worker independently validates the authenticated home kernel key and exact Room/slice provisioner binding before running the bounded screen helper. A leased provider first forwards the normal runtime-tool call to home; a direct-home provider enters the same home authority directly. Status returns the home-owned canonical viewport and a client-attachment marker, not the worker's private viewer or display details. OCR and text lookup may reference an opaque artifact ID from `slice_screenshot`; the worker resolves it only after verifying source kind, media type, Room, slice, size, stored bytes, and PNG signature. Caller-supplied Room image paths are rejected. Text lookup emits every non-overlapping occurrence in visual reading order with native screenshot-pixel coordinates. Its result preserves `match` as the first occurrence or null and adds `matches` plus `match_count`; these additive fields live inside the existing opaque runtime-tool payload and do not change a typed relay or local-daemon shape, so they require no additional protocol-version or client-minimum bump. Raw helper stdout and stderr, artifact paths, viewer URLs, and find queries are absent from Debug output and the worker result. Observation results are bounded to 256 KiB per helper stream and do not enter the mutating Room Action ledger. The focused direct-home and leased-provider drills cross the real encrypted relay and verify authority, canonical dimensions, Unicode OCR, multiple-match, no-match, and native-scale coordinates, opaque artifact reuse, cross-Room rejection, redaction, and cleanup. No local daemon request or response changed, so client minimum versions remain unchanged; home and worker kernels must share relay peer protocol v38.

Provider runtime calls to `slice_mouse` and `slice_keyboard` reuse those Computer actions. For a leased agent, the worker forwards the authenticated call to home through the existing runtime-tool relay route. Home derives the Room and agent Actor from the active provider run, admits the Action against the authoritative Room, and returns the resulting Action and Environment metadata. The physical worker only executes the existing controller command. It does not create a private Room authority or a second action history. Keyboard contents remain absent from the Action record and result. The tool argument additions for horizontal scroll, pointer button, and key repeat are backward-compatible JSON fields, and the execution reuses the existing v302/v37 command shapes, so this correction requires no protocol version bump.

Protocol v303 and relay peer protocol v39 add the shared Computer clipboard contract. Human clipboard writes are a `clipboard_write` Room Action and require the authenticated caller's current desktop takeover, runtime generation, and opaque idempotency key. The action's content identity uses the same home-keyed, domain-separated HMAC rule as keyboard text. History retains only UTF-8 byte and character counts. `ReadRoomEnvironmentClipboard` is a separate human-only observation that requires the same takeover and runtime generation but does not enter the Action ledger. Both directions are bounded to 256 KiB and carry their text in zeroizing values whose Debug output is redacted. The home sends writes and reads only to the Room's bound worker over authenticated relay peer protocol v39; the worker revalidates the home/Room/slice binding and uses the physical helper's stdin/stdout without putting content in arguments. `slice_clipboard_write` gives direct-home and leased providers the write path through the same home-owned Room Action authority; no agent clipboard-read tool exists. A provider running directly inside a local slice may use the same stdin-only physical write helper when no Room controller is present. Existing v302 Room requests retain their wire shape and remain decodable. A restored worker binding with no advertised relay version, or with relay peer protocol v38 or older, fails closed until it rebinds at v39. Existing clients retain their prior minimum version. A client that writes or reads the human clipboard contract requires local daemon protocol v303, and a home/worker pair using it must both support relay peer protocol v39.

`pnpm --filter @chariox/cli run computer-clipboard:x11-drill` exercises the physical clipboard helper against real Xvfb, Chromium, and `xclip` in the existing slice image. It verifies exact empty, Unicode, whitespace, trailing-newline, repeat-read, and 256 KiB boundary behavior; forces helper failure to prove plaintext temporary-file cleanup; scans logs and captured output for clipboard residue; records only digest and size metadata; enforces bounded container resources; and removes all disposable state on success or failure.

`CARGO_TARGET_DIR=/absolute/shared/cargo-target pnpm --filter @chariox/cli run computer-input:room-e2e-drill` crosses the complete local product path with one home kernel, one headed worker slice, a slice-bound agent runtime MCP, and direct local plus relay-attached remote TUIs. It verifies physical pointer move, single click, right click, double click, text-selection drag, and two-axis scrolling; non-US keyboard text under the physical X11 locale; select-all and replacement; exact key repeat; preserved focus; exact agent and human clipboard writes; human-only clipboard read after takeover; agent rejection during takeover; redacted attributed Action history; content-free keyboard and clipboard notices; and cleanup and leak scans. Physical X11 text reuses the pinned Selkies XTEST keyboard implementation and its persistent, recyclable Unicode key mappings. A 40 ms process-local cadence with an X11 round trip after each key prevents delayed input from continuing after the kernel kills the input process group. Input reset releases all held keys, including printable keys, and pointer buttons. Typing does not touch the clipboard or insert through the DOM. The slice container reserves its configured provider-listener ranges from Linux ephemeral source-port allocation before Chromium starts, so outbound browser traffic cannot make provider-bridge startup intermittent. The agent establishes keyboard focus with a Room-authorized physical pointer click before injecting X11 keyboard input. The drill also cancels physical keyboard input after it has begun: an authenticated human first takes desktop ownership, starts a human Action, and explicitly cancels that own Action through local TUI `/room cancel`; a second agent Action is cancelled by remote TUI human takeover. Cancellation and takeover use one per-Room interrupt lane that remains responsive while the ordinary session lane awaits physical execution. Both paths must stop further typing, record exactly one requested cancellation, reset physical input before reuse, withhold human ownership until reset completes, and project the terminal Action to both TUIs without retaining input. Pointer drag must select text without changing the Chromium window geometry. Each TUI Action notice includes the Action sequence so consecutive Actions with identical actor, mode, kind, and outcome remain individually observable. `computer-input-cancellation:room-e2e-drill`, `computer-pointer:room-e2e-drill`, and the older `computer-clipboard:room-e2e-drill` name are aliases for the same aggregate drill. The drill reuses the v303/v39 contract and adds no serialized shape.

Physical keyboard text retains the 64 KiB UTF-8 limit. Its worker deadline is
five seconds plus 100 ms per Unicode character, allowing the 40 ms physical
cadence and keymap/scheduling overhead. The home uses that same deadline plus
ten seconds for relay delivery and completion. This does not delay explicit
cancellation or takeover. Other Computer actions retain their short deadline.
The standalone typing helper has a two-hour watchdog above the maximum valid
worker deadline; SIGTERM restores lifted modifiers and releases its active key
and layout-group lock. Kernel SIGKILL cancellation still requires the existing
explicit input reset before target ownership is released.

The same v302/v37 Computer actions use the existing `CancelAction` command; the
Room Action ID is also the worker execution identity, so no new serialized shape
or protocol version is required. The worker registers the live screen helper
before execution, and cancellation terminates that helper's complete process
group. It then releases the supported modifier keys and mouse buttons before the
original command reports `ActionCancelled`. The home keeps the Action and its
input reservation non-terminal until that response, so a pending human takeover
cannot be granted while physical input may still be active. A reset failure is a
visible execution failure rather than a false cancellation acknowledgement.

`pnpm --dir apps/cli computer-secret-input:x11-drill` exercises that Computer path against a real Xvfb display and focused password control in the existing slice image. It verifies the exact value by digest without retaining the secret, confirms the clipboard is unchanged, captures a masked screenshot, scans OCR, logs, and helper output for leakage, confirms no Browser Controller participates in input, enforces CPU, memory, and process limits, and removes the disposable container on success or failure.

The v276 takeover semantics also displace queued Actions from every other Actor on the target, preserve queued work belonging to the new owner, and retain the first accepted cancellation cause when explicit cancellation and takeover overlap.

The current `session_id` is the wire identity for the product Room until a deliberate migration introduces `room_id`. New code must not create both identities for the same runtime domain. `environment_id` identifies the default shared Environment within that Room.

### Environment snapshot

Protocol v282 adds durable physical placement through two shared requests:

- `BindRoomEnvironmentSlice { session_id, slice_ref }` is membership-scoped and requires the Room owner. It reserves a headed slice for the Room and returns its canonical slice ID. Repeating the same assignment is idempotent. Assigning a second slice to the Room, assigning another Room's slice, ambiguous references, and conflicting worker references are rejected.
- `GetRoomEnvironmentSlice { session_id }` is readable by Room members. Both requests return `RoomEnvironmentSlice { binding }`; `binding` is null when unassigned, otherwise it contains `session_id`, `slice_id`, `owner_kernel_id`, and `worker_kernel_ref`. No provider account data, endpoints, or credentials are included.

The reservation is the optional `environment_session_id` in the durable slice record. Old records decode as unassigned and retain their prior JSON shape. A successful bind is committed through `slice.updated` before it is published in memory. Stop and Room deletion do not erase the physical reservation. There is no implicit reassignment or unbind request that could expose a retained browser profile to another Room.

These requests configure placement only. They do not start a container, move a controller, admit a viewer, or persist the full Environment action ledger. Binding must be consumed and revalidated by the worker/controller and secure viewer routes before multi-Room product enablement. Clients invoking placement require v282; clients using only earlier Environment controls keep their existing minimum versions. Rollback to a kernel that does not understand the reservation is not safe for multi-Room use.

Protocol v283 and relay peer v19 add `RoomBrowserController` requests for controller
acquire, tab/viewport reconciliation, and release. The home kernel resolves its
persisted Room-to-slice reservation and uses that slice's recorded relay endpoint.
The worker executes physical controller operations only. Environment lifecycle,
stable Tab identities, actor ownership, and projections remain home-kernel state.
These operations do not require an agent execution lease.

The worker must be provisioned with the home kernel ID, home encryption public
key, Room ID, and slice ID through `CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID`,
`CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY`, `CHARIOX_ROOM_ENVIRONMENT_SESSION_ID`,
and `CHARIOX_ROOM_ENVIRONMENT_SLICE_ID`. Partial or inconsistent bootstrap data
is invalid. Each request must match all four values, including the decrypted
sender key and authenticated transport peer ID. The worker must never learn
ownership from the first request or fall back to a different Room's controller.
Worker-local Room lifecycle requests cannot claim the provisioned controller;
only the authenticated home relay path may acquire, reconcile or release it.

An already-running worker without this binding rejects controller access. It
must be restarted through the provisioner after binding the Room; binding alone
does not restart running agents. Older workers reject the new request variant
and require an upgrade before this routing can be used. This checkpoint routes
startup, reconciliation, and shutdown. Protocol v284 and relay peer v20 extend
the same authenticated route with structured snapshots. The worker validates
the target/document and returns bounded physical observations; the home validates
them again and assigns Room-owned opaque element references. Home agents in a
bound Room can discover and call the existing status, find, text, and text-wait
runtime tools without running inside the slice. Tool discovery and dispatch
derive the slice from the provider run's Room, never from caller-supplied IDs.
Unbound home agents do not gain access to local screen helpers.

Protocol v285 and relay peer v21 add locator actions to the same physical route.
Home-owned element resolution, stale-reference checks, actor admission, action
serialization, and terminal history surround worker execution. The worker validates
action parameters and timeout before sending input to its controller; the home
validates the returned target/document and action kind before recording completion.
Fill payloads are excluded from request debug formatting. This is not a vault or
secret-insertion acceptance claim. Home MCP advertises click, fill and submit
alongside the read tools. The public-path routing drill observes changed page
state and the home action ledger, and verifies that human input ownership blocks
agent mutations until explicit release. Browser-tab takeover uses the same
browser-component readiness as browser actions while the desktop is starting;
desktop takeover still requires desktop readiness, and controller recovery
blocks new input admission. Navigation and the remaining tools are not yet enabled for
home agents.

Protocol v286 and relay peer v22 carry a fresh 128-bit execution identity with
each locator action and add `CancelAction` on the same bound-worker route.
Cancellation still requires the provisioned home ID/key, Room and slice tuple.
The worker tracks only live physical executions by Room and execution identity;
it does not create another action ledger or decide input ownership. A stale or
unknown identity is a no-op. The home retries cancellation with the same identity
if it races worker registration, retaining the original execution future.
Cancellation delivery bypasses the original action's slice operation guard and
supervisor lock so it can reach a busy controller. The original action keeps
its guard until its response. The controller reads `browser.cancel` alongside
its bounded serial operation queue, using the original stdio request ID.
`CancellationRequested` is only a delivery acknowledgement. Only the original
operation's `ActionCancelled` response confirms physical cancellation and lets
the home finish the action as cancelled and grant pending human ownership.
The controller checks cancellation before input and between pointer movement
and button press, while keeping the browser available for subsequent actions.

Protocol v287 and relay peer v23 distinguish graceful cancellation from a
forced physical fence. If controller cleanup does not complete inside the
combined command and action timeout, the worker kills and reaps the controller,
then reports the fence to the home. The home finishes the Action as cancelled,
starts the controller against the surviving browser, and reconciles its new
generation. Reconciliation invalidates old element references while preserving
stable Room tabs, external browser state, and the single human input owner. The
cancelled call does not return until recovery either succeeds or fails visibly;
failure leaves Browser and Browser Controller health unavailable with a
recovery diagnostic.

Protocol v288 and relay peer v24 add non-mutating locator-action receipt
recovery. The worker retains the last 256 terminal receipts in memory, keyed by
Room and execution identity. A receipt stores the terminal result and a SHA-256
request fingerprint, not the fill payload. If the encrypted terminal response
is lost, the home sends `RecoverAction` with the identical request envelope. An
identical completed request replays its receipt, and an identical in-flight
request waits for the original execution; neither sends physical input again.
Reusing an execution identity with a different target, document, node, action,
or timeout fails closed. An evicted receipt or worker restart makes recovery
return explicit loss of completion proof and never turns the recovery request
into a new physical Action. Existing clients' minimum versions remain unchanged
because this is a home-worker transport contract. The real-relay drill discards
one encrypted response after the external browser records its mutation, then
proves that the public Room tool succeeds, the Action ledger completes, and the
physical click count increases exactly once. A second fault removes the receipt
before recovery and proves a clear failure, one physical click, and subsequent
fresh-action availability.

Protocol v310 and relay peer v44 extend the same execution registry to uploads.
The authenticated agent upload path uses kernel mutation admission and records
the agent, Tab, document revision, and terminal outcome. Human ownership blocks
dispatch. Upload and RecoverUpload carry one home-generated execution ID; the
worker fingerprints the target, document, node, and file list. A duplicate or
lost-response recovery returns the original receipt without uploading again.
Missing receipts and identity reuse with different arguments fail closed.
Existing clients' minimum versions remain unchanged; workers must negotiate
the new peer version rather than silently ignoring upload cancellation.

Human takeover or Action cancellation reaches the controller through the same
CancelAction/stdio browser.cancel path as locator input. Uploads check the
AbortSignal after asynchronous preparation and immediately before sending files
to Chromium, then release any resolved object before reporting cancellation.
An acknowledgement alone does not prove physical completion or rollback. If
files were already dispatched, the final physical result remains authoritative.
A stalled controller is fenced by the existing bounded cancellation timeout
before the home releases input ownership.

Protocol v311 and relay peer v45 extend that execution registry to download
setup and permission changes. Each admitted configuration has one random
execution ID and a fingerprint binding the command kind, target, document,
permission name, and setting where applicable. RecoverDownloadConfiguration
and RecoverPermission only recover the original execution; missing receipts
or mismatched arguments never dispatch a new mutation. Workers must negotiate
the new peer version. Existing Web and TUI minimum versions are unchanged.

Configuration uses the same CancelAction/stdio browser.cancel path as uploads.
The controller checks cancellation before changing download behavior or browser
permission state. Once the CDP mutation has been dispatched, its physical
result remains authoritative; cancellation acknowledgement is not rollback.
The kernel retains the shared input reservation until terminal proof or the
existing bounded controller fence settles the execution.

Protocol v312 and relay peer v46 extend the same execution registry to tab
activation and closure, history navigation and reload, URL navigation, and
dialog replies. Human browser actions and agent actions pass their execution
ID through the shared mutation lane. RecoverLifecycle binds that ID to the
typed operation, target, document, and arguments, and only reads the original
receipt. A lost receipt cannot cause a second physical action. Cancellation
before physical dispatch settles the Action as cancelled; after dispatch the
physical result remains authoritative. Queued URL navigation keeps the Tab
selected at admission even if browser focus changes. Existing client minimum
versions remain unchanged; workers must negotiate peer v46.

Protocol v313 adds `PrepareBrowserImport`, `ApproveBrowserImport` and
`CancelBrowserImport` terminal requests. These carry consent metadata only,
never cookies. The kernel derives the initiating user from trusted local or
verified relay caller context, checks Room membership, verifies attachment
ownership and interactive capability, and requires relay client identity to
match that attachment. Consent binds the source client/key/realm, source store,
exact domain/partition selection, overwrite choice, destination Environment,
runtime generation, Tab and document revision. Approval must repeat the exact
selection while it is current; it cannot be replayed. Agents, services, peers,
unverified relay callers and automation-only attachments cannot approve imports.
Cancellation requires the owning user and Room but may follow navigation or
Environment shutdown. Pending consent expires after 120 seconds.

The response is `BrowserImportConsent` with an opaque request ID and status
`prepared`, `approved` or `cancelled`. Approval alone does not apply cookies.
Encrypted connector pairing, exclusive destination application, writer
quiescence and durable crash recovery remain required before an apply request
can be exposed. Existing Web/TUI minimum versions and relay peer version stay
unchanged; clients using these new consent requests require protocol 313.

Protocol v314 adds `ClaimBrowserImportSource` and `AuthorizeBrowserImportSource`,
each carrying `{request_id, selection}` with the same strict metadata-only
selection. Claiming consumes an approval once and returns `source_claimed`.
Authorization returns `source_authorized` only while that read remains current.
Both requests revalidate the authenticated initiating client/key/realm, Room
membership, interactive attachment ownership and destination generation/document.
The source connector must check authorization before reading cookies, between
queries and before releasing the batch. A cancelled, expired or stale read must
discard its result. Authorization does not extend the original 120-second expiry.

The reading phase owns no destination writer: cancellation removes it and expiry
allows a new request. Destination application still requires a private, separate
claim within trusted exclusive execution. Once that claim succeeds, source-read
authorization is no longer valid. There is no public apply request, cookie payload
or crash-recovery bypass in these requests. Source clients require protocol 314;
existing Web/TUI minimums and the relay peer version remain unchanged.

Protocol v315 requires live import-consent validation on transport replay. All
five browser-import requests bypass the shared command-result cache. Reusing a
command ID cannot return an earlier approval, source claim or authorization after
cancellation, expiry or destination changes. The ledger remains the authority
for one-use transitions; clients must treat a failed replay as denial, not retry
cookie reads using a previously successful response. Import clients require
protocol 315; unrelated Web/TUI and peer minimum versions are unchanged.

Protocol v316 binds all five import relay requests to the authenticated client's
sender key. After decrypting the request and before dispatch, the kernel requires
a Client identity with a future expiry and a public-key thumbprint matching the
encrypted sender key. A copied token alone cannot authorize import reads, change
consent or cancel an import. Missing identity, missing key, expired identity and
non-client subjects are denied before ledger mutation. Ordinary browser requests
retain their existing ephemeral-key behavior. Import connectors must use their
paired key throughout consent and source reads and require protocol 316; general
Web/TUI and peer minimums are unchanged.

Protocol v317 wraps the encrypted response to each of those five import requests
as `{request_nonce, response}`. The nonce is copied from the authenticated request
envelope, not its outer relay request ID or reusable command ID. Import clients
must pin the kernel sender key and compare this nonce with the exact request
attempt before accepting its response. A retry uses a fresh encryption nonce and
cannot accept an earlier attempt's response, even when its command ID is reused.
This prevents a relay from replaying or swapping old consent replies under new
outer request IDs when the client retains its paired key. Pre-317 unbound replies
must be rejected. Import clients require 317; ordinary relay replies, local IPC
response shapes, general Web/TUI minimums and relay peer version are unchanged.

Protocol v321 and relay peer v49 complete the private browser-cookie delivery
path. Protocols 319 and 320 were already assigned, so the recovery commands and
exact domain-result shape advance monotonically. A paired client sends
`{browser_import_delivery:{request_id,selection,payload_base64}}` only as the
plaintext of the existing authenticated encrypted `client_request` frame.
`payload_base64` decodes to a JSON array bounded to 512 cookies and 512 KiB. The
authenticated ingress rejects ciphertext above 768 KiB before decode and admits
at most 32 concurrent deliveries independently of connector limits.
This envelope is not a `LocalDaemonRequest`, is rejected on unencrypted/public
request paths, and requires the same live Client identity, attachment thumbprint,
sender key, realm, consent request, immutable selection, and current destination
generation/document used for the source read. Consent/source metadata requests
remain cookie-free.

The home kernel claims exclusive Environment execution and writes its durable
recovery row before routing the redacted `import_cookies` peer command. The worker
passes the payload only to its bound browser-controller process. That process
uses the encrypted mode-0600 recovery journal, quiesces cookie writers, validates
the target/document and kernel-owned scope, applies through CDP, verifies browser
readback, durably records the outcome, and removes the journal before returning
exact ordered `{domain,status,cookie_count}` entries covering every frozen
selected domain once. `imported` requires a positive count and `no_cookies`
requires zero. On controller startup/reconnect the kernel alone reacquires the
Room controller, resumes matching outcome cleanup or rolls the encrypted journal
back, and clears quarantine only after the worker verifies journal deletion.
Uncertain application retains quarantine and returns a
fixed error; it is never cached or automatically replayed. Ordinary action/event
history and diagnostics receive only command kind, IDs, byte counts, and the
bounded result, never cookie values. Workers must negotiate relay peer v49 and
delivery clients require local protocol 321. Consent-only clients may remain at
317. Kernel and Linux slice images must be deployed together for this version.

Protocol v288 also removes the worker's advisory restart result. After
a fence, the home is the only authority that starts and reconciles the
controller.

Protocol v289 and relay peer v25 route dialogs, download-directory setup,
uploads, and permission decisions through the same authenticated Room worker
controller. The home still resolves stable Tab and opaque element identities,
checks document revisions, and owns agent input admission; the worker validates
the decrypted home ID/key plus Room/slice tuple before touching the physical
browser. Dialog prompt text and upload paths are redacted from request debug
formatting. Upload path bounds are validated during relay deserialization as
well as at the home API, and controller responses never return filesystem paths.
The existing dialog runtime tool is advertised to a home agent only after this
physical route is available. The real encrypted-relay drill covers nested frame
and shadow-root references, a shadow-root click that opens a popup, stable popup
Tab reconciliation, public dialog handling, download setup, file upload,
permission changes, caller isolation for every new command, and process cleanup.
Existing clients' minimum versions remain unchanged because these are
home-worker transport additions, not new public local-daemon request shapes.

Authenticated agent download setup and permission changes also use the shared
kernel mutation queue and Action ledger. Download setup reserves the Desktop
and every current Tab because Chromium applies it browser-wide. Permission
changes reserve the Desktop and current Tabs on the selected Tab's HTTP(S)
origin. Human ownership of an affected target blocks dispatch; ownership of an
unrelated-origin Tab does not block a permission change. Before a queued change
executes, the kernel checks that its target set still matches the reservation.
A changed scope fails with an explicit refresh-and-retry error. Controller
restart recovery settles the admitted Action before reconciling the controller.
This admission change adds no serialized fields or protocol variants.

Protocol v290 and relay peer v26 route bounded browser-event polling through
the authenticated Room worker that owns the browser controller and event
journal. The worker validates generation, cursor, and batch limits before
returning strictly ordered console, network, page, target, dialog, download,
crash, and browser lifecycle events. The controller removes console arguments,
request headers and bodies, URL credentials/query/fragment data, dialog text,
and unsafe network errors before an event enters the journal. Relay diagnostics
expose event identity and data-field names but redact all event data values.
The home maps controller target IDs to kernel-owned stable Tab IDs and drops
events for targets outside that Room. Cursor catch-up and replay gaps remain
explicit, bounded outcomes rather than implicit loss. The real encrypted-relay
drill covers event routing, stable Tab projection, secret-canary exclusion,
cursor resume, caller isolation, and controller cleanup. Existing clients'
minimum versions remain unchanged because the public local-daemon request shape
does not change.

The Room runtime MCP publishes the controller-backed browser integration tools
under both their stable `chariox.*` names and the existing unqualified provider
aliases. `slice_browser_status` returns the controller's
`browser_generation`; agents pass that generation with a bounded cursor and
limit to `slice_browser_events`. A replay gap is an explicit unsuccessful tool
result whose structured payload still carries `replay_gap`, `next_cursor`, and
the current generation so the caller can refresh state instead of guessing.
`slice_browser_downloads` and `slice_browser_permission` act on the focused
stable Room Tab. `slice_browser_upload` accepts an opaque element reference and
one through twenty bounded absolute paths inside the slice. Upload paths remain
out of Debug output, relay diagnostics, and tool results. These tools are
advertised to an authenticated home agent only when its Room has a bound
long-running controller route. They have no one-shot helper fallback. This MCP
adapter added no local-daemon or relay serialization, so that checkpoint
remained at protocol v290 and relay peer v26. The encrypted home-to-worker drill invokes every tool
through the authenticated runtime MCP route and verifies stable Tab projection,
physical controller effects, path redaction, cursor resume, and cleanup.

MP-08/MP-10/MP-11, local protocol v426 adds `RoomBrowserArtifact` and the
shared `slice_browser_artifact` tool. Attached clients supply Room, attachment,
Tab and a capture/read/inspect operation; provider calls derive Room from the
authenticated run and use its focused Tab. Image/network capture requires the
observed browser generation; completed downloads additionally require an
observed GUID. Home publishes bounded operational-only artifacts with opaque
IDs, SHA-256, safe names and runtime/browser/Tab/document/viewport/observation
identity. Reads are limited to 128 KiB and verify the stored bytes and current
identity. Inspection is limited to 256 KiB; PDF extraction requires installed
`pdftotext` with child time/output/resource limits. The provider may request
native MCP image bytes from the same CDP Page capture. Unix-socket clients requesting
inline bytes receive them only when the complete encoded response fits the local
WebSocket response 1 MiB inline-image budget. Otherwise capture returns artifact metadata for existing bounded
chunk reads; the native MCP image bound remains 8 MiB. No new serialized shape
is needed for this delivery bound. Protected images are
conservatively masked in full; protected download bytes are withheld. Passive
network attachments retain actual allowlisted CDP metadata, omitting cookie,
auth and bodies. `slice_browser_upload` also accepts Room-owned opaque
`artifact_ids`; home verifies and transfers bytes through the existing upload
admission, staging and recovery path. The new controller peer Artifact variant
and opaque upload variant require relay peer protocol v73. The existing lease
admission/rebind and hosted token installation/confirmation gates reject v70
peers before these operations; image preflight requires v73 and matching runtime
source lineage. Local client protocol is v426. See
`docs/BROWSER_CONTROLLERFILES_ACCEPTANCE.md` for bounds and validation limits.

`slice_screenshot` returns inline PNG data as the standard MCP `image` content
block rather than embedding Base64 in the textual result. The companion text
and `structuredContent` retain only screenshot metadata, so provider context
does not receive a second encoded copy. Inline screenshots must have the PNG
signature and are read through a 16 MiB hard limit. Calls that request only a
path retain the existing text-only response for a provider local to its slice.
For an agent attached to a Room-owned Computer, home derives the authoritative
Room and agent from the provider run, captures through the existing authenticated
Room screenshot peer, and returns opaque artifact metadata instead of honoring
or exposing a worker path. A leased provider forwards the call worker-to-home;
home then captures and reads the bound worker artifact in ordered 128 KiB chunks,
enforces the 16 MiB inline bound before allocation, and verifies the final size,
EOF position, Base64, and SHA-256. Screenshot observation does not create a
mutating Room Action. This routing adds no new serialized peer shape because it
reuses the existing screenshot capture/chunk and runtime-tool forwarding types.
The native MCP image change corrects the implementation to the already-negotiated
MCP `2025-03-26` content model. Neither correction changes a Chariox local-daemon
or relay shape, so neither requires a Chariox protocol version bump.

Protocol v292 and relay peer v28 add the reverse worker-to-home path for Room
browser runtime MCP. A provider running on a leased slice still discovers the
normal `slice_browser_*` tools, but its worker kernel sends those calls through
the encrypted relay to the home kernel. The home kernel validates the relay
sender against the active remote-agent binding, verifies that the same worker
owns the Room's reserved slice, and performs the action under the home Room and
home agent identities. Tool arguments and results cross the encrypted wire but
their Debug representations redact URLs, selectors, fill text, and upload
paths. `slice_paste_secret` stays on its dedicated vault path and is not part of
this forwarding contract.

Relay peer v29 adds `recovery_required` to the physical Room controller
response. When a worker discovers that its controller restarted before an
operation, it returns the new controller process generation instead of only an
error string. The home finishes an admitted mutation as failed, starts
controller recovery, invalidates every old element reference, reconciles the
kernel-owned Tab registry, and restores Browser and Browser Controller health
before returning the retry error. The failed mutation is never replayed. A
caller must rediscover elements before retrying, and repeating the old opaque
reference fails locally as stale. Stable Tabs and existing input ownership are
preserved when reconciliation can prove their physical identities. This is a
home-worker transport addition, so local daemon protocol v292 and existing
client minimum versions do not change.

The previous protocol milestone routes the remaining legacy browser
compatibility tools through the authenticated Room worker. `slice_open_url`
normalizes an HTTP or HTTPS URL, submits one kernel-owned `navigate` Action for
the authenticated agent and focused stable Tab, sends one physical navigation
request, reconciles the resulting document identity, and records the terminal
Action outcome. A lost navigation response is not retried because repeating a
mutation without a durable receipt could duplicate physical work.
`slice_browser_wait_for_selector` and `slice_browser_wait_for_idle` are bounded
read operations against the same worker controller and document identity.
Navigation URLs and selector values cross the encrypted wire but remain out of
request and response Debug output. Worker deserialization and the controller
both enforce URL, selector, and timeout bounds. These old public tool names are
advertised to a home provider only after this physical route exists; slice-local
one-shot behavior remains available during migration. The real encrypted-relay
drill discovers and invokes all three tools, checks the external browser state,
stable Tab reconciliation, agent attribution, completed navigation Action,
caller isolation, redaction, and controller cleanup. Existing clients' minimum
versions remain unchanged because this is a home-worker transport change, not a
new public local-daemon request.

Cancellation during other Browser operations still requires further resiliency
validation; this is not full cancellation acceptance for every Browser and
Computer operation.

Secure viewers still require work before product enablement. Existing clients'
minimum versions remain unchanged because their public request shapes have not
changed.

The home-side public `SpawnAgent`, `SpawnAgents`, `CreateSession`, and `MoveAgentToRemote` paths reject known slices reserved for another Room with `environment_slice_access_denied`. Slice names/IDs and known worker aliases/IDs share the check. Admission also rejects shared worker identities, including collisions discovered after binding, for direct slice references as well as worker lookups. Admission holds a slice operation guard so a competing bind or lifecycle operation cannot race it; a failure releases the guard. An unassigned slice with an unambiguous worker keeps legacy behavior. This adds no serialized request/response fields and does not replace worker-side authorization or viewer-token validation.

`SpawnAgent`, successful `SpawnAgents` batches, and `CreateSession` retain the canonical slice identity from admission through worktree-scope validation and attachment, including when the caller supplies a known worker alias or ID instead of `slice_ref`. Session creation also applies the same slice worker-readiness check to aliases and explicit slice references. Mixed batches preserve local target slots and share one guard across aliases of the same slice. Public deletion releases worker execution before deleting the shared home-agent record once and detaching it from its recorded slices. This checkpoint does not enable multi-Room browser execution.

If a later spawn fails in a worker-backed batch, the home rolls back successfully created agents through the same deletion path before returning the original error. Admission guards remain held during rollback. If cleanup fails, the home retains the agent, records its canonical slice attachment, and returns a cleanup-retry error naming the affected agent. The live regression covers a worker worktree-placement failure after the first successful spawn; transport loss during rollback, persistence-write failure and restoration of prior focus require additional drills.

If worker cleanup cannot be confirmed, public deletion returns an explicit cleanup-retry error and retains the home agent and slice membership. An unreachable worker is not evidence that its agent stopped; deletion must not silently forget a potentially live execution. Reconciliation after a worker loses its lease state, and retries after partial worker cleanup, require separate failure-path validation.

Worker cleanup uses the relay URL and token retained in the remote execution binding, including a slice-private relay distinct from the home's default relay. The local live drill uses separate relay servers and tokens to verify creation and deletion without relying on the worker being visible on the home relay.

The worker remembers its last 256 completed leased-agent deletions and 256 completed execution-lease deletions by ID so repeated `DestroyLeasedAgent` and `DestroyExecutionLease` requests can return the same encrypted acknowledgements after response loss. Both use one retention policy and do not retain deleted prompt history. Unknown IDs, evicted receipts and worker restarts still fail closed; a missing record alone does not prove deletion. The real-relay drills cover response loss at each phase, followed by successful public deletion and slice detachment. Restart reconciliation remains separate validation work.

`DestroyExecutionLease` also cleans up agents still owned by that lease through the existing leased-agent deletion path before removing the lease. Other leases may share the hidden worker Room and must remain usable. The real-relay regression launches three synthetic managed provider processes across two leases in one backing Room, releases the two-agent lease, verifies its processes stop, and sends input to the surviving lease's provider before final cleanup. This is local worker lifecycle evidence, not validation of real provider execution or worker-restart reconciliation. Error handling inside individual agent teardown still requires failure-path validation.

The internal cleanup error retains its source rather than converting every cause into a transport failure. Relay errors preserve the source's existing code and retryability while adding the retained-agent explanation. Admission cardinality failures are internal invariant errors, not retryable transport failures. These internal Rust error types add no serialized request, response or event fields.

A full Environment snapshot carries at least:

- `session_id`
- `environment_id`
- `runtime_generation`
- lifecycle and health state
- current saved-state generation when present
- Browser Controller, browser, desktop, and streamer health
- canonical viewport dimensions, scale, revision, and current owner
- ordered Room-visible tabs and the focused Tab
- present Actors and current input ownership
- pending human takeovers and the active Actions blocking them
- active and recently terminal Actions
- snapshot event cursor

Health details may name a failed managed process and a safe diagnostic code. They must not contain environment variables, command lines with credentials, browser data, page content, clipboard content, or provider payloads.

### Tab and document identity

A Tab projection carries:

- `tab_id`
- optional controller-local target metadata restricted to diagnostics
- URL, title, lifecycle, and focus state
- `document_revision`
- last activity Actor and timestamp
- structured observation availability

An element reference is opaque to clients. Its validation scope includes `environment_id`, `runtime_generation`, `tab_id`, and `document_revision`. An action using a stale reference fails with `stale_element_reference` and returns enough metadata to request a fresh observation. It must not retarget by text, selector, index, or coordinates without a new explicit Action.

### Actor and presence projection

An Actor projection carries:

- `actor_id`
- kind, either `human` or `agent`
- safe display label and stable presentation color
- presence state
- current observed mode when useful
- owned input targets
- active Action IDs

Attachment identity and Actor identity are distinct. A human may reconnect through another Attachment and retain the same Actor identity. An agent may change provider runs without becoming another Actor. Presence never grants permission or input ownership.

Kernel-derived human Actor IDs use `user:<user_id>` and kernel-derived agent Actor IDs use `agent:<agent_id>`. Human labels use the kernel-safe `Local user` or `Room member` fallback until an authenticated profile projection supplies a display name; raw user IDs do not become labels. Multiple live Attachments for one user project as one present human Actor. Active session agents project as present agent Actors; when the final user Attachment leaves or an agent is removed, the Actor remains in the snapshot as `disconnected` so event and Action history keep stable attribution. Agent aliases update only the safe display label, never Actor identity.

### Action envelope

Every Browser and Computer Action uses one kernel-owned envelope:

- `action_id`
- a stable kernel-assigned sequence number
- optional idempotency key
- `session_id`, `environment_id`, and `runtime_generation`
- `actor_id`
- mode, either `browser` or `computer`
- Action kind and redacted arguments
- target kind and target identity
- optional `tab_id`, `document_revision`, or viewport revision precondition
- queued, started, and terminal timestamps
- state, one of `queued`, `running`, `completed`, `failed`, or `cancelled`
- whether controller cancellation has been requested while the Action remains non-terminal
- redacted outcome or structured failure

Action acceptance validates Room membership, Environment generation, capability grant, target existence, preconditions, ownership, and queue capacity before execution. The kernel assigns order. Provider tool completion, a browser event, or a returned screenshot does not by itself prove Action completion; the Action ledger does.

Vault-backed input carries a credential reference and expected-target policy in the Action envelope. The resolved value travels only through the existing scoped secret-delivery path to the approved local input target. Keyboard text, clipboard content, and resolved secret values are never copied into Action history.

### Input targets and concurrency

Input target kinds are:

- `browser_tab`, identified by `tab_id`
- `desktop`, identified by `environment_id`

Observations do not reserve a mutation target. Their results carry the generation and revision observed so callers can detect staleness.

A structured browser mutation reserves its Tab. A Computer mutation reserves the desktop. If that mutation can affect the focused browser Tab, the kernel also reserves that Tab. A browser mutation that opens, closes, or focuses Tabs reserves the desktop and every affected Tab. Other mutations on separate Tabs may proceed concurrently. The kernel rejects or queues an Action when its required target is reserved; clients never implement their own lock queue.

The initial queue outcomes are:

- `accepted`, with the Action already running
- `queued`, with stable queue position or ordering metadata
- `rejected_busy`, when policy does not queue the Action
- `rejected_saturated`, when bounded queue capacity is reached
- `rejected_takeover`, when a human owns the target

Queue and reservation waits have bounded deadlines. Cancellation and process loss must release every target reservation.

### Human takeover

Takeover requests identify the Room and input target. The kernel derives the human Actor from the authenticated session caller; clients cannot claim another Actor identity. When no agent Action reserves the target, the response is `granted` and its authoritative snapshot shows the human owner. When an active agent Action blocks takeover, the response is `cancellation_required`; its snapshot projects the pending human Actor and blocking Action IDs, and each blocking Action projects `cancellation_requested` until the controller reports a terminal state. Takeover cancels queued Actions on the target from agents and other humans, while preserving queued Actions from the new owner. The first accepted cancellation cause wins: an explicit request followed by takeover records `requested`, while takeover followed by an explicit request records `human_takeover`. A later response may report `granted` only after every blocking Action has reached `cancelled`, `failed`, or `completed` and the target belongs to the human Actor. A queued Action from the new human owner starts as soon as the blocking Action is terminal; takeover ownership must not block that same Actor's queued work.

Takeover emits ordered Action and ownership events. Every attached client projects the same transition. A takeover request is idempotent for the same Actor and target. A conflicting human request follows Room permission policy rather than last-writer-wins behavior.

Explicit Action cancellation carries only the Room and Action IDs. The kernel derives the caller's Actor. An Actor may cancel its own work; a human may cancel an agent Action only while owning or awaiting takeover of at least one affected input target. Repeated cancellation is idempotent. Cancelling queued work may promote the next eligible Action, while cancelling running work only requests controller cancellation and retains all reservations until a terminal result arrives.

Authenticated cancellation derives readiness from the recorded Action's mode,
using the same browser-component readiness as action admission and takeover.
It does not require the desktop to finish starting before accepting cancellation
of an admitted browser Action. Actor and target-ownership checks still apply.
The relay drill covers a pending human owner cancelling while a second Action
is queued; physical interruption of running input remains separate validation.

Release is explicit. Disconnect may start a bounded expiry policy, but reconnect during that interval retains ownership. Expiry emits an ownership event and leaves the target unowned. It never assigns an agent automatically.

### Viewport contract

The canonical viewport carries:

- CSS width and height
- device scale factor
- desktop pixel width and height
- revision
- owner Actor when a user input owner controls resize

Clients submit viewport requests with the revision they observed. The kernel accepts one transition or rejects it as stale, unauthorized, unsupported, or unsafe. When the desktop already has an input owner, only that Actor may change the canonical viewport. An accepted response is complete only when browser layout, desktop resolution, streamer dimensions, screenshot coordinates, and input coordinates agree on the new revision.

The managed Linux headed image uses Xorg dummy modes for physical resize. Its pinned H264 path supports even desktop-pixel dimensions from 64 through 4096 on each axis; unsupported physical sizes are refused before the canonical revision changes. CSS dimensions remain independent (for example, CSS width 393 can use an even physical width). The kernel verifies physical and capture/framebuffer geometry before committing the candidate revision; a failed apply restores the previous display without restarting Chrome. A failed rollback marks the Browser and Room degraded.

This behavior requires the updated host kernel, compatible worker kernel, and managed Linux image together. Existing saved images using Xvfb retain their current physical size and allow CSS-only reconciliation through support overlays; physical-size changes are refused. They require the updated image for physical resizing. Generic local/Mac CDP and headless controllers do not require Linux display tooling. The explicit noVNC rollback backend verifies an existing RFB connection receives DesktopSize after resize; it does not rely only on a new viewer's initial geometry.

Viewer-only scaling is local presentation state and does not change the canonical viewport.

Pointer presence uses desktop-pixel coordinates from the canonical viewport. Each pointer carries one kernel-derived Actor ID and the viewport revision that makes its coordinates meaningful. Each Actor has one stable closed-enum presentation color derived from the Actor ID. Clients map that semantic color to their palette. They do not send CSS colors or choose another Actor's identity.

### Planned requests

The smallest request set is:

- `environment.state.get` (serialized in local daemon protocol v269)
- `environment.start` (serialized in local daemon protocol v270)
- `environment.stop` (serialized in local daemon protocol v270)
- `environment.retry` (serialized in local daemon protocol v270)
- `environment.viewport.update` (serialized in local daemon protocol v271)
- `environment.pointer.update` (serialized in local daemon protocol v295)
- `environment.screenshot.capture` and `environment.screenshot.read` (serialized in local daemon protocol v296)
- `environment.input.takeover` (serialized in local daemon protocol v272)
- `environment.input.release` (serialized in local daemon protocol v273)
- `environment.events.get` (serialized in local daemon protocol v275)
- `environment.action.submit` (human pointer clicks serialized in local daemon protocol v294; complete human mouse and keyboard input in v302; clipboard writes in v303)
- `environment.clipboard.read` (serialized in local daemon protocol v303)
- `environment.action.cancel` (serialized in local daemon protocol v277)
- `environment.history.list` (serialized in local daemon protocol v279)

Mutating Browser and Computer tools submit through `environment.action.submit`; they do not add provider-specific action request types. Human clipboard reads use the dedicated observation above. Existing public runtime MCP tool names may remain as adapters over these kernel-owned requests.

Slice save, restore, reset, and backup remain the existing slice lifecycle requests. Their Environment effects appear through Environment lifecycle and generation events instead of a parallel save authority.

### Planned events

The smallest planned pushed-event set is:

- `environment_snapshot`
- `environment_lifecycle_changed`
- `environment_health_changed`
- `environment_tabs_changed`
- `environment_viewport_changed`
- `environment_presence_changed`
- `environment_pointers_changed` (serialized as `PointersChanged` in local daemon protocol v295)
- `environment_input_ownership_changed`
- `environment_action_changed`

Each event carries `session_id`, `environment_id`, `runtime_generation`, and the normal monotonic kernel `event_id`. Structural deltas carry a base revision. A mismatched base revision or replay gap forces a fresh Environment snapshot.

### Recovery and history

Action history is kernel-owned and append-only. History entries use the Action envelope plus safe diagnostic and artifact references. Raw display frames, screenshots, DOM snapshots, network bodies, clipboard values, and secrets are not embedded in the ledger. Their bounded artifacts follow Room permissions and retention policy.

The Milestone 1 implementation retains complete Action history and idempotency records in memory so snapshot compaction cannot make a completed physical mutation repeatable. This is not the production retention implementation. Before the local persistence gate closes, the kernel must append these records to `OperationalHistoryStore`, retain only a bounded hot ledger in the Environment, and prove that paging and idempotent replay cross the hot-store boundary without repeating an Action. A lossy in-memory cap is forbidden because evicting an idempotency record could turn a response-loss retry into a second physical mutation.

After reconnect, a client resumes from its last kernel event cursor. Replay preserves Action order and terminal state. After a replay gap, the client discards optimistic Actions and applies one full snapshot. It must not resubmit an Action unless the kernel reports that the original idempotency key is unknown or retryable.

Process recovery follows these rules:

- completed Actions are never repeated
- queued Actions remain ordered only when their preconditions and target generation still hold
- a running Action without durable completion proof becomes failed or cancelled
- stale element references fail and require rediscovery
- controller or browser recovery must not create duplicate Tabs
- streamer recovery does not change Environment, Tab, Action, or input ownership identity
- worker or kernel recovery reconciles ownership before admitting new mutations

### Compatibility policy

Protocol v268 clients know slice display endpoints and one-shot browser/computer tools but do not know the shared Environment contract. Protocol v269 clients may read the complete Environment snapshot. Protocol v270 clients may also request start, stop, and retry through the kernel-owned lifecycle lane. Protocol v271 clients may update the canonical viewport. Protocol v272 clients may request authenticated human takeover and observe pending takeover state. Protocol v273 clients may explicitly release their input target. Protocol v274 clients understand stable Action sequence numbers and queued Actions. Protocol v275 clients may replay bounded ordered Environment events or recover from a gap with an authoritative snapshot. Protocol v276 clients can distinguish a still-running Action whose controller cancellation has been requested. Protocol v277 clients may request authenticated Action cancellation. Protocol v278 clients can render the Action timeline and redacted terminal outcome without inferring completion from a controller response. Protocol v279 clients may page redacted Action history independently of the bounded hot snapshot. Protocol v294 clients may submit an attributed human pointer click after taking explicit desktop input ownership. The request carries the runtime generation, viewport revision, an opaque idempotency key, coordinates, button, and click count, but never accepts a caller-supplied Actor identity. Protocol v302 clients may submit the complete human mouse and keyboard input set with the validation and redaction rules above. Protocol v303 clients may write and read the human clipboard contract. During migration:

Protocol v295 clients may render Actor colors and pointer presence and may publish or clear their authenticated pointer without gaining input ownership.

- the kernel keeps the old tool names behind a compatibility adapter
- compatibility calls still enter the kernel-owned Action path once it exists
- an old client may observe the display but cannot claim human takeover or canonical viewport ownership
- the kernel rejects unsafe concurrent legacy mutations instead of allowing split authority
- unknown Environment events remain ignorable only when the client's behavior stays safe
- a client that needs takeover, Action history, stable Tabs, or canonical viewport requires the new minimum protocol version

No minimum version changes for clients that do not use these Environment controls. A released client that invokes `environment.start`, `environment.stop`, or `environment.retry` must require protocol v270 or newer; one that updates the canonical viewport must require v271 or newer; one that requests or depends on human takeover state must require v272 or newer; one that releases input must require v273 or newer; one that renders Action ordering or queue state must require v274 or newer; one that replays Environment events must require v275 or newer; one that renders active cancellation state must require v276 or newer; one that cancels Actions must require v277 or newer; one that renders Action timing or terminal outcomes must require v278 or newer; one that lists Action history must require v279 or newer; one that submits human pointer clicks must require v294 or newer; one that renders or publishes pointer presence must require v295 or newer; one that captures a Room screenshot or reads its chunks must require v296 or newer; one that submits human pointer movement, drag, scroll, text, or key input must require v302 or newer; one that writes or reads the human clipboard contract must require v303 or newer.

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
- a generic WebSocket transport for agent endpoints is explicitly deferred until after Chariox has integrated more than one agent family and can derive a better common denominator from real integrations

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

Workflow trigger and deployment direction:

- HTTP, schedule, and event-notification triggers created on the current kernel
  (an event-notification trigger is an `event_based` publication that App
  automations target; protocol 365) remain attached to the editable source workflow and its source session
- accepting a trigger invocation MUST enqueue it through the workflow endpoint's
  normal queue path; it MUST NOT create a hidden session, cloned agents, or a
  separate queue namespace
- workflows in one session run independently. Prompts and handoffs are scheduled
  per agent, so unrelated agents may execute concurrently while work targeting a
  busy shared agent queues durably with its workflow, run, node, edge, and
  occurrence identity preserved
- an endpoint may maintain a bounded pool of runtime instances. It reuses an idle
  instance before cloning another, and every clone preserves the source agents'
  execution configuration and extension grants without copying active runs,
  transcripts, or credentials
- a local HTTP gateway is an ingress process for a source workflow trigger. It
  resolves the current publication definition from the kernel for each request
  and invokes the existing source session; starting the gateway does not export
  or materialize a workflow package
- multiple triggers MAY feed one workflow and therefore share its agents and
  configured queue namespace
- exporting or deploying a workflow is the boundary that captures an immutable
  package. A publication package contains `publication.json`,
  `workflow.snapshot.json`, `requirements.json`, `apps.json` for a workflow
  that uses Apps (protocol 366), optional generated app assets, and packaged
  scripts
- a packaged/self-hosted or Chariox-hosted deployment materializes its own
  kernel-owned session in the destination kernel. That deployed session is
  independent from the source session because it is a separate execution
  environment, not because a trigger was created
- protocol 282 adds optional `runtime_key` to `MaterializeWorkflowPublication`.
  A destination-owned key binds one immutable publication/snapshot to one
  runtime session and agent map. Repeating it, including after kernel restart,
  returns that runtime without reinstalling its initial queues or schedules.
  A conflicting snapshot, disabled publication, ended session, or changed agent
  ownership fails closed. Omitting the key still creates an independent runtime.
  The gateway appends `:replica-N` to `CHARIOX_PUBLICATION_RUNTIME_KEY` for each
  configured replica. Keys do not authorize access or transfer credentials.
- materialization acknowledges only after atomically persisting the initial
  session and agents. Subsequent queues, schedules, and runs use the ordinary
  kernel durable-state path. Recovery requires the same kernel identity, durable
  state and workspace mapping; a key alone is not a persistence mechanism.
- `CHARIOX_PUBLICATION_CONTROL_STATE_DIR` separates a publication kernel's
  retained state from its disposable private configuration. It selects the
  durable database, workflow definitions/code/artifacts, session and operational
  history, and monotonic event counters. Provider-account registry/home paths,
  managed-context transfer stores, relay credentials, and runtime capability
  files remain outside that root and are reconstructed from current authorized
  bindings. This is process configuration, not an additional protocol field.
  Ordinary kernels retain their existing storage layout when it is unset.
  The publication image accepts only `/var/lib/chariox/publication-control`,
  owned by the kernel identity with mode 0700, and requires explicit stable
  kernel, machine, materialization-key and workspace identities. Neither app
  actions nor the HTTP gateway can access this directory. The runner must
  mount and lifecycle-manage the matching deployment-owned volume; this
  environment setting does not create a persistent volume by itself.
- a kernel holds an exclusive process-lifetime lease on its durable store.
  Deployment replacement must stop the previous state owner before starting its
  successor. The lease is released only after the last owned store reference
  and durable writer are gone; database observers do not become schedulers.
- protocol 283 adds `ActivateWorkflowPublicationRuntime` with `publication_id`
  and the complete distinct `runtime_keys` set. A kernel using retained publication
  control storage starts with autonomous work held. Restoring state or attaching
  a client does not activate it. The gateway validates this boot's
  provider/credential/extension bindings, prepares every replica and
  attaches, then requests activation. The kernel requires every enabled retained
  runtime to match a successful materialization by its owner in this process.
  `WorkflowPublicationRuntimeActivated` acknowledges that exact set. Invalid or
  incomplete preparation leaves schedules and restart recovery held without
  advancing occurrence state. Activation is process-local, never restored, and
  replaces the speculative startup grace period only in these prepared kernels.
  Ordinary kernels retain automatic startup recovery. Stopping the listener
  cancels pending recovery even if publication activation never occurs.
- protocol 284 adds `ImportNativeProviderAccountProfile`. The authority owner
  can explicitly register the kernel host's provider-native scope without a
  client-supplied path, changing an existing profile, or copying credentials.
- protocol 319 adds `PrepareManagedEnvironmentGitCredentialEnrollment`. This
  explicitly requests a Cloud-authorized managed-context ticket for enrolling
  selected Git credentials into an existing managed environment; creating an
  environment with Git credentials set to `none` remains an opt-out.
  - initial `gitCredentials: { kind: "none" }` performs no Git export or helper
    setup
  - create/copy with `kind: "selected"` remains part of the immutable launch
    plan and the ordinary managed-context transfer
  - post-creation enrollment uses a separate, Git-only plan authorized by Cloud
    with the target Machine credential; the target kernel validates the exact
    source/key/realm binding, imports through the encrypted context package,
    and does not replace or publish a development workspace
- protocol 339 requires `RequestManagedEnvironmentReimage` to carry a fresh
  `contextPlan` using the same input contract as managed-environment creation.
  The kernel forwards the exact selected or explicit-empty plan to Cloud and
  preflights every selected provider account for managed-context export before
  making the destructive reimage request. Cloud binds the normalized selection
  into idempotency, creates a new context identity with no inherited manifest,
  and authorizes the live source realm or an explicit-empty realm. Reimage does
  not introduce another context-transfer authority or credential path.
- protocol 340 coordinates the managed-activity HTTP contract; it does not add
  a `LocalDaemonRequest` or `LocalDaemonResponse`. Reports to both
  `/v1/managed-kernels/activity` and `/v1/disposable-workers/activity` MUST
  include `activityChangedAt` in the canonical JSON covered by the machine
  credential HMAC. The value is the true kernel-owned aggregate activity
  transition time, encoded as canonical RFC 3339 UTC with exactly millisecond
  precision and `Z` (for example, `2026-09-22T12:30:00.000Z`). It is not the
  HTTP send, retry, or receipt time and MUST remain stable across delayed
  delivery, retries, reporter restart, and kernel restart. A later transition,
  including a rapid busy-to-idle cycle, receives its own later timestamp.
  Managed-environment signatures cover `accountId`, `activityChangedAt`,
  `environmentId`, `kernelId`, `machineId`, `runningAgentCount`, and `sequence`;
  disposable-worker signatures replace `environmentId` with `allocationId`.
  Cloud receivers for both routes MUST accept, validate, and persist this signed
  field before protocol-340 kernels are rolled out; producer and receiver must
  not be deployed independently. Existing web/native minimum protocol versions
  do not change because clients do not consume this kernel-to-Cloud field.
- protocol 341 adds the authenticated, read-only
  `GetManagedEnvironmentReimagePreflight` request and
  `ManagedEnvironmentReimagePreflight` response. The response pins the retained
  provider server, generation, runtime identity, and the desired immutable
  image/profile/release/source evidence before a client observes the old kernel
  or asks for destructive confirmation. This read neither authorizes nor starts
  reimage, but clients depend on its new response shape, so it requires the
  normal local-daemon protocol bump; the protocol-339 mutation remains the sole
  reimage admission request.
- protocol 342 adds optional `managedRepositoryRoot` to managed-environment
  creation and returns Cloud's validated, persisted value in every managed
  environment summary. Omission keeps the ordinary `/home/chariox` default.
  The value is create-only: selecting an existing environment displays its
  summary value, and reimage, session, browser, and worker requests do not
  accept overrides. Clients that send a custom root require kernel protocol
  342; older callers that omit it retain the previous create request shape.
  Cloud validates and persists the selected root before the kernel projects it
  back to the Waiting Room. The v342 protocol snapshot covers omitted and
  custom create values plus default and custom summary projections; the focused
  managed API drill covers request serialization and authoritative projection.
- protocol 343 and relay peer protocol 56 add the worker's originating home
  prompt ID to forwarded runtime tool context. The home kernel accepts a
  forwarded `chariox.send_agent_message` only while that prompt is active for
  the bound home agent, leased agent, and worker provider run. Calls carrying a
  settled home prompt ID cannot message another agent. The local
  client shape is unchanged, so web and native minimum versions do not change.
- protocol 344 adds the Claude `setup_token` value for `StartProviderLogin.method`.
  The kernel runs the official `claude setup-token` command as a managed
  terminal login whose `ProviderLoginStart.login_kind` is
  `terminal_setup_token`. Its 40x1000 PTY output is rendered by a terminal
  emulator. For this login kind, `terminal_output_base64` carries only the
  rendered screen text plus kernel notes, never raw PTY bytes. Every `sk-ant-`
  run is replaced with a marker. The kernel captures the token from the
  rendered screen only when three conditions hold: it is a complete
  `sk-ant-oat01-` token, more output follows it or the CLI has exited, and it
  is the only distinct token on screen. It then verifies the token with a
  no-model `claude -p /usage` call and stores it through the same vault path
  as `provider setup-token`. Draining, input, cancel, and completion for one
  login are serialized, and completion takes effect only from `running`. If
  the encrypted Chariox Vault is locked, the workflow stays `running` and its
  `interaction` becomes a secret vault-passphrase prompt. A new vault's
  passphrase must be entered twice. The next `SendProviderLoginInput` is
  consumed as that passphrase, not written to the exited provider CLI. A wrong
  passphrase keeps the prompt open. The normal 10-minute workflow timeout and
  cancellation drop the rendered screen and the captured token. A Claude
  profile that is signed out natively but has a stored setup token keeps the
  observation recorded at verification. The message shapes are unchanged. A
  client that offers the method requires kernel 344. Older kernels reject it
  through the normal enrollment-method validation.
- relay peer protocol 57 requires a worker capable of supplying the originating
  turn for `chariox.send_agent_message`. A new home kernel rejects a v56 worker
  at peer binding before provider dispatch rather than failing on a missing
  tool field mid-turn. The local daemon shape and client minimums do not change.
- relay peer protocol 69 (with local protocol 365) drops the workflow event
  capability flags: `RemoteWorkflowTurnContext.event_context_enabled` /
  `event_actions_enabled` and the leased provider run's
  `workflow_event_actions_enabled`. A v69 peer reads the extra fields of an
  older peer (v68 or below) and ignores them; an older peer cannot decode a
  v69 provider run, so home and worker must both run v69.
- protocol 288 adds `ListAppInstallations`, `GetAppInstallation` and
  `GetAppInstallationJournal` on the same local/relay terminal path. The kernel
  derives ownership from the authenticated caller; requests cannot name an owner
  or host path. Local IPC uses the existing linked-user identity bridge, with the
  local identity used for an unlinked kernel. Unverified relay callers cannot
  inherit that local identity. Lists use an exclusive `after` installation ID
  and a `limit` of 1–100 (default 50); journals retain at most 64 completed updates
  plus the pending update. Generations are opaque decimal strings in client
  projections. Approval handles, authority references and host paths remain
  private. `AppRequestFailed` returns stable bounded error codes. These inspection
  requests do not stage, approve, activate or execute an App; installed metadata
  does not assert worker health or sandbox verification.
- protocol 289 adds `BeginAppPackageUpload`, `PutAppPackageUploadChunk`,
  `GetAppPackageUpload` and `AbortAppPackageUpload`. Every terminal uses the same
  authenticated kernel path and opaque owner-bound upload handle; clients cannot
  provide an owner, host path or expiry. Begin binds a client retry ID to an exact
  size and SHA-256. Chunks are at most 512 KiB decoded and acknowledge only durable
  offsets. Repeated begin/status/chunk requests consult the upload ledger rather
  than the transport result cache. Abort retains its receipt until the original
  30-minute expiry and cannot resurrect through a delayed begin retry. Package
  bytes are omitted from command/audit payloads and Debug output. The
  `AppPackageUploadStatus` response exposes bounded progress and phase, with
  stable `AppRequestFailed` codes. Uploaded bytes are untrusted; this transport
  does not enroll a publisher, approve capabilities, activate or run App code.
- protocol 290 adds `app` to the existing agent extension grant/revoke and
  serialized binding contracts. Its name is an installation ID; environment,
  credential and max-safety overrides are invalid. A binding selects App tools
  and does not cache permission or assert that a worker is running. Explicit
  user grants and permitted agent self-grants use the same binding mutation;
  agent requests use the existing Ask/YOLO policy and RuntimeInteraction path.
  SDK 0.2 event declarations include a signed positive `schemaVersion` and
  require a kernel protocol floor of 290. Occurrences include `occurredAtMs`
  and, for scheduled bindings, `scheduleRevision`, preserved on retries. The
  worker frame remains v1; the SDK payload and binding snapshots are versioned
  together. These contracts do not yet assert App workflow delivery readiness.
- protocol 294 pairs SDK 0.6 with the worker's bounded HTTP stream contract:
  `http.open`, `http.write`, `http.headers`, `http.read` and `http.cancel`.
  The worker frame remains v1. Stream IDs are opaque and scoped to one worker;
  each operation rechecks the installation's current signed network authority
  on the kernel writer. DNS resolution, peer-address checks and TLS happen in
  the kernel. This slice supports anonymous HTTPS to declared origins/methods;
  credentials and critical-operation receipts are not yet connected, so routes
  requiring them return explicit errors. Neither redirects nor retries occur
  implicitly. Buffered SDK requests compose the stream operations under one
  original deadline and size bound. Cancellation and failed response publication
  dispose of the exact stream; a lost body chunk cannot be silently retried.
- protocol 297 adds `BeginAppPublisherEnrollment`, `GetAppPublisherEnrollment`
  and `CancelAppPublisherEnrollment` with `AppPublisherEnrollmentStatus`.
  Begin carries a public Ed25519 key, publisher/key identities, an exact decimal
  expected revision and the session for human review. The transport derives the
  owner; it cannot supply a trust decision or approval authority. Only the
  kernel's private RuntimeInteraction challenge can enroll the key. Status
  reports a historical approved revision, which later revocation may supersede.
  Stable request IDs use the owner's durable ledger across local/relay/browser
  requests; responses bypass the older caller-independent transport cache.
- protocol 296 pairs SDK 0.7 with worker-global `fetch` and `chariox.http.fetch`.
  Native Web value objects and body streams use the existing five kernel HTTP
  operations; each redirect opens a newly authorized destination under the
  original lifetime. The shared Fetch fixture pins response behavior and limits.
  Raw HTTP and event wire fixtures retain their unchanged protocol-294 floor.
- protocol 295 adds `BeginAppInstall`, `GetAppInstallOperation` and
  `CancelAppInstallOperation` on the existing authenticated terminal path.
  Begin durably binds a retry ID, session, opaque upload and package digest
  before slow verification, returning operation status promptly. Retained kernel
  work verifies the package against already enrolled trust and presents its
  signed metadata/capabilities through the existing human interaction. The App
  and terminal request cannot supply an owner, key enrollment, approval or host
  path. Restart issues a fresh pending decision; it does not restore consent
  from an unanswered interaction. Status preserves historical operation identity,
  and cancellation fences preparation/activation on the same durable writer.
  Generations remain opaque strings. (Information-set declarations were later
  removed from the package contract.)
- protocol 344 merges the Chariox Apps line (protocols 288-297 above, developed
  on the Apps branch in parallel with main's 298-343) onto main. It adds no shape
  beyond those two lines; clients depending on App requests require 344.
- protocol 345 adds owner-scoped App worker control and automations on the
  same local/relay terminal path. `GetAppWorker` and `ControlAppWorker`
  (`start`/`stop`/`restart`) return `AppWorker` with a phase of `not_started`,
  `starting`, `running`, `dormant` (idle-stopped; the next tool call, wake or
  event starts it), `stopped` or `failed`, plus `enabled` (false after a user
  stop, which on-demand use never overrides). `restart` is a user stop followed
  by an explicit start; if that start fails the App stays stopped (`enabled`
  false) until the next explicit `start`. `ListAppAutomations`,
  `ConfigureAppAutomation` (expected revision zero creates) and
  `DisableAppAutomation` return `AppAutomations`/`AppAutomation`; one automation
  routes one App event to one workflow endpoint and queue, resolved under
  workflow ownership. The kernel derives the owner; requests name only the
  installation and cannot supply an owner, generation or host path. Automation
  requests use the active release's verified catalog and need no running worker.
- protocol 346 adds `OpenAppView {session_id, installation_id}`, which returns
  `AppViewOpened {installation_id, target_id, origin}`. The view is a managed
  Tab in the session's Room browser, so people and agents share one DOM and
  profile. The kernel re-verifies the active release and serves only its signed
  `ui/` files on a per-owner, per-installation `https://app.<label>.invalid`
  origin through browser request interception. Each installation has a distinct
  registrable domain beneath the reserved `.invalid` suffix: parent-domain
  cookies and `document.domain` cannot cross installation boundaries. Upgrading
  from the former shared parent origin closes legacy App Tabs; browser-local
  storage from those origins is not migrated (kernel-owned App data is retained).
  All other requests from the Tab
  are blocked, a strict CSP applies, and popups are closed. The room-controller
  relay command `app_view` (`open`, `calls`, `respond`) carries this between
  the home and worker kernels. `window.chariox.call(tool, input)` runs the App's
  own tool as the human owner through the same catalog validation and durable
  path as agent tool calls; the kernel binds each call to the Tab's
  installation, never to page-supplied identity. A view call runs as the view
  owner whoever drives the Tab (a person or an agent in the shared Room): the
  view is the owner's surface and there is no separate view privilege
  (V-SDK-04); critical effects still require kernel human validation, which a
  view click cannot supply. The App document's CSP includes
  `sandbox allow-scripts allow-same-origin allow-forms` (no popups, top
  navigation, downloads or modals), responses send `X-DNS-Prefetch-Control:
  off`, and WebRTC constructors are removed before App code runs (an in-page
  defense per document; a browser-level WebRTC policy is future work). Every new
  controller CDP connection (whatever command caused it) drops the previous
  connection's App Tabs and closes App-origin Tabs it does not own, before that
  command lists any Tab (a slice restart's restored App windows are closed
  before the Room start's reconcile, and one still closing drops out of it
  instead of failing it). Each poll
  reports the controller's open App targets; the kernel drops bindings for
  closed Tabs registered before that poll and stops polling when none remain.
  Each call also names the Tab's document (its top-level CDP loader) and each
  poll reports every open Tab's current document: a call whose Tab closed,
  reloaded or navigated is cancelled (the worker gets `cancel` and the call's
  slot is freed), and the controller answers a call only in the document that
  made it. These fields are optional; an older controller's calls end only with
  their Tab. UI files are limited to 2 MiB per view.
- protocol 347 adds `UninstallApp {installation_id, expected_generation}`,
  returning `AppInstallation` with no active release. A stale
  `expected_generation` returns `conflict` before any side effect. Otherwise the
  kernel records a user stop, withdraws the dormant catalog, then deactivates the
  installation at that generation, fencing all prior generations; an update
  committed in between returns `conflict` and leaves the App user-stopped. Open
  App views stay on screen but are unbound, so their calls fail. App data, user
  workflows and agents are retained; wakes and events of the inactive
  installation are refused by the normal start gate. The installation keeps
  the release its data belongs to; its automations are disabled and its inbox
  routes and connection grants removed. `BeginAppUpdate` on such an
  installation, at its current generation, is a reinstall into the kept data:
  the same publisher only, no data-schema downgrade, and always a new approval
  ("Reinstall App"), even for unchanged capabilities. Nothing removed at
  uninstall comes back; the owner adds automations, routes and grants again.
- protocol 348 adds `GetAppLogs {installation_id, after_sequence?, limit?}`,
  returning `AppLogs {installation_id, entries}` oldest first (at most 200 per
  page). Entries come from the SDK's `log.write` (level `debug|info|warn|error`,
  message up to 4 KiB of UTF-8, object fields up to 8 KiB), stored per owner and
  installation with the last 1000 kept and 50 writes per second per worker.
  They are App-authored data: never written to the kernel log, and clients
  display control characters escaped.
- protocol 349 adds `BeginAppUpdate {session_id, request_id, installation_id,
  expected_generation, upload_handle, expected_package_digest}`, a local
  replacement of the caller's installation with a newly uploaded release of the
  same App and publisher. It returns `AppInstallOperationStatus` and then uses
  the install operation requests. A release that declares exactly the active
  release's capabilities is approved by kernel policy; any capability change
  asks the owner ("Update App"), and declining keeps the old release.
  A stale generation or another unfinished install/update of the installation
  is refused. The current implementation supports structured-state schema
  migrations: after approval it fences admission and snapshots the installation's
  structured state, drains the old worker (not a user stop), and runs the staged
  worker's migration steps before its health check. Commit requires the target
  schema and a successful health check. A failure before commit restores the
  structured-state snapshot and leaves the old generation active; interrupted
  migration retries rewind to that snapshot before running the steps again.
  The old generation restarts on use. App data is kept. The new
  generation must run to pass its health check, so an App the user had
  stopped is running after a committed update. Views opened
  on the old generation answer `APP_VIEW_STALE` until reopened. A tool call
  that meets the update fails with `APP_UPDATING`: one in flight when the old
  worker is drained (the App may have acted), or one that finds no worker
  while the approved update is under way. A call whose worker was stopped at
  its memory limit fails with `APP_MEMORY_LIMIT`.
- protocol 350: opening an App view foregrounds the App in its session and
  binds it to the session's focus agent with the same `ExtensionGrant::App` an
  explicit grant or an agent's self-grant creates (a direct user action, so no
  separate approval, under YOLO or Ask). `AppViewOpened` gains
  `bound_agent_id` (null with no focus agent or when the opener does not own
  the App). A later focus change binds the new focus agent too; bindings are
  additive grants. Uninstall revokes the App's grants from every agent, which
  refreshes their runtime tool catalogs.
- protocol 351: App view Tabs carry `app` in the Room snapshot
  (`{installation_id, panel?}`; absent on every other Tab). An App view opens
  in its own fullscreen browser window. Every App view has the private
  conversation panel: `app.panel` is a strip at the right of the desktop
  (380 CSS px, at most a third of the canonical width, full height) in desktop
  pixels, plus the session's focus `agent_id`. The App page lays out in the
  rest (the controller narrows its viewport) and has no panel API; the trusted
  terminal draws the focus agent's conversation in the strip, outside the
  App's page. A focus change updates every App Tab's `agent_id` at once, and a
  viewport change moves the panel; each change emits `TabsChanged`. While the
  slice's Room browser controller does not lay App pages out beside the strip
  (a controller that predates the automatic panel, or a viewport too narrow
  for it), App Tabs keep `installation_id` and have no `panel`. (Before this,
  the page reserved the area with `window.chariox.panel.reserve`.)
- protocol 352: `CreateAgentWorkflow {session_id, agent_id, reason:
  trigger|deploy, surface: web|tui|cli, alias?}` creates a visible workflow
  for one of the caller's own agents when it gets a trigger or deployment:
  one node for that agent and one entry endpoint (`AgentWorkflowCreated
  {workflow, endpoint, session}`); the client then completes the trigger or
  deployment setup on that endpoint. `WorkflowDefinition.origin
  {source_agent_id, reason, surface, created_at_ms}` records why it exists;
  the alias (default `<agent>-<reason>`) is ordinary and editable. Binding Apps
  or Extensions to an agent never creates a workflow. Metaagents cannot use it.
- protocol 353: the installation inbox. `CreateAppInboxRoute {installation_id,
  route_id, event_name, source_event_type, source_event_version}` routes one
  external event type to an App's signed `incoming` (or `both`) event;
  `RemoveAppInboxRoute {installation_id, route_id}` and `ListAppInboxRoutes
  {installation_id}` answer `AppInboxRoutes {installation_id, routes}`, each
  with `pending`, `delivered`, `failed` and `expired` occurrence counts. A
  route grants the App nothing else. Removing a route stops new acceptance;
  already accepted occurrences retain their original event and installation
  for delivery, and their receipts retain the normal dedupe window even if
  the route name is reused. The dedupe scope is `(owner_id, installation_id,
  route_id, occurrence_id)`, regardless of changes to the generator,
  connection or source event type/version. Within the retention window,
  reusing that scope with the same payload is a duplicate; a different
  payload is a conflict, refused and acknowledged by generator delivery.
  Use a new `route_id` for a different source whose occurrence ids may overlap.
  `ListAppInboxRoutes` lists only existing routes, so removal hides the retained
  occurrences' counts. Reusing the name includes that name's retained pending,
  delivered, failed and expired counts, even if its source changed. Uninstall
  still clears the installation inbox. An occurrence is validated against the
  active release's signed schema and recorded (deduplicated by route and
  source occurrence) before the source is acknowledged; the kernel then sends
  `events.deliver {name, occurrence_id, payload}` at least once, starting a
  stopped worker on demand. A handler error retries with backoff and fails
  (poison) after 8 attempts; an update or start waits without spending one;
  unsettled occurrences expire after 7 days; payloads are dropped when
  settled. The dedupe window is 14 days from acceptance: a replay within it is
  answered as a duplicate, and settled occurrences older than it are pruned
  (by the kernel's delivery pass, also for an idle or uninstalled App), so the
  `delivered`, `failed` and `expired` counts cover about the last 14 days.
  `TestAppInboxRoute {installation_id, route_id, occurrence_id,
  payload}` accepts one occurrence as a source would (`AppInboxOccurrenceAccepted
  {installation_id, route_id, occurrence_id, duplicate}`). Event
  generator subscriptions for routes follow with the packaged Slack App.
- protocol 354: user-selected external file grants. An App whose signed
  manifest declares `capabilities.externalFiles: ["user_selected"]` calls
  `host.pick_file`, which returns a pending reference. The kernel shows the
  App's owner a trusted kernel-operation prompt (id `app_file_pick_<operation>`,
  subject `file_pick:<operation>`) in their most recent session. Its only
  choice is Decline. The owner answers from a terminal with `GrantAppFile
  {session_id, operation_id, files: [{name, contents_base64}]}` (at most 8
  files, 512 KiB together so an answer fits one relayed request; final name
  components only,
  matching the App's accepted suffixes), which answers `AppFileGranted {operation_id, files}` and
  closes the prompt. Only the owner can answer; App code, views and agents
  cannot. Grants are private copies that the App imports once with
  `files.import`. They expire after 30 minutes, or when the App updates.
- protocol 355: `SaveAppFileExport {session_id, operation_id}` takes a copy of
  a file an App offered with `files.export`. The offer is shown to the owner
  as a kernel prompt with subject `file_export:<operation>` and a Decline
  choice. The reply is `AppFileExport {operation_id, name, contents_base64}`,
  released to the owner only. The prompt closes after the first save, but the
  owner may take the offer again until it ends, so a failed or cancelled local
  save can be retried: a client keeps `operation_id` and tells the owner how
  (both terminals show `/app file save OPERATION`). An offer ends when it
  expires or when the App that made it is updated or uninstalled. The name is
  a final name component with no control or invisible format characters. The
  terminal chooses where to save it (a browser download, or
  `/app file save OPERATION "PATH"`, which never overwrites a file).
- protocol 356: App view reconnection. A call from an open App view built for
  an older generation, or from a view the kernel lost track of (for example
  after a kernel restart; only the session host's own active installation),
  is answered `APP_VIEW_RELOADING`. The kernel then binds the Tab to the
  current generation and sends the room controller's `app_view` command
  `{op: "reload", target_id, entry, assets}`. That command serves the Tab the
  current generation's signed view assets and reloads it in place (same Tab,
  window and panel). An App can keep drafts across the reload in its own web
  storage. After a restart, the kernel polls every Room bound to a slice once,
  so leftover views reconnect instead of hanging.
- protocol 357: Tab accessibility outline. `GetRoomEnvironmentTabAccessibility
  {session_id, tab_id}` returns `RoomEnvironmentTabAccessibility
  {session_id, tab_id, document_revision, nodes, truncated}` for any Room
  member. `nodes` is the Tab's accessibility tree in document order
  (`element_ref`, `parent_ref`, `role`, `name`, and when set `value`,
  `description`, `disabled`, `focused`, and `states`: what a reader announces
  about the control, among `checked`, `not checked`, `mixed`, `pressed`,
  `not pressed`, `expanded`, `collapsed`, `selected`, `required`, `invalid`),
  bounded to 2000 (`truncated` is also set when the controller cut its
  snapshot at its own 5000-node bound, which cuts the deepest nodes first).
  The Room browser controller's snapshot nodes gain the same `states`, and
  the snapshot gains `accessibility_truncated` for that cut (both absent from
  older controllers, whose full 5000-node snapshot counts as cut). It holds
  what a reader announces: ignored nodes, inline text boxes, unnamed layout
  wrappers and text its parent's name already says (an aria-labelled button's
  text) are left out, and their children hang from the nearest kept ancestor.
  Nodes are in document order (depth first over the Tab's accessibility
  tree), so each follows its parent and hoisted text keeps its place.
  Terminals present it so App views and other pages can be read with a screen
  reader or keyboard; it grants no input.
- One App Tab (no shape change, with this release's kernel and controller):
  a Room holds one Tab per installation. `OpenAppView` again (from another
  terminal, or after a kernel restart) returns the same `target_id`, shows
  that Tab and navigates it again with the current assets. A view's first
  call after it loads re-projects the Room, so the Tab shows the App's title
  and URL. Bridge call ids are unique per document, so an answer meant for
  the previous document never resolves a call in the new one.
- protocol 358: App inbox routes fed by event generator connections.
  `CreateAppInboxRoute` takes an optional `connection {generator_id,
  connection_id, connection_scope, filter?}`: the owner's connection at an
  event generator (AEGS). The kernel checks the connection with that
  generator before it stores the route, then subscribes to
  `source_event_type` at the generator and
  claims the route at the event delivery service (AEDS) under an opaque
  `app-route-...` binding id that names no owner or App. A delivery for it is
  validated against the App's signed incoming schema and recorded in the
  inbox before AEDS is acknowledged; one that can never land (the route was
  removed, a different event type, a payload the schema refuses, or other
  content under an accepted occurrence id) is logged and acknowledged. The App
  receives `{source: {generator_id, connection_id, event_type,
  event_type_version}, occurred_at, text, metadata, artifacts, reply_context}`
  as the event payload, deduplicated by the source occurrence id.
  `AppInboxRouteSummary` shows the `connection`. The route grants the App no
  use of the connection beyond receiving these occurrences.
- protocol 359: App connection grants. `GrantAppConnection {installation_id,
  generator_id, connection_id}`, `RevokeAppConnection {installation_id,
  connection_id}` and `ListAppConnections {installation_id}` answer
  `AppConnections {installation_id, connections: [{generator_id,
  connection_id, granted_at_ms, actions}]}`. Only the owner can grant; the
  kernel checks the connection with its generator, and the App's signed
  manifest must declare that generator under `capabilities.connections
  [{generator, actions}]` (shown in the install approval). The App then calls
  `connections.list` and `connections.action` (see the App SDK wire contract):
  the kernel runs a declared action through the generator's reviewed action
  endpoint as the owner's pseudonymous event owner, with an idempotency key
  scoped to the installation. The App never holds the provider credential;
  anything else is refused (`CONNECTION_NOT_GRANTED`, `CAPABILITY_REQUIRED`).
  An action whose request may have reached the generator without a definite
  answer (no reply, or a 5xx from the generator or a gateway in front of it)
  fails with `APP_CONNECTION_OUTCOME_UNCERTAIN`. The kernel never replays it,
  and it is retryable only when the App supplied its own `idempotencyKey`.
- protocol 360 (retired in 365): an event binding moves to an App. `MoveEventBindingToApp
  {session_id, binding_id, installation_id, route_id, event_name,
  automation?: {automation_id, event_name}}` turns a workflow event binding in
  the kernel's event environment into an App inbox route on the same
  connection, scope, filter and event type (checked with the generator as
  `CreateAppInboxRoute` is). Under the event interest lock the binding is
  paused first, so the event service never routes its events twice; then the
  optional App automation sends the App's outgoing event to the binding's
  publication and queue, the route is created, and a binding with actions
  becomes a connection grant when the App's manifest declares that generator.
  Any refusal undoes the earlier steps, reactivates the binding and answers
  `AppRequestFailed`; success answers `EventBindingMovedToApp {binding_id,
  installation_id, route, connection?, automation?}`. The paused binding is
  kept for the owner to remove once the App serves its events.
- protocol 361: App sets. `GetAppSet {}` answers `AppSet {schema:
  "chariox.app-set.v1", installations: [{installation_id, app_id, release,
  capabilities, automations, inbox_routes, connections}]}`: the caller's
  active installations with the release, the signed capabilities they approved
  and their configuration, read through the same owner-scoped requests. It is
  the versioned description a kernel copy installs from (Phase 2); App data is
  never part of it. It fails closed: an installation that cannot be read
  completely, or whose release changes while the set is read, fails the whole
  request with its App error code, so a copy never pairs a release with
  another release's capabilities. Configuration (automations, inbox routes,
  connections) is read as it stands at that moment; an owner edit made during
  the read may or may not be included.
- protocol 362: resource filters. An event generator resource may carry
  `filter`, the event filter that narrows an App inbox route to it when other
  resources share its `connection_scope` (Slack channels share their
  workspace's scope and carry `{"event.channel": id}`). Clients merge it into
  the route's filter; no client special-cases a generator.
- protocol 363: App data deletion. `UninstallApp` gains `delete_data` (omitted
  when false): the kernel also deletes the App's structured state, wakes,
  logs and file handoffs and the release it kept, so the installation can no
  longer be reinstalled into. Where a platform gives Apps private storage, the
  supervisor deletes it first (the macOS worker's storage: #496 and its kernel
  wiring; Linux: the storage helper, #497). `UninstallApp` on an already uninstalled
  installation, at its current generation, deletes its kept data the same way,
  which also finishes a deletion interrupted after the uninstall. App
  installation summaries gain `data_kept`: uninstalled, with data an update
  can reinstall into. Linux App storage is root-owned: the storage helper's
  `delete` request (a new helper operation) removes it; an installation still
  leased is busy.
- protocol 364: the workflow event reply surface is removed; replies go
  through an App's granted connection (protocol 359). `CreateWorkflowEventBinding`,
  workflow event bindings and publication `event-bindings` templates drop
  `reply_mode`; `RemoteWorkflowTurnContext` drops `event_reply_enabled`; the
  `reply_to_event` runtime tool is gone. Workflows cannot post
  `notification.reply`: creating a binding that enables it is refused, and
  `event_action` refuses it for bindings persisted before 364. Older peers,
  persisted bindings and publication `event-bindings` documents that still
  carry the removed fields are read with them ignored.
- protocol 365: direct workflow event bindings are retired. Events reach
  workflows only through Apps: an App inbox route (protocol 358) receives the
  generator's events, and an App automation sends the App's outgoing event to
  an `event_based` publication. `CreateWorkflowEventBinding`,
  `ListWorkflowEventBindings`, `SetWorkflowEventBindingStatus`,
  `TransferWorkflowEventBinding`, `TestWorkflowEventBinding` and
  `MoveEventBindingToApp` are removed with their responses, as are the
  `event_context` and `event_action` runtime tools (they served direct-binding
  runs only) and `RemoteWorkflowTurnContext.event_context_enabled` /
  `event_actions_enabled`. `ListEventConnectionDependencies` answers
  `EventConnectionDependency {installation_id, route_id?, active}`: the App
  inbox routes (`route_id`) and App connection grants (no `route_id`) that use
  the connection; `EventConnection.attached_trigger_count` counts the active
  ones, and `RemoveEventConnection` (with `confirm`) is refused while any
  remains. `EventConnectionRemoved` loses `deactivated_bindings`.
  `EventDeliveryStatus.active_route_count` counts active App routes. The kernel
  claims only App routes at the event service and resumes only its default
  environment; a delivery for any other binding id is logged and acknowledged,
  never retried. Exported publication packages no longer carry
  `event_bindings_path` or `event-bindings.example.json`; a server reading an
  older package ignores both. Sessions and durable workflow state written by an
  older kernel load with their bindings ignored; peers that still send the
  removed turn-context fields are read with them ignored (relay peer
  protocol 69).
- protocol 366: a workflow publication carries its App plan. The first
  successful `ExportWorkflowPublicationPackage` by the publication's owner (the
  deployment preparation) pins `WorkflowPublicationDefinition.apps`
  (`chariox.publication-apps.v1`): each App granted to an agent of the
  publication snapshot or feeding the publication through an active App
  automation, with its source installation, app id, release version, publisher
  id, key id and key fingerprint, package digest, data schema version and
  approved capabilities digest, its grants by agent (`agent_id`, `node_ids`),
  the automations targeting this publication, its active inbox routes (route,
  event, source event type/version, generator connection) and its connection
  grants. It names generator connections but carries no App data and no
  secret. A pinned plan never changes, so later exports — including the
  deployment bind's digest check — do not follow App updates. The package of
  an App-bound workflow adds `apps.json` (the plan) and the deployment contract
  `capabilities.apps` (its `apps`); an App granted to an agent but not
  installed fails the preparation, and an App-bound publication (an App granted
  or an App automation feeding it) without a plan (never prepared by its
  owner) fails the export; a failed export pins nothing. `requirements.json`
  follows the publication snapshot's agents instead of the source agents'
  current grants, and App grants are no longer refused there.
- protocol 367: `PrepareDeploymentApps {session_id, request_id,
  publication_ref, deployment_id, release_id, package_digest}` asks the
  publication's owner once, in one kernel-operation interaction of the
  publication's session, to deploy the workflow together with the Apps of its
  pinned App plan: it lists each App release (app, version, publisher, key
  fingerprint, signed capabilities) and each generator connection the copy
  will use. Each pinned release is re-read from the local release store and
  re-verified against the owner's current publisher trust first; a changed
  signer or capabilities digest fails with `Conflict`. It answers
  `DeploymentAppsConsent {consent: {request_id, interaction_id, deployment_id,
  release_id, package_digest, status, expires_at_ms}}` with `status`
  `awaiting_approval`, `approved`, `declined` or `expired` (the owner has five
  minutes to answer, and an approval approves installs for five minutes after
  it, then reports `expired`); the same `request_id` replays the record and
  reports the answer, other facts under it are a `Conflict`, and a new
  `request_id` asks again.
  The answer is recorded durably by the kernel; no request can supply an
  approval. A deployment copy's install (from the local release store, tagged
  with its deployment) is approved by the `kernel_deployment_consent:<interaction>`
  policy only for a release in an approved consent — same app, publisher, key
  fingerprint, package and capabilities digests — whose capabilities digest
  the owner approved interactively before (`kernel_operation_human`); anything
  else asks the owner as for any install. Copy installations are absent from
  `ListAppInstallations` (its page query excludes them) and carry
  `AppSetInstallation.deployment_id` in the App set. `PreviewDeploymentApps {session_id, publication_ref}` is read-only
  and needs no export: it answers `DeploymentAppsPreview {publication_id,
  pinned, plan}` with the pinned plan (`pinned: true`) or else the plan the
  owner's current App set gives, each App with its signed `capabilities`
  (the pinned releases' are re-read from the release store and re-verified);
  `plan` is `null` when the workflow uses no App. Only the publication's owner
  may preview.
- protocol 394: owner-side revoke of App file grants. `RevokeAppFileGrants
  {installation_id, operation_id?}` ends the caller's open file requests
  (`host.pick_file`, protocol 354) of that installation, or only the one named.
  An unanswered request's prompt closes; granted files the App has not
  imported are dropped, including one an import holds at that moment (an
  import that is already publishing still lands). The App reads the request as
  `expired`, and `files.import` of its grants fails `NOT_FOUND`; files it
  already imported stay in its private data. It answers `AppFileGrantsRevoked
  {installation_id, requests, files}`: the requests it ended and the unimported
  files it dropped. An `operation_id` that is not this installation's is
  `NotFound`; one that already ended ends nothing. Terminals: `/app file
  revoke INSTALLATION [OPERATION]`.
- protocol 397: agent substitutes are per-turn only. When a provider fails a
  turn — an error result, a structured error code (Codex `codexErrorInfo`,
  Claude `StopFailure`, OpenCode session errors), the provider process exiting
  mid-turn, or a provider timeout the kernel detects — the kernel reruns that
  same turn, as the same active prompt, on the agent's next configured
  substitute in order, and on the one after it if that also fails. A user
  cancel is not a provider failure, and a turn that completes is never rerun
  whatever its text says. Each rerun records a notice naming the cause, for
  example `This turn runs on claude-opus-5-5 because gpt-6.1-sol failed: model
  at capacity (server_overloaded).`; when no substitute is left the turn fails
  with its provider error. The substitute's provider run serves only that turn:
  the next turn starts on the agent's configured profile, and the agent's
  profile never changes. `AgentSubstituteAction` loses `Activate` and
  `Primary`; `AgentInstance` loses `primary_provider`, `primary_model`,
  `primary_effort`, `primary_account_profile`, `active_substitute_index` and
  `last_substitution`. An agent persisted on a substitute by an older kernel
  loads on its primary profile. Remote (leased) agents are not rerun on a
  substitute.
- protocol 407 adds `quarantined` to `AppWorker.phase` on the same local/relay
  path. A failed worker exhausting the supervisor restart limit (four consecutive
  failures) reports `quarantined`; failures one through three remain `failed`
  during restart backoff. The original `failure` diagnostic is retained, and
  `enabled` still means the user has not stopped the worker. Recovery requires
  the existing `ControlAppWorker` action `start`, which clears the failure count.
  Clients display "quarantined · explicit start required" and offer this action.
  The focused quarantine relay test covers the status boundary, durable
  explicit-start admission reset, and an unaffected neighbouring App; it does
  not dispatch the client-facing start request. Clients requiring this distinction
  depend on protocol 407; other web/native minimums need not change.
- App-bound local deployments (P1.20, no request or response shape change): a
  bound `local_runtime` deployment of a publication with a pinned App plan
  runs as a pinned independent copy on the owner's kernel, not in the source
  session. Starting it (`BindWorkflowPublicationDeployment`, recovery, a
  runtime restart) first verifies the package digest, then requires the
  owner's approved `PrepareDeploymentApps` consent for exactly that deployment,
  release and package digest (otherwise the bind fails), and then, idempotently:
  materializes a hidden session with runtime key
  `deployment:<deployment_id>:<release_id>` that keeps the publication's kind
  (an `event_based` trigger stays event-based); installs each pinned release as
  the deployment's own copy through the consent policy above (one install
  request per deployment, release and App; a copy of another release of the
  same data schema version is updated in place, a different schema version
  fails closed; an install that needs the owner's answer, fails or does not
  finish in two minutes fails the bind with the reason); moves the copy
  agents' App grants from the source installations to the copies;
  configures the plan's automations on the copies targeting the copy's
  publication, endpoint and queue; grants the copies the plan's generator
  connections (same owner and kernel: no new sign-in, no generator change);
  and recreates the plan's inbox routes on the same connections with the
  filters of the owner's routes. Handover: while a copy's route is active the
  owner's own route on the same event interest is paused (it accepts no
  occurrence, and its subscription claim is sent `active: false`); it resumes
  when no copy route claims that interest. Copies, routes, automations and
  grants the release no longer names are removed; the hidden sessions of the
  deployment's other releases are deleted, so binding a previous release
  (rollback) re-applies that release's plan; this is fail-closed, so a rollback
  that cannot apply leaves no copy running for either release. When applying
  the copy fails, the copy's routes are removed so the owner's routes resume.
  Every install request retries from the latest one it made: a spent request
  (failed, cancelled, or whose copy a `Stop` removed) moves on to the next, so
  a deployment can be stopped and started any number of times. Each App's
  install is awaited in turn inside the bind, so a bind of many new Apps can
  outlast a client's timeout; recovery completes it. `chariox serve source` then runs
  against the copy session; the source publication keeps the binding and
  records `deployment.app_copy_session_id`, and the copy's publication carries
  the same binding so its endpoint registration uses the deployment's stable
  tunnel. Recovery skips copy publications and re-applies the copy from the
  source. `ControlWorkflowPublicationRuntime` `Stop` of such a deployment
  uninstalls its copies with their data (and so their routes, automations and
  connection grants), resumes the owner's routes and deletes the copy
  sessions. The kernel also removes, on its runtime reconcile, the copies whose
  source publication was deleted (with its session), disabled, marked stopped
  or bound to another deployment; the owner's routes resume. Events an
  automation accepted but could not deliver are recorded in the App's log.
  App data is never copied from the owner's installations.
- protocol 377: App plans are per release, not pinned per publication. Every
  successful `ExportWorkflowPublicationPackage` by the owner (a new deployment
  release) reads the owner's current App set and packages that plan; the
  kernel records it by the export's package digest in
  `WorkflowPublicationDefinition.release_app_plans` (`{package_digest, plan}`,
  newest last, the last 16 kept) and `apps` is the latest plan. A granted App
  that is no longer installed fails the export. The deployment bind, recovery
  and a rollback (binding an earlier release) re-export with that release's
  recorded plan, so the package digest still verifies and each release runs
  with the App versions it was exported with; an App-bound release whose plan
  is not recorded fails the bind (a publication prepared before 377 keeps its
  single plan for all releases). A copy is updated in place to the release's
  App release; a newer data schema migrates the copy's data as any App update
  does, and a release with an older schema than the copy fails closed.
  `PrepareDeploymentApps` consents to the plan of exactly the requested
  package digest (unknown: `InvalidRequest`); when the owner already approved
  this deployment with exactly the same App releases (app, publisher, key
  fingerprint, package and capabilities digests) the consent is recorded
  approved without a prompt, and its install window starts then.
  `PreviewDeploymentApps` answers the plan the next release would package (the
  owner's current App set; `pinned` now means a release was prepared) and,
  with the new optional `package_digest`, that release's recorded plan as
  `release_plan`, each App with its stored release's capabilities.
  The deployment contract's `compatibility.minimum_local_daemon_protocol_version`
  is now the package format's protocol (367), not the exporting kernel's: the
  bind and recovery re-export a bound release and compare digests, so a kernel
  protocol bump must not change existing packages. It is raised only when a
  package needs a newer kernel to run. A release exported before 377 keeps the
  publication's single pre-377 plan after later releases record their own; a
  377 release whose plan was pruned (more than 16 releases ago) has none, so
  its bind fails with "no App plan recorded" and `PrepareDeploymentApps` and
  `PreviewDeploymentApps` refuse it. A publication that had Apps and uses none
  now (its last grant or feeding automation removed) packages and records an
  explicit empty plan (`apps: []`) for its next release, never the previous
  release's; a release whose plan names no App binds and runs from the source,
  with no copy and no Apps consent: `PrepareDeploymentApps` answers it `approved` at once, without a prompt or a stored consent. A release exported before the publication used any App (no plan recorded while later releases record theirs) is treated the same way.
- protocol 378: every successful owner `ExportWorkflowPublicationPackage`
  also records the release's inputs digest in
  `WorkflowPublicationDefinition.release_inputs` (`{package_digest,
  inputs_digest}`, newest last, the last 64 and the bound release's kept):
  the package digest of its files without the kernel's templates (`.env.example`,
  `run.sh`, `README.md`, `public/index.html`, `public/app.js`,
  `public/styles.css`), with `deployment-contract.json` counted without the
  fields a kernel upgrade changes (`package_id`, `artifact.content_digest`,
  `compatibility.minimum_kernel_version` and the template entries of
  `presentation.assets`). A kernel change to how the rest of the contract is
  derived (routes, credential slots, capabilities) still fails the bind of an
  existing release, which must then be rebound. The deployment bind and
  recovery verify a release with a recorded inputs digest by re-exporting it and
  comparing inputs digests, so a kernel upgrade that changes those templates
  (for example the contract's `minimum_kernel_version`) keeps its deployments
  bound; any change to the workflow's own files still fails the bind. A release
  without a record (exported before 378, or pruned) is verified by its whole
  package digest, as before.
- protocol 381: `AppInstallOperationStatus` phase `queued`. An install or
  update the owner (or kernel policy) approved stays `queued` until the kernel
  claims its start, usually while it waits for a free App worker slot (at most
  four Apps run at once); it is `starting` from the claim until it commits.
  Earlier kernels reported this wait as `awaiting_approval`, then as
  `starting`. Clients treat `queued` like any unfinished phase.
- protocol 392: critical approvals need the Chariox passkey (the encrypted
  vault's passphrase). A `RuntimeInteractionChoice` may carry
  `requires_passkey: true` (absent means false); only kernel-operation
  decisions set it (today the approve choice of an App's critical-action
  validation), and an agent's interaction that sets it is refused.
  `RespondToInteraction` gains optional `passkey` (a string, redacted in
  command projections, never logged or stored) and `passkey_remember_minutes`
  (1 to 15). Answering such a choice needs a passkey the kernel verifies
  against a pinned commitment to the vault key (the KDF parameters and a hash
  of the derived key, kept durably and taken from the vault the boot
  configuration names when the kernel first unlocks it or first sees a
  passkey that opens it; a later vault path or file change never moves it,
  only a passphrase change does, see below; the vault's unlock state is
  unchanged), or an open remember window: a verified passkey with `passkey_remember_minutes` accepts
  the owner's critical approvals without it for that long, in kernel memory
  only, independent of the vault's own unlock window. Otherwise the answer is
  refused with `PASSKEY_REQUIRED`; a wrong passkey with `PASSKEY_REJECTED`;
  after five consecutive wrong passkeys the owner's attempts are refused with
  `PASSKEY_RATE_LIMITED` for 30 seconds, doubling per further failure up to
  15 minutes. Without the encrypted Chariox vault the approval fails closed
  (`PASSKEY_UNAVAILABLE`). Denying needs no passkey, and a passkey sent for any
  other choice is ignored. Each check appends a durable
  `critical_approval.passkey` event (outcome only). `/credential vault
  manage` offers Change passphrase (`ManageCredentialVault` answers with
  action `passphrase_changed`; no request or response shape changes): three
  secret prompts take the current passphrase and the new one twice. For the
  boot vault this rotates the passkey: the current passphrase must verify
  against the pin, under the same limit; the pin moves with the re-keyed
  vault file through a durable `critical_approval.passkey_verifier_move`
  record that a restart settles from the file, so the two never disagree;
  every remember window ends; and a `critical_approval.passkey_rotation`
  event records the outcome only. Clients prompt for the
  passkey with hidden input only for a `requires_passkey` choice; to a remote
  kernel it travels inside the end-to-end encrypted relay request.
- protocol 344 and relay peer protocol 58 add `room_browser_available` to the
  home-authored remote extension manifest. The field defaults to false and is
  omitted when false. It advertises the Room's shared browser independently of
  the leased agent's execution placement. It does not grant authority: home
  validates current Room membership, worker/run binding, and Environment
  binding on every forwarded browser call. Client minimums remain unchanged.
  Successful Environment binding and deletion enqueue manifest refreshes for
  agents already leased in the Room, without waiting for another prompt.
  Refreshes share the leased-agent operation lane with grant updates and
  retries, and recompute the manifest after acquiring that lane. The binding
  operation does not wait for relay I/O. Until delivery succeeds, tools may
  remain hidden after binding or advertised after deletion; forwarded calls
  still validate the current binding at home. Stop and input release do not
  remove the Environment binding. Live validation must cover updates to an
  already-running agent, not only an Environment bound before agent launch.
- protocol 345 adds owner-authenticated, read-only
  `GetManagedEnvironmentReimageReceipt` and `ManagedEnvironmentReimageReceipt`.
  The home kernel reads Cloud's existing receipt route using its authenticated
  Cloud session and rejects a response for a different environment. This request
  does not admit a rebuild, authorize context transfer, or introduce another
  runtime authority. Clients using this request require protocol 345; existing
  web/native minimum versions remain unchanged. The request/response snapshot
  and managed-control drill cover owner/session admission, URL escaping,
  environment binding, and incomplete versus finalized receipt projection.
  `apps/cli/scripts/path1-cloud-reimage-capture.mjs` uses this shared request
  against the reviewed local home kernel. It checks the selected operation,
  generation and release binding and retains only allowlisted receipt fields
  in a new external mode-0600 file. It is not the full fresh-equivalent rebuild
  gate and does not independently verify Cloud's receipt digest.
- relay peer protocol 60 adds durable queued-steer receipts and the
  `ReconcileLeasedPromptSteerReceipt` operation. It carries the exact queued
  home prompt, target active home prompt, worker provider run, and execution
  lease. The existing `GetLeasedPromptReceipt` remains read-only. A receipt names the exact queued
  home prompt, target active home prompt, worker provider run, and execution
  lease, and reports `steer_dispatching`, `steer_accepted`, or
  `steer_rejected`. The worker persists `dispatching` before provider enqueue,
  changes it to accepted only after local dispatch accepts the input, and
  records rejection only when non-admission is known. After a lost steer reply,
  the home keeps the exact queue item durably held and queries only the current
  matching worker binding. It removes that item after an exact accepted receipt
  or releases it after an exact rejected receipt. Under the worker run lane,
  reconciliation records a durable rejected tombstone when no receipt exists;
  a delayed original steer then encounters that tombstone and cannot enqueue.
  Dispatching, stale, or conflicting receipts never replay or promote the uncertain item. The
  focused fake-relay regression covers lost-reply acceptance, restart hold,
  receipt reconciliation, and at-most-once worker input. The local daemon
  request and response shapes are unchanged, so client minimum versions do not
  change; home and worker kernels must both support relay peer protocol 60.
  Lease operations also bind the authenticated home daemon and sender key to
  the worker's execution lease. Leases created before that binding was stored
  cannot pass the new authorization check after an upgrade; the home must
  rebind them with a current peer protocol instead of reusing the old lease.
- relay peer protocol 61 adds `ResolveLeasedProjectEnvironmentSetupTarget` and
  `LeasedProjectEnvironmentSetupTargetResolved`. Before starting Project setup,
  the home kernel derives the selected worker from the agent binding and asks
  that authenticated lease worker for its actual platform. An explicitly
  supplied worker or platform remains an assertion and must match the resolved
  values; empty fields request resolution. The worker verifies the leased
  agent and home session/agent binding before replying. The local-daemon shape
  and client minimum versions do not change; remote Project setup requires
  both home and worker kernels to support relay peer protocol 61.
- relay peer protocol 62 gives original leased prompts a durable worker admission
  receipt using the existing `GetLeasedPromptReceipt` response. A receipt with
  no `target_home_prompt_id` and an exact `execution_lease_id` reports
  `steer_dispatching` before the worker starts provider admission, then
  `active` or `completed` after admission, or `steer_rejected` only when
  non-admission is known. A lost reply must be reconciled against the current
  leased agent and execution lease; an absent or dispatching receipt is not
  proof that resubmission is safe. Worker restart retains the receipt, so a
  delayed duplicate cannot launch a second provider prompt. The wire shape
  is unchanged, but the receipt's original-prompt semantics require both home
  and worker kernels to support relay peer protocol 62. Local-daemon client
  minimum versions do not change.
- relay peer protocol 63 adds
  `AcknowledgeLeasedProjectEnvironmentSetupDefinition` and
  `LeasedProjectEnvironmentSetupDefinitionAcknowledged`. When a leased worker
  generates a utility-origin Project definition whose persistence belongs to
  the home kernel, it keeps the setup attempt in `Preparing` and returns the
  definition in the existing setup status. The home persists the definition
  before acknowledging it. The request binds the leased-agent ID, operation ID,
  attempt, project ID, home session and agent IDs, and the definition digest;
  the response must echo the operation, attempt, project, and digest. The home
  checks the owner-scoped setup target and active binding. The worker accepts
  the acknowledgment only for its active attempt and lease, matching home
  session/agent and project, and a digest equal to both the staged definition's
  recomputed digest and its status digest. An identical acknowledgment is
  idempotent; a mismatch or conflicting repeat is rejected. Worker validation
  starts only after the matching acknowledgment; if it does not arrive within
  the existing 30-second wait, setup fails before validation. Both home and
  worker kernels must support relay peer protocol 63 or newer; a missing or
  older worker version is incompatible and requires rebinding. Local-daemon
  request/response shapes and client minimum versions do not change.
- protocol 350 adds optional `disk_layer_mb` and `disk_home_mb` to the Linux
  slice settings in the existing user-config response and coordinates the
  signed managed auto-stop quiescence HTTP contract. Quiescence adds no
  LocalDaemon request/response variant. Kernel-to-Cloud REST v1 uses
  `/v1/managed-kernels/auto-stop/quiescence/poll`,
  `/v1/managed-kernels/auto-stop/quiescence/ack`, and
  `/v1/managed-kernels/auto-stop/quiescence/release-ack`. Cloud must deploy and
  verify all three routes and their validators before protocol-350 kernels roll
  out. The legacy timer/direct auto-stop path must be disabled before either
  side is enabled; the producer and receiver must not be deployed independently.
  Missing routes, timeouts, malformed or unsupported v1 responses, and missing,
  invalid, or stale acknowledgements leave the stop pending and retain the
  admission fence. There is no legacy auto-stop fallback. The quiescence
  contract does not change web, native, or CLI minimums because clients do not
  consume it. The optional disk-cap fields do not change minimums for clients
  that do not use them; a client that reads or writes those fields must gate
  that capability at protocol 350.
- protocol 351 adds `CreateDisposableWorker`, `GetDisposableWorker`,
  `ReleaseDisposableWorker`, `KeepDisposableWorkerRunning`,
  `PrepareDisposableWorkerContextTransfer`, and `KeepManagedEnvironmentRunning`.
  Disposable selections bind `allocationId`, `homeKernelId`, and
  `homeRelayRealmId`. The authenticated home kernel derives Cloud account and
  session authority; clients must not supply credentials or account authority.
  Before mutating an existing allocation, the home reads it and verifies its
  allocation, home, and realm binding. Context-transfer tickets also bind the
  returned worker machine and kernel. Create preserves `clientRequestId` for
  Cloud idempotency; transport failure must not trigger an automatic mutation
  retry or a fallback to a different home or Cloud authority.
  Protocol numbers 344–365 were independently allocated on the Apps branch;
  a numeric minimum alone does not prove these controls exist. Protocol 366
  adds `RelayStatus.capabilities`, defaulting to an empty list when absent.
  Clients consuming these controls require protocol 367 and must query the
  selected home through its authenticated kernel connection before mutation.
  Require `disposable_worker_control_v1` for disposable controls and
  `managed_environment_keep_running_v1` for managed keep-running. Verify the
  response's daemon and machine binding; reject missing capabilities even on
  a numerically newer kernel. Relay discovery advertises the same markers but
  is not sufficient proof of the connected kernel's support. These are kernel
  implementation capabilities, identical on ordinary and managed kernels,
  not permission grants or an alternative to operation authorization.
  Other client minimums remain unchanged. Deploy the matching Cloud allocation,
  context-transfer, and keep-running routes before enabling these controls on
  a signed capability-bearing home, then connect the updated client. A 366
  release does not incorporate the divergent Apps branch or automatically
  authorize upgrades from its releases; signed compatibility must name proven
  predecessor contracts. The focused source checks
  are `ipc-disposable-worker-requests.test.ts`,
  `local/api/tests/protocol_shapes/disposable_worker.rs`, and
  `runtime/disposable_worker_control/tests.rs`. Live acceptance must create
  through the selected home, observe the returned allocation and worker
  identities, reject foreign-home selection, exercise keep-running and context
  transfer, and release with authoritative provider-resource deletion evidence.
  Source tests alone do not prove that live drill passed.
- protocol 367 adds guarded Unix control sessions. Legacy Unix IPC remains one
  length-prefixed request, one response, then EOF. For disposable-worker controls
  and managed-environment keep-running, the client sends
  `{"GuardedControlSession":{"version":1}}` on one socket. The kernel dispatches
  ordinary `RelayStatus` and replies with
  `{"session":{"version":1},"response":{"RelayStatus":{"status":{...}}},"error":null}`
  without closing. After validating capabilities and kernel identity, the client
  sends exactly one guarded control request on that same connection. The kernel
  uses the ordinary local caller/router and closes after its response. Frames
  retain the 1 MiB limit and 30-second I/O deadlines; the TypeScript client also
  bounds the whole exchange by its request timeout. Unsupported negotiation, EOF,
  timeout, or capability mismatch fails closed, with no fallback connection or
  automatic mutation replay. Numeric versions never replace capability checks.
  This describes the pre-KA framed Unix transport. KA protocol 404 replaces
  that listener with the shared kernel websocket at `ws+unix://`, admitted by
  OS process identity and access grants. Protocol 451 grants ordinary authority
  throughout the local kernel, including its managed execution environments.
  First-party terminal control uses the authenticated TCP or relay websocket
  path; external Unix grants cannot attach to another kernel.
- protocol 402: every connection has a class from a fixed vocabulary:
  `terminal` (the kernel's local token on TCP loopback, or a relay client with
  a user id), `external_agent` (reserved for access grants, not assigned yet),
  `kernel_agent` (an agent the kernel launched, by its per-run runtime MCP
  bearer), `host` (`CHARIOX_KERNEL_LOCAL_AUTH_TOKEN(_FILE)`), `relay_peer`
  (another kernel or a hosted service, by its relay identity) and
  `unauthenticated` (neither: a laptop connection without its token in log
  mode, a relay client without a user id). The kernel assigns it at admission,
  records it on the caller and in command traces, and each
  `critical_approval.passkey` event names it as `connection_class` (null for
  the kernel's own callers). A `RespondToInteraction` `passkey` from a
  `kernel_agent`, `host`, `relay_peer` or `external_agent` connection is
  refused with `PASSKEY_NOT_ACCEPTED` before verification: it is not audited
  and does not count toward the owner's limit. Nothing else changes; which
  connections answer as terminals is unchanged until enforcement.
- protocol 403: passkey popups (kernel access plan D1). Every passkey prompt
  is a kernel-owned pending interaction; today each critical approval is one.
  It is projected as a popup to every terminal connected as its owner,
  attached to its session or not: every subscription, session or waiting
  room, local or relayed, of a connection that may submit a passkey
  (`terminal`, and `unauthenticated` until enforcement) carries the new event
  `passkey_prompts_changed { prompts: [PasskeyPrompt] }` with the prompts
  pending for that connection's user. It is sent when the subscription starts
  (possibly empty) and whenever the set changes, and is never replayed. A
  `PasskeyPrompt` holds only what the kernel registered: `kind`
  (`critical_approval`), `session_id`, optional `session_alias`,
  `interaction_id`, `title`, `message`, `approve_choice_id`,
  `refuse_choice_id`, `requested_at_ms` and `expires_at_ms`. In a shared
  session only the decision's owner gets it. It is answered with
  `RespondToInteraction` on its session and interaction: the approve choice
  with `passkey` (and optionally `passkey_remember_minutes`), or the refuse
  choice without one, from any of the owner's terminals. The first verified
  passkey or refusal resolves it and the prompt leaves every terminal's set;
  a later answer is refused with `PASSKEY_ALREADY_ANSWERED` without
  verifying, auditing or counting its passkey. Passkey answers are verified
  one at a time and each holds its turn until its answer is applied, so of
  two simultaneous right passkeys only the first is checked. A wrong passkey
  answers nothing: the prompt stays open, and the failure is audited and
  counts toward the lockout as in protocol 392. A kernel decision that
  requires the passkey must have exactly one approve choice (marked
  `requires_passkey`) and one refuse choice. At its deadline the prompt
  leaves the set, a passkey sent for it is no longer checked, and the
  decision times out to whoever raised it. Clients no longer ask for the
  passkey inside their approval panels; it is typed only into the popup.
- protocol 404: process-bound external agent access over the existing
  `local_socket_path`. The kernel serves the same websocket envelopes on a
  Unix socket with mode 0600 in an owned 0700 directory. Its default address is
  `/tmp/chariox-<effective uid>/<sha256>.sock`: the full SHA-256 covers the
  domain `chariox-unix-socket-v1\0`, lexically normalized absolute config home,
  a NUL separator, and the retained endpoint kernel ID from the registry (even
  when the runtime announces a canonical slice reference). This keeps maximal
  slice identities and deep
  homes below the OS address limit and separates kernels in different homes.
  CLI discovery uses the same derivation; TCP refusals name the actual socket.
  `CHARIOX_DAEMON_SOCKET` remains an explicit override; startup rejects addresses
  exceeding the platform byte limit before touching the filesystem. It rejects other
  UIDs, `Origin`, and bearer authorization headers. macOS identifies the
  peer with `getpeereid` and `LOCAL_PEERTOKEN`, including the audit token's
  process version. Linux uses `SO_PEERCRED` and the process start time.
  Every use checks the process identity again to prevent PID reuse.

  An unapproved Unix peer can only send `RequestKernelAccess` with
  `holder_pid` and optional `lifetime_minutes` (protocol 451 removes `session_id`; old session-bearing requests are rejected). The holder
  must be the peer or an OS-verified ancestor. The kernel raises an owner-only
  passkey popup naming its verified executable, pid, local-kernel authority, and lifetime. No session must exist. Access popups use the kernel-wide interaction routing id `kernel-access`; this is not a session or grant scope.
  `RespondToInteraction` on this routing id returns `KernelAccessDecisionResponded { interaction_id }`, with no session projection.
  Grant and extension prompts have kind `access_grant` or `access_extension`,
  `lifetime_minutes`, and `max_lifetime_minutes`. Protocol 470 adds an optional
  `requester` object to both `RuntimeInteraction` and `PasskeyPrompt` for grant
  and extension decisions, established from the same OS-verified holder:
  `executable` (full path), `pid`, `process_start_id` (opaque decimal string,
  Linux start ticks or macOS unique process ID), `process_exec_version` (macOS
  exec version, zero on Linux), and optional `provider_harness` (`codex`,
  `claude`, or `opencode`). Harness recognition matches the configured native
  executable (including Codex's official npm native package); unknown paths
  omit it. This field is attribution, not vendor/signature attestation or new
  authority. Text remains display-only, with quoted/escaped executable paths;
  clients must never parse requester identity from it. TUI labels use the
  structured object and show identity unavailable for old kernels. New access
  requests and kernel-wide approval replies require protocol 470; refusals
  remain supported on protocol 451 and legacy session-scoped replies retain
  their existing route. Other client/relay/native minimums are unchanged.
  MP-08 / MP-10 / MP-11 focused drill:
  `python3 apps/cli/scripts/live-kernel-access-requester-drill.py --kernel <built-kernel> --cli <compiled-cli> --codex-profile <approved-product-linked-profile> --source <commit> --output <external-evidence-dir>`.
  This real outside-Codex drill refuses the grant through TUI keyboard input,
  never approving access or changing a shared provider login. `--local-cli`
  is supplementary regression evidence only, not provider acceptance.
  Approve needs a fresh
  terminal passkey; the critical-approval remember window never applies.
  The owner may choose a lifetime through the approve answer's numeric
  `custom_reply`. Refuse needs no passkey.

  `KernelAccessGranted` returns public `KernelAccessGrant` metadata, never
  a credential. Protocol 451 removes `session_id` from this metadata. A grant
  authorizes the live holder and its OS descendants across the whole LOCAL
  kernel: every ordinary session/global request a terminal can make, including
  session creation/attachment, agent prompts/spawns, workflows, App installs and
  bindings, routine approvals and Vault use through kernel-owned flows.
  Kernel-launched agents receive no external authority even if the holder is
  their ancestor. Normal ownership and membership checks still apply.
  The holder cannot answer critical/passkey-required approvals or payments,
  mint/extend grants, change the passkey/access configuration, read/export
  secrets, attach to a REMOTE kernel or issue kernel-peer requests. Internal
  leased-worker execution remains the local kernel’s responsibility.
  A holder may request `/sudo` for a local agent in any local session; each
  request requires a fresh terminal popup confirmation. No remember window
  applies and the holder never submits the passkey or becomes a sudoagent.

  `ListKernelAccessGrants` and `RevokeKernelAccessGrant { grant_id }` are
  available to terminals and local grant holders and scoped to the caller's owned grants; a null grant id
  revokes all of them. Expiry, explicit revoke, holder exit,
  passkey rotation, and kernel shutdown revoke authority. Idle subscriptions,
  queued commands, cached replies, and event replay check live authority.
  Workflow controls also recheck after provider-lane and cancellation-settlement
  waits; remote workflow cancellation carries the same command authority.
  Direct router paths retain the canonical grant/request too. Setup cancellation
  rechecks after ordering gates, Meta cancellation retains authority, and local
  PTY input and remote terminal sends recheck before enqueue.
  Capability closures recheck before blocking shell/file/artifact effects. Room
  controller commands recheck after relay discovery/enqueue and local blocking
  waits; browser mutations recheck execution-gate and action-admission waits
  before execution and home-state completion. Invalidated queued actions are
  retired without execution. Stale remote binding recovery retains authority
  through the app lock and worker discovery, before lease/account/agent creation
  or home binding persistence. Local controller jobs recheck under the supervisor
  ownership lock; computer helpers recheck inside their blocking process queue.
  A Unix connection binds to its first approved or admitted grant and never
  switches authority. Fresh connections prefer the holder’s own eligible
  process grant over an ancestor’s. There is no session-based selection or
  response filtering. Session end does not revoke a local-kernel grant.
  Grants stay in memory and do not survive a restart. Durable grant events
  record metadata and outcomes; terminal-answer and passkey verification
  events correlate by interaction id. They contain no passkey or bearer.

  TCP and relay access requests return a pointer to the Unix socket; neither
  transport can use a grant. Existing TCP token and tokenless log-mode
  behavior remains until enforcement. `LocalIpcClient` supports
  `ws+unix:///absolute/socket`. `chariox access request
  [--holder-pid <pid>] [--minutes <minutes>] [--socket <path>]` waits for
  the popup and prints public grant metadata. MP-08 / MP-10 / MP-11: the default
  holder is the nearest installed official provider in the CLI launcher's
  OS-verified ancestry, including native Codex behind its npm launcher. Unknown
  programs retain the grandparent fallback and the External program label.
  Selection preserves exact PID/start/exec identity and grant admission fences.
  Terminal controls are `chariox access list`,
  `chariox access revoke <id|--all>`, `/kernel access list`, and
  `/kernel access revoke <id|all>`.

  Kernel user config settings are live runtime policy. Set/unset is supported;
  unset restores the default. An extension popup is raised at the notice
  time and a verified answer starts a new term.

  ```toml
  [kernel_access]
  grant_default_minutes = 480
  grant_max_minutes = 1440
  grant_extend_notice_minutes = 5
  request_timeout_minutes = 10
  ```
- protocol 412: local access enforcement.

  TCP websocket admission now requires the generated local token. Missing, wrong,
  malformed and stale credentials receive HTTP 401 before command dispatch. Its
  body names `<state>/kernel-local-auth/<port>.token` and the configured
  `ws+unix:///absolute/socket` endpoint. `LocalIpcClient` reports that diagnostic
  as a non-retryable `authentication_failed` error. It continues reading the
  private token file for each reconnect; managed host-token kernels retain their
  existing admission behavior and owner-decision reply routing. Hosts remain
  unable to submit passkeys or answer credential prompts. Remember-window
  approvals are restricted to the terminal class; host controllers cannot use them.
  If the generated token file cannot be written or the listener address is
  unavailable, startup fails before publishing local presence. First-party
  clients send the local token only to loopback endpoints; direct LAN access,
  including a non-loopback `CHARIOX_KERNEL_HOST`, is unsupported. Physical
  devices use the Cloud/relay path.

  Terminal authority follows the admitted `terminal` connection class, rather
  than the command transport source. Only that class may submit a passkey or
  receive owner passkey popups. Unauthenticated Unix peers can only request
  access; approved external peers keep the process-bound, whole-local-kernel grant
  path from protocol 404 and cannot answer critical approvals. Relay identities,
  per-run runtime MCP admission and the publication gateway keep their existing
  credential paths. No first-party minimum version rises: token-aware clients
  also work with older log-mode kernels, and this change adds no request or event
  shape that a client requires.

  Focused validation: `runtime_transport::tests::laptop_kernel_websocket_enforces_local_tokens`,
  `runtime::command::tests::terminal_status_requires_the_admitted_terminal_class`,
  `runtime_transport::tests::kernel_access_grants`, and
  `apps/cli/scripts/lib/private-kernel-local-auth.kernel-test.mjs` (set
  `CHARIOX_LOCAL_AUTH_KERNEL_BINARY` to the candidate binary). The private-kernel
  drill checks both state-root conventions, control/event lanes and token rotation
  without provisioning any provider account. Grant/passkey drills use temporary
  test vaults. The owner's real passkey sitting remains separate.
- protocol 413: terminal `/sudo <prompt>` raises a fresh-passkey popup of kind
  `sudo`. The resulting authorization is kernel-memory state, attached to one
  exact provider turn; yield and interruption consume its ephemeral binding.
  Queued sudo never enters the durable prompt queue. `KernelAccessGrantsListed`
  gains `sudo_turns`, public entry/terminal/agent/run/prompt attribution without a
  credential or time expiry. `RevokeKernelAccessGrant` also revokes sudo entry
  handles; a null id and passkey rotation revoke queued entries and interrupt
  running sudo turns. The existing runtime MCP exposes `chariox_kernel_request`
  to a live sudo turn. Critical replies use the shared interaction authority and
  append `kernel_access.sudo_approval` receipts naming the turn and human entry.
  No spawned/forked agent inherits elevation.
- protocol 415: a live external grant holder may send
  `RequestKernelSudo { agent_id, prompt }` over the Unix socket. The kernel
  resolves the exact local target agent and its session and raises the same `sudo`
  popup, naming the OS-established executable/PID, target/session and full
  requester-supplied prompt. Only host terminals answer it. The outcome is
  `KernelSudoRequested { agent_id }`; no passkey is accepted from the requester.
  `KernelSudoTurn.requester` optionally carries public grant metadata; its
  `terminal_id` names the winning host terminal once authorized. Pending external
  entries require a live grant through final dispatch and expire if unanswered.
  Rotation, revoke all and session end revoke these entries as well. `/meta`
  retains delegation-only behavior for one release and emits a notice pointing
  to `/sudo`. See `KERNEL_SUDO.md` and `scripts/kernel-access-sudo-drill.sh`.
- serving either a live source trigger or a deployed package MUST validate
  provider/model bindings, extension requirements, and credential requirements
  before it accepts traffic
- if a packaged provider/model is unavailable, a deployment binding may
  substitute another available provider/model without mutating the package
- Cloud publication deployment is a control-plane record plus a runtime backend.
  It is not a new workflow authority and does not replace the kernel-owned
  publication runtime session.
- v1 Cloud deployment supports two backend modes:
  - `local_runtime`: a public publication ingress routes to a user's local
    `chariox serve` process over an outbound connector
  - `hosted_container`: a public publication ingress routes to one Docker
    container per deployment on the publication runner
- OpenShip-managed Chariox Cloud APIs own deployment records and control commands
  only. Runtime publication traffic should terminate at the dedicated
  publication ingress and route from there to the active backend.

Workflow run history queries:

- `ListWorkflowRuns` is a bounded keyset-paginated query. Clients MAY provide
  `limit` and the opaque `cursor` returned by an earlier page.
- `WorkflowRunsListed` returns the selected `workflow_runs` plus an optional
  `next_cursor`. The absence of `next_cursor` means the history is exhausted.
- the kernel merges bounded hot, durable-history, and in-progress legacy
  migration pages. It MUST NOT replay or scan the complete lifetime run history
  to serve one request.
- legacy terminal runs remain readable until their normalized durable migration
  transaction commits; a failed or interrupted migration MUST NOT hide them.

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
auth secrets, Chariox Cloud user session credentials, workflow prompt payloads,
artifacts, outputs, or traces.

Public deployment URL contract:

- HTTP triggers are rooted at `public_base_url`
- `GET /` opens the human/browser-compatible viewer or form
- `GET /<prompt>` invokes the workflow with an address-bar prompt path
- `POST /invoke` invokes the workflow from a form or API request
- `GET /.well-known/chariox/publication/status` returns publication status

Workflow trigger V1 exposes HTTP GET/POST only. SSE remains an internal HTTP
progress mechanism and is not a selectable trigger type. Agent tool servers and
kernel/relay transport channels are independent of the workflow trigger model.

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
- HTTP triggers share one viewer HTML app. The viewer renders output/status on
  the left and exposed traces on the right.
- HTTP can invoke from an address-bar GET path or from the shared viewer form;
  result pages receive publication progress through the internal SSE stream.

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

- remote Chariox terminals invoke an HTTP trigger through its configured ingress,
  not by bypassing that trigger and directly calling the workflow endpoint
- when a local-only HTTP trigger is invoked remotely, the kernel/relay may tunnel
  the HTTP request and response between the remote terminal and the local server
- the relay remains transport-only and must not inspect workflow prompts,
  artifacts, outputs, or published transport payloads
- Cloud publication ingress forwards HTTP and its internal SSE progress stream
  to the active backend target and must preserve streaming semantics.
- Chariox Cloud should not proxy runtime publication streams. It may create,
  list, start, stop, and observe deployment metadata, and the web terminal may
  embed `public_base_url` in the central panel.
- If the active backend is unavailable, HTTP returns an unavailable page or a
  structured invocation error as appropriate to the request.
- Hosted containers receive scoped deployment/runtime identity only. They must
  not receive a general Chariox Cloud user account token.
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

MP-08: Local daemon protocol v371 adds the kernel-owned, value-free Project environment manifest query and environment input contract. Candidate B v370 shutdown observation remains a v370 client dependency. Project values resolve only inside the exporting kernel and never enter manifest projections.


MP-08 / MP-10 / MP-11: The Project environment feature stays at local protocol 371 and moves relay peer 64 to 65 for authenticated leased Project environment installation. Interactive slice/M28 start requests opt into the shared kernel-owned Ready-to-move RuntimeInteraction; API requests default to unattended decisions. Review projections contain names, use sites, sources and decisions only. Missing values use the existing secret reply path to Vault or remain named as skipped inputs. Exported Project values are sealed to the authenticated target/context; the target resolves launch bindings locally. Provider-neutral user rules travel in the kernel context, while provider home transfer carries credentials only.

MP-08 / MP-10 / MP-11: Local protocol v373 and relay peer v66 add lease-bound worker Environment query/Adjust, explicit selected private-file retrieval, and source-worker export/reuse. GetProjectEnvironmentManifest.agentId selects the execution kernel. Pending interactions are projected in waiting-room activity, including unattached utility sessions. CreateSlice.source_slice_ref selects a running home-owned Docker slice of the same Project/repository selection; its actual worker refreshes the manifest and seals values directly to the next worker, while the existing development exporter captures its owned mounted repository snapshot. The home routes the opaque sealed layer and does not decrypt worker values. The target binds subsequent leases to its existing Project state without source contact. Reaching the source is required only for explicit retrieval of a previously omitted private file. Imported manifests and source receipts are independent private target state; values remain absent from public projections. Metadata-only Codex utilities use ephemeral native threads and settle through native item/completion events, without durable turn-list reads.

MP-08 / MP-10 / MP-11: Ordinary local prompt admission and leased-worker reuse resolve the exporting kernel's current Project bindings before reusing an idle provider process. Changed bindings retire the idle process before native conversation resume, releasing Codex's thread writer; active turns are never replaced for environment changes. Native TUI refresh reports that the TUI must restart when its selected inputs change. Account activation merges native credentials without dropping Project bindings. Secret-file length changes do not trigger metadata discovery; incremental discovery preserves kernel-owned unchanged selections, while unsupported names/locators remain rejected. Automatic queued-turn promotion and native-TUI refresh require further validation before acceptance.

### MP-08 / MP-10 primitive MCP results (protocol 375)

Runtime script results may be any JSON value. The shared MCP response producer
wraps non-object values as `{ "result": <value> }` before serializing both
`structuredContent` and its text representation. Object results retain their
existing fields; image extraction is unchanged. This follows the
[MCP structured content contract](https://modelcontextprotocol.io/specification/2025-06-18/server/tools#structured-content).
The correction is shared by ordinary and worker provider runs. Local protocol
375 records the serialized result correction; relay peer protocol remains 67.
Web/native minimum versions remain unchanged because they do not depend on this
MCP-only behavior. Provider-free transport/script tests are focused source
proof, not MP-08 or MP-10 acceptance on a signed fresh Path-1 release.

### MP-08 / MP-10 terminal workflow event authority (protocol 376)

Terminal `workflow_run_updated` events come from the kernel's archival update
stream exactly once per recipient. Hot-session snapshot diffs publish only
nonterminal workflow runs; they cannot duplicate the archival terminal event.
The focused WebSocket terminal-transition drill verifies Running, Completed,
durable lookup after archival, and absence of duplicate terminal updates.
Serialized fields and the relay peer contract are unchanged by this correction. Existing web/native
minimum supported versions stay unchanged because this restores the existing
terminal-event contract without adding a required client field or operation.

### Protocol 405: native approval origin

Native approval producers capture `NativeInteractionOrigin` before an asynchronous handoff.
`RequestNativeProviderTurnInteraction.origin` is required; runtime interactions forwarded over
relay carry the same `native_origin`. The origin identifies a kernel prompt or native turn
and its provider run. Workspace trust uses the explicit `provider_startup` scope.
`RemoteNativeInteractionContext.home_prompt_id` freezes the matching home prompt while the
worker prompt still matches. The home kernel never substitutes its current prompt.
Stale requests resolve as `timed_out` with no choice or reply, without applying timeout
defaults or publishing an answerable approval. Pending approvals are withdrawn on turn,
provider, agent, or session termination; all terminal projections receive the removal and
withdrawal notice. The Claude native approval client requires protocol 405; display-only
clients require no new minimum.

Managed Codex/OpenCode approvals retain the actor's original kernel prompt identity.
Codex approval `turnId` must match its active provider turn; OpenCode permission
`tool.messageID` must belong to its active user message. Native turns use the recorded
provider turn identity. Events without a usable turn/message identity are denied rather
than borrowing the currently running turn (including legacy Codex approvals without a turn ID).

`RequestNativeProviderTurnInteraction` and relay `forward_native_turn_interaction`
replace the previous unbound request variants. Older kernels reject these unknown variants
instead of silently ignoring origin fields; the native client reports a protocol-405 minimum.
### Protocol 409: App clipboard copy-out and link opening

`host.clipboard_write {text}` and `host.open_link {url}` on the App worker SDK
channel create a pending, owner/installation/generation-bound offer:
`{operationId, state:"pending", expiresAtMs}`. Room App views expose
`window.chariox.host.writeClipboard(text)` and `openLink(url)` through the same
host methods, after the kernel validates the view binding and generation.
Neither backend nor view code performs the host action itself. There is no
clipboard-read method. Clipboard offers require the signed `capabilities.clipboard: ["write"]` declaration;
link offers need no separate capability. Neither requires external-file access.

Clipboard text and each of its JSON/visible escaped representations are limited
to 256 KiB, so the complete trusted prompt fits one terminal projection. URLs are limited to 8 KiB, must
be absolute HTTP(S) URLs with a host, and cannot contain whitespace, control
characters, backslashes, invisible Unicode formatting or nonempty userinfo
(username/password). The exact submitted URL is shown and returned without
normalization; the prompt also shows its parsed ASCII destination host (punycode
for IDNs). Each installation may have
four unanswered host offers; they expire after five minutes, or when its
active generation changes or it is uninstalled. Settled payloads are dropped.

The file-export prompt pump projects one `RuntimeInteraction` to the owner's
terminals (local TUI, remote TUI and web), with id `app_host_<operationId>` and
kernel subject `host_action:<operationId>`. It shows the exact URL or an escaped,
complete representation of the offered text, plus a Decline choice and no
approval on timeout. App views and agents cannot settle this prompt.

An explicit human gesture in a trusted terminal sends
`AcceptAppHostAction {session_id, operation_id}`. The kernel validates a human
client, owner, showing session, pending interaction, generation and deadline,
and arbitrates this take against decline/expiry under the existing interaction
lock. It returns `AppHostActionAccepted {operation_id, action}` once:
`action` is either `{kind:"clipboard_write", text}` or `{kind:"open_link", url}`.
A generic `RespondToInteraction` can decline but cannot take the offer; clients
must use this dedicated acceptance path. Stale, declined, expired, foreign or
already accepted offers fail without returning a payload. No owner, URL or
text can be supplied by the accepting client. Acceptance consumes the offer;
a failed host action or lost reply requires a new App request.

The accepting terminal performs the action on its own machine. TUI users type
`/app host accept` after closing the approval panel selects the sole pending
host offer in the attached session only when it matches the last offer displayed
in that terminal's approval panel. Unviewed/replaced offers require re-opening
the panel or an explicit ID. Multiple offers require
`/app host accept OPERATION`. Copying uses the renderer-backed OSC 52/native
clipboard helper and always shows the escaped text as a visible fallback
because OSC 52 has no acknowledgement; link opening
uses the existing default-browser shim and always prints the exact URL. Web
clients must copy with `navigator.clipboard.writeText` from a trusted click,
open a new tab with `noopener`, and show a visible fallback on failure. They
must preserve browser user activation across kernel settlement (for example,
show a fresh Copy/Open button after successful settlement). App iframe/Room gestures only create offers and never
count as the human's acceptance. Clients exposing acceptance require protocol
409; unrelated clients keep their existing minimum version.


### Protocol 416: expired App receipt refusal

Protocol 416 adds `AppRequestFailed {code: "receipt_expired"}` for an
  evicted owner-scoped App control command identity. Controls keep durable
  at-most-once response receipts for the newest 512 identities. On admission
  pressure the least-recently-used eligible identity can be evicted only when
  all its receipts are older than the existing 24-hour command retention
  window; pending effects and unknown-age receipts are protected. Replays
  update access order without extending that window. A synced SHA-256 identity
  marker is committed before removing a response, and survives restart and
  compaction: an evicted replay is refused and never re-executed, including
  stop and uninstall. Marker storage is bounded to 50 MiB; reaching the bound
  refuses new receipt-bearing work rather than deleting replay fences; fresh
  Stop/Uninstall controls retain their capacity exception. In-memory markers
  use fixed-size 32-byte digests under this separate storage bound. CLI, TUI and shared web
  shell display an explicit receipt-expired message and do not retry it.
  Once an identity is evicted, a protocol416 fence in the response journal is
  preserved through compaction. Legacy kernels fail closed on that journal
  rather than redispatch an expired identity after rollback; their App control
  requests report storage unavailable until a supporting kernel is restored.

### Protocol 418: user-domain App views

`OpenUserAppView`, `ListUserAppViews`, `CloseUserAppView`,
`GetUserAppViewFrontend`, `CallUserAppView` and `SubscribeUserAppViews` address
ephemeral owner-scoped App view instances with no session, Room or slice.
Bundles contain only verified signed frontend assets; page calls retain the
existing durable App tool/host-action path and omit the optional SDK
`room_id`. The owner snapshot subscription includes detached kernel
`RuntimeInteraction` decisions. `AnswerUserDomainInteraction` uses the
existing terminal/passkey gate and shared pending-interaction authority;
a detached passkey popup has an empty session routing field. Room App views
retain their existing protocol. See [App views without a session](MULTIDOMAIN_APP_VIEWS.md)
for host isolation, lifecycle and migration. No relay-peer shape changes.

### MP-08 / MP-10 / MP-11: Room Computer access (local 461, peer 92)

Room membership grants Computer control by default. The owner can revoke a specific agent's Computer control through the existing Access command (`revoke_grants`) and restore it with `grant_room_computer`. Since local 461, `grant_room_computer` requires `agent_id`: an explicit `agent_id: null` restores every owned agent after `revoke_grants` with `agent_id: null`, and an omitted `agent_id` is rejected; the TUI exposes this as `/access grant all` and prints a visible notice for both bulk revoke and bulk restore. Older kernels reject the null shape, so clients gate only the bulk restore on 461. Drill: `node apps/cli/scripts/live-room-computer-access-drill.mjs --evidence <dir>` (real kernel and TUI, dev-stub agents). Membership and Browser access remain intact. The kernel checks the restriction at input admission, queued dispatch and active cancellation; workers retain the existing authenticated Room peer binding. Access snapshots project each owned agent's Room Computer permission. Room permission changes advance the existing owner Access cursor; the kernel captures permissions and cursor together so delayed projections cannot restore stale access.

Slice Computer accessibility uses the existing controller and native AT-SPI backend through bounded `chariox.slice_accessibility` snapshots and `chariox.slice_target_action` actions. Handles belong to an observer and tree revision; stale or foreign handles fail closed. The controller joins the desktop session's private D-Bus address, scopes visible processes to the desktop user, applies the current observation policy and routes target actions through the normal Room Action ledger and cancellation path. The relay remains encrypted transport only.

### MP-08 / MP-10 / MP-11: protected desktop display (local 474, peer 96)

A human terminal subscribes with `KernelBrowser` Computer `display_subscribe`,
its current native `surface_id` and generation, video codecs, bounded bitrate,
and DPR 1 or 2. The kernel returns the native source identity and physical
geometry alongside the existing numeric controller generation and viewer lease.
Only this new desktop viewer requires protocol 474. Agent MCP retains bounded,
on-demand protected observations and cannot subscribe to continuous video.

The owned user-domain desktop uses the shared XDamage/XShm source, video encoder,
credit window and encrypted CXD1 relay transport. It has no separate streaming
service. Kernels built with `native-display` read the owned desktop root with the
kernel's own native worker (`--display-native-worker`, desktop mode: XShm, x264,
private raster slots and packet files, as for the browser display); others use
the Python XShm helper. Before any encode, export or client sees a readback it is
masked against native accessibility snapshots: the helper takes one before and
one after its read; the native path binds a readback between the last snapshot
that ended before it began and one requested after it arrived, all agreeing,
with at most 200 ms unobserved (otherwise it is masked whole or read again).
Unknown ownership, uncovered or unproved windows, incomplete trees, changed
trees and registered private state fail closed. Protection changes retire
capture and encoder state before acknowledgement. This does not assert that the
live protection matrix has passed.

A human actor's Desktop ownership (takeover or human Computer input) lasts while
one of its desktop video leases is live. Relay clients have no gone event: a
lease lapses 60 s after the last display request on it, or ends on unsubscribe,
and the kernel then retires that actor's Desktop ownership before admitting
other input. Viewers release ownership before closing; a reopen awaits that
acknowledged release.

Multiple admitted viewers share one protected capture source. Each lease remains
bound to its authenticated terminal, native generation and controller generation;
disconnect, expiry and last-viewer close release its resources. Per-credit
cancellation does not own the encoder lifetime. The existing display attach,
next and unsubscribe operations address these leases. This section adds no
relay frame of its own; its events use the display encoding below. Peer 97
remains unused.

Display event encoding (local 466, peer 96). Only `kernel_browser_frame`
events change; every other event stays JSON. On the local WebSocket they are
binary messages; over the relay they are the encrypted plaintext. The format is
`CXD1`, a big-endian u32 header length, the JSON event header naming its
segments, then the raw segment bytes in order. JSON plaintexts never start with
`CXD1`. These events are sent only on a display delivery: a local
`display_subscribe` or a relay subscribe with scope `kernel_browser_display`. Clients
that never register display delivery never receive `CXD1`. Peer 96 adds an
optional `CXR1` WebSocket binary relay envelope, negotiated with the
`chariox-relay-binary-v96` subprotocol: `CXR1`, a big-endian u32 header length,
a JSON routing header of at most 4 KiB, then 16 B to 1 MiB of ciphertext. The
relay reads only the routing header. Kernels fall back to JSON/base64 for older
relays or receivers and for events over 1 MiB. Rollout: deploy a web bundle
that decodes `CXD1` before kernels at 466 or later serve it Browser display;
an older bundle cannot parse these frames.

The shared presenter accepts desktop pixels only at the exact admitted physical
geometry, with CSS dimensions divided by the selected DPR. It converts viewer
coordinates to native coordinates and routes input, takeover and release through
the existing kernel Computer admission and actor ledger. Browser viewports retain
their existing path. A Cloud client must expose the native source and use this
shared presenter before the desktop feature is user-visible.
