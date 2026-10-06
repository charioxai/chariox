# One agent model: room tools, /sudo and durable wakes

## MP-08 / MP-09 / MP-10 / MP-11 — authority and source

Design study, 2026-10-06; no implementation or runtime acceptance is claimed.
The ten owner decisions in the agentmodel brief govern this plan. `/sudo`
elevates an ordinary agent at prompting; there is no `/sudo task`, mandatory
task plan, special supervisor agent or new goal command. Every agent can
delegate and receive results. Yielding ends a turn, not necessarily the work.
Approvals belong to the user, including when the requesting agent is elevated.
The coordinator's binding 11:26/11:27 UTC additions specify no-reply messaging
and urgency; the defaults and drills below incorporate both.

Source read for this study:

| Input | Exact identity and use |
| --- | --- |
| OSS public `main` | `e325afa580d81954e2c179757fc53fa02ed2a2b3`; supplied lane worktree already matches fetched main. All unqualified paths below refer to this commit. |
| PR #900, `md/round2-on-main` | `6dde21a8c10c9ef2b9f7f0a271cace00591f0bd1`; user-domain access, kernel browser, Vault protection and display/mirror seams. These are dependencies, not main behavior. |
| PR #885, `wf/outbound-notifications` | `4998b1120f16a201eae06b1891c21770e9730b2e`; durable delivery and uncertain provider sends. Read this successor, rather than the older local branch of the same name. |
| Required plans | `AGENTS.md`, `docs/BROWSER_COMPUTER_USE_END_TO_END_PLAN.md`, `docs/MANAGED_PATH1_PARITY_INVENTORY.md`, `docs/M20_DOCKER_SLICE_BROWSER_STATE_VALIDATION_PLAN.md`. |
| Superseded sudo design | Remote `docs/sudo-task-mode` and main's `docs/SUDO_TASK_MODE_PLAN.md` are absent. `docs/KERNEL_SUDO.md` and `runtime/state/sudo/*` show the current one-turn implementation; this plan supersedes its conflicting behavior. |

Kernel paths below abbreviate `apps/kernel/src/`. MP-08 requires one runtime
path across ordinary and managed placement. MP-09 covers wake/watch activity
and mandatory shutdown. MP-10 requires real-path comparison evidence. MP-11
means the owner-narrowed behavioral matrix and current security-anchor reviews
(2026-10-05), not exact-blob review of every non-security source file. This
document closes no MP acceptance item. The newer multidomain decision replaces
the older plan's Selkies/noVNC direction; invest in shared controller, protected
display and mirror seams. M20 remains the separate slice persistence contract:
saving a slice does not save the user-domain browser outside it.

## MP-08 / MP-11 — inventory and destination

Today's tool names live in `transport/runtime_tools.rs`, their provider aliases
in `transport/runtime_tools/meta_tool_names.rs`, and schemas in
`transport/runtime_tools/meta_tool_specs.rs`. Admission in
`runtime/state/tool_dispatch/meta.rs` requires a metaagent. Command dispatch in
`runtime/router/meta_runtime_command/dispatch.rs` additionally requires a task
plan for selected mutations; target lookup uses `controlled_by_metaagent_id`.
Those gates must become room authorization, not simply disappear.

The following inventory covers every `chariox.meta.*` name. Names in the first
column are suffixes of `chariox.meta.`; all proposed names are design names.
Legacy provider aliases follow the same mapping and cannot bypass admission.

| Current suffixes | Destination |
| --- | --- |
| `session_overview` | All agents: `chariox.room.overview`, room roster/workflows/activity and safe pending-interaction metadata. Remove task/completion recommendations. |
| `search_commands`, `list_commands`, `command_docs` | All agents: `chariox.commands.*`; report actual scope, required authority and routed status. |
| `search_guides`, `list_guides`, `read_guide` | All agents: `chariox.guides.*`; retain workspace-aware guides. |
| `run_command` | All agents: `chariox.room.run_command`, using the existing typed request router. Out-of-room operations require live `/sudo` and an explicit target; no arbitrary IPC passthrough. |
| `list_events`, `read_event`, `ack_event` | All agents: `chariox.events.list/read/ack`, private recipient inbox; read/ack are bookkeeping, not permission decisions or evidence of provider delivery. |
| `subscribe_events`, `unsubscribe_events`, `list_subscriptions` | All agents: `chariox.events.subscribe/unsubscribe/subscriptions`, durable room subscriptions. Delegation completion feedback is automatic. |
| `turn_overview`, `turn_blob` | All agents: `chariox.history.turn/read`, any room agent's public prompts, answers and tool records; always redact before return. |
| `subscribe_trace`, `poll_trace`, `wait_trace`, `unsubscribe_trace` | All agents: `chariox.trace.*`, optional bounded live trace of room agents. `wait_trace` remains a short wait; durable yield replaces indefinite polling. |
| `read_task`, `update_task`, `read_plan`, `update_plan`, `complete_task`, `mark_blocked` | Dropped. Ordinary prompts/history plus durable waiting state express progress; no task authority is recreated under another name. |
| `resolve_runtime_interaction` | Dropped for every agent, including `/sudo`. User terminal responses remain; agents can request/read safe interaction status. |
| `workflow_code.create`, `workflow_code.read`, `workflow_code.list`, `workflow_code.update`, `workflow_code.delete`, `workflow_code.validate`, `workflow_code.apply`, `workflow_code.run` | All agents: `chariox.workflow_code.*`, room-scoped definitions/runs through existing workflow services. |
| `workflow_code.export`, `workflow_code.import`, `workflow_code.package_export`, `workflow_code.package_import`, `workflow_code.source_export`, `workflow_code.source_export_directory`, `workflow_code.source_export_dir`, `workflow_code.canvas_contract` | All agents, same room scope and existing artifact/path validation; `source_export_dir` remains an alias of `source_export_directory` until alias retirement. No private overlays in exported model content. |
| `workflow_registry.list`, `workflow_registry.get`, `workflow_registry.add`, `workflow_registry.add_from_workflow`, `workflow_registry.delete`, `workflow_registry.load`, `workflow_registry.run` | All agents, room bindings and authorized library reads. Mutating an owner-wide registry outside the room requires `/sudo`; publication keeps its existing trust/admission checks. |

