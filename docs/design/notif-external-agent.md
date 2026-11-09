# Notification subscriptions for the owner's outside agents (notif-external-agent)

Status: design note, no product code (lane notifext round 1, MP-08 / MP-09 / MP-10 / MP-11).
Base: union OSS head `ebfb28d9f2` (local protocol 999, relay peer 438). Rows: ON-17, ON-18,
and AT-40 on the reach surface.

Every statement under "Current state" cites the code at that head. Everything under
"Design" is a proposal, not behaviour.

## 0. Summary

- The outside agent already has a durable, cursor-based, no-polling event stream: the
  grant's *event part* (`SubscribeEvents` / `WaitEvents` / `AckEvents`). It is keyed by the
  grant id and survives a kernel restart while the grant's holder lives.
- The Notification trigger (#1275) already routes workflow-exit and owner-action
  occurrences to subscribers with durable receipts, a 7-day deadline, dedupe, ancestry loop
  drops and a per-source fan-out cap. Its subscribers today are workflow entry points and
  agents.
- The design adds one subscriber type, **outside agent grant**, whose "queue" is the grant's
  own event part. A delivered occurrence becomes one `notification` event in that part, in
  the same SQLite transaction that settles its receipt. Subscribe is a new `sources` field
  on the existing `SubscribeEvents` request, so there is one subscription per grant and one
  way to read it.
- AT-40 already holds on the reach surface for decisions raised on the same kernel
  (section 1.3). What is not shown is the live cross-kernel case, and the tracker says so.
- One existing behaviour must change for ON-17's restart replay: grant-bound notification
  subscriptions are retired on every kernel start (`runtime/state/mod.rs:1263`), while the
  grant's event part is kept for a surviving holder (`runtime/state/mod.rs:889`).
- Protocol: local 999 to **1000**. The peer pair stays **438** for slices S1 to S4, and
  I expect it to stay there for remote sources too (section 2.10). The web feature minimum
  does not change.

## 1. Current state (read-out)

All paths are under `apps/kernel/src/` unless they start with `apps/` or `docs/`.

### 1.1 How a grant is authenticated and scoped

- A grant is an OS-verified Unix-socket peer holding a local-kernel access grant. Its
  connection class is `KernelConnectionClass::ExternalAgent`
  (`local/api/types/kernel_access.rs:7-27`). The Unix transport admits frames in
  `admit_frame` (`runtime_transport/unix_access.rs:6`); a `/sudo` window of the same peer is
  checked first (`unix_access.rs:56-110`).
- A grant's public metadata is `KernelAccessGrant {grant_id, owner_user_id, holder_pid,
  holder_executable, holder_harness, term, expires_at_ms, covered}`
  (`local/api/types/kernel_access.rs:221`). The id is "a revoke handle, never an access
  credential". The term is `Minutes(n)` or `UntilHolderExits`; the second "outlives a kernel
  restart while its holder runs; a timed grant does not" (`local/api/types/kernel_access.rs:47-54`).
- Scope is a deny-list, not an allow-list: `authorize_external_request` refuses
  `restricted_request` (passkey, Vault, pairing, relay config, credentials) and
  terminal-only interaction answers (`runtime/state/kernel_access/policy.rs:40-62`,
  `policy.rs:283`). Notification and event requests are not on the list.
- A grant acts for its owner: `event_source_owner(id)` returns the grant's `owner_user_id`
  (`runtime/state/kernel_access/reach.rs:311`); `event_source_alive` is "the grant still
  lives" (`reach.rs:319`).
- Event requests are routed before the ordinary router: `dispatch_authorized` requires
  `ExternalAgent`, resolves `access_grant_summary(caller_id)` ("grant revoked or expired"
  otherwise) and calls `KernelGateway::event_request`
  (`runtime/router/dispatch.rs:120-143`).
- A Room agent's *reach* identity is different: `reach:<run_id>`, a live provider run, never
  a token the agent holds (`runtime/state/kernel_access/reach.rs:1-22`). It uses the same
  event requests at need `ReachNeed::Room` (`reach.rs:38-46`) and the same event inbox keyed
  by the reach id (`reach.rs:311-316`). Ended reach runs end their subscription
  (`reach.rs:325`).

### 1.2 The event stream a grant already has

- Requests: `SubscribeEvents{targets}`, `UnsubscribeEvents{targets|all}`,
  `GetEventSubscription`, `ListEvents{since,limit}`, `WaitEvents{since,timeout_ms,limit}`,
  `AckEvents{cursor}` (`local/api/types/outside_events.rs:13-60`). CLI: `chariox events
  subscribe|unsubscribe|subscription|list|wait|watch|ack` (`apps/cli/src/events-command.ts`);
  an MCP server for harnesses is `apps/cli/src/events-mcp.ts`.
- A target is `{kernel, session_id, agent_id?}`. A target must be on this kernel or a paired
  one, in a session the owner is a Room member of, not ended
  (`runtime/state/outside_events.rs:42-80`, `runtime/kernel_gateway/events.rs:28-72`).
- Storage: one `Part` per id in `agent_outside_subscriptions`, whose rows are in the shared
  `agent_inbox` table with `room_id=''` and `agent_id=<grant id>`
  (`durable_state/agent_lifecycle/outside.rs:34-47`, `outside.rs:78-96`,
  `outside/model.rs:30-70`). `UNIQUE(room_id,agent_id,source_id,occurrence_id)`
  (`durable_state/agent_lifecycle.rs:97`) makes an append idempotent per occurrence.
- Cursor: the inbox sequence. `ListEvents`/`WaitEvents` return rows above `since`
  (`outside_events.rs:131-158`); `AckEvents` moves `ack`/`floor` and deletes rows at or below
  the cursor (`outside.rs:340-352`). An expired cursor answers `EventsCursorExpired`
  (`outside_events.rs:150-158`). So delivery is **at least once until acknowledged**; an
  unacknowledged row is returned again.
- A wait is one held request: `outside_wait` sleeps 100 ms in the kernel between checks, up
  to 900 s, and ends with a `SubscriptionEnded` receipt if the grant ended
  (`outside_events.rs:160-230`). The client does not poll.
- Retention: count based. `PENDING_LIMIT` is 100,000 rows per part
  (`outside.rs:14`); a full inbox is trimmed with headroom and replaced by one `Gap` event
  per source kernel (`outside/retention.rs:14-150`). An ended part keeps its rows and its
  ending receipt 24 hours (`retention.rs:7`). There is **no 7-day time bound** here.
- Redaction: every published event passes `project`, which redacts secrets
  (`outside.rs:543-548`).
- Producers (all publish into matching parts): agent run state, interactions (pending and
  resolved), terminal exits, steer delivery, process state, session state
  (`outside_events.rs:232-266`; `runtime_interaction_owned_state/registration.rs:192,229,408,452`;
  `runtime_interaction_owned_state.rs:571,844`; `agent_lifetime.rs:360`; `maintenance.rs:93`).
  A decision the holder may answer is projected as a `question`; one only the owner's
  terminal may answer is a `pointer` (`outside_events.rs:273-330`).
- Restart: `Mutation::Restart{keep}` ends every home part not in `keep`
  (`outside.rs:355-366`); the keep list is the grants whose holder is not confirmed gone
  (`runtime/state/kernel_access.rs:1073-1083`, applied at `runtime/state/mod.rs:886-894`).
  Pumps for other kernels' parts resume at start (`kernel_gateway/events/parts.rs:5-24`).
- Revocation: a grant that ends ends its part (`reason` such as `grant_ended`); a wait on an
  ended part returns `SubscriptionEnded` (`outside_events.rs:172-186`, `outside.rs:106-135`).
  Tests: `outside_router_collects_states_and_ends_a_running_wait_on_revoke`,
  `outside_event_requests_are_refused_once_the_grant_is_revoked`
  (`runtime/router/tests/outside_events.rs:50,173`).

### 1.3 AT-40 on the reach surface: does it hold?

AT-40 reads: a driver (outside agent) is woken by a driven agent's `RuntimeInteraction`
card through the event channel, without polling.

What holds, with evidence:

- Producer: a pending `RuntimeInteraction` publishes a `pending` interaction event to every
  part following that session or agent, with its question or pointer
  (`runtime_interaction_owned_state/registration.rs:192-229`, `outside_events.rs:273-330`).
  A decision already open when a holder starts following is replayed once as its `pending`
  event (`outside.rs:374`, `replay_open_decisions`).
- Grant surface, same kernel: tests that a grant's part receives the question, the pointer,
  the close and a second answer's `interaction_closed`
  (`runtime/state/kernel_access/test_support/choice_events.rs:192,243,278`).
- Reach surface, same kernel: `a_driver_is_woken_by_the_decision_a_driven_agent_raises_and_answers_it`
  (`transport/mcp_server/tests/reach.rs:950`) follows a session with `events_subscribe`,
  waits with `events_wait`, receives the `pending` card, answers it with
  `RespondToInteraction`, and asserts that only the followed room reports.
- Live: `scripts/drills/agent-reach.mjs` scenario `driver` (header, line 12) runs it with real
  Claude, Codex and OpenCode agents on two kernels.

What is **not** established:

- The tracker row `agent-triggers/AT-40` has the note "[2026-11-05T16:53Z latest round: NOT
  RUN] ... same-kernel blocked-task source alone does not establish this" and coverage
  `cross-kernel: NOT RUN`, `guest: NOT RUN`, `shaped-network-soak: NOT RUN`. I read this as:
  the mechanism is in the code and unit-tested; the cross-kernel live run on the union head is
  open.
- A cross-kernel unit test for an *interaction* crossing a gateway pump does not exist; the
  gateway event tests cover health, refusal, busy kernel, cleanup and lost acknowledgements
  (`runtime/kernel_gateway/events/tests.rs:6,269,385,482,627,662,702,738,766`).
- Whether a reviewer-exit-like *workflow* occurrence reaches a grant: it does not today
  (section 1.5). AT-40 is about decision cards, which do reach it.

Confirming runs on this head (`ebfb28d9f2`, through the build slot, with
`RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1`; a first run of the reach test without those two
variables aborted with a stack overflow, which is the harness setting AGENTS.md names, not the
test):

- `transport::mcp_server::tests::reach::a_driver_is_woken_by_the_decision_a_driven_agent_raises_and_answers_it`: ok.
- `runtime::state::kernel_access::test_support::choice_events` (4 tests): ok.
- `runtime::state::workflow_notification_router::tests::external_grant_notification_subscription_ends_with_its_grant`: ok.

These are unit/in-process tests, not live acceptance. Verdict: **AT-40 holds on the reach and
grant surfaces on one kernel at unit level; the live cross-kernel cell stays open.**

### 1.4 The Notification trigger and its subscriber types (#1275)

Spec: `docs/EVENT_TRIGGER_PROTOCOL.md` ("Private workflow notifications", "Blocked-task
decision sources"). Code:

- **Sources.** `NotificationSourceKind::{WorkflowExit, OwnerAction}`
  (`local/api/types/workflow_notification.rs:138`). A workflow exit is registered with
  `RegisterWorkflowNotificationSource` (`runtime/state/workflow_notification_router.rs:111`).
  Each Room has one `owner_action` source "Agent needs your action", per Room owner, with a
  hashed id (`runtime/state/owner_action_notifications.rs:9-46`); its occurrence is one per
  blocked-task interaction id (`owner_action_notifications.rs:48-78`).
  Tables: `workflow_notification_exit_sources`, `workflow_notification_occurrences`,
  `workflow_notification_grants` (`durable_state/workflow_notifications.rs:100-122`).
- **Subscribers.** `WorkflowNotificationTargetKind::{WorkflowEndpoint, Agent}`
  (`workflow_notification.rs:130`). A subscription is an `app_automations` row with
  `source_kind='workflow_completion'` and the subscription JSON in `notification_json`
  (`workflow_notifications.rs:660-687`). App events use the same table with another
  `source_kind`; their agent target is "not yet built" in the spec.
- **Occurrence delivery.** The source emits one receipt per active subscription in
  `app_outbox` (`insert_receipt`, `workflow_notifications.rs:689-722`). The router pump
  `route_workflow_notifications` (`workflow_notification_router.rs:405`) sweeps, then handles
  `retryable` receipts (accept) and `accepted` receipts (queue), at most 8 per pass, rotating
  each with a one-second `next_attempt_at` before it tries (`router.rs:445-470`). Local
  workflow targets are queued with one atomic `Queue` operation; local agent targets go
  through `accept_local_agent_notification` and then the async agent step
  (`router.rs:458-465`, `agent_trigger_notification.rs:496-560`).
- **Retention.** One constant, `NOTIFICATION_RETENTION_MS` = 7 days
  (`workflow_notification.rs:5`). `deadline_ms` is emit time plus that; `accept` rejects an
  expired envelope with `Expired` and one beyond now+7d+5min as invalid
  (`workflow_notifications.rs:740-757`). An agent's prompt keeps the original deadline
  (`EVENT_TRIGGER_PROTOCOL.md`, last paragraph).
- **Dedupe.** The receipt key is (owner, source, subscription, occurrence); a replay with
  the same digest is `Duplicate`, with another content `notification occurrence conflict`
  (`workflow_notifications.rs:768-790`). Diagnostic rows keep the last 4,096 occurrences
  (`MAX_OCCURRENCES`, `workflow_notifications.rs:50,127`).
- **Loop rules.** `ancestry_identity(sub)` is `agent:<id>` for an agent and a JSON tuple
  `[kernel, session, workflow]` for a workflow endpoint (`workflow_notifications.rs:85-100`).
  `accept` drops an occurrence whose ancestry already holds the subscriber's identity:
  diagnostic `workflow_notification_loop_dropped`, receipt `failed`, ack `LoopDropped`
  (`workflow_notifications.rs:758-766`); more than 256 ancestry entries is invalid
  (`:752-755`). Attach refuses an exit for an entry point of its own workflow and any
  recursive cycle among the owner's subscriptions (`workflow_notification_router.rs:198`,
  `workflow_notifications.rs:457`). An agent invoking a workflow while its active prompt
  came from a notification passes its ancestry plus `agent:<id>` to the run
  (`runtime/state/agent_notification_ancestry.rs:6-52`).
- **Admission fence and Steer lineage.** For an agent target, a receipt that already names
  its prompt is "admitted durably"; whether that prompt is queued, ran, was steered or was
  cancelled, the occurrence is delivered once (`agent_trigger_notification.rs:543-552`).
  `ReserveAgentLineage` commits the receipt's lineage with the admission
  (`workflow_notifications.rs:858-870`); `BindAgentSteer` binds a manual Steer to the
  notification prompt (`:872-887`, `prompt_transcript_owned_state.rs:1149`). An uncertain
  writer commit fences further transitions until restart (spec, "Delivery").
- **Limits.** 32 subscribers per source (`MAX_SUBSCRIPTIONS`), 1,024 active subscriptions
  per owner, 1,024 sources per owner, 1,024 pending receipts per subscription
  (`MAX_PENDING`), 64 KiB payload with 8 KiB reserved for the envelope
  (`workflow_notifications.rs:29-52,660-687,689-700`).
- **Peers.** Remote delivery uses the relay peer channel
  (`runtime/state/workflow_notification_peers.rs`, peer 437/438).

### 1.5 What a grant can do with notifications today

- A grant can already call `ListWorkflowNotifications`, `RegisterWorkflowNotificationSource`
  and `AttachWorkflowNotification`: they are not restricted requests (section 1.1); the only
  extra gate is the cloud-profile owner check (`workflow_notification_peers.rs:14`). The
  router sees it as `shared_access = self.notification_grant().is_some()`
  (`workflow_notification_router.rs:104`, `workflow_notification_grants.rs:9-15`).
- Under shared access the listing is limited to this kernel's sources and subscriptions
  (`router.rs:255-267`) and `attachable_source` skips the cached sources of other kernels
  (`router.rs:326-340`); attaching a remote source is refused with "shared access cannot
  attach remote workflow notifications" (`router.rs:173`).
- But the subscriber a grant can attach is a **workflow entry point** the owner has, never the
  grant itself. `NotificationOperation::Granted{grant, attach}` records the binding in
  `workflow_notification_grants` (`workflow_notifications.rs:181,511-521`).
- Such a binding never outlives its grant: revoke, expiry and holder exit call
  `retire_notification_grants(Some(grant))` (`runtime/state/kernel_access.rs:365,460`;
  `workflow_notification_grants.rs:33-84`), delivery re-checks the grant through
  `with_live_notification_grant` (`workflow_notification_grants.rs:87-112`), and **kernel
  start retires every one** (`runtime/state/mod.rs:1262-1263`, comment "Access grants are
  process-bound and never survive a kernel restart"). The last point disagrees with
  `UntilHolderExits` and with `grant_inboxes_to_keep`, which keeps the event part of a
  surviving holder (`kernel_access.rs:1073`).
- Tests for the existing grant binding:
  `external_grant_notification_subscription_ends_with_its_grant` and
  `revoked_grant_fences_an_already_selected_accepted_notification`
  (`workflow_notification_router.rs:898,1010`).

### 1.6 Gap, in one line

A grant can read decision cards from its event part and can attach *workflow* subscribers, but
no workflow exit or owner-action occurrence can reach the grant as an event. Today the outside
agent polls (reviewer-wait, lane-wait).

## 2. Design

### 2.1 Principles

1. The kernel holds the subscription; the grant only reads its event part. No new stream.
2. The existing Notification trigger stays the only router: source, receipt, deadline, dedupe,
   loop drop, fan-out cap. The new subscriber type is a third `WorkflowNotificationTargetKind`,
   not a parallel pipeline (AGENTS.md: one job per module, no parallel paths).
3. The grant's event part stays the only delivery channel: cursor, ack, gap, replay, end
   receipt are reused as they are.
4. Only the grant's own owner, only the grant itself, only this kernel's sources in v1.

### 2.2 Data model

One new `WorkflowNotificationTargetKind::OutsideAgent` (snake_case `outside_agent`). A
subscription is the same `app_automations` row (`source_kind='workflow_completion'`) with:

| field | value |
|---|---|
| `subscription_id` | `wf_subscription_<random>` as today |
| `source_id` | the followed source |
| `owner_user_id` | the grant's `owner_user_id` (also `created_by_user_id`) |
| `target_kernel_id` | this kernel |
| `target_kind` | `outside_agent` |
| `endpoint_id`, `publication_id` | the grant id (the unique key already includes the publication id, so a second subscribe of the same source by the same grant is the existing idempotent `replaces_active` case, `workflow_notifications.rs:664-682`) |
| `session_id` | the source's Room, so a Room delete or end is visible to the liveness check |
| `queue_id` | constant `grant` (like the constant `agent` for agent targets) |
| `agent_id`, `trigger_id` | none |

Keyed by: kernel (the table is per kernel), owner, source, grant id. The Room is not a key
beyond `session_id` above. The binding row in `workflow_notification_grants`
(`owner_id, subscription_id, grant_id`) is written in the same transaction (the existing
`Granted` wrapper) so the grant's id is also the retirement key.

`validate_subscription` (`workflow_notifications.rs:623-657`) needs one arm for the new kind
(`endpoint_id == publication_id == grant id`).

Receipts reuse `app_outbox`: pending, then `accepted`, then a new settle step (below).

The grant's `Part` gains one field, `sources: BTreeMap<String /*source_id*/, String /*subscription_id*/>`,
so `GetEventSubscription` can list what the grant follows without a join. The row in
`app_automations` is the authority; the part field is a projection rewritten with it
(same transaction).

### 2.3 Requests

Additive fields on the existing event requests, so there is one subscription concept per grant:

```text
SubscribeEvents   { targets: [EventTarget], sources?: [String] }      // source ids; targets may now be empty if sources is not
UnsubscribeEvents { targets?: [EventTarget], sources?: [String], all?: bool }
GetEventSubscription {}  ->  EventSubscription { ..., sources: [EventSourceFollow] }
EventSourceFollow { source_id, display_name, kind, session_id, subscription_id, available }
```

`validate_event_targets` currently refuses an empty list ("follow 1-1024 event targets",
`outside_events.rs:48-52`); the rule becomes "1-1024 targets and sources together".
`UnsubscribeEvents{all}` keeps meaning "end everything of this grant" (it ends the part:
`Mutation::Unsubscribe`, which also ends the source subscriptions through the new retirement
below).

Choosing a source needs a list of the sources the grant may see. Two options:

- A. Reuse `ListWorkflowNotifications{session_id}`, which a grant can already call and which is
  already scoped (section 1.5). The agent needs the Room ids first (`ListSessions`).
- B. (recommended) One small new read, `ListNotificationSources {}` -> `NotificationSources
  {sources: [WorkflowNotificationSourceSummary]}`, grant-only, owner-wide, this kernel only,
  enabled and available sources only, built from `notification_inventory(owner)` +
  `summaries` (`workflow_notification_router.rs:45-86`). It returns the same summary type the
  picker uses (no outputs, prompts or credentials,
  `workflow_notification.rs`, `WorkflowNotificationSourceSummary`).

CLI/MCP: `chariox events sources` (list), `chariox events subscribe --source <name|id>`,
`events unsubscribe --source`, `events subscription` prints `sources`; `events_mcp` gets the
same through its existing subscribe/unsubscribe tools. `<source>` resolves like the Room tool
resolves a notification source: a display name, `<kernel>/<name>`, or a source id; the
longest run of leading words naming exactly one source wins
(`docs/EVENT_TRIGGER_PROTOCOL.md`, "Room tool"). Name resolution happens in the CLI against
`ListNotificationSources`; the kernel takes ids only.

### 2.4 Authority and scope check

On `SubscribeEvents{sources}` from an `ExternalAgent` connection, in the kernel (not the
client):

1. The existing event-request gate: class `ExternalAgent`, live grant
   (`dispatch.rs:120-143`). A `reach:` caller (a Room agent) is **not** admitted for
   `sources` in v1: the Room agent already has `chariox.notification_subscribe` for itself
   (`EVENT_TRIGGER_PROTOCOL.md`, "Room tools"), and ON-18 limits this to outside agents
   through a grant.
2. Source visibility is the existing rule, evaluated for the grant's owner, with shared access
   semantics: `attachable_source(sessions, owner, source_id, shared_access = true)`
   (`workflow_notification_router.rs:326`): the source is in `notification_inventory(owner)`
   (so `owner_id == the grant's owner`), is `enabled`, is `available`, and is on this kernel.
   For the source's Room, `session.has_member(owner)`
   (`router.rs:95`). Sources of Rooms where the owner is only a guest are not in the owner's
   inventory and are refused with the same "source not available" the router uses, which does
   not tell whether the id exists.
