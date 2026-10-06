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
Round 2 owner answers (2026-10-06, PR #905) supersede the short-window proposal
and unresolved peer-administration question: hour-scale elevation, descendant
control, no silent dormancy and exhaustive acceptance are requirements below.
The review inbox's binding 11:45/11:48 UTC decisions supersede this round's
transitive proposal and goal retirement: direct children only; retain `/goal`
as the explicit hybrid continuation loop. Plain prompts use lighter guards.
This revision starts at design commit `3077fff6bf9b02e589b057d3133d22869bb6f6bd`;
the source inspection identities in the table remain unchanged.
Round 3 starts at `05754fc6a8e30ef30d8f9b3a7357d522f94d0d6b` and incorporates
the binding 12:22/12:34 UTC inbox additions: post-overhaul evals and the two
flagship external-service build/deploy variants below. Runtime pins stay fixed.

Source read for this study:

| Input | Exact identity and use |
| --- | --- |
| OSS public `main` | `e325afa580d81954e2c179757fc53fa02ed2a2b3`; original study's runtime baseline. All unqualified paths below refer to this commit; round 2 changes docs only. |
| PR #900, `md/round2-on-main` | `6dde21a8c10c9ef2b9f7f0a271cace00591f0bd1`; user-domain access, kernel browser, Vault protection and display/mirror seams. These are dependencies, not main behavior. |
| PR #885, `wf/outbound-notifications` | `4998b1120f16a201eae06b1891c21770e9730b2e`; durable delivery and uncertain provider sends. Read this successor, rather than the older local branch of the same name. |
| Current `/goal` shortcut | `apps/cli/src/command-actions.ts` and `root-workflow-shortcuts.ts` at the OSS base route `/goal` to registry template `planner-worker-reviewer`, reusing the focused agent and spawning worker/reviewer nodes. This is not yet the goal state/loop designed below. |
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
| `workflow_code.create`, `workflow_code.read`, `workflow_code.list`, `workflow_code.update`, `workflow_code.delete`, `workflow_code.validate`, `workflow_code.apply`, `workflow_code.run` | All agents: `chariox.workflow_code.*`, room-scoped definitions/runs through existing workflow services. Read/validate/run peers' public definitions; update/delete/apply only self-created or direct-child-created objects under the creator rule below. |
| `workflow_code.export`, `workflow_code.import`, `workflow_code.package_export`, `workflow_code.package_import`, `workflow_code.source_export`, `workflow_code.source_export_directory`, `workflow_code.source_export_dir`, `workflow_code.canvas_contract` | All agents, same room scope and existing artifact/path validation; `source_export_dir` remains an alias of `source_export_directory` until alias retirement. No private overlays in exported model content. |
| `workflow_registry.list`, `workflow_registry.get`, `workflow_registry.add`, `workflow_registry.add_from_workflow`, `workflow_registry.delete`, `workflow_registry.load`, `workflow_registry.run` | All agents, room bindings and authorized library reads; room mutations obey direct creator checks, including replacement/import/apply. Owner-wide registry administration requires `/sudo`; it cannot bypass the peer restriction. Publication keeps its existing trust/admission checks. |

The command catalog is `runtime/metaagent_command_registry/catalog/*`.
Canonical entries are mapped below; aliases inherit their canonical policy.
`Allow` in today's docs is not proof of implementation: `routed=false` must
remain visible and cannot be promoted by changing the advertised tool list.

| Catalog file and entries | Destination and qualification |
| --- | --- |
| `agent.rs`: `session overview`, `agent list`, `agent spawn`, `prompt` | All agents. Spawn any supported agent in the room on authorized placement/accounts. Make `prompt` delegation through the same message/prompt admission, with automatic result correlation; prevent workflow-claim theft. |
| `agent.rs`: `agent alias`, `agent delete` | All agents for their own direct spawned children only, using immutable home-kernel creator identity. No destructive authority over self, ancestors, siblings or other peers. |
| `agent.rs`: `agent focus` | Drop from model-callable authority. User focus stays a terminal action; an agent cannot focus itself to mint browser/App privileges. Request a capability grant instead. |
| `workflow.rs`: `workflow list`, `workflow new`, `workflow resolve`, `workflow alias`, `workflow node add`, `workflow node remove`, `workflow node instructions`, `workflow node can-complete`, `workflow node intermediate-output`, `workflow node wait-for-all-inputs`, `workflow node max-turns`, `workflow endpoint new`, `workflow endpoint alias`, `workflow edge add`, `workflow edge remove`, `workflow run`, `workflow runs`, `workflow get-run`, `workflow cancel`, `workflow resume` | All agents, room-scoped services. Read/run/observe peers' workflows; definition edits and run cancel/resume obey direct creator checks. Node output settlement retains its exact active claim. Event delivery cannot create another scheduler. `max-turns` remains an explicit workflow option. |
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

**Destructive scope: direct children only (11:45 UTC owner decision).**
Persist an immutable `spawned_by_agent_id` and room on each agent at the home
kernel's spawn commit, including leased agents and workflow-created nodes
(record the initiating agent, never the worker as creator). Agent A can
rename/delete B only when B's recorded creator is A in that same room. If B
spawns C, A must ask B through ordinary messaging to change C; B independently
passes the mutation fence. This keeps authority at the agent that created the
object and prevents an ancestor from destructively sweeping a whole chain.
No control over grandchildren, self, ancestors, siblings or pre-existing peers,
even under `/sudo`. Aliases, messaging, adoption and running a peer workflow
cannot change creator identity. Preserve creator tombstones for attribution,
not inherited authority; a removed parent leaves its children to the user to
manage. Legacy/unknown creator identity fails closed. The user retains normal
administration through terminal interactions.

Workflow definitions and runs each record their immutable creating agent.
Agents may edit/rename/delete definitions and cancel/resume runs created by
themselves or their direct children only; grandchild-created objects require
asking that child. Any room agent may read/run/observe a peer's public workflow:
its new requested run belongs to its initiating agent, but conveys no authority
to edit the peer definition or cancel the peer's runs. Imported/copied
definitions receive a new object ID/creator; replacement cannot overwrite a
foreign object. Recheck exact creator, room, version and run/lease at the shared
mutation fence, including aliases and native/raw bridges. Out-of-room sudo
admin cannot destroy unrelated agents/workflows. Before deleting its child,
an agent must settle/cancel its own authorized obligations; refuse automatic
cascade into grandchildren or foreign resources. Deletion reports failure to
waiters. Outstanding children of the removed agent remain visible for user
intervention, never silently deleted or adopted.

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
   default is 60 minutes; offer 1, 2, 4 and 8 hours, with a proposed maximum of
   8 hours per verification. This is a separate elevation duration selector,
   not the existing 1–15-minute passkey-remember setting. A remembered passkey
   for another action cannot mint elevation.
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

Every web, local TUI and remote TUI projects the same `/sudo` status row with
remaining time, absolute expiry, Revoke and Extend controls. Extend opens one
kernel interaction naming the current agent/window and selected new duration;
the owner enters the passkey again. On fresh verification, compare-and-swap
the current elevation ID/revision/epoch and set expiry to verification time
plus the selected duration (maximum 8 hours ahead), never add hidden banked
time. Concurrent Extend submissions settle once; stale responses reject.
An expired window requires a fresh elevation interaction and clearly shows
that regular work continued. An extension never replays a denied operation or
resurrects a revoked/rotated/restarted/placement-changed window. Reconnect reads
the kernel's deadline, not a client's countdown. Agents cannot press Extend,
submit a passkey or renew themselves.

At 10 minutes remaining, persist one warning per elevation revision and notify
all attached terminals; reconnect reprojects it, extension resets it. Expiry
emits a durable visible notice, removes privileged tool availability and keeps
ordinary work running. Fence queued/in-flight privileged actions again before
their effect; safely stop/settle a cancellable operation, reconcile an already
committed/uncertain effect, and explain any refused step. Do not kill unrelated
regular work or turn expiry into a silent provider/tool failure. During a wait,
expiry updates the row immediately and queues a regular re-evaluation event
through the same wake path; obligations, sources and deadlines survive.

**Trade-off flagged again:** an hour-scale grant increases unattended exposure
compared with a one-turn or minute-scale grant. Owner answers authorize that
trade-off for long tasks; the 8-hour cap is a proposal, not an existing limit.
Fresh extension, per-call fences, revocation, non-inheritance and fail-closed
restart remain required. Restart invalidates even a newly extended grant;
long delegation-chain drills must show explicit owner reauthorization.

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
ordinary final response is a done candidate subject to the obligations check
below; no agent-controlled task/budget authority is introduced.

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

## MP-08 / MP-09 / MP-10 / MP-11 — no silent dormancy

Every provider turn must settle into exactly one explicit disposition: `done`
(final answer, work complete), `waiting` (at least one live wake source and a
mandatory deadline), or `blocked` (needs the user, with one kernel-owned
RuntimeInteraction on all terminals). `working` describes an active admitted
turn. The later 11:48 UTC owner decision also permits explicit `/goal`
`continue`, atomically linked to its next working turn; plain prompts get no
general continuation loop. A bare stop, empty final, tool-only end,
provider EOF or adapter error is classified and reconciled, never left idle
with unfinished work. Explicit user stop/session cancellation is recorded as
a cancellation disposition, settles owned work and disables automatic wakes;
it must not masquerade as a completed task. No new agent-controlled task plan,
task budget or `complete_task` tool returns.

**Durable model, home-owned.** Extend the single writer with these records;
proposed names are contracts, not existing types:

| Record | Required fields and invariant |
| --- | --- |
| Task execution | Causal task ID for a plain prompt/delegated work or explicit goal and their resumed turns; owner/room/agent, current run/lease epoch, state/revision, safe progress summary, last progress sequence/time, no-progress wake count, done-correction receipt. Independent user prompts get separate tasks; overview cannot hide an older incomplete task behind a newer done turn. |
| Obligation | ID, task/creating turn/agent, kind (delegate, workflow run, watcher, owned process, hand-off), exact resource/run/source identity and epoch, lifecycle (`open`, `settling`, `satisfied`, `cancelled`, `failed`), completion cursor/receipt and cleanup disposition. Failed obligations require visible handling; resource disappearance alone does not settle the task. |
| Wait | Task/revision, source registration IDs and cursors, required obligation coverage, deadline timestamp plus live monotonic bound, wait reason, start time, last notification, next sweep/check and wake-intent ID. The safety deadline is separate from source liveness; it cannot hide loss of all sources. |
| Turn settlement | Task/turn/run/lease, candidate end state, final public answer reference, ledger revision, validation outcome, one correction attempt and delivery receipt. A stale settlement cannot override a newer turn or owner cancellation. |

Persist obligation creation with dispatch intent **before** delegate/workflow/
process launch, watcher registration or hand-off I/O; uncertain dispatch remains
open for reconciliation. Result commits settle the exact obligation and retain
unhandled failure/output until the parent acknowledges it. Reading a trace or
delivery ACK alone is not settlement. Workflow schedules and explicitly started
ongoing watchers are open obligations until cancelled or explicitly transferred
through a kernel-recorded, accepted owner handover; a final answer cannot detach
them. No arbitrary PID can become a process obligation. A caller's cancellation
is not terminal until the resource settles; cancel uncertainty prevents done.

**Enforcement points.** One named task-lifecycle/obligations service below the
router owns creation, settlement, wait validation, progress and recovery.
Provider adapters use only their official completion/hook/protocol seams;
Claude's native stop hook can supply early feedback, but the authoritative
check also runs at kernel turn settlement for Codex, OpenCode, Claude, native
TUI, workflow nodes and leased runs. No provider hook can bypass it. `yield`
includes task, sources and finite deadline; explicit blocked intent creates
or binds a typed user-required interaction. A final answer submits done intent.
Before committing any terminal state, compare-and-swap the active turn and
ledger revision, close the turn's mutation admission, and reject racing stale
tools. Creation before the fence appears in the check; creation after it is
denied. Only the accepted state emits final completion feedback to delegators.
Before accepting any done candidate, run the bounded original-request
completion check below, for both top-level prompts and delegated work.

- `done`: require a nonempty final answer and no unresolved obligations created
  for the task. If any remain, withhold done/completion notification and re-prompt
  **once** through ordinary dispatch with the safe list of obligation IDs,
  reasons and allowed wait/cancel actions. Persist that correction receipt
  before sending. A second invalid done, failed correction delivery or inability
  to resume the provider raises `blocked`; neither restart nor a new run grants
  another correction. Publish the purported answer as progress, not completion.
- `waiting`: in one transaction check at least one live admitted source, a
  future deadline and coverage of every open obligation. Covered sources must
  report terminal failures too; an uncovered obligation needs explicit cancel
  settlement or a new subscription. A valid wait commits only at native turn
  settlement, then releases provider resources through the normal adapter.
  Zero sources, omitted/past/infinite deadline or failed durable write refuses
  yield. If the provider has already ended, classify the invalid end and apply
  the same one-correction-then-blocked rule. No open task may end as idle.
- `blocked`: create/reuse one typed interaction bound to task/state revision
  with the reason, safe obligations list, owner action and finite reminder/
  escalation schedule. Project and notify all terminals; only the appropriate
  human can answer. Do not hide/cancel obligations automatically. They remain
  supervised and their outcomes update the interaction without unblocking it;
  only an explicit user response resumes
  regularly via one receipt or cancels explicitly. Pending hand-offs count here,
  with their own action expiry. No-response escalates visibly, never auto-approves.

**Deadline, dead sources and sweep.** A wait deadline creates a durable
`deadline_reached: re-evaluate` occurrence once per wait revision. It wakes the
agent even if a healthy source has not completed; a new wait needs a new finite
deadline and progress check. Source failure emits `source_lost` immediately;
loss of every dependent live source forces re-evaluation without waiting for
the safety deadline (required-source loss is reported even if other sources
remain). Delegate failure/deletion, workflow cancellation, lost process and
revoked App registration all participate. Source liveness means authoritative
state/cursor or fresh supervised lease, not cached UI or stale heartbeat.

Run an event-driven check on source/receipt transitions plus a kernel sweep
every 30 seconds, and before scheduling after restart. Sweep waiting tasks with
no live sources, overdue deadlines, missing wake intents or wedged deliveries.
Persist one wake intent per revision; reconcile receipt uncertainty before any
retry. If acceptance cannot be resolved within 2 minutes, raise a delivery-
blocked interaction and notify the user; do not launch a duplicate provider
turn. Writer failure exposes degraded/blocked status and retries persistence,
never acknowledges an undurable wait. The existing active-provider liveness
reconciler also routes dead `working` turns here. Sweeps share ordinary managed
admission/quiescence, and do not keep an otherwise idle machine powered on.

**Progress guard and visibility.** Propose N = 3 consecutive wake turns without
visible progress, persisted per task across restarts. Progress is a new public
artifact/result, handled obligation outcome or completed operation with safe
evidence, not a repeated status sentence, timer reschedule, ACK, new empty
delegate or cursor change alone. The kernel compares normalized public results
and resource receipts; it cannot prove the semantic completeness of arbitrary
model work. At N, stop automatic wake-loop scheduling and raise `blocked` with
the repeated reasons and next owner action. The owner may resume/cancel; resume
records an explicit new budget. Separately, 15 minutes waiting without progress
triggers a long-wait notification even if no wakes occurred. Coalesce reminders
per task/revision without hiding later failure or blocked transitions.

Room overview and every web/local/remote TUI show `working`, `waiting on X until
T`, `blocked on Y`, or `done`, open-obligation counts and last visible progress.
Waiting/blocked stops the active-turn timer and uses its own wait duration.
Long-wait and blocked notifications persist for reconnect; collaborators see
safe status, resource owners get the actionable interaction. Sudo remaining
time/near-expiry/expiry is an independent projection, never mistaken for done.

| Failure boundary | Required reconciliation |
| --- | --- |
| Home restart / crash during settlement | Reconcile provider runs and leases, replay ledger/waits/correction receipts, fence old turns, sweep overdue/dead sources before scheduling. Uncertain launches/actions are never replayed blindly; fail-closed sudo loss is visible. Timer recovery coalesces missed occurrences and clock rollback cannot extend waits indefinitely. |
| Partition / worker restart / remote or leased agent | Home retains task/lineage/ordering authority; worker reports exact authenticated run/lease/source receipts and performs the same turn-end check. Provisional worker final cannot mark home done. Missing freshness yields a visible uncertain/lost source and bounded re-evaluation/block; replacement fences old completions and never creates a second authority. |
| Lost ACK / wedged provider delivery | Reconcile the same intent/receipt; deadline/sweep can escalate uncertainty but cannot bypass FIFO or duplicate a provider submission. Blocked correction/wake delivery remains visible on all terminals. |
| Sudo expires or is revoked during wait | Preserve ledger/wake deadline, emit notice, wake regularly for re-evaluation, and deny privileged effects at home and worker. The owner must verify a fresh extension/authorization; delegates stay regular. |
| Managed shutdown / clients absent | Idle sources may allow normal MP-09 shutdown; due waits sweep on next normal start, with no Cloud auto-start promise. Kernel status/notifications persist without an attached client; reconnect shows overdue/blocked reasons and elapsed time. |

The ledger detects tracked work left open; the completion check below also
asks about the original objective. Neither can prove arbitrary real-world
completeness. Acceptance must check actual artifacts/deploy receipts. Every kernel launch/tool path creating tracked work
must register obligations; unsupported/untracked asynchronous dispatch fails
closed or surfaces a user-required block rather than promising wake safety.

## MP-08 / MP-09 / MP-10 / MP-11 — /goal hybrid loop and completion check

`/goal <objective>` stays the explicit long-task mode. It belongs to an ordinary
agent on the same event/obligation services, not a planner/worker/reviewer caste
or a new task-tool authority. `/sudo /goal <objective>` is a proposed client
spelling for one typed goal prompt plus a fresh elevation interaction; both
metadata fields enter normal kernel prompt admission. Goal duration and sudo
duration are independent; delegates inherit neither elevation nor goal mode.
The root goal judges their correlated completion events against its objective.

Persist a goal ID/schema version, owner/room/agent, protected public objective
and original prompt reference, revision/state, created time and accumulated
active/wait elapsed time, task ID/obligation links, check-in interval/next due,
evaluation intent/receipt/result, continuation sequence, progress counter,
blocked interaction and final answer/evidence references. Objective changes are
explicit owner edits with a new revision and evaluation fence. Kernel states
are `working` (including bounded `evaluating` phase), `waiting`, `blocked` and
`done`; explicit owner cancellation has its own disposition/history. No separate
goal scheduler, provider account or private history store. Persist continuation
intent before provider I/O; home ordering/current worker lease still apply.

**At each goal work-turn end**, validate ledger and evaluate against the
original objective and public results. If achieved and no unresolved
obligations, commit done with the verified final answer. Otherwise, if the
outcome needs the user, commit blocked. Otherwise, if live sources exist,
commit waiting with obligation coverage and a finite check-in deadline.
Otherwise immediately re-prompt the same agent to continue, using ordinary
dispatch and the next durable continuation sequence. This transient scheduling
phase is visible as working; it cannot become idle or falsely waiting. Source
loss/check-in wakes re-evaluate and take the same branches. The progress guard
counts immediate no-progress continuations as well as wakes, so the loop cannot
spin indefinitely. Work-turn EOF/error is reconciled before evaluation; provider
failure is not proof of achieved work. Explicit user stop disables the loop.

**Who evaluates, and cost/loop bounds.** The kernel owns the check and asks the
same official-provider agent once to confirm the original request is complete,
listing safe open obligations, new public receipts/artifact refs, and pending
failures. The agent returns achieved + final answer/evidence, continue, waiting
with sources/deadline, or blocked with the required user action; kernel policy
validates that classification and all bindings. It never accepts achieved over
open obligations or unresolved failures. Goal evaluation runs once after every
work turn, including delegated-result/check-in wakes; plain top-level and
delegated prompts run it only on a done candidate. A plain prompt has ledger,
deadline and dead-source guards plus the bounded invalid-end correction; it
has **no automatic continuation loop** when no sources remain.

Evaluation is one correlated native provider turn through normal dispatch,
with proposed 60-second service bound and 8 KiB public result limit. Its context
is the original request plus a bounded safe delta/obligation list, with paged
public references when needed; include coverage so truncation cannot mean done.
No additional evaluator agent or provider; no secret/private reasoning input.
Persist intent and result against the objective/ledger/work-turn revision;
lost ACK reconciles exactly, never spends a second evaluation blindly. A stale
result re-enters the revision check before accepting completion. Mark this
turn's origin `completion_check`; its own end cannot recursively trigger another
check, dispatch work or mint authority. On timeout/malformed result, make one
visible user-required recovery interaction, not an unbounded evaluation retry.
Cost is at most one check turn per work-turn revision, plus ordinary continued
work; the guard bounds no-progress attempts but does not cap a progressing
multi-hour goal. Surface check count/elapsed/cost metadata if the harness
provides it. Provider confirmation is evidence of its assessment, not proof of
arbitrary semantic correctness; drills independently verify deployment/artifacts.

A done claim with open obligations retains the one-correction-then-blocked
rule. The check lists them once; if the agent instead selects valid wait/cancel/
continue, follow that disposition. A second claim against the unresolved list
blocks, even for `/goal`. A legitimate not-achieved check with no sources
continues only for goal mode. Plain prompts that still need work receive the
single corrective turn when their ending was invalid; they cannot gain goal
mode through repeated final answers or agent-created messages.

**Check-in cadence and UI.** Propose default 10 minutes, owner-configurable
1–60 minutes per goal (plain waits use the same maximum deadline interval).
Every waiting deadline is at most `now + interval`, also bounded by an earlier
source/action deadline; recheck source progress, not merely source existence.
No infinite waits or suppressed periodic check-ins. With a 10-minute default,
the independent 15-minute notification measures total no-progress wait duration
across check-ins. Restart sweeps overdue checks once, coalescing missed intervals
without resetting progress. Sudo expiry/revoke is a separate immediate event:
continue regular goal work if possible; if elevation is still required, block
with the Extend/fresh authorization interaction. An owner-confirmed extension
can resume; an agent or background wake cannot. Restart always requires fresh
sudo and leaves goal/objective/deadlines intact.

Every web/local/remote TUI shows one goal row: objective, working/evaluating/
waiting/blocked/done, elapsed active and waiting time, sources/next check-in,
last progress, open-obligation count and blocked reason, with owner Pause/Resume/
Cancel and interval controls. Pause is explicit owner stop with suspended-loop
metadata; it cannot hide live obligations. Resume rechecks resources, admits one
continuation and never renews sudo. Goal state and `/sudo` remaining/Extend rows
are projected from the same home revisions after reconnect. Combined clients
show the same state; Cloud/relay own neither goal nor evaluation.

**Only the user can unblock.** Source events may update a blocked interaction
but cannot auto-resume it. Bind first owner response to task/goal revision;
revalidate current resource/target/lease and dispatch once. Every case below
notifies all terminals immediately and carries finite reminders and safe scope:

| Blocked case | Interaction and explicit resume path |
| --- | --- |
| Decision or missing input | Ask the exact question; owner provides a choice/input or cancels. Resume with the safe answer and unchanged objective unless owner edits it. |
| Hand-off: 2FA/human check/model refusal/site disallows automation | Protected click/code/secret interaction on existing target; owner acts or cancels. Resume from status only after target revalidation, never with typed bytes. |
| Missing user-owned resource: account, quota/billing, credential not in Vault | Name required account/resource and product setup path, never request model-visible credential. Owner links/funds/provides it through protected UI and explicitly retries. |
| Unanswered approval or expired sudo still needed | Project the existing owner-only approval or fresh Extend/elevation interaction; owner verifies/answers or cancels. Receipt alone resumes authorized work, never replays an uncertain effect. |
| Progress guard / unresolved runtime delivery or evaluation | Show failed/no-progress receipts and safe actions: inspect/fix resource, explicitly resume or cancel. Owner resume records a new guard budget; a reconnect or source heartbeat alone cannot unblock. |

**Migration from today's `/goal`.** Source at the pinned base implements the
workflow registry shortcut `planner-worker-reviewer`, not this native durable
goal loop. Keep the command and existing objective/invocation/history readable.
Version the new goal schema; migrate an existing invocation only with an
authoritative objective/owner/entry-agent binding, tracking its existing workflow
run and nodes as obligations. Never restart template nodes, replay a launch,
or elevate them. Where the original objective cannot be recovered safely, keep
the legacy run draining and ask the owner to adopt/edit it explicitly as a goal.
Atomic migration receipt/fence prevents legacy scheduler and new loop both
dispatching the same work. Completed/cancelled legacy runs stay terminal;
active ones reconcile once after restart, block on uncertainty and preserve
public history/cursors. New `/goal` uses the shared loop after protocol/client
gates; old clients keep safe legacy reads and receive an upgrade requirement
for changed behavior. Do not retire `/goal`, user-authored workflow templates
or `/loop` merely to remove obsolete `/meta` machinery.

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
and capability grants, immutable spawn/creator lineage, task/obligation and
turn-settlement records, event origin/cursor/registration and yield/wait state,
deadlines/progress/block interactions, process/timer status, elevation status/
warning/extension/revocation/leased context, goal objective/state/evaluation/
continuation/check-in/migration projections,
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
legacy attribution only; they cannot prove spawn lineage or destructive rights.
Backfill lineage only from authoritative historical spawn receipts; otherwise
leave destructive agent administration human-only.

Keep `/goal`, rebuild it on the shared hybrid event loop above, and migrate
its existing workflow-backed records without creating a second scheduler.
Legacy objective/workflow/history remain readable; no goal or metaagent gains
elevation. PR #873's `/meta` retirement must wait until goal continuation,
sudo windows, durable wakes, regular delegation and real-path drills pass.
Do not merge its deletion first. Remove obsolete mode prompts, task-plan
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

| PR / size / dependencies | Scope; mandatory appendix cells |
| --- | --- |
| 1 — room admission/tools, M; main | Room tools, immutable creator lineage and obligation registration at dispatch; A01, G01–G18 + S01–S04. MP-08/MP-10/MP-11. |
| 2 — durable events/yield, L; 1 + #885 | Inbox/receipts, explicit turn-end states, obligation enforcement, sweep/progress guard and all-client visibility; A02, G01–G18 + S01–S04. MP-08/MP-09/MP-10/MP-11. Split persistence and lifecycle enforcement at service boundaries if needed; neither is accepted alone. |
| 3 — process/timer watchers, M; 2 | Owned watchers/processes and MP-09 activity/recovery; A03, G01–G18 + S01–S04. MP-08/MP-09/MP-10/MP-11. |
| 4 — sudo windows/policy, M; 1 + 2 | Hour-scale grants, fresh extension, status/warnings and approval prohibition; A04, G01–G18 + S01–S05. MP-08/MP-10/MP-11. |
| 5 — user-requested capability grants, M; 1 + #900 | Prompt-caused resource grants and executable App-agent admission; A05, G01–G18 + S01–S04. MP-08/MP-10/MP-11. |
| 6 — Vault generator/login, M; 4 + 5 | Opaque generation, protected login and leakage negatives; A06, G01–G18 + S01–S04. MP-08/MP-10/MP-11. |
| 7 — hand-off, L; 2 + 5 (+ 6 for save) | Protected owner interaction, blocked obligations and safe resume; A07, G01–G18 + S01–S04. MP-08/MP-10/MP-11. |
| 8 — payment confirmation, M; 4 + 6 + 7 | Single-use owner confirmation and uncertain effects; A08, G01–G18 + S01–S04. MP-08/MP-10/MP-11. |
| 9 — history search, M; 1 + protected history projection | Room-authorized public FTS, redaction/invalidation; A09, G01–G18 + S01–S04. MP-08/MP-10/MP-11. |
| 10 — leased wakes/elevation, M; 2–4 | Home ordering, worker fences and placement parity; A10, G01–G18 + S01–S04. MP-08/MP-09/MP-10/MP-11. |
| 11 — retirement/migration, S–M; 1–10 + 12 accepted | Legacy drain/migration, minimum versions and final integration; A11, G01–G18 + S01–S04, then flagship FA01–FA10 and I01–I03. MP-08/MP-09/MP-10/MP-11. |
| 12 — hybrid goal loop, M; 2–4 + 10 | Persisted objective, bounded completion check, immediate continuation/check-ins, blocked owner paths and goal migration; A12, G01–G18 + S01–S06. Land before 11. MP-08/MP-09/MP-10/MP-11. |

After the overhaul PRs and their functional gates, run the separate **Evals and
optimization** phase below. It does not gate overhaul merge or MP acceptance.

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

## MP-08 / MP-09 / MP-10 / MP-11 — appendix: exhaustive acceptance matrices

The numbered tables below replace one-line drills. Factor the repeated axes to
keep the plan readable; **every Cartesian cell is mandatory**, not a choice of
representative combinations. Before each PR starts, the shared e2e runner must
materialize the entire expected cell set in an external machine-readable
manifest. Each row stores its expanded ID, numbered steps, assertions, evidence
paths, base/candidate source and binary identities, result and blocker owner/
action. The gate compares expected IDs with results: missing, duplicate, blank,
skipped, N/A or unknown results fail. No provider/client/placement substitution
or source-only pass. A runner failure cannot silently shorten the manifest.
At this revision the gating expected set is 19,224 A cells plus 432 I cells
(three scenarios × 72 axis combinations × two integration phases) and 1,440 FA
cells (ten controlled flagship cases × 72 axes × two integration phases).
The 576 FB showcase cells (eight cases × 72 axes) are separate and non-gating.
These are coverage
requirements, not executed results; the planning manifest initializes each
separate ID to B0 with its steps/evidence. Later PR splits must expand the same
requirements for every new PR; they cannot reduce coverage.

**Cell numbering.** For PR nn, enumerate
`Ann-{provider}-{client}-{placement}-{environment}-{case}` over **all** values
in the axis table, G01–G18 and that PR's S cells. For example
`A04-CX-W-L-O-S02` is fresh extension on Codex/web/local/ordinary. The steps for
each cell are, in order: (1) the axis setup below, (2) that PR's S01 feature flow
to reach the tested seam, (3) the numbered case steps, (4) evidence and owned
cleanup. G cases inject faults/negatives into that feature flow; S01 runs once,
not twice. Use isolated tasks/resources for each destructive variant. Each
variant named within a cell is required and individually asserted. Record
exact fault point before/after commit and first failing seam on base.

| Axis | Numbered setup steps / required values |
| --- | --- |
| Provider | CX = Codex, OC = OpenCode, CL = Claude. 1. Select a product-linked account and pin official harness version. 2. Launch the selected real provider through the built kernel; no SDK, stub or alternate runtime. |
| Client | W = web, T = local TUI, R = remote TUI, B = combined web + local + remote TUI. 1. Attach built real client(s), web with real entry/flags via Playwright, TUI binary via PTY. 2. Drive prompts and owner interactions through that UI. 3. Reconnect and inspect the same task. B rotates prompt/answering clients in both directions and asserts every projection; other cells drive only their selected client, plus the existing browser attachment when pixels are needed. |
| Placement | L = provider on home kernel; R = leased provider on a distinct real worker kernel through the real relay; S = provider in a product-provisioned Docker slice selected by `slice_ref`, attached through home admission. 1. Provision lane-owned topology through product commands. 2. Verify execution and home/worker/run/lease bindings. 3. Keep the user-domain Chromium outside the slice. All 12 PRs affect provider/task paths, so S is relevant to each; include worker-loss subcases on S. |
| Environment | O = ordinary kernel; M = same reviewed runtime on a fresh/fresh-equivalent managed Path-1 host. 1. Pin OSS/Cloud/binary/image identities. 2. Run with normal product launch, compare O/M outcomes after excluding only signed deployment and mandatory shutdown. M requires coordinator-provided authorized infrastructure; this study authorizes no provisioning/deployment. |

Browser/App positive cells target an admitted user-domain resource on its owning
execution kernel; never route leased user-domain control across kernels. Where
that positive leg is unavailable, retain BLOCKED with the exact coordinator
dependency; still execute the cross-kernel denial subcase. Room Browser/Computer
tools use their existing home-owned routing. Local cells of PR10 retain local
control plus a real leased child; its R/S cells exercise remote primary runs.

**Evidence E, required for every cell:** screenshots per web/view step, TUI
screen captures per terminal step, redacted console/kernel/relay/worker/provider
logs and a safe timeline of task/obligation/wait/interaction/run/lease/receipt
IDs; assertions against actual artifacts or first-party service receipts,
commands and exit codes, RED base seam and GREEN candidate results, resource
samples and exact owned cleanup. Never capture passkeys, credential payloads
or private pixels. Private synthetic-canary checkers return booleans/counts;
scan before retaining evidence. A log assertion alone cannot prove UI flow.

**Results, explicitly assigned to every expanded cell:** initial design result
is `BLOCKED(B0)` for all A/I/FA/FB IDs: coordinator must assign implementation lanes
and supply the implemented, built OSS/Cloud candidate and real e2e stack; this
document changes no runtime. Replace B0 per cell at execution with `PASS`,
`FAIL(first seam)` or `BLOCKED(reason, owner, exact action)` and evidence.
Provider login, approved external-service sandbox account, authorized managed
host or named unlanded dependency are actionable blockers; an implementation
bug, missing test, elapsed time or skipped validation is FAIL, never BLOCKED.
All table results below apply separately to every expanded ID, including M.

A PR is done only when every cell is PASS or has an evidence-backed, owner-
actionable BLOCKED disposition and **none is omitted**. BLOCKED is not a feature
PASS or MP acceptance: keep the affected feature gate open, publish the owner's
unblocking action and rerun on the exact final integration. Do not remove `/meta`
or declare runtime acceptance while a replacement's required flow is blocked.
MP-11 additionally requires current semantic reviews of applicable security
anchors: sandbox, Vault/credentials, relay admission/peer gates, signing/trust
pins, App capabilities/keys, KA/sudo/passkey, signal guards and browser
observation protection. Non-security source uses reviewer workflow/CI, not
per-anchor exact-blob review. Review coverage cannot substitute for behavior.
Retain focused fail-first tests; each changed behavior must also show real-path
RED on base and GREEN on candidate. Unchanged security/regression cells may
already pass on base; do not deliberately weaken base to manufacture RED.

### MP-08 / MP-09 / MP-10 / MP-11 — common cells for every PR

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| G01 | 1. Execute S01. 2. Finish/cancel all created work explicitly. 3. Reconnect. Final answer and artifacts agree, no open obligation or dormant task, one authoritative run/state/history. | UI state and zero-open ledger before/after reconnect; artifact/service receipt. | BLOCKED(B0) |
| G02 | 1. Pause at registration, wait settlement and effect-commit boundaries in separate runs. 2. Restart only owned home kernel, then owned worker/provider separately. 3. Reconnect/resume. Durable obligations/cursors/corrections survive; elevation is lost visibly; no duplicate effect/run. | Before/after ledger, restart timeline, all-client state and renewed owner authorization. | BLOCKED(B0) |
| G03 | 1. Disconnect selected client during work and wait, reconnect. 2. Partition owned relay/home-worker path. 3. Wait for lease loss, then restore it. Work continues locally or becomes visibly uncertain/blocked; bounded detection, no split authority. | Disconnection/recovery screens, heartbeat/lease and source-loss timeline. | BLOCKED(B0) |
| G04 | 1. Drop ACK after actual provider acceptance, then after feature effect commit in separate runs. 2. Allow retry/sweep. Exact receipts reconcile once; uncertain effects block automatic replay and surface to user within 2 minutes. | Acceptance/effect receipt, count = 1, uncertainty UI and reconciliation logs. | BLOCKED(B0) |
| G05 | 1. Redeliver identical source occurrence, wake intent, completion and owner response twice. 2. Reconnect and redeliver again. One logical admission/wake/interaction action; no extra provider run or external effect. | Original/deduplicated IDs, service effect count and answer timeline. | BLOCKED(B0) |
| G06 | 1. Queue a privileged call while waiting. 2. Expire elevation; separately revoke elevation, room/resource grant and worker lease before effect. 3. Wake/resume. Each current fence denies stale authority visibly; regular authorized work proceeds. | Expiry/revoke rows and exact before-effect denials, no stale effect. | BLOCKED(B0) |
| G07 | 1. Complete a source concurrently with registration/yield. 2. Race resource creation with done and deletion/cancellation with wait. 3. Race two owner responses/extension requests and stale status delivery. Atomic revisions produce one outcome, no orphan obligation or stale state overwrite. | Barrier/commit order, competing receipts, final UI/ledger and effect counts. | BLOCKED(B0) |
| G08 | 1. Create a second room with public synthetic markers. 2. Ask tested agent to use its IDs/aliases in every exposed read/mutate/subscribe operation. Deny before lookup output, counts, snippets, trace, target discovery or side effects. | Both-room unchanged artifact counts, safe denial and leakage absence. | BLOCKED(B0) |
| G09 | 1. At the real admitted transport boundary, fault sender/source identity and room/kernel/machine/lease binding in separate submissions. 2. Retry with valid identity. Forged requests deny before mutation, valid request still works; relay remains opaque transport. | Boundary fault receipt, admission logs, unchanged foreign resources. | BLOCKED(B0) |
| G10 | 1. Retain a request/completion from the old provider run/lease. 2. Replace run/placement, then deliver the stale request through the real boundary. Reject old tool authority, completion and lineage mutation; current run remains usable. | Run/epoch pairs, denials, current-run UI and ledger. | BLOCKED(B0) |
| G11 | 1. Elevate parent. 2. Spawn child → grandchild and workflow node, send/forward messages and wake them. 3. Try privileged operations, passkey/approval replies and Extend from each descendant. All stay regular; owner-only verification and unrelated peer destruction remain denied. | Status rows for entire chain, denied operations and unchanged peer objects. | BLOCKED(B0) |
| G12 | 1. Exercise passkey, Vault, hand-off and private-overlay synthetic canaries via protected owner/product paths. 2. Try page/process echoes and malicious output. 3. Scan model/tool/history/trace/events/FTS, DOM/screenshots/mirror, logs and retained evidence. No secret is observable/searchable; relay/Cloud receive no plaintext. | Boolean leak report for each sink, protection epochs, safe pixels and bounded malicious output. | BLOCKED(B0) |
| G13 | 1. Start an owned source that stays healthy. 2. Yield with a 60-second deadline. 3. Let it lapse, then repeat with restart before the deadline. One `deadline_reached` wake re-evaluates; new waiting requires a fresh finite deadline. | Countdown screens, durable occurrence/time and real provider re-evaluation. | BLOCKED(B0) |
| G14 | 1. Wait separately on delegate, workflow, process and App source. 2. Delete/fail delegate, cancel workflow, lose process and revoke App. 3. Remove all sources before deadline; also lose one required source while another lives. Immediate failure event wakes/blocks, never eternal waiting. | Per-source cause and bounded wake time, visible failed obligation and resolution. | BLOCKED(B0) |
| G15 | 1. Cause three consecutive wakes with only repeated status/deadline changes. 2. Restart between wakes 2/3. Escalate once to blocked with one user interaction; no fourth autonomous loop. 3. Repeat with a real result between wakes; counter resets. | Counter/progress receipts, blocked notification/screens and negative-loop count. | BLOCKED(B0) |
| G16 | 1. Keep delegate/workflow/watcher/process/hand-off obligations open in separate runs. 2. Cause provider to claim done, then claim done again without resolving them. First claim re-prompts once with safe obligation list, second blocks; no completion feedback. 3. Resolve/cancel and finish successfully. | Correction receipt, purported final shown as progress, second block and terminal zero-open ledger. | BLOCKED(B0) |
| G17 | 1. End with no state, empty/tool-only final and provider EOF in separate runs. 2. Attempt waiting with no sources, no deadline and a past deadline. 3. Wait 15 real minutes without progress. Invalid ends classify/correct/block; long wait notifies every attached terminal and reconnecting client. | State classification, denial/correction screens and timed long-wait notification. | BLOCKED(B0) |
| G18 | 1. Fail the owned durable writer at registration/settlement. 2. Restore it. 3. Wedge a wake delivery and advance past its bound; lose source/deadline scheduling once to exercise sweep. No undurable success, sweep recovers within 30 seconds or delivery blocks within 2 minutes, exact intent retained. | Write failure, sweep/receipt timeline, degraded/blocked projection and recovery. | BLOCKED(B0) |

### MP-08 / MP-10 / MP-11 — A01 room admission and descendant control

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Create ordinary A through selected client. 2. Prompt A to spawn B and C, message pre-existing peer P, create and run a workflow. 3. Receive all answers without `/meta`/task plan; inspect P's public history/trace. | Room tool listing, prompt/result correlations and obligation creation/settlement. | BLOCKED(B0) |
| S02 | 1. Let B spawn D and a workflow. 2. A may edit/cancel B-created workflow/run but cannot rename/delete D. 3. A messages B to rename/delete D; only B may execute it. Direct creator identity admits each effect; waiters get deletion failures. | Immutable creator receipts, A denial, B mutation and message correlation. | BLOCKED(B0) |
| S03 | 1. Try rename/delete against self, parent, sibling C from B, peer P, legacy unknown-lineage agent and other room. 2. Try peer workflow edit/cancel/delete/alias/apply/import replacement, including sudo/raw/native aliases. All deny. | Denials at shared fence and unchanged object versions/runs. | BLOCKED(B0) |
| S04 | 1. A runs P's workflow and observes it. 2. A cancels only its new run; cannot edit P's definition/cancel P's run. 3. Race deletion of B against B spawning D; reject stale spawn or preserve D visibly for user intervention, with no implicit adoption/cascade. | Independent run creators, race revision and complete owned-only cleanup. | BLOCKED(B0) |

### MP-08 / MP-09 / MP-10 / MP-11 — A02 durable wakes and turn enforcement

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Parent delegates, registers completion and yields with source/deadline. 2. Child answers. 3. Parent wakes, handles result and finishes. UI shows working → waiting → working → done with no open obligations. | Timeline, native settlement, ledger and one completion event. | BLOCKED(B0) |
| S02 | 1. Send default message to busy, idle and yielded receivers. 2. Let each finish. Non-urgent waits for next busy turn, idle/yielded wakes; sender gets ordinary answer, receiver sends no courtesy reply. 3. Opt into reply; one correlated reply, no feedback loop. | Inbox/user-backlog separation, reply flags and feedback counts. | BLOCKED(B0) |
| S03 | 1. Send urgent messages to busy, starting, idle/yielded and explicitly stopped receivers. 2. Exercise native steer rejection and uncertainty. Exact-turn steering or later wake/pinned uncertainty follows policy; stopped stays visibly pending. | Real provider steer/acceptance receipts and all-client stopped/pending state. | BLOCKED(B0) |
| S04 | 1. Run two tasks on one agent, one waiting/one complete. 2. Restore after restart with overdue wait and corrupt source receipt. 3. Cancel via owner UI. Older task stays visible; quarantine/block replaces guesses; cancellation settles resources without declaring success. | Per-task roster, recovery/sweep and cancellation disposition. | BLOCKED(B0) |

### MP-08 / MP-09 / MP-10 / MP-11 — A03 process and timer watchers

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Prompt real agent to start an owned CLI process that emits a marker then exits. 2. Register output/exit and a timer with deadlines, yield. 3. Handle each wake and cancel remaining registrations. | Public process handle, real output/exit, timer and cleanup receipts. | BLOCKED(B0) |
| S02 | 1. Run real `gh pr checks --watch` on coordinator-approved read-only PR. 2. Yield on process output/exit. 3. Complete after observed checks. Core parses no GitHub semantics. Missing approved PR is a named owner blocker. | CLI version/exit, public PR identity, safe output and provider continuation. | BLOCKED(B0) |
| S03 | 1. Flood owned stdout/stderr with malicious text/escapes/canary echo. 2. Cancel a resisting child; attempt PID reuse and targets 0/1/-1/undefined/NaN/non-owned PID. Reject unsafe signals, bound tails/matcher, retain actual cleanup proof. | Signal guard denials, start/group ownership checks and resource/tail bounds. | BLOCKED(B0) |
| S04 | 1. Wait on future timer, permit normal managed idle STOP. 2. Normally start again after due time. 3. Reconcile a lost process without relaunch. Due timer coalesces once; MP-09 policy remains intact. O uses ordinary restart as control. | Shutdown/last-turn-finished times, restart wake, `process_lost` and no relaunch. | BLOCKED(B0) |

### MP-08 / MP-10 / MP-11 — A04 hour-scale sudo

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Submit `/sudo` long task and verify owner passkey through selected UI. 2. Confirm default 60-minute expiry; repeat with 2/4/8-hour selections and reject >8 hours. 3. Yield/wake and execute privileged typed operation inside grant. | Kernel duration/monotonic binding, status/countdown on each attached client and successful fenced effect. | BLOCKED(B0) |
| S02 | 1. Click Extend from each client; B answers in another terminal and reverses direction. 2. Wrong passkey, cancelled interaction and stale revision leave deadline unchanged. 3. Fresh verification sets selected duration from now; race two responses. One extension, never silent or banked. | Interaction/extension receipts, pre/post expiry, no passkey in logs. | BLOCKED(B0) |
| S03 | 1. Keep task active for the real 60-minute window, with wait spanning expiry. 2. Observe warning at 10 minutes and expiry on every terminal. 3. Wake, deny queued privileged call, continue regular work and request fresh elevation. No accelerated clock substitutes for this duration proof. | Timestamped warning/expiry/regular progress, countdown reconnect and refusal reason. | BLOCKED(B0) |
| S04 | 1. Revoke, rotate passkey, change placement and restart in separate elevated runs. 2. Try old Extend response/wake. Each invalidates elevation; fresh owner verification is required. 3. Commit effect before expiry and drop ACK; reconcile without replay. | Epoch/placement fences, restart status and exact effect receipt. | BLOCKED(B0) |
| S05 | 1. Attempt approval/passkey/payment resolution via legacy meta tool, raw kernel request, shell/native bridge and delegated agent. 2. Owner answers normally through client. All model paths deny; same kernel interaction settles once. | Shared admission denials, owner response and one effect. | BLOCKED(B0) |

### MP-08 / MP-10 / MP-11 — A05 capability grants

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. User asks regular agent to open/attach trusted App and owning-kernel browser. 2. Agent uses them, user changes focus. 3. Retained authorized use succeeds; revoke then denies. | Prompt causation/grant IDs, real App operation and browser/view change. | BLOCKED(B0) |
| S02 | 1. Try unrelated existing tab/App acquisition and agent focus to self-grant. 2. Try untrusted App/executable key and forged capability. No minting authority or tool publication. | App trust/admission checks and unchanged grant set. | BLOCKED(B0) |
| S03 | 1. Transfer explicit permitted resource subset to child. 2. Attempt unrelated resource or elevation inheritance. 3. Revoke parent binding, inspect child fences. No implicit widening. | Parent/child grant attribution and bounded revoke outcomes. | BLOCKED(B0) |
| S04 | 1. Leased agent uses Room Browser/Computer route. 2. Attempt home user-domain browser from worker; deny. 3. Exercise real native App-agent MCP path and revoke its source while waiting. Missing executable adapter is an explicit coordinator dependency. | Same Room tab/action identity, denied cross-kernel call and source-loss wake. | BLOCKED(B0) |

### MP-08 / MP-10 / MP-11 — A06 Vault generation and login

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Owner elevates real agent and grants first-party browser target. 2. Agent generates opaque credential and registers/logs in via protected fill then observed click. 3. Verify authenticated session outside model. | Handle-only result, protected target view and server-side boolean auth receipt. | BLOCKED(B0) |
| S02 | 1. Drop ACK after generation commit, retry same operation. 2. Lock Vault, then owner unlocks via UI. 3. Revoke during fill wait. Same committed handle, no new secret or stale insertion. | Generation receipt/dedup, unlock interaction and denied insertion. | BLOCKED(B0) |
| S03 | 1. Change origin, document/node or protection epoch after discovery. 2. Try wrong handle scope and expiry mid-wait. 3. Echo synthetic secret in page/errors/pixels. Reject before secret resolution/effect, protection hides every echo. | Target fences, no-effect count and G12 scan per sink. | BLOCKED(B0) |
| S04 | 1. Owner supplies approved public-service account through product paths. 2. Real agent logs in; owner handles service challenge. 3. Reconnect and verify usability. Missing account/challenge action names owner and exact next step. | Redacted real-service auth/session proof and resumed real provider. | BLOCKED(B0) |

### MP-08 / MP-10 / MP-11 — A07 protected hand-off

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Real agent reaches controlled automation-disallowed step, requests owner click/code/secret in separate tasks. 2. Owner acts in scoped view/protected entry. 3. Agent resumes from safe status, settles hand-off obligation and completes. | Blocked interaction on every terminal, one bound action and safe resumed answer. | BLOCKED(B0) |
| S02 | 1. Race two terminals responding; reconnect a third. 2. Replay late response. 3. Navigate/replace target during pending request. One claimed action; stale target cancels/requires fresh request. Single-client cells use two instances of that client. | Interaction revision, claimed response and physical effect count. | BLOCKED(B0) |
| S03 | 1. Let hand-off expire without owner action. 2. Restart with a pending request and revalidate target. 3. Request unsupported opaque region, complete using existing takeover attachment. No auto-answer/replay or second browser. | Timeout/reminder/block state, target revalidation and browser identity. | BLOCKED(B0) |
| S04 | 1. Enter synthetic secret/code; choose save-to-Vault separately. 2. Cause page echo and model/history search. 3. Finish or cancel. Input never becomes context/history/trace/pixels; saved secret adds no read grant. | Handle-only save receipt and full G12 absence report. | BLOCKED(B0) |

### MP-08 / MP-10 / MP-11 — A08 payments

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Elevated real agent prepares first-party checkout. 2. Owner confirms exact payee/amount/cart/action through client. 3. Verify one service receipt; repeat a second payment requiring new confirmation. | Bound interaction details, two distinct confirmations/receipts. | BLOCKED(B0) |
| S02 | 1. Replay confirmation; change amount/payee/origin/document in separate runs. 2. Attempt commit under old confirmation and sudo alone. All deny; unknown payment surface requires owner hand-off. | Binding mismatches, no new charge and hand-off screen. | BLOCKED(B0) |
| S03 | 1. Omit owner response, expire/revoke while pending. 2. Drop ACK after actual commit. 3. Reconcile site receipt. No timeout approval or second click/charge, uncertainty visible. | Pending/expired screens, service count and receipt reconciliation. | BLOCKED(B0) |
| S04 | 1. Use owner-approved real-site sandbox/test account. 2. Prepare, confirm and verify test payment through browser/App. No live-money charge required; missing sandbox/account names exact owner action. | Real sandbox receipt and protected browser/App action trace. | BLOCKED(B0) |

### MP-08 / MP-10 / MP-11 — A09 public history search

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Peer generates public prompt/answer/tool result markers. 2. Agent searches each, opens detail and paginates via client prompt. 3. Restart/reconnect, repeat on leased history. IDs/snippets/detail are authorized and stable. | Search/detail UI, public event IDs and index coverage/cursor. | BLOCKED(B0) |
| S02 | 1. Try foreign-room counts/snippets/detail and guessed event/blob refs. 2. Try missing/ambiguous owner/room scope under sudo. Deny before ranking/snippet generation; no existence leak. | Scoped query fences and boolean foreign-marker absence. | BLOCKED(B0) |
| S03 | 1. Generate private-overlay/Vault/hand-off echoes through protected paths. 2. Search before/after rebuild. 3. Invalidate/delete public rows, search cached and paginated results. No secret/stale token remains searchable. | G12 sink report, invalidation cursor and zero-hit assertions. | BLOCKED(B0) |
| S04 | 1. Interrupt incremental indexing/rebuild, restart. 2. Query excluded legacy unknown-provenance rows and retention gap. 3. Resume sanitized rebuild. Coverage stays explicitly incomplete, no raw-history fallback. | Index version/coverage UI and sanitized rebuild receipt. | BLOCKED(B0) |

### MP-08 / MP-09 / MP-10 / MP-11 — A10 leased wakes and elevation

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Launch real leased delegate through home/worker/relay. 2. Yield/wake/completion and default no-reply feedback. 3. Inspect identical home/worker task/ledger/run correlation. Home alone commits completion. | Both-kernel receipts and one home completion/public answer. | BLOCKED(B0) |
| S02 | 1. Message busy/idle/yielded remote receivers with both urgency modes and explicit reply. 2. Partition after acceptance, replace lease and deliver old completion. No duplicate run, stale completion or reply chatter. | Steer/inbox/receipt timeline, old-lease denial and result counts. | BLOCKED(B0) |
| S03 | 1. Owner authorizes exact leased elevation on admitted execution kernel. 2. Wait, extend freshly, expire/revoke and restart worker/home. 3. Try inherited and cross-kernel browser authority. Both-end fences deny stale grants and delegates stay regular. | Home/worker expiry/epoch checks, renewal screens and domain denial. | BLOCKED(B0) |
| S04 | 1. Repeat on O/M and slice, lose worker process/App source. 2. Allow deadline/sweep. 3. Normally stop/start managed host and resume overdue work. Parity excluding shutdown/deployment only; no hidden dormant task. | Paired normalized results, source-loss/deadline wake and MP-09 timings. | BLOCKED(B0) |

### MP-08 / MP-09 / MP-10 / MP-11 — A11 migration and retirement

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Open real persisted legacy meta tasks/subscriptions and goal history. 2. Drain/migrate cursors idempotently, restart twice. 3. Ordinary agent delegates → yields → wakes → searches → browser → hand-off, sudo does typed Vault/login/payment. | Preserved event/history IDs, regular identity and zero-open final ledger. | BLOCKED(B0) |
| S02 | 1. Attach old/minimum-version web/native/TUI clients. 2. Attempt dependent protected operations. 3. Upgrade/reconnect mixed clients. Correct upgrade diagnostic, no protected input fallback or protocol drift. | Rust/TS snapshot/hash checks and minimum-version screens. | BLOCKED(B0) |
| S03 | 1. Retire obsolete meta/task entry points only after replacement acceptance; keep `/goal` on the hybrid runtime. 2. Exercise old aliases/raw/native bridges. 3. Inspect unknown legacy lineage. No elevation/approval/destructive bypass; legacy history remains readable. | Removed-tool catalog, denial logs and immutable lineage migration receipts. | BLOCKED(B0) |
| S04 | 1. Run full final expected cell manifest and security-anchor review on exact integrated head. 2. Compare ordinary/managed MP-09 activity/shutdown scenarios. 3. Verify every temporary owned resource and evidence leak scan. Any omitted cell, FAIL or unreviewed security anchor holds retirement/MP acceptance. | Complete manifest and current security review, paired results and exact cleanup. | BLOCKED(B0) |

### MP-08 / MP-09 / MP-10 / MP-11 — A12 goal acceptance matrix

| Case | Exact steps and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| S01 | 1. Owner submits `/goal` to build a two-artifact app. 2. Agent ends after first artifact with no live source; goal check says incomplete. 3. Kernel immediately continues, second artifact completes, one bounded completion check confirms done. Ordinary prompt control uses no continuing goal loop. | Original objective, evaluation/continuation receipts, goal row and verified two artifacts. | BLOCKED(B0) |
| S02 | 1. Goal delegates real work, yields with 1-minute check-in interval, source stays live without progress. 2. Check-in wakes ask whether sources progress; verify configurable 10-minute default and 60-minute maximum in separate tasks. 3. Kill required/all sources and restart before due check. No infinite waits, missed intervals coalesce, no-progress guard persists. | Deadline/source-liveness/check-in timeline, all-client wait countdown and restart guard. | BLOCKED(B0) |
| S03 | 1. Try done with each obligation kind still open, then repeat false claim. 2. Timeout/malform evaluation and drop its acceptance ACK separately. 3. Deliver stale evaluation after objective edit. At most one check per work revision, no evaluator recursion or duplicate spend, bounded correction then user block, stale result fenced. | Check counts/origin/60-second bound/8 KiB result, corrected goal row and no recursive turn. | BLOCKED(B0) |
| S04 | 1. Trigger each blocked category: decision/input, 2FA/refusal/automation prohibition, missing account/quota/billing/credential, unanswered approval/expired required sudo, progress guard. 2. Deliver source progress/reconnect while blocked; neither resumes. 3. Owner resolves through protected product UI or cancels. Exactly one revalidated resume/cancel, human input stays protected. | Each category's interaction/notification, owner response receipt and boolean leakage scan. | BLOCKED(B0) |
| S05 | 1. Start `/sudo /goal`, wait through warning/expiry, then Extend with fresh passkey. 2. Pause/resume from each UI, restart home/worker and edit objective. 3. Child/grandchild completion feeds root goal evaluation; delegates stay ordinary, A cannot delete grandchild. Goal survives sudo loss and explicit owner stop; no implicit elevation or goal inheritance. | Goal/sudo rows in every client, elapsed counters/revisions, direct-creator denials and safe reauthorization. | BLOCKED(B0) |
| S06 | 1. Open legacy `/goal` planner-worker-reviewer invocation at pinned base. 2. Migrate once with recovered objective/run/node bindings, restart twice and replay migration ACK. 3. Exercise unknown objective and completed/cancelled runs. Preserve work/history, ask owner to adopt unknown goal, no duplicate nodes or dual scheduler; `/goal`, user templates and `/loop` remain available as specified. | Migration receipts, preserved objective/run/history, node launch counts and old-client diagnostic. | BLOCKED(B0) |

### MP-08 / MP-09 / MP-10 / MP-11 — final integration scenario suite

The top end-to-end scenario is **“/sudo agent builds and deploys a web app with
external services”**, using `/sudo /goal` for durable overall completion. Run
variant A before I01–I03; variant B is a periodic live showcase, not a gate.
Both use the real stack, mixed official providers and protected user-domain
Chromium outside slices. The home kernel remains runtime authority; Cloud's
publication runtime serves the produced app, never proxies agent traffic.

**Owner prerequisites, checked before dispatch:**

| Variant | Owner/coordinator action and blocking seam |
| --- | --- |
| A — controlled acceptance | Supply exact paired built OSS/Cloud and `scripts/e2e-stack` (equivalent isolated stack until available), authorized staging publication target/namespace and public URL, product-linked access to all three providers, enrolled passkey and owner available for approvals/unlock/hand-off/renewal. Supply Stripe test-mode account/access through protected product configuration; no live-money credentials. Provide isolated Mailpit, local OIDC and DB services reachable from the publication runtime. Supply authorized fresh Path-1 host for M cells. Missing staging permission, Stripe access or owner action is an explicit BLOCKED with owner and next step; this plan authorizes no lane deployment. |
| B — live showcase | Owner supplies a dedicated mailbox with credentials in the Vault, an **existing Google account** authorized for Firebase Spark (free) Auth + Firestore + Hosting, access to Chariox Cloud publication, linked provider plans and an agreed usage/time/$ ceiling. Owner remains available for 2FA, phone/human verification, approvals and passkey renewal. Any optional Supabase/Clerk-style service must permit normal email sign-up for this use and stay within the approved free/test scope; missing account/quota/billing is BLOCKED, never an implicit paid upgrade. |

**Matrix expansion:** `FA{01..10}-{provider}-{client}-{placement}-{environment}-
{before-retirement|after-retirement}` and
`FB{01..08}-{provider}-{client}-{placement}-{environment}`, using all 72 axes
above with the selected provider as root and all three providers in the team.
Each FA cell first runs FA01 to reach its seam; each FB cell first runs FB01.
Other required variants in a row get separate asserted subruns. Materialize
every ID with steps, E, extra evidence and explicit result; no omitted cells.
FA must show real-path base RED → candidate GREEN for changed behavior and pass
on both integration heads. FB records PASS/FAIL/BLOCKED per scheduled campaign;
unscheduled cells remain explicit BLOCKED(coordinator, schedule showcase),
never PASS. FA/FB receipts cannot substitute for each other or other MP gates.

#### MP-08 / MP-09 / MP-10 / MP-11 — variant A: controlled acceptance matrix

| Cell | Exact numbered user flow and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| FA01 | 1. Owner submits `/sudo /goal build and deploy a web app with auth, a DB and external services`, verifies the default window and grants the owning-kernel browser. 2. Root delegates frontend to Claude, backend/infra to Codex and review/test to OpenCode (rotate roles with root axis); all delegates stay regular. 3. Build sign-up/login and DB create/read/update; use Mailpit verification and local OIDC. 4. Complete owner-confirmed Stripe test checkout, deploy to Chariox Cloud staging publication runtime and verify the served app in real Chromium. 5. Start a finite post-deploy URL watcher, consume its result, settle all work and finish done. | Source/artifact digest, mixed-team lineage/results, verified mail/OIDC/DB/test-payment receipts, publication receipt/live URL and final zero-open room ledger. | BLOCKED(B0) |
| FA02 | 1. Generate sign-up password with `vault.generate`, retry after lost generation ACK. 2. Kernel injects login/sign-up fields, agent clicks observed action; receive verification in Mailpit through browser UI. 3. Controlled verification challenge requires protected owner hand-off, then resume once. | Same opaque handle/receipt, one account and delivered mail, protected target/hand-off outcome, boolean full-sink absence scan. | BLOCKED(B0) |
| FA03 | 1. Sign in through local OIDC redirect/popup and callback. 2. Create/update DB record in deployed UI, logout, login and read it again. 3. Attempt another user's record and wrong-origin secret fill. Deny both without leaked credential or foreign data. | Stable tab/callback, authenticated identity and DB persistence assertions, authorization denials and no-effect checks. | BLOCKED(B0) |
| FA04 | 1. Prepare Stripe test checkout in app. 2. Withhold owner confirmation; sudo alone cannot commit. 3. Owner confirms bound amount/payee/cart/action, drop ACK after test effect and reconcile; replay/change details rejects. | Test-mode assertion, one confirmation/charge receipt, safe uncertainty UI, no duplicate charge or live-money effect. | BLOCKED(B0) |
| FA05 | 1. Hold the real publication acknowledgement during deploy wait. 2. Restart only owned home kernel, reconnect real clients and revalidate retained goal/waits/receipts. 3. Owner freshly reauthorizes after fail-closed sudo loss; reconcile publication and continue once. | Restart/fault seam, same goal/obligations, fresh authorization, one publication/artifact and provider continuation receipts. | BLOCKED(B0) |
| FA06 | 1. Keep real task running across the default 60-minute sudo window; capture 10-minute warning and expiry during privileged work still needed. 2. Queued privileged step denies and becomes visibly blocked, regular work remains admitted. 3. Owner extends/reauthorizes with fresh passkey through selected client (cross-client in B), resume and deploy once. | Real elapsed/warning/expiry timeline, shared goal/sudo rows, denial and new window revision, owner response and single publish. | BLOCKED(B0) |
| FA07 | 1. Root claims done while delegate, deploy wait and post-deploy watcher obligations remain open. 2. Bounded completion check lists them; one correction selects valid wait/continue, completes actual work and reaches done. 3. Separate subrun repeats false done; block visibly and require owner resume before finishing. | Purported answer recorded as progress, check/correction counts, blocked interaction, verified final app and zero open obligations. | BLOCKED(B0) |
| FA08 | 1. Fail a real delegate run before its artifact is delivered. 2. Parent receives correlated failure, handles it and replaces/reassigns explicitly without duplicate launch; missing user resource requires blocked interaction. 3. Complete failed work and review before goal done. | First delegate failure/provider seam, parent wake and handled failure, replacement lineage/results, no dormant agent. | BLOCKED(B0) |
| FA09 | 1. Post-deploy watcher observes live URL, auth and DB behavior. 2. Inject unhealthy response then recovery; yield with finite check-ins, show long-wait notification and restart recovery. 3. Cancel/settle watcher explicitly before done. Test ordinary/managed activity and normal shutdown/start without a new Cloud wake scheduler. | Live Chromium assertions plus watcher output/exit, source/deadline/check-in timeline, O/M activity and MP-09 receipts, cleanup. | BLOCKED(B0) |
| FA10 | 1. Combine deploy-wait restart, sudo expiry/extension, open-obligation done claim and delegate failure in one FA01 run at recorded barriers. 2. Resolve only required owner interactions, verify live app and payment/mail/DB receipts. 3. Finish root done and every delegate explicitly done/cancelled, zero open obligations or dormant/unclassified agents; remove only owned runtime/publication/fixture resources. | Complete fault/recovery timeline, real-client screenshots/logs per step, exact heads, one-effect counts, final goal/room state and verified cleanup. | BLOCKED(B0) |

#### MP-08 / MP-09 / MP-10 / MP-11 — variant B: live showcase matrix (non-gating)

| Cell | Exact numbered user flow and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| FB01 | 1. Owner submits the same flagship `/sudo /goal` prompt with approved service scope and budget. 2. Mixed team builds auth/data app, uses dedicated mailbox and existing Google/Firebase account, publishes to Firebase Hosting on Spark and Chariox Cloud. 3. Watch both live URLs, independently verify auth/data round trip and settle work before done. | Two deployment receipts/URLs/artifact bindings, real browser flow, team and goal ledger, measured success. | BLOCKED(B0) |
| FB02 | 1. Kernel injects dedicated mailbox login from Vault. 2. Agent retrieves verification through browser UI; owner handles 2FA/human checks via protected interaction. 3. Resume from safe status only and verify mail outcome. | Mailbox access/outcome without message challenges or credentials in evidence, hand-off reason/count/duration and leak scan. | BLOCKED(B0) |
| FB03 | 1. Use existing Google account; never automate Google account creation. 2. Configure Spark Auth + Firestore + Hosting through permitted flows, owner handles account/phone/verification steps. 3. Prove auth and persisted data on hosted app; quota/billing requirement blocks, no automatic upgrade. | Approved account/project metadata, Spark plan assertion, hosted auth/data receipts, protected hand-offs and any exact blocker. | BLOCKED(B0) |
| FB04 | 1. If owner elects Supabase/Clerk-style service, record allowed normal email sign-up scope before starting. 2. Generate Vault password, inject and verify via mailbox; human/refusal/automation-restricted step hands off. 3. Verify permitted service integration. Without an elected service record BLOCKED(owner, select/authorize optional service), not invented success. | Scope decision, opaque generation receipt, verification/integration assertion and safe hand-off history. | BLOCKED(B0) |
| FB05 | 1. Encounter verification, model refusal, disallowed automation or missing user resource. 2. Surface owner-only blocked interaction across clients; never evade refusal or create replacement identity. 3. Owner completes permitted isolated step/provides resource and explicitly resumes, or cancels with honest unsuccessful result. | Reason/notification, protected action and single resume/cancel, service outcome and hand-off totals. | BLOCKED(B0) |
| FB06 | 1. Span a long build/deploy wait with goal check-ins and expiry notice. 2. Owner freshly extends/reauthorizes when needed; reconnect to retained state. 3. Verify publication on both targets and settle post-deploy watchers; no dormant agent or implicit renewal. | Real timing, sudo/goal UI, deployment reconciliation and final zero-open ledger. | BLOCKED(B0) |
| FB07 | 1. Freeze success rubric and limits before prompt. 2. Measure independent task success, total/active/wait wall time, tokens and API-equivalent $, hand-off count/reasons/time, agents/providers and delegation count/tree. 3. Include failed/blocked attempts, unknown usage and plan throttling; report no cost as zero merely because it ran on a plan. | Per-attempt metrics/price snapshot and coverage, grader assertions, failure seam and hand-off/delegation ledger. | BLOCKED(B0) |
| FB08 | 1. Run private evidence absence checks before retention. 2. Remove only campaign-created test data/deployments/watchers and revoke temporary grants; retain owner mailbox/Google identity, Vault entries and linked credentials. 3. Record final product state, resource recovery and cleanup. | Boolean scan, exact owned cleanup/retention list, service receipt and final resource samples. | BLOCKED(B0) |

Run I01–I03 on the exact final paired OSS/Cloud build, after PRs 1–10 + 12, then
again after retirement. IDs are `I{01..03}-{provider}-{client}-{placement}-
{environment}` over every axis above (72 cells per scenario per integration
phase). Steps/evidence/results expand exactly like A cells. Mix all three
providers in delegation chains, rotating the selected provider as root;
never use simulated time for hour/multi-hour duration acceptance.

| Scenario | Exact numbered user flow and expected behavior | Expected evidence beyond E | Result |
| --- | --- | --- | --- |
| I01 | 1. Owner submits one `/sudo /goal build this web app and publish it on Vercel` prompt, selects default 60 minutes and verifies passkey. If no approved account, name an equivalent owner-controlled first-party deploy target in the manifest before starting. 2. Root delegates frontend/test/package work across real providers; delegates stay regular, root retains privileged publish step. 3. Include a real ≥15-minute external build/check wait with finite deadline and long-wait notification; total task lasts >60 minutes. 4. Observe 10-minute warning, Extend from another attached client where B applies, freshly verify to 2 hours. 5. Wake, publish via permitted protected target, verify served app in real Chromium plus deploy receipt. 6. Complete all delegates/workflows/watchers/processes/hand-offs, finish with zero open obligations and no waiting/blocked/unclassified agent. No second user task prompt or silent renewal. | Timeline with original prompt/passkey receipt, real elapsed duration, warning/extension, chain lineage/regular child status, per-step UI, deployed URL/artifact digest and final room/ledger. | BLOCKED(B0) |
| I02 | 1. Owner starts `/goal` on A for a real ≥3-hour task; A → B → C chain uses all providers and owned workflow/process/timer waits. 2. Restart owned home at hour 1 and worker at hour 2; partition/reconnect relay once. 3. Sweep overdue/dead sources and reauthorize root sudo via fresh owner passkey when lost; none inherited. 4. A requests B to settle/delete its own child C; A cannot destructively act on C. In a separate branch lose B with C alive, show owner intervention and no inherited rights. 5. Resume retained chain/results, settle every obligation and verify actual final artifacts, all agents explicitly done/cancelled. Failure/blocked cases notify user and resume only on owner action. | Monotonic ≥3-hour run, both restart/partition records, lineage and correction counters across restart, deadline/source-loss wakes, reauthorization UI, final artifact/zero-open ledger. | BLOCKED(B0) |
| I03 | 1. Owner prompts a real agent to finish a browser task requiring a protected human code/secret step. 2. Agent requests hand-off, becomes blocked on every terminal, retains its task/obligation and finite interaction expiry. 3. Disconnect/reconnect client; restart owned kernel while pending, revalidate target; verify restart removes sudo. 4. Owner supplies protected input through same bound interaction (fresh request if stale), then explicitly reauthorizes if needed. 5. Agent resumes once from safe outcome, finishes target operation and final answer, no open obligations or private input in context/history/pixels/logs/index. | Before/after blocked/hand-off screens, target generation, one action/resume receipt, real service outcome and boolean full-sink scan. | BLOCKED(B0) |

## MP-08 / MP-09 / MP-10 / MP-11 — Evals and optimization (after overhaul, non-gating)

Start scored runs after PRs 1–12 and controlled acceptance pass; freeze an
accepted reference build first. This phase does not gate their merge or MP
closure. Aim to establish, through measured comparisons, Chariox's accuracy
versus **API-equivalent $, tokens and wall time Pareto frontier** for multi-agent
and long-running work. No claim of “best” without comparable public evidence.

Baselines are each official Codex, Claude and OpenCode harness alone. Compare
Chariox planner + workers, mixed Claude/Codex teams, strong planner + cheap
workers, verifier loop and best-of-n. Pair configurations on identical tasks,
versions, seeds, tools, budgets and graders; separate unequal model/tool/access
conditions. Best-of-n pays for every candidate and selection. Use efficient-
benchmarking subset selection from a disjoint pilot/development set, freeze
selection and report uncertainty; optimize without tuning on held-out answers.

| Eval order / workload | Overhaul features exercised (PR numbers above) |
| --- | --- |
| 1 — SWE-bench Verified Mini (50-task pilot) | Room delegation/direct-creator isolation (1), correlated results/wakes (2/10), process/test watchers (3), public history (9), goal checks and verifier completion (12). |
| 1 — Terminal-Bench 2.0 (89-task pilot) | Ordinary/leased/slice tools (1/10), process ownership/cancellation and timers (3), durable waiting/progress guards (2), bounded goal continuation (12). |
| 2 — GAIA validation subset | Mixed-team coordination (1/2/10), admitted browser/App resources (5), protected Vault/hand-off when task allows (6/7), public history (9), original-objective completion (12). |
| 2 — tau2-bench airline | No-reply/urgency/correlated teamwork (1/2), admitted actions and explicit human approvals where supported (5/7/8), history (9), goal obligations/checks (12). Benchmark tool simulators remain grader targets, not provider runtimes. |
| 2 — Long-Horizon Terminal-Bench | Durable goals, deadline/check-in/no-progress guards (2/12), watchers (3), hour-scale sudo/fresh owner extension where allowed (4), leased recovery (10), history/migration (9/11). |
| Open Chariox multi-agent suite: parallel bundles | Public independent task bundles/graders; fan-out/results, direct-child rights and cancellation (1/2), watchers (3), leased parity (10), overall completion (12). |
| Open suite: mixed difficulty | Public tasks with known grading; planner/worker assignment, mixed/cheap teams, verifier and best-of-n; room authority/results (1/2), history (9), leases (10), completion checks (12). |
| Open suite: long-horizon build+deploy with restarts | Public tasks/graders and first-party services; flagship FA flow, goals/wakes/watchers (2/3/12), sudo (4), capability/Vault/hand-off/payment (5–8), history (9), leased restart and migration (10/11). |

Pin maintainers' task/rule versions and licenses before each pilot; preserve
official graders and distinguish subset/local reproduction from official
scores. Publish the open suite's task provenance, three track definitions,
grader code and reproducible runs without secrets. Mark a feature unexercised
where benchmark rules prohibit it; these mappings are intended coverage, not
proof that every public task supports every feature.

Run on linked provider **plans within usage windows**. Claude/OpenCode execute
through Chariox leased agents: Chariox transfers selected provider access via
the normal lease; never use builder logins, SDKs or copied owner credentials.
Codex also uses the official harness/product-linked path. Record account role
without auth data, harness/model/effort, rate/usage limits and throttled waits.
Owner prereqs: linked plans/leases, accepted binaries, task environments and
graders, approved compute/time/API-equivalent budget and any human actions.
Public submissions require explicit owner OK; this plan grants none.

Per task/config/seed retain success/accuracy, all input/output/cache/reasoning
tokens exposed by the harness, total/active/wait wall time, retries/checks,
delegation/hand-off counts, source/binary/model identities, exact commands and
exit codes, grader output, protected traces and owned cleanup. Price actual
model usage at a pinned dated official API price snapshot even on plans;
report actual plan/compute spend separately. Unavailable token/price fields are
unknown with coverage bounds, never zero or an exact $ claim. Include failures,
rate limits and all agents/evaluators in totals. Plot non-dominated accuracy/$,
accuracy/tokens and accuracy/time configurations with paired uncertainty;
retain raw records and budget cutoff. Each optimization reruns affected real-
path functional/security/recovery gates before adding a frontier point.

## MP-08 / MP-09 / MP-11 — owner decisions and remaining proposal

Round 2 resolves the old questions: default 60-minute sudo with fresh extension,
fail-closed restart, no inheritance; destructive agent/workflow actions limited
to immutable direct creator identity. The 11:45/11:48 UTC inbox decisions
retain `/goal` as an explicit hybrid continuation loop and limit destructive
control to direct children; they supersede earlier proposals. All proposed
policy constants remain **owner-ratifiable before implementation**, including
the approved 60-minute default direction:

| Constant | Proposed value, subject to owner ratification |
| --- | --- |
| Sudo default | 60 minutes |
| Sudo maximum per fresh verification | 8 hours |
| No-progress guard | N = 3 consecutive wakes/continuations |
| Kernel sweep | 30 seconds |
| Goal check-in | 10 minutes, configurable 1–60 minutes |
| Long-wait notice | 15 minutes without progress |
| Uncertain delivery escalation | 2 minutes |

Coordinator/owner may ratify or amend those values; record any amendment without
weakening finite deadlines, visible escalation or fresh passkey extension.
No owner decision blocks completion of this documentation assignment.