The command catalog is `runtime/metaagent_command_registry/catalog/*`.
Canonical entries are mapped below; aliases inherit their canonical policy.
`Allow` in today's docs is not proof of implementation: `routed=false` must
remain visible and cannot be promoted by changing the advertised tool list.

| Catalog file and entries | Destination and qualification |
| --- | --- |
| `agent.rs`: `session overview`, `agent list`, `agent spawn`, `prompt` | All agents. Spawn any supported agent in the room on authorized placement/accounts. Make `prompt` delegation through the same message/prompt admission, with automatic result correlation; prevent workflow-claim theft. |
| `agent.rs`: `agent alias`, `agent delete` | All agents for objects they control; preserve destructive peer ownership checks pending the owner question below. No supervisor caste. |
| `agent.rs`: `agent focus` | Drop from model-callable authority. User focus stays a terminal action; an agent cannot focus itself to mint browser/App privileges. Request a capability grant instead. |
| `workflow.rs`: `workflow list`, `workflow new`, `workflow resolve`, `workflow alias`, `workflow node add`, `workflow node remove`, `workflow node instructions`, `workflow node can-complete`, `workflow node intermediate-output`, `workflow node wait-for-all-inputs`, `workflow node max-turns`, `workflow endpoint new`, `workflow endpoint alias`, `workflow edge add`, `workflow edge remove`, `workflow run`, `workflow runs`, `workflow get-run`, `workflow cancel`, `workflow resume` | All agents, room-scoped services. Existing definition-control checks protect peer mutation; event delivery must not create an independent workflow scheduler. `max-turns` is an explicit workflow option, not a quota imposed on regular agents. |
| `workflow.rs`: `workflow node intermediate-output-schema`, `workflow endpoint bind`, `workflow endpoint remove`, `workflow run-output-schema`, `workflow max-turns`, `workflow node extensions` | All-agent destination, but currently unrouted; advertise unavailable until the underlying shared operation exists. Do not make these prerequisites for this overhaul. |
| `workflow.rs`: `workflow tutorial basic`, `agent app guide` | All agents, guides only. |
| `workflow.rs`: `workflow pane` | Dropped from agent execution; client layout stays with the user. |
| `capability.rs`: `extension grant app`, `extension revoke app`; `mcp list`, `mcp show`, `mcp install-json`, `mcp update-json`, `mcp grant`, `mcp uninstall`, `mcp revoke`, `mcp import`; `skill list`, `skill show`, `skill install`, `skill update`, `skill grant`, `skill uninstall`, `skill revoke`, `skill import`; `extension import providers` | All agents for room capability bindings on the user's request. Self-grants are allowed within that request's recorded resource scope. Global installation/update/removal requires `/sudo` when it affects other rooms; imports expose safe metadata, never credential-bearing provider config. Apps still require trusted installation and capability admission. |
| `system.rs`: `slice` | All agents for room-bound lifecycle on the existing shared slice service; owner-wide slice administration requires `/sudo`. Preserve busy-save rejection and user-owned saved-state policy. |
| `system.rs`: `credential list`, `credential get` | All agents, metadata for granted room credential handles only. Owner-wide metadata enumeration requires `/sudo`; neither operation returns values. |
| `system.rs`: `credential upsert-json`, `credential remove`, `credential vault` | `/sudo` for owner Vault metadata/write management, through typed safe operations. Regular agents may request unlock/status for an already granted handle; only the user supplies unlock input. Raw JSON values are never a model input. |
| `system.rs`: `credential secret mutation` | Drop the raw-value command; replace with `/sudo` generation and protected user-entry actions below. |
| `system.rs`: `session new` | `/sudo` may create an explicitly targeted session on the owner's kernel through normal creation, without inherited elevation. Regular agents remain in their room. |

## MP-08 / MP-11 — regular-agent surface and enforcement