3. The grant's authority is re-checked at commit: `attached_within_grant` pattern
   (`workflow_notification_grants.rs:18-30`): if the grant ended while attaching, the new row
   is retired and the request fails.
4. Refusals use `events_refused` (the public code, `outside.rs:21-23`) with constant
   sentences, like the existing event refusals.

A `/sudo` window acts for the owner too, but its id is `sudo:<entry>`, not a grant
(`notification_grant()` ignores it, `workflow_notification_grants.rs:11-15`), and
`access_grant_summary` reads only real grants (`kernel_access.rs:546-554`), so `event_request`
already answers "grant revoked or expired" for it (`dispatch.rs:133-137`). `sources` inherits
that: the part is keyed by the grant id only (`events.rs:74-80`).

### 2.5 Forwarding on the grant's event stream

**Event shape.** One new variant of `OutsideEventValue`
(`local/api/types/outside_events.rs:74`):

```text
Notification {
  source_id, source_kind: workflow_exit | owner_action,
  display_name, occurrence_id,
  output: { text, artifacts? }       // the envelope's WorkflowOutputPayload, already redacted
  deadline_ms,                       // emit time + 7 days
}
```

with the common `OutsideEvent` fields: `kernel`, `machine`, `session_id` = the source's Room,
`agent_id` none, `summary` = "<display_name> finished" (or the owner-action title),
`at_ms` = occurrence time, `refs.interaction_id`/`task_id` copied from the owner-action
locator so a driver can answer the decision it names with the existing
`RespondToInteraction`. Ancestry is not in the event: it is kernel-derived, the outside agent
cannot use it, and it can be long.