Add `chariox.events.yield`, `chariox.process.start/watch/cancel`,
`chariox.history.search` and `chariox.handoff.request` alongside the mapped
tools. Preserve existing `chariox.send_agent_message`; regular messaging
already has active-turn and remote-origin regressions in
`runtime/router/tests/agent_messaging.rs` and `remote_agent_message_origin.rs`.

Messages default to `reply_requested=false` and `urgency=non_urgent`. The
receiver does not owe a reply: its ordinary turn answer is completion feedback
to the sender. Receiver instructions say to handle the message as attributed
work/data and avoid courtesy replies or acknowledgement chatter. An explicit
`reply_requested=true` asks for a correlated reply through the message tool;
that reply does not itself request another reply. Record message ID, sender,
target, urgency, reply preference and completion correlation atomically with
admission. Notify the sender when the receiving turn ends, including its safe
answer/reference or failure/cancellation, for ordinary messages as well as
delegation. A yielded turn reports waiting/progress, not a claim that the work
is finished. Several messages in one turn retain distinct correlation IDs;
delivery ACK and turn completion are separate. A completion notification is
not a new request/message and cannot create a feedback loop.

Non-urgent messages stay in the event inbox for the receiver's next turn; an
idle or yielded receiver wakes on ordinary admission, while an explicitly
user-stopped receiver keeps a visible pending item. Urgent messages steer the
exact running turn directly through the existing provider seam. Idle/yielded
receivers wake normally for urgent messages too. Starting providers retain
the event until a real turn is available. Remote urgency uses the same leased
steer/receipt route, with no alternate prompt authority; partition, unsupported
steer and uncertain outcomes follow the wake delivery rules below. Neither
urgency nor reply preference conveys elevation or approval authority.

Derive owner, room/session, agent and provider-run identity from admitted MCP
or the authenticated home-owned lease, never arguments. Resolve references
only within that session; explicitly reject ambiguous aliases and foreign
room objects. Room membership permits spawning, messaging, reading public
work and creating/running workflows regardless of the old controller link.
No per-agent spawn, workflow or delegation quotas. Memory/output bounds and
temporary resource backpressure still apply uniformly; do not inherit the
old notification subscription counts as product quotas.

Use one named room-tool admission module under the runtime, called by local
MCP, shell/native bridges and leased forwarding. Replace the meta gates in
`runtime/state/tool_dispatch/meta.rs`, target checks in
`runtime/router/meta_runtime_command/request.rs`, trace/history checks in
`runtime/state/tool_dispatch/meta/{session_tools,trace}.rs`, and workflow
access in `runtime/state/workflow_access_owned_state.rs`. Keep the router as
wiring. Recheck membership, live run, grant epoch and target binding after
async waits and before mutation/results; reject stale worker leases. Shell
descendants must use the current process-bound admission, not inherited
owner-terminal authority. Catalog filtering is assistance, not enforcement.

A user request to open/attach an App or kernel browser records an explicit
capability grant with owner, room, agent, resources and prompt causation. It
uses #900's `runtime/user_domain_access.rs` and
`runtime/state/user_domain_access_runtime.rs`, rather than simulated focus.
Regular agents cannot grant themselves arbitrary existing tabs or installations.
Delegates may receive a subset of the parent's authorized resource grants,
with attribution; they never inherit elevation or unrelated resources.
Browser/notes retained grants retain their existing resource/idle/revoke rules.
Vault fills require the additional authority described next. App bindings do
not impersonate the human App frontend, publish unvalidated tools, or answer
App approvals. #900's App-agent MCP adapter is still unimplemented; capability
granting must not falsely advertise that gap as working App control.

## MP-08 / MP-11 — /sudo at prompting

Reuse `runtime/state/sudo/{entry,policy,lifecycle,receipts,process}.rs`,
`runtime/state/critical_approval_passkey.rs` and
`runtime/state/passkey_prompts.rs`. Today sudo is local/regular-only, ends at
yield, accepts a broad `chariox_kernel_request`, and permits critical approval
resolution. Its fresh-sudo check also rejects `passkey_remember_minutes` even
though the ordinary critical-approval flow supports 1–15 minutes. These are
explicit implementation changes, not existing window behavior.

1. The owner submits `/sudo <prompt>` in a real terminal. One kernel-owned
   interaction names the agent, full prompt and finite elevation window. Verify
   a fresh passkey through the existing terminal-only, rate-limited verifier;
   passkey bytes never enter prompt/history/provider context.
2. Bind a non-transferable elevation entry to owner/kernel/session/agent,
   authorizing terminal, initial prompt, expiry and revocation epoch. Proposed
   default is 5 minutes with the existing 1–15-minute selector, subject to owner
   ratification. A remembered passkey for another action cannot mint elevation.
3. Queueing, delegation, yielding and waking cannot extend expiry. Check the
   same entry on each new turn and every privileged call, including after
   waits. Expired wakes run as regular agents and can request owner renewal.
   Wall-clock rollback cannot extend a live monotonic deadline. Proposed
   restart behavior is fail closed: preserve value-free audit, invalidate live
   elevation and wake regularly until the owner reauthorizes.