Output passes the two existing redactions: the envelope's `protect_envelope`
(`workflow_notifications.rs:73`) and the event `project` (`outside.rs:543`).

**Delivery step.** A new `NotificationOperation::AppendToGrantInbox{subscription, envelope}`
that, in **one** transaction on the single durable writer (both stores use it:
`DurableWriterRequest::WorkflowNotification` and `::AgentLifecycle`, same connection):

1. re-checks the grant is live and the part has not ended;
2. inserts the `agent_inbox` row with `occurrence_id = receipt_id(subscription, envelope)`
   (`INSERT OR IGNORE`, so a repeat appends nothing);
3. sets the receipt to its settled state (the existing `queued`-with-a-handle state; the
   handle is the inbox sequence);
4. runs `bound()` for the part.

Both executors take the writer's one `&mut connection` (`durable_state.rs:1716-1743`), so one
transaction can touch `agent_inbox` and `app_outbox`. The step calls a helper beside
`outside::publish` that appends to one part by id; it does not need the `Followed` target index,
which only serves target matching (`outside/followed.rs`, `Mutation::changes_targets`).

This is the same shape as `Queue` (`workflow_notifications.rs:467`): the receipt cannot say
"delivered" without the row, and the row cannot be added twice. It is the admission fence for
this target: a crash between steps does not exist, and a repeated attempt after a lost
response finds the row by its unique key.

The router pump gets one more arm next to the agent arm
(`workflow_notification_router.rs:458-465`): `OutsideAgent` local targets go through
`accept` (dedupe, deadline, loop drop) and then `AppendToGrantInbox`. A pending receipt
whose grant is gone is left to expire, as a workflow target's is
(`with_live_notification_grant`, `router.rs:473`).

**Ordering.** The cursor is the inbox sequence, so a reader sees events in commit order. For
local sources the pump appends in the order it takes the receipts
(`ORDER BY next_attempt_at_ms, sequence`, `workflow_notifications.rs:306`), with at most 8
per pass and one-second rotation: events of **one source** are in emit order unless a pass
fails and retries one; the event carries `at_ms` for a reader who needs emit order. Order
across sources is not defined. I would not promise more.

**Latency.** The pump's floor is one second (`router.rs:543`), plus `outside_wait`'s 100 ms
loop. So a reviewer-exit-like occurrence reaches a waiting grant in about one to two seconds.
That is a bound on kernel work, not client polling.

**Delivery guarantee.** At least once until acknowledged, exactly once per (subscription,
occurrence) in the inbox:

- the receipt dedupe makes a source replay a `Duplicate`;
- the unique inbox key makes a repeated append a no-op;
- the grant sees an event again only when it has not acknowledged it (`ListEvents` before
  `AckEvents`). A client keeps its own `occurrence_id` set if it acts on events before
  acknowledging.