4. Explicit revoke, passkey rotation, session end, agent removal or placement
   change ends elevation immediately. Stop pending privileged operations at
   their next fence; already delivered physical input cannot be undone.
   Spawned/forked agents and workflow nodes never inherit it.

Beyond room powers, sudo allows explicit administration of the owner's other
sessions/resources on this kernel and typed Vault/digital-task operations.
Keep the existing restrictions on minting access/sudo, pairing, relay/Cloud
identity configuration, passkey changes, credential export and secret reads.
Constrain `chariox_kernel_request` to the same operation policy. Remove all
agent paths to `RespondToInteraction`, including critical decisions and native
approval responses; dropping only `chariox.meta.resolve_runtime_interaction`
would leave the current sudo bypass open.

**Vault generation and login.** Add a typed `chariox.vault.generate` operation:
only metadata/password policy enters the tool. A kernel CSPRNG generates a
bounded password, writes it directly through `secret/vault.rs` under the Vault
lifecycle lock and returns an opaque credential handle. No stdout, model
argument/result, clipboard, debug log or event contains the value. Use a
request receipt so retries return the same committed handle, not a new value;
ambiguous writes reconcile before retry. No model-supplied password command.

Build login on #900's `runtime/state/kernel_browser_secret_runtime.rs` and
`chariox.kernel_browser_paste_secret`: opaque handle, observed tab/generation/
document/node and authorized origin; default `submit=false`. Extend admission
so a live sudo window can authorize a retained granted target without regaining
focus. The kernel fills the discovered login fields; the model selects/clicks
the observed login action and never types credentials. Recheck origin, target,
grant, elevation, Vault lock and observation policy after waits. Retain the
capture/input barrier and protected DOM/pixel/mirror path, including echoed
values, errors and retired credentials. A browser grant or unlocked Vault alone
never supplies this elevated login authority; existing explicit human-approved
fills remain available.

**Payments.** Sudo permits preparing a permitted digital task, including a
payment. Each payment commit requires a new owner confirmation, regardless of
the elevation or passkey remember window. Bind a single-use confirmation to
payee, amount/currency, order/cart digest, origin and observed document/action;
changed details require a new confirmation. Serialize consumption at the kernel
commit boundary. Lost acknowledgements mean an uncertain payment, never an
automatic second click or charge; reconcile with the site/App receipt or ask
the user to inspect it. Enforce at the protected browser/App action seam as well
as tool admission. Unknown/opaque payment surfaces require user hand-off; do
not claim the kernel can reliably classify every arbitrary website's charges.

Official provider models apply their own usage policies, and sites apply their
terms and automation restrictions. Elevation is kernel authority, not a model
policy override. Keep secrets out of context and offer the isolated user action
below when the model refuses or automation is disallowed. Do not disguise the
task or ask another provider to evade the refusal.

## MP-08 / MP-09 / MP-10 / MP-11 — durable wake service

Reuse the durable writer/SQLite transaction boundary in `durable_state.rs`,
the event inventory in `runtime/metaagent_event.rs` and
`runtime/state/metaagent_event_owned_state.rs`, and the ordinary prompt dispatch
services. Current meta delivery is tied to one owner metaagent, persists some
event records best-effort, and creates visible event prompts. Generalization
must commit before reporting registration/yield success; logging a persistence
failure and proceeding is insufficient.

Persist registrations (recipient room/agent, source/filter, cursor, mode,
deadline and lifecycle), occurrences (stable source/occurrence identity and
safe payload), delivery intents (recipient, original turn/run/lease, submit
attempt and receipt) and waiting state. Store them through the existing single
writer; extend shared inbox/outbox/receipt primitives where compatible. Keep
workflow notifications' own publication/queue rules intact rather than forcing
every agent wake to be a workflow invocation. Redact payloads before durable
storage and render them as attributed untrusted data. Bounded hot caches must
not evict unacknowledged occurrences; expose cursor gaps/retention coverage.

`events.yield` takes registration IDs and the inbox cursor last seen. In one
transaction validate live registrations, mark the agent waiting, and check for
already pending occurrences. Only acknowledge yield after the provider turn
settles on its native seam. An event racing settlement stays durable and is
scheduled after settlement; one arriving before registration is recovered by
the source cursor. Refuse a supposed wait with no wake source/deadline. An
ordinary final response may still finish work; no goal state is required.

Delegation atomically records its correlation and completion subscription
before submitting the child work. Route success, failure and cancellation to
the delegator with answer/output references and a safe bounded answer excerpt.
The recipient can read the full public answer/history. This applies to every
agent and nested delegation; it is not a broadcast wake of the entire room.
Detect self-trigger loops and coalesce repeated source updates without dropping
completion occurrences. Cancellation/session end closes registrations; a
missing delegate produces a terminal failure event, not eternal waiting.