No separate per-event receipt from the grant is needed: `AckEvents{cursor}` is the receipt,
and it already deletes the rows (`outside.rs:340-352`).

**Cursor and retention.** One bound is new: notification rows must not outlive their 7-day
deadline unread. The part's count bound (100,000) and the end receipt's 24 h stay as they are.
`bound()` gains a first step that deletes `notification` rows whose `deadline_ms` has passed
and records them in the same per-kernel `Gap` it already writes for dropped rows, so the grant
learns "N events dropped; resync" instead of silently losing them (`retention.rs:107-150`).
The receipt side already expires through `Sweep` (`workflow_notifications.rs`, `Sweep{now}`).
Open question 2 asks whether 7 days is the right bound for a part whose holder is alive.

**Replay after a kernel restart.** The part and its rows are durable; `Restart{keep}` keeps a
part whose grant holder is not gone (`outside.rs:355-366`, `mod.rs:886-894`). Required change:
`retire_notification_grants(None)` at start (`mod.rs:1263`) must not retire an `outside_agent`
subscription whose grant is in `grant_inboxes_to_keep`; every other grant-bound row, and every
`outside_agent` row whose grant is not kept, is retired as now. Occurrences emitted while the
kernel was down are handled by the receipts' ordinary persist-before-send retry; the grant's
`WaitEvents` after reconnect returns them. A timed grant does not survive a restart, so its
part ends with `kernel_restarted` and its subscriptions retire with it.

**Replay after the grant reconnects.** The grant is a process; the part is keyed by the
grant, not by the connection. A new connection of the same grant calls `ListEvents{since}` or
`WaitEvents{since}` with its last cursor (or none, and it gets everything unacknowledged),
as today.

A new grant (new process) does not inherit a previous grant's subscriptions: grants are
process-bound (`mod.rs:1262`). See open question 1.

### 2.6 Loop rules, Steer and the admission fence

- **Loop rule, same as agents.** `ancestry_identity` gets an arm `outside_agent` ->
  `grant:<grant_id>`; `accept` applies the existing contains-check, so an occurrence whose
  ancestry already holds `grant:<id>` is dropped with `workflow_notification_loop_dropped`
  and the 256-entry bound applies (`workflow_notifications.rs:758-766`).
- **What this does and does not cover.** For agents the kernel can see the chain: an agent
  whose active prompt came from a notification passes its ancestry to the workflow it starts
  (`agent_notification_ancestry.rs`). For an outside agent the kernel cannot see why it acted.
  A workflow the grant starts is not given `grant:<id>`, so A -> workflow -> exit -> A is not
  closed by ancestry in v1. It is bounded instead by: one receipt per occurrence, no coalescing
  and no re-emission; the 32-subscriber, 1,024-subscription and 1,024-pending limits; the
  grant's own term (an owner decision) and the holder process's lifetime. I would say this
  plainly in the spec and not claim the agent rule. Open question 3 offers an optional,
  explicit `in_response_to: occurrence_id` field on the requests that start workflow runs, so
  a driver that wants kernel loop protection can ask for it.
- **Steer lineage.** An outside agent has no kernel-owned active prompt, so there is no
  `ReserveAgentLineage` or `BindAgentSteer` for this target. A *driven* agent that the
  outside agent steers in response to a notification is an ordinary Steer by the grant; it
  creates no lineage. A notification that arrives for a **kernel agent** keeps its existing
  fence untouched.
- **Admission fence.** For this target it is the single-transaction append of 2.5. The
  agent-target fence (a receipt naming its prompt, `agent_trigger_notification.rs:543`) is not
  reused because it exists to span a prompt admission that is not atomic with the receipt; the
  inbox append is.
- **Writer fence.** An uncertain writer commit fences further transitions until restart (spec,
  "Delivery"): `AppendToGrantInbox` goes through the same writer, so it inherits the fence.

### 2.7 Lifecycle and revocation

| event | effect |
|---|---|
| grant revoked, expired or holder exits | `retire_notification_grants(Some(grant))` (existing calls, `kernel_access.rs:365,460`) disables the subscriptions; the part ends (`grant_ended`) and its end receipt is the last event the grant can read, kept 24 h. A pending or accepted receipt of a retired subscription is not appended (live-grant check inside the transaction). Nothing is delivered after the revoke commits. |
| kernel restart, until-exit grant, holder alive | part and subscriptions kept (2.5). |
| kernel restart, timed grant or holder gone | part ends `kernel_restarted`; subscriptions retired. |
| source disabled, workflow deleted, Room ended or transferred | no new occurrences; `GetEventSubscription` shows the source as `available: false`; the subscription stays until the grant unsubscribes or ends, as a workflow subscriber's does (`router.rs:473`). Receipts expire by their deadline. |
| owner changed (the Room's owner changes) | the owner_action source is per Room owner and `Register` disables the former owner's source row (`workflow_notifications.rs:431`), so the former owner's source stops emitting and the new owner has a new source id; a grant of the former owner receives nothing further. (`require_attached`, `workflow_notifications.rs:724`, only checks that the subscription row is still active and unchanged.) |
| grant's owner no longer a Room member | `session.has_member(owner)` is re-checked at append like `agent_notification_target_live` does for an agent (`agent_trigger_notification.rs:638`); a failed check detaches the row. |
| Room deleted | as "Room ended": the source is gone, the subscription is detached by the same reconcile step agent subscriptions use (spec, "Lifecycle"). |
| `UnsubscribeEvents{sources}` | disables those subscriptions (same `Detach` operation); rows already appended stay readable until acknowledged. |

### 2.8 Limits

- Per grant: at most 64 followed sources (my number; a coordinator follows a handful) and the
  existing 1,024 event targets, 100,000 inbox rows.
- Per source: the existing 32 subscribers, shared with workflow and agent subscribers. A grant
  counts as one. Refusal text: "notification fanout limit" (existing).
- Per owner: the existing 1,024 active subscriptions.
- Per subscription: the existing 1,024 pending receipts and the retained-bytes cap
  (`insert_receipt`, `workflow_notifications.rs:689-702`).
- Event size: the envelope's 64 KiB limit; the event is bounded by it.

### 2.9 What is explicitly not allowed (ON-18)

- No URL, webhook, email, push, file, Slack or any other destination, and no field that takes
  one. The only new destination is "my own grant's event part".
- No general outside subscription: not for a guest, a Room agent via this request, a
  `reach:` caller, a `sudo:` window, another owner's grant, another grant of the same owner,
  or an agent subscribing someone else.
- No remote kernel's source through a grant in v1 (slice S5 below, behind its own decision).
- No subscription option: no filter, event list, delivery mode, queue name or retention field;
  the request is `deny_unknown_fields` like the existing ones.
- No replay of occurrences emitted before the subscription (the same as Attach).
- The kernel never stores or forwards the grant id outside this kernel.

### 2.10 Protocol

Serialized shapes that change, all in the local protocol (`LOCAL_DAEMON_PROTOCOL_VERSION`,
`local/api/types.rs:489`):

- `SubscribeEventsRequest.sources`, `UnsubscribeEventsRequest.sources`,
  `EventSubscription.sources` (new optional fields, additive on the wire).
- `OutsideEventValue::Notification` (a new variant of an internally tagged enum: an old
  client that receives it fails to decode the batch, so this is **not** additive for a
  client reading events).
- `ListNotificationSources` / `NotificationSources` (new request and response, if option B).
- `WorkflowNotificationTargetKind::OutsideAgent` (appears in `ListWorkflowNotifications`
  responses for the owner's own listing, `listeners` and `subscriptions`; a client that
  decodes it with a closed enum would fail).

Decision: bump **local 999 to 1000** in S1 and take the first of these shapes in the same
change, with a snapshot test `protocol_shapes/` file, the hash test, and
`apps/kernel/managed-upgrade-protocol-transitions.json` (current release and the one before,
`[999, 1000]`; AGENTS.md "Development stage"). Per AGENTS.md "Protocol changes" step 3, no
client minimum is raised unless a client needs the behaviour: the web UI does not read the
grant's event stream or `ListNotificationSources` (the only consumers of `EventsListed` are
`packages/kernel-client/src/kernel-types.ts` and the CLI's events files), so
**the web feature minimum does not change**. The CLI/TUI minimum for `events --source` is the
CLI's own version gate.

**Peer: 438 stays through S1 to S4.** Local-source delivery does not use `RelayPeerRequest`;
the gateway pump forwards `LocalDaemonRequest::SubscribeEvents` as a terminal-class client
(`kernel_gateway/events.rs:97-130`), so remote parts ride the local protocol too. If S5 follows
a source on another kernel through that same forwarded `SubscribeEvents{sources}`, no peer
shape changes either; the home kernel imports the remote part's events with `Mutation::Import`
as it does today. If S5 instead needs `SubscribeWorkflowNotifications` to name a grant target
across kernels, `RELAY_PEER_PROTOCOL_VERSION` 438 -> 439 and the transitions file change with
it. The next pair the coordinator hands out is 1000/439; I propose S1 uses 1000 and only S5
may use 439.

## 3. Risks and open questions for Miguel (5, each with my recommendation)

1. **Subscriptions across grants.** A grant is a process; the coordinator that restarts gets a
   new grant id and loses its subscriptions and its unread events (2.5). Should an owner's
   subscription be tied to the owner's *outside-agent identity* (for example a named
   subscription survives a new grant of the same owner), or stay per grant?
   *Recommendation: per grant in v1.* A durable name would be a second identity the owner has
   not approved; the coordinator re-subscribes on connect in one request, and the part's
   open-decision replay shows the same pattern for targets. Revisit only if re-subscribing is
   a real cost in the drill.