**Delivery and ordering.** Give each recipient a durable increasing sequence;
preserve source occurrence identity across reconnect/restart. Uniqueness of
source/occurrence/recipient prevents duplicate admission and yields one logical
wake intent. Persist send intent before provider or relay I/O. Reuse #885's
`durable_state/workflow_notifications.rs`, `durable_state/app_event_delivery.rs`,
`runtime/state/notification_delivery.rs` and existing worker steer receipts:
local `submit_epoch` and remote exact kernel/machine/lease/run/target-prompt
correlation distinguish accepted, rejected and uncertain sends. Enqueue success
is not provider acceptance; turn completion/restart is not proof of rejection.
Hold uncertainty for exact receipt reconciliation or explicit expiry/failure,
never fall back to a duplicate new turn. Transport retry is allowed with the
same occurrence only where recipient admission deduplicates it.

Thus exactly-once means durable admission and logical scheduling, not guaranteed
model execution or exactly-once external effects. Provider seams lacking
authoritative acceptance leave an observable uncertain delivery. Inbox `ack`
does not repair that ambiguity. Keep fairness with queued user work and per-agent
FIFO wake admission; an uncertain earlier item blocks that agent's later wakes,
not other agents. Coalesced observational updates report their skipped range.

For a running turn, default non-urgent delivery queues an event in the event
inbox, not the queued-user-prompt list. Explicit urgent delivery uses the
existing native steer path only against the exact still-running prompt. Recheck after
provider-lane acquisition. Proven rejection/unsupported steer queues a later
wake; uncertainty stays pinned to the original attempt. Workflow events obey
existing node claims. An explicitly user-stopped agent stays stopped with a
visible pending event; only a kernel waiting agent auto-wakes.

**Recovery and remote execution.** On restart replay committed registrations,
inbox cursors and send intents before scheduling. Reconcile ordinary provider
runs and leases first; rebuild timers, subscribe to durable agent/workflow
sources, and query exact receipts. Home owns recipient ordering, waiting state
and wake prompts; workers execute through
`runtime/state/remote_prompt_dispatch_runtime.rs` and
`remote_prompt_worker_submission_runtime.rs`. Worker process observations enter
over its current authenticated lease. Fence old generations and forged source
identities; the relay only carries encrypted envelopes. Partition retries never
allocate a second provider run for a pending wake. Sudo can be home-authorized
for a leased agent only through the existing leased prompt admission with a
bound expiry/revocation context and checks at both ends; no reusable privilege
token, secret or new relay authority. Until that leg passes its drill, refuse
leased sudo explicitly. #900 still forbids cross-kernel user-domain browser
control: a remote wake does not grant access to the home's browser. Room-owned
Browser/Computer routing remains its separate existing path.

**Failures and activity.** Disk-full/writer failure refuses registration/yield;
unsupported provider yield leaves the turn running with a clear error. Corrupt
registration/receipt is quarantined with safe status, not guessed delivery.
Expired events, cancelled processes, unavailable workers and disabled sources
produce explicit outcomes. Pending events survive terminal disconnect; a kernel
restart does not automatically replay process launches or browser mutations.
Scheduling a wake joins `runtime/managed_kernel_activity.rs` admission and the
existing quiescence fence before starting a turn. A live owned process or
admitted wake counts as work; a future timer/idle subscription alone does not
keep a managed machine alive indefinitely. Preserve last-turn-finished idle
timing, minimum-runtime and all shutdown triggers. After a legitimate shutdown,
durable timers become due on normal restart; this design adds no Cloud power-on
scheduler and makes no asleep-machine punctuality promise.

## MP-08 / MP-09 / MP-11 — generic watchers

| Source | Kernel contract |
| --- | --- |
| Process exit/output match | Register a kernel-owned process handle, not an arbitrary PID. Start with argv, explicit room workspace and authorized execution placement through ordinary kernel process services. An agent can run `gh pr checks --watch` as a normal command; the kernel understands only exit/output. No application parser. |
| Timers | Durable one-shot or interval deadline, cursor and missed-fire policy. Default coalesces missed intervals to one event with count; persist next due time. Use monotonic live waits and persisted timestamps for recovery; clock jumps cannot duplicate an occurrence. |
| Agent/workflow | Reuse kernel prompt settlement and workflow completion/intermediate-output commits. Atomic source event capture prevents completion-before-subscription races. |
| App events | Authenticated, capability-admitted App occurrences on the same generic inbox channel, using existing App receipts/outbox. An App may interpret GitHub/Slack; core must not. |

Process watchers own start identity/lease, argv digest, descriptors, cancellation
and cleanup. Watch only handles started by this run or explicitly handed over
by the kernel; never attach to arbitrary owner processes. Persist intent before
launch; an uncertain launch is reconciled, never automatically rerun. After
kernel/worker loss, a verifiably supervised process may be reattached; otherwise
emit `process_lost` and let the agent/user choose a new launch. A persisted PID
alone cannot establish ownership. Cancellation verifies PID/start identity and
all owned group members; reject 0, 1, -1, missing/NaN and non-owned targets.