2. **7 days for a live holder.** The notification deadline is 7 days, while a part keeps rows
   until acknowledged or 100,000 rows. Should unread notification rows expire at 7 days (my
   design, with a `Gap`), or be kept until acknowledged like decision cards?
   *Recommendation: expire at 7 days with a `Gap`.* It is the one retention constant the
   product states ("no retention fields"), and an outside agent that ignores an event for a
   week has missed it, not lost state it could not rebuild (the source can be read again).
3. **Loop protection for outside agents.** The kernel cannot see why an outside agent acted
   (2.6). Accept that v1 relies on caps plus the owner's grant term, or add an optional
   `in_response_to` on workflow-starting requests that stamps `grant:<id>` into the run's
   ancestry?
   *Recommendation: accept caps in v1; write `in_response_to` as a later slice only if a
   drill shows a loop.* The field is a new surface on a request many clients use for a risk
   nobody has shown.
4. **Remote sources through a grant (reviewer on another kernel).** The reviewer workflow's
   kernel may not be the one the coordinator's grant is on. Today shared access refuses
   remote workflow notifications (`workflow_notification_router.rs:173`). Should S5 let a
   grant follow a source on a paired kernel, by forwarding `SubscribeEvents{sources}` to that
   kernel as the existing event pump does for targets?
   *Recommendation: yes, as the last slice, only after the same-kernel drill passes.* It
   reuses the gateway's part, cursor and cleanup machinery and needs no peer bump if done
   through the forwarded request; but it doubles the failure matrix (link loss, cleanup owed,
   token renewal), so it earns its own round and its own live drill.
5. **Who may subscribe: guests' Rooms and Room agents.** In v1 only the owner's own sources
   for the owner's own grant (2.4). Should a Room *collaborator's* outside agent (a grant held
   for a collaborator) follow that Room's sources?
   *Recommendation: no.* Grants today are issued to the kernel owner by the owner's decision
   (`KernelAccessGrant.owner_user_id`), and the spec's guest rule ("this room's sources
   only") is for Room members' agents. ON-18 says the owner's own outside agents; staying
   inside it keeps the scope check to one line.

Risks not turned into questions:

- *Clock and ordering:* 2.5 promises commit order of the cursor only.
- *Event size:* up to 64 KiB per event times the 100,000 row cap is an upper bound of GBs of
  SQLite for one part. The existing cap is by count, not bytes; a per-part byte cap should
  be added in S2 if the drill with large outputs shows growth (the receipts have one:
  `MAX_RETAINED_PAYLOAD_BYTES`).
- *Writer contention:* the append runs on the single writer, like the existing queue step;
  one transaction per occurrence.
- *Secrets:* the output is redacted twice through the existing helpers; the event never
  contains a Vault value. The rule "never hand a Vault value to the model on its own"
  (AGENTS.md "Secrets and agents") holds because the notification output is what the workflow
  exit produced, which the agent could already read from the workflow's run.
- *Cloud boundary:* no provider, region, price or Cloud-only logic is added. No path is added
  to `scripts/control-plane-paths.allow.json`, and no entry to the leak list.

## 4. Test and drill plan

### 4.1 Acceptance rows

- **ON-17.** An outside agent, driving a kernel through its grant, subscribes itself to a
  source it may see (a workflow exit and the owner-action source), and receives each
  occurrence as one `notification` event on its event stream, with no polling, every
  occurrence delivered, replayed after a restart and after reconnect, retained 7 days,
  loop rules as for agents.
- **ON-18.** Only the owner's own outside agents through a grant: no URL/webhook/third-party
  destination, no general outside subscription.