Stream-drain stdout/stderr without blocking the provider/relay pump. Bound
retained tails, spool growth, line/pattern length, match rate and matcher work;
use a linear-time matcher, sanitize terminal escapes and report truncation.
Apply the shared secret/private-overlay policy before matching or persisting
model-visible output. Unknown protection state refuses output events. A match
is untrusted data with process/source attribution, not a command or approval.
Default watchers emit one occurrence per armed condition; rearming is explicit.
Resource backpressure reports failure without silently evicting wake sources
or imposing delegation quotas. Session end cancels owned watchers/processes;
worker loss cannot authorize host process inspection or access another room.

## MP-08 / MP-10 / MP-11 — isolated user hand-off

Extend `session/runtime_interactions.rs` with a typed hand-off payload and
kernel-owned operation subject, retaining
`session/runtime_interactions/subject.rs`'s exactly-one-subject rule. Extend
`local/api/types/terminal_interaction.rs` with protected response routing,
not a generic custom-choice text reply that gets rendered to the model.
Reuse `runtime/state/runtime_interaction_owned_state/{registration,maintenance}.rs`
and `runtime/state/passkey_prompts.rs` projection/first-response conventions.

A request identifies one action: click an observed element/region, enter a code,
or enter a missing secret with an optional save-to-Vault choice. The kernel
binds owner/session, originating run/event, browser/App target, generation,
document/node or region, protection epoch, permitted action and expiry. It
validates the target and creates one interaction visible on every authorized
open web/local/remote TUI terminal in the session. Collaborators see a pending
status; only the resource owner gets protected input/view authority. Mobile
uses the same protocol when implemented.

Reuse #900's `runtime/state/kernel_browser_mirror.rs`,
`kernel_browser_secret_runtime.rs` and protected display capture/input sink.
The scoped view is a crop or inert subtree from the existing browser, with
short-lived interaction-bound input authority. Do not create another Chromium,
browser profile, viewer server or raw CDP channel. The mirror currently masks
password/payment/OTP regions and opaque frames; it cannot simply be unmasked
for a hand-off. Prefer a kernel-owned secret/code input widget. A clickable
region uses safe labels and protected pixels; unsupported opaque targets ask
the owner to use the existing full browser takeover for that isolated step.
TUI renders safe target/status plus the existing view attachment; a terminal
without live-view support opens the same kernel attachment, not an external
runtime proxy, and remains able to submit the protected input or cancel.

The owner input travels over the normal encrypted terminal path to the kernel,
then directly to the bound browser/App operation. Typed bytes never enter
provider prompts/tool results, history, event payloads, analytics, console
captures or relay/Cloud plaintext. Register observation protection before
injection, including page echoes and future frames. Saving a new secret writes
directly to Vault under explicit owner choice; it grants no new model read
authority. The model receives only completed/failed/cancelled/expired and safe
target metadata. A hand-off click does not count as payment confirmation unless
it is explicitly the single-use, bound owner payment interaction.

Atomically claim the first response/action before I/O; other terminals close
the popup and late replies get already-answered. Navigation, target replacement,
revocation, takeover conflict, session end and timeout cancel or require a fresh
request. No automatic answer on timeout. Uncertain physical input is never
replayed. Reconnect reprojects the same interaction; restart restores safe
pending metadata but revalidates a fresh target before allowing action. Audit
only actor, binding, timestamps, action kind and outcome. A refusal is a reason
to hand off, not a permission for the model to resolve its own interaction.

## MP-08 / MP-10 / MP-11 — session history search

The existing operational store already reads `history_events` with session/
agent/provider/workflow filters in `history/operational_query.rs`; text search
is escaped SQL `LIKE`, not FTS. Reuse `history/operational_session.rs`,
`runtime/history_executor.rs`, `runtime/history_requests/*` and leased history
projection. Add a rebuildable SQLite FTS5 index alongside that store, keyed by
canonical event ID/sequence and owner/session/agent, with a redaction/index
version. Index public prompts, answers and normalized tool call/result text.
Return bounded snippets and opaque event references, then use the same scoped
read API for detail; no filesystem paths or unrestricted blob resolver.

Authorize the room and requesting live agent/lease before querying, ranking,
counting, snippets or pagination. Join authorized event IDs inside the query;
postfiltering top hits leaks existence and loses valid results. Any agent in
the room can search/read any room agent's public work, including delegated and
leased work. Sudo may explicitly query the owner's other sessions under live
policy; it cannot read another owner's private material.

Build an explicit public-history projection before both persistence to FTS and
return. Exclude private overlays/hidden system context, provider auth/login,
passkeys, Vault values/handles' secret bodies, protected hand-off input and
private reasoning. No file crawling or indexing attachment/private-overlay
bodies. Apply Vault echo scrubbing to tools/errors/URLs before tokenization;
an opaque redacted placeholder cannot retain raw search tokens. Reuse the
observation/provenance protection under `runtime/state/room_secret_observation.rs`
and provider-history normalization (`provider_output_policy/tool_history.rs`),
but neither alone proves full history redaction. Legacy rows with unknown
provenance remain excluded until sanitized; protection failure fences reads.