- **AT-40 (reach).** A driver is woken by a driven agent's `RuntimeInteraction` through the
  event channel, including across kernels.

### 4.2 Fail-first tests (kernel, through real entry points)

Each fails on the base head and passes after its slice.

1. `durable_state/workflow_notifications/tests.rs`: an `outside_agent` subscription accepts
   an occurrence once, appends one inbox row and settles the receipt in the same commit;
   a replayed occurrence is `Duplicate` and appends nothing; a conflicting body is refused.
2. Crash fence: append, drop the response, run the pump again: still one row (unique key).
3. Router (`runtime/router/tests/outside_events.rs` style, real `CommandRouter`, grant id as
   caller): `SubscribeEvents{sources}` from `ExternalAgent` works; from a `Terminal`, a
   `reach:` caller and a `sudo:` authority it is refused; a source of another owner and a
   guest Room's source are refused with the same sentence as an unknown id.
4. Emit a workflow exit and an owner-action occurrence: `WaitEvents` returns them in commit
   order; `AckEvents` deletes them; `ListEvents` before ack returns them again.
5. Revocation: revoke the grant, then emit: nothing is appended; the wait returns
   `SubscriptionEnded`; `revoked_grant_fences_an_already_selected_accepted_notification`
   style test for an already-accepted receipt.
6. Restart: until-exit grant with a live holder keeps part and subscription across
   `KernelRuntimeState` rebuild and receives an occurrence emitted before the restart; a
   timed grant ends with `kernel_restarted` and its subscription is retired.
7. Retention: a notification row past `deadline_ms` is replaced by a `Gap`; fan-out and
   per-grant limits.
8. Loop: an occurrence whose ancestry holds `grant:<id>` is `LoopDropped`.
9. Protocol: snapshot and hash test for 1000, `deny_unknown_fields` on the requests, decode
   of the new event variant, managed-upgrade transitions test.
10. CLI: `events subscribe --source`, `events sources`, `events subscription` against a stub
    send function (`apps/cli/src/events-command.test.ts` style).

### 4.3 Real-kernel drill (the owner's rule: real path)

Name: `scripts/drills/notif-external-agent.mjs`, built like `agent-reach.mjs` (real kernel
binary, local relay, real built CLI). Scenario with the real reviewer-like workflow:

1. Start kernel A. A real provider agent (Codex gpt-6-luna low) or the coordinator CLI holds a
   grant (`chariox events` over the Unix socket, until-exit term).
2. Register a workflow with a notifying exit; the grant runs `events subscribe --source` and
   `events wait`.
3. Run the workflow so it finishes through that exit. **Proves:** the wait returns one
   `notification` event within about 2 s; the client issued exactly one request (count the
   frames: no repeated `list`); the output text is the exit's output.
4. Repeat for the `owner_action` source with a real blocked-task decision; the event carries
   `refs.interaction_id`, and `RespondToInteraction` through the grant answers it (AT-40).
5. **Restart replay.** Emit an occurrence, stop the kernel before the grant reads it,
   start it again with the same home: the grant, still alive, gets the event on its next
   `wait`; the event arrives once after `ack`.
6. **Revoked grant gets nothing.** Revoke from the terminal, emit: the old connection's wait
   returns `SubscriptionEnded`; a new grant of the same holder sees no event and has no
   subscription.
7. **Dedupe.** Emit the same occurrence twice at the source: one event.
8. **Red on base.** Run steps 2-3 on the base binary: `SubscribeEvents{sources}` is refused as
   unknown field, so the drill fails there (recorded as the red run).

Each step records the frame log, the kernel's trace, and screenshots of the TUI/terminal that
shows the event, under the evidence directory, never in the repository. The cross-kernel AT-40
cell is the same scenario with the driven agent on kernel B and the grant on kernel A,
including a reconnect of the link (existing `gateway events` pump), which is the open
`cross-kernel` coverage cell of the tracker row.

## 5. Implementation slicing (notifimpl)

Each slice is one bounded round, with its own fail-first tests, one publishable head, and no
reliance on the next.

- **S1 Protocol and data model.** Local 999 -> 1000. `sources` fields, `Notification` event
  variant, `OutsideAgent` target kind, `validate_subscription` arm, `ancestry_identity` arm,
  snapshot/hash/transition files, kernel-client types. No behaviour yet: requests with
  `sources` are refused "not available". Tests 9 and the shape part of 3.
- **S2 Subscribe, scope and delivery (local sources).** `SubscribeEvents{sources}` through
  `event_request`, the scope check (2.4), `AppendToGrantInbox`, the router pump arm, the
  `GetEventSubscription` projection, limits. Tests 1-4 and 8.
- **S3 Lifecycle and restart.** Retirement on revoke/expiry/exit for the new kind, the
  kept-grant exception at start (`mod.rs:1263`), `UnsubscribeEvents{sources}`, the liveness
  re-checks (owner, Room), 7-day expiry with `Gap`. Tests 5-7.
- **S4 Client and drill.** `ListNotificationSources` (option B), CLI `events sources` and
  `--source`, `events_mcp` tools, docs (`EVENT_TRIGGER_PROTOCOL.md`, `PROTOCOL.md`), then the
  real-kernel drill of 4.3 with red-on-base, and the AT-40 same-kernel live cells for Claude,
  Codex and OpenCode as the tracker's coverage keys require.
- **S5 Remote sources (conditional on question 4).** Forward `SubscribeEvents{sources}` to the
  paired kernel; home kernel imports; cleanup owed on unsubscribe/revoke; the cross-kernel
  AT-40 live cell and the shaped-network soak cell. Peer 439 only if the forwarded path is not
  enough.

S1 + S2 are the smallest change that makes ON-17 true on one kernel; S3 is what makes it safe
to ship; S4 is what the owner's rule calls done.