Advance the index cursor with public projection writes; rebuild incrementally
on schema/redaction change with an explicit incomplete status. Redaction or
retention/deletion invalidates affected indexed rows and snippets immediately;
rebuild from sanitized source, not a raw-history trigger. Protect FTS backing
pages, WAL and temporary files like history state; sensitive invalidation must
also remove stale searchable copies and caches. Existing archival retention
is unchanged: search reports its coverage and does not fabricate full provider
history. Remote queries use home-owned, lease-admitted history projections;
Cloud and relay maintain no index.

## MP-08 / MP-10 / MP-11 — protocol and migration

Reserve coordinator allocations before implementation; choose no local or
relay numbers in this plan. New serialized needs are room-tool availability
and capability grants, event origin/cursor/registration and yield/wait state,
process/timer status, bounded elevation expiry/revocation/leased context,
hand-off target/view/protected input/payment confirmation, and history query/
result/redaction coverage. Reuse `RuntimeInteraction`, prompt origins and
existing E2EE leased envelopes; extend rather than fork terminal transport.
For each changed shape bump the coordinator-assigned shared version, update
Rust/TS snapshots and hash guards, gate only dependent web/native features,
and drill old-client rejection plus mixed-client reconnect. No silent fallback
of protected inputs to ordinary prompt text or legacy agent approval tools.

Land all-agent tools behind a transition flag while `/meta` continues for
existing sessions. Preserve old aliases through the new room authorization;
remove task tools from the new surface. Let existing tasks drain with old
durable records readable; never auto-elevate a metaagent or convert a task into
a sudo grant. Translate legacy subscriptions/cursors idempotently, preserving
event identities and reconciling uncertain sends. Keep controller links as
legacy attribution until peer administration migration is decided.

Recommend retiring `/goal`'s current persistent goal mechanism after a separate
dependency audit of parser, tools, durable recovery and UI. Ongoing work is an
ordinary prompt plus registered events, without `complete_task` or a special
budget authority. Keep legacy goal history readable and stop accepting new
goals only at the final migration gate. PR #873's `/meta` retirement must wait
until sudo windows, durable wakes, regular delegation and real-path drills
pass; do not merge its deletion first. Remove obsolete mode prompts, task-plan
guards, monopoly event targeting and code only once no live recovery depends
on them. The docs-only study changes no protocol or runtime behavior.

## MP-08 / MP-09 / MP-10 / MP-11 — PR sequence and acceptance

Sizes are review estimates excluding generated snapshots/drill artifacts:
S ≈ 200–500 changed lines, M ≈ 500–1,000, L ≈ 1,000–1,500; split a larger PR
at the named service boundary. Each PR includes its focused fail-first tests
and its real-path drill. Sequential dependencies below avoid deleting `/meta`
before replacements work; hand-off and history can follow the room admission
foundation independently of watchers.

Every drill uses the built real web app entry/flags, real CLI/TUI, real kernel,
a real local relay and real official provider(s) when provider runs are touched.
Drive prompts/actions via Playwright web UI and the TUI, not internal IPC alone.
Use the shared `scripts/e2e-stack` when available; otherwise build the equivalent
isolated stack. A first-party service is a controllable test target, never a
replacement for real runtime/provider components. Run the same scripted flow
on base to produce RED, then candidate for GREEN, with deterministic faults at
the relevant acceptance boundaries. No fixture-only pass closes a PR.

| PR / size / dependencies | Scope and required real-path red-to-green drill |
| --- | --- |
| 1 — room admission/tools, M; main | From web create an ordinary agent; from TUI ask it to spawn two delegates, message an existing peer and create/run a workflow without `/meta` or task plan. Parent receives each delegate's answer. Attempt foreign-room IDs/aliases, stale run and forged lease; deny without leakage. Exercise both clients/reconnect. Repeat the flow with Codex, Claude and OpenCode official harnesses. MP-08/MP-10/MP-11. |
| 2 — durable events/yield, L; 1 + #885 | A real parent registers completion, yields and is visibly waiting in web/TUI; child finishes and wakes it with the answer. Send default no-reply/non-urgent messages to busy, idle and yielded agents: busy delivery waits for the next turn, idle/yielded delivery wakes, and senders get answers without reply chatter. Opt into a reply and verify correlation; urgent messages steer the active turn. Race registration/yield, duplicate delivery, kill/restart only the drill-owned kernel, and lose acceptance ACK after a real provider write. Observe one logical wake, receipt reconciliation/uncertain status and continued public conversation; no queued user-prompt duplication or feedback loops. MP-08/MP-09/MP-10/MP-11. |
| 3 — process/timer watchers, M; 2 | Through web prompt a real agent to start a lane-owned CLI fixture process, register output/exit and timer, and yield. TUI observes its wakes; include a real `gh pr checks --watch` on a coordinator-approved read-only test PR without core GitHub logic. Flood output/malicious text, cancel, restart, reuse a PID and disconnect worker; bounded safe tails, no unsafe signal/relaunch, explicit lost process. Show future timer permits legitimate idle shutdown and due event recovers on restart. MP-08/MP-09/MP-10/MP-11. |
| 4 — sudo windows/policy, M; 1 + 2 | Submit `/sudo <prompt>` in web and answer the passkey in TUI, then reverse clients. A real agent yields/wakes inside and outside the selected window; privileged calls succeed then refuse, regular work continues. Revoke/rotate/restart and queue past expiry. Try approvals through meta aliases, raw kernel request and shell/native bridge; all agent answers refuse. Delegates remain regular. MP-08/MP-10/MP-11. |
| 5 — user-requested capability grants, M; 1 + #900 | In web ask a regular agent to open/attach a trusted App and kernel browser, then switch focus while TUI observes retained allowed use and explicit revoke. Unrequested unrelated tab/App acquisition and agent-created focus cannot grant authority. Verify native App admission and executable App-agent path once that dependency exists; until then that row is blocked, not accepted. Room browser paths still work for leased agents; user-domain cross-kernel calls refuse clearly. MP-08/MP-10/MP-11. |
| 6 — Vault generator/login, M; 4 + 5 | Owner authorizes a real provider agent to generate a credential and register/log into a first-party site in the kernel browser from web, with TUI observing the same tab. Verify authenticated session server-side; lost ACK returns the same handle. Test locked Vault, stale document, wrong origin and expiry during wait. Scan transcripts, console/log captures, DOM/pixels/mirror, events and index using private synthetic canary checks that report booleans only. Include owner-approved public-service login or identify the missing account/owner action precisely. MP-08/MP-10/MP-11. |
| 7 — hand-off, L; 2 + 5 (+ 6 for save) | A real agent hits a controlled refusal/automation-disallowed step and requests click/code/secret in turn. Owner completes via web scoped view and TUI protected entry; both show one interaction and agent resumes from safe status. Race two terminals, navigate, timeout/reconnect and echo entered input; no model/trace/pixel leakage, no second action. Unsupported region uses the existing takeover view. MP-08/MP-10/MP-11. |
| 8 — payment confirmation, M; 4 + 6 + 7 | Real agent prepares a first-party checkout through the browser shown in web. TUI owner confirms each distinct payment; verify service receipt. Reuse confirmation, alter amount/payee, omit response and drop commit ACK; no stale approval or second charge. Confirm opaque surface requires human step. Validate a real site's owner-approved test/sandbox payment path; missing owner/account blocks acceptance, with no live-money charge required by the drill. MP-08/MP-10/MP-11. |
| 9 — history search, M; 1 + protected history projection | Through web ask a real agent to find a peer's prompt, answer and tool result; TUI drills detail/pagination after kernel restart, including leased history. Foreign-room search/count/snippet and guessed blob references refuse. Private-overlay, Vault echo and hand-off canaries never match. Exercise invalidation/reindex and deletion; coverage is honest and no stale token remains searchable. MP-08/MP-10/MP-11. |
| 10 — leased wakes/elevation, M; 2–4 | Real home/worker kernels across local relay run a leased official-provider delegate. Web/TUI drive no-reply completion feedback, explicit reply, urgent/non-urgent delivery to busy/idle/yielded remote receivers, and partition/reconnect, lease replacement and expiry; home ordering and one worker run survive. Sudo wake stays within its exact window, expires/revokes at both ends, no privilege inheritance; cross-kernel user browser access stays denied. Repeat placement on a fresh ordinary/Path-1 comparison with authorized coordinator infrastructure. MP-08/MP-09/MP-10/MP-11. |
| 11 — retirement/migration, S–M; 1–10 accepted | Open persisted legacy `/meta` tasks/subscriptions and `/goal` history through real clients; drain/migrate without elevation, missing events or duplicate wakes. After retirement ordinary agents perform the combined delegate → yield → wake → search → browser → hand-off flow, and sudo does Vault/login/payment confirmation. Old clients fail with the correct minimum-version message. Reconnect web/local/remote TUI and rerun ordinary/managed activity and security matrix. Only then retire #873 paths and obsolete task machinery. MP-08/MP-09/MP-10/MP-11. |

Before calling any implementation done, retain exact OSS/Cloud/source/binary
and provider identities, commands/exit codes, RED first seam, GREEN assertions,
screenshots and redacted console/log captures per step, resource samples and
owned cleanup under the lane's external evidence directory. On builder2 keep
10 GiB disk/12 GiB MemAvailable, use at most four cargo jobs under its compile
lock and avoid concurrent full Node suites. Never retain typed secrets in
evidence; private checkers report only absence/results. Remove only exact
lane-created state/processes/containers after verifying ownership and preserve
shared reviewer/key/account state. Missing credentials, owner payment/login
action or unlanded real client/App support are explicit blocking seams; source
tests cannot relabel them accepted. No infrastructure action is authorized by
this design study.

## MP-08 / MP-11 — owner questions

1. Ratify the proposed 5-minute default/1–15-minute elevation window and
   fail-closed loss of elevation on kernel restart. Wakes themselves persist
   and run regularly after restart; no automatic renewed authority.
2. Do room powers include renaming/deleting peers and editing/cancelling their
   existing workflows? Spawn, communication, public history and workflow
   creation/run are already decided. Until clarified, retain existing object
   control for destructive peer administration without restricting those
   decided powers.
