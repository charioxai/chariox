# Event trigger protocol

Status: implementation contract; local daemon protocol 998 / relay peer 437 (MP-08 / MP-10).

## Boundaries

An enabled `event_based` workflow publication accepts two source kinds: App
outgoing events through App automations, and the owning user's workflow exit
notifications. Both use source-neutral kernel target admission and ordinary workflow
queues. App signing, event schema and capability checks remain in the App adapter.
An agent subscribes only itself (an agent trigger of kind `notification`); no agent
emits workflow notifications or subscribes another agent.

Protocol 365 retired direct AEGS-to-workflow bindings and their create/list/status/
transfer/test requests, exported binding templates, `event_context`, `event_action`
and reply tools. Protocol 452 explicitly amends the Apps-only rule to admit private
workflow sources. It does not restore direct AEGS bindings. AEGS implementations
normalize provider events; AEDS routes these to App inboxes only. The kernel owns
all inbox acceptance, queue admission, dispatch and run state.

The registry is a paginated control plane. Catalog pages never participate in event
delivery and a client must never download the complete catalog.

Registry manifests are immutable and signed. Version 1 canonicalizes the complete
manifest with the `signature` member omitted using RFC 8785 JSON Canonicalization
Scheme, computes `sha256` over those UTF-8 bytes, and signs the same bytes with
Ed25519. The registry verifies both the digest and signature against the publisher's
active trust key before indexing a version. The dummy fixture includes a public test
key and a valid signature; production publisher private keys never enter the
registry or a kernel.

The signed manifest contains generator identity and version, display metadata,
upstream provider, publisher, protocol version, categories, authorization contract,
event definitions, and deprecation metadata. Registry attestations that change
without a generator release—operator, verification level, availability, install
count, recommended placement, and the resulting `manifest_digest`—are deliberately
outside the signed payload and are rejected if embedded in a publisher manifest.
The registry supplies and authenticates those catalog fields separately.

An optional signed `actions` array describes provider actions. Apps access them
through explicitly granted connections. Workflow notification subscriptions carry
no provider actions, credentials, reply capability or event runtime tools.

## Typed generator facts (delivery 4, management 5)

Manifest schema version 2 accepts a signed `payload_schema` and `examples` on
an event definition. Version 1 remains accepted for text-only definitions.
A schema is a closed Draft 7 object, at most 64 KiB. Its examples are named
`{name,data}` objects: at most eight, 8 KiB each, 32 KiB together. Each must
validate against that exact schema. The schema and examples enter the manifest's
canonical digest and Ed25519 signature. An event type/version is immutable;
changing its contract requires a new version.

`PublishEventRequest.data` and `EventDeliveryEnvelope.data` are optional JSON
objects, at most 64 KiB, depth 32, 4,096 values, with safe integers only. A typed
event requires data matching the publisher schema. A text-only event omits data;
the kernel never guesses a schema from arbitrary metadata. AEDS provisions the
producer's schema with its scoped publishing capability. The kernel pins the
catalog schema and manifest digest when creating the App inbox source, then
checks delivery data before acknowledging. The signed App incoming schema must
also accept the complete wrapper, including `data` and `data_schema`.

The wrapper's schema identity contains the source event type/version, canonical
schema digest and exact schema. Apps can preserve these facts in their own signed
outgoing event. Inventory, urgency and batching policy belong to the consumer's
workflow configuration, rather than upstream normalization.

## Identities

- `generator_id` identifies a publisher-scoped AEGS implementation. It does not
  reserve an upstream provider name. Registry records separately identify publisher,
  upstream provider, operator, and verification level.
- `connection_id` is an AEGS-issued opaque authorization handle for one provider
  account or tenant. Provider credentials remain at the AEGS.
- `binding_id` in AEDS/AEGS management is an App inbox route identity. It is
  not a workflow notification subscription.
- `event_interest_key` is the SHA-256 digest of generator ID, event type and version,
  connection scope, and canonical filter.
- `environment_id` identifies one execution environment. A kernel defaults this to
  its stable daemon identity unless deployment supplies an explicit environment ID.
- `delivery_id` identifies one AEDS-to-kernel delivery. `occurrence_id` identifies the
  upstream fact after AEGS source deduplication.

AEDS's wire contract permits multiple routes for an interest. The kernel accepts
several App inbox routes with the same generator, event type, scope and filter; only
a repeated route ID is refused (`Conflict`). A route's `binding_id` is
`sha256(owner, installation ID, route ID)` and every route claim carries revision 1,
so AEDS refuses a claim only when another kernel already holds the same `binding_id`.
A second kernel that recreates the first kernel's installation and route IDs
therefore receives nothing; it uses a different route ID. The refusal is not an
error to the caller: the kernel logs `event route conflicts with another route` and
the route gets no deliveries. This document does not claim live AEDS fanout
qualification. App automations independently fan out declared outgoing occurrences
to workflow and agent targets, each named by the handler.

## Delivery

The kernel/AEDS event-delivery wire protocol is version 5. Version 3 carried the
optional provider-owned `reply_context` through `PublishEventRequest` and
`EventDeliveryEnvelope`; version 4 replaces the bearer token of the connection with a
challenge: AEDS opens every WebSocket with `{"type":"challenge","nonce":...}` (fresh and
single-use), and the kernel's `hello` carries its `kernel_id` (the fingerprint of its
identity public key), the `public_key` and a `signature` over the nonce and id. A hello
whose id is not the key's fingerprint, whose signature does not verify, or that answers
another connection's nonce is refused with `invalid_kernel_credentials`; an operator
admission policy (`open` with per-kernel caps, or an `allowlist` of kernel ids) may refuse
with `kernel_not_admitted`. Open admission is also capped for the gateway as a whole; the
refusals are retryable `gateway_rate_limited` (the connection is closed) and
`gateway_route_capacity` (claims over the cap are not accepted). Version 5 carries typed `data` (see
above). Peers that do not advertise version 5 are rejected.

The kernel maintains one authenticated outbound WebSocket connection to AEDS. It
sends its stable kernel/environment identity, route claims, and an optional resume
cursor on connect and reconciliation. App routes currently rely on durable inbox
receipts for deduplication and send no last-accepted cursor. AEDS durably stores pending delivery before sending
and retries until the kernel acknowledges or the delivery expires.

The kernel validates the envelope, binding and signed incoming App schema, then
atomically persists the App inbox occurrence and receipt. It acknowledges after
that durable acceptance, before the App handler or workflow runs. Inbox deduplication
is scoped to the route and source occurrence, so a lost acknowledgement or a new
network delivery ID cannot create another inbox occurrence on that route.

The App handler must explicitly emit to a named automation; incoming delivery does
not implicitly trigger every automation. Outbox deduplication is per automation.
The kernel subsequently commits the outgoing receipt and queued workflow prompt
atomically through ordinary workflow admission. The prompt follows ordinary queue
ordering and idle dispatch behavior. Transport expiry, inbox retention and outbox
retention are separate bounds; an ACK does not establish successful App handling
or workflow completion.

AEDS wire payloads permit a prompt up to 1 MiB and 32 artifact references. The
App adapter has narrower 64 KiB payload/prompt ceilings; it does not silently
truncate. Artifact references are metadata and confer no file, URL or credential
access.

## Private workflow notifications (local 998 / peer 437; MP-08 / MP-10 / MP-11)

A subscription has no options. Nothing about it is configurable except its source
and its target: there are no events, filters, output fields, queue names, delivery
modes or retention fields on any request, and every former option field is refused as
unknown. Retention is one kernel constant, 7 days (`NOTIFICATION_RETENTION_MS`),
applied to undelivered workflow and App occurrences alike through the existing deadline
check.

**Sources.** Each *exit* of a workflow is its own source.
`RegisterWorkflowNotificationSource {session_id, workflow_ref, exit_node_id, enabled}`
(`exit_node_id` is an exit node id or the Exit inspector's `exit-N`) is the exit's
"Generate a notification when the workflow finishes" switch. The source id is stable
per exit. A run emits only when it finishes through an exit whose switch is on, carrying
that exit's final output; a run that fails before reaching an exit, a cancelled run and
intermediate outputs emit nothing. There is no emit tool. Completion and source receipts
commit in the normal workflow transaction.

**Naming.** The workflow is the proxy: `display_name` is its alias (or id). When more
than one exit of the same workflow has the switch on, the exits are named
`<name>-<ordinal>`, the ordinal being the exit's position among the workflow's exits (the
Exit inspector's `exit-N`). Source summaries carry `exit_node_id`, `exit_ordinal`,
`name` and `display_name`; names are derived from the workflow's current definition.

**Subscribers.** A workflow entry point or an agent.
`AttachWorkflowNotification {session_id, source_id, publication_ref}` subscribes an
enabled `event_based` publication of the owner; an occurrence always queues a run on the
publication's own queue. Attach refuses an exit of workflow W for an entry point of W
(local or remote) with `workflow notification cycle`; longer cycles are stopped at delivery
by ancestry. `DetachWorkflowNotification {session_id, subscription_id}` disables local
admission and sends a peer unsubscribe when remote.

An agent subscribes itself with `CreateAgentTrigger` whose source is
`{kind:"notification", source:{workflow:{source_id}}}` (an App event is
`{app:{installation_id,event_name}}`, see Apps below). The request carries no
`busy_policy` (a value is refused) and `UpdateAgentTrigger` refuses one: a notification is
always queued as a prompt of the agent, and the agent's pane shows it as an ordinary
queued prompt it can send now or cancel. The prompt is the trigger's own text followed by the
quoted occurrence in `<chariox-trigger-input kind="notification">` (output and
`display_name`, `<` and `>` escaped), under the `runtime/notification` template. The
subscription is the same `app_automations` row a workflow subscriber has, with
`target_kind: agent`, `agent_id` and `trigger_id`; its receipt stays `accepted` until the
prompt is queued, then becomes `queued` with the prompt id. An agent target has no workflow
queue; its row carries the constant queue id `agent`.

**Lifecycle.** A subscription lasts as long as its agent trigger. Deleting the trigger
detaches at once (local disable plus peer unsubscribe). An agent archived, removed, a room
ended, or a collaborator who left are caught by the pump's reconcile step each pass:
a subscription whose agent or trigger is gone is detached, the source's next delivery is
refused at the target, and nothing is queued.

**Guests.** A room member and a member's agent list and subscribe to this room's
workflow sources only (and, with Apps, this room's installations' events): nothing of
the owner's other rooms or kernels, no remote sources, no cached kernel inventory.

**Room tool.** `chariox.room.run_command` admits `trigger notification <source> [prompt]`
and `trigger remove <trigger-id>` for the calling agent only; the request carries the
caller's agent id and no agent argument (naming another agent is refused). `<source>` is a
display name, `<kernel>/<name>`, a source id, or `app:<installation>/<event>`; the longest
run of leading words that names exactly one source wins.
`trigger add schedule (--every <N>s|m|h | --cron '<six fields>' [--timezone <zone>])
[--prompt <text>]` gives the calling agent one recurring schedule trigger (its heartbeat):
every firing is one prompt of that agent, a firing is skipped while the previous one is
still queued or running, and it repeats across turns and kernel restarts until
`trigger remove`. The kernel validates it as any schedule trigger. `trigger list` lists the
caller's own triggers with their schedule, next run and firings.

**Envelope.** `{source_id, display_name, occurrence_id, output, ancestry, deadline_ms}`.
`deadline_ms` is the emit time plus the retention constant. There is no status, subject or
fields: a notification says that the exit finished and what it produced.

**Ancestry and loops.** An ancestry entry is `[kernel_id, session_id, workflow_id]` encoded
as a JSON tuple, or `agent:<agent_id>` for an agent target. The kernel records ancestry in
the durable receipt and the run's kernel-created invocation context. Admission drops a
notification whose target is already in its ancestry with `workflow_notification_loop_dropped`
(A→B→A, A→B→C→A); a 256-entry bound drops with its own diagnostic. Every other occurrence is
delivered: there is no coalescing.

**Outside agent grants (local 1000).** The owner's own outside agent, holding a local access
grant, is a third subscriber type, `outside_agent`: `SubscribeEvents {targets, sources}` names
source ids of this kernel, `UnsubscribeEvents {targets, sources, all}` ends them, and
`GetEventSubscription` lists them (`sources`). The subscriber is the grant itself: its "queue"
is its own event stream, there is no URL, webhook, filter, queue or retention field, and a Room
agent's reach identity, a `/sudo` window or another owner's grant cannot follow a source. A
source the owner cannot see, one that is not on this kernel and an unknown id are all refused
with `source not available`. An accepted occurrence (deduplicated, dated and loop-checked as
for any subscriber, with `grant:<grant_id>` as its ancestry identity) becomes one `notification`
event in the grant's stream in the same commit that settles its receipt, so a repeated attempt
adds nothing. The event carries the source, `occurrence_id`, the redacted output, `deadline_ms`
and, for an owner action, the task and interaction ids the holder can answer with
`RespondToInteraction`. Delivery is at least once until `AckEvents`. An unread notification
expires at its deadline (7 days); the holder then reads one `gap` event. The follow belongs to
its grant: revoke, expiry and holder exit end it, and a restart keeps it only for a grant that
lasts until its holder exits and whose holder lives. A new grant follows again. At most 64
sources per grant. Paired kernels do not inherit grants: a source of another kernel is refused.
`ListNotificationSources {}` answers `NotificationSources {sources}` (the picker's summaries, no
outputs): the enabled, available sources of this kernel that the grant's owner may follow, and
every one of them is accepted by `SubscribeEvents`. Listing creates the owner-action source
("Agent needs your action") of each of the owner's Rooms, so it can be followed before any task has
blocked, and the "run finished" source of each of their workflows (kind `workflow_run`, named
`<workflow> run finished`). A "run finished" source emits one occurrence for every run of its
workflow that completed or failed, whether or not it reached an exit, with the run id as the
occurrence id. Its output is JSON: `status` (`Completed` or `Failed`), `run_id`, `workflow_id`,
`prompt` (the run's invocation prompt, at most 2,000 characters) and, for a failed run,
`failure` (the first failure message, at most 2,000 characters). A stopped run emits nothing.
Listing also offers one `app_event` source for each active inbox route of the owner's installed Apps
(`session_id` is the installation, `workflow_id` the route, `exit_node_id` the incoming event, `name` the App's
incoming event name followed by `@<connection scope>` when the route has one). The kernel publishes
a newly accepted occurrence of the route to every grant that follows it in the commit that accepts
it, with the inbox occurrence id as the occurrence id and the event's `text` (at most 32 KiB) as the
output message; a repeated occurrence adds nothing, and an event nobody follows queues for no one.
A route that is removed or whose installation is no longer active leaves the listing. Only
a local access grant sends it; a terminal, a reach caller and a `/sudo` window are
refused as for the other event requests. `chariox events sources` lists, `chariox events subscribe
--source ID|NAME` follows, `chariox events unsubscribe --source ID|NAME|all` unfollows, and `chariox
events mcp` offers the same as tools next to `chariox_next_notification`, which returns each
`notification` or `gap` record once.

**Transport.** Local delivery is direct; remote delivery uses the existing E2EE relay peer
channel. Peer 437 carries the per-exit source fields in `ListWorkflowNotificationSources`,
renames `SubscribeWorkflowNotifications {source_workflow_ref, target_ref}` to `{source_id,
target_ref}`, admits an agent `target_ref` (the subscriber kernel owns target admission), and
`DeliverWorkflowNotification` carries the reduced envelope. Remote requests require an
unexpired sender-key-bound kernel or machine identity, matching realm and owner. Remote lease
workers reject kernel-wide notification requests. Peer 439 adds a required `generation` to
subscribe and unsubscribe, which the source applies only when it is not older than the newest it has
seen for that subscription, and answers a stale one with `WorkflowNotificationSuperseded`. Relay
remains ciphertext transport.

**Delivery.** Bindings use `app_automations.source_kind=workflow_completion`; pending and
accepted/queued receipts use `app_outbox`, beside `app_event`. Persist-before-send and retry
continue until ACK or expiry. ACKs are `accepted`, `duplicate`, `expired` and `loop_dropped`;
ACK never means run completion. The target retains accepted work while busy, paused or full.
Deduplication prevents repeated queue insertion; conflicting occurrence content is refused.
An uncertain writer commit fences further transitions until restart. Workflow-target
occurrences are queued by the existing App pump and atomic queue handoff; agent-target
occurrences are queued by the pump's agent step through the agent trigger path.

**Limits.** Payloads and prompts use the App path's 64 KiB ceilings; the final output
reserves 8 KiB for envelope fields, and source persistence and target acceptance both
reserve the rendered prompt's header (the encoded envelope is limited so its prompt fits
65,536 bytes). Oversize refusals leave no receipt and a visible diagnostic. At most 1,024
sources and active subscriptions per owner, 32 subscribers per source and 1,024 pending
receipts. Envelope output uses the shared secret redaction policy before forwarding and
persistence.

`ListWorkflowNotifications {session_id}` is the shared TUI/web picker request. The owner
sees local sources and the cached sources of other kernels (a picker open refreshes the
kernel inventory, four concurrent source-list requests, three-second deadline, at most 128
kernels); a room member sees this room's sources only. Offline sources are unavailable and
cannot be attached.

Shared shell/TUI commands:

- `/workflow notifications on|off <workflow> <exit>`
- `/workflow trigger notification list`
- `/workflow trigger notification attach <source>`
- `/workflow trigger notification detach <subscription-id>`
- `/trigger notification <source> [prompt]` (the focused agent)

**Apps (not yet built).** App outgoing events stay App automations delivered through AEGS and
the App pump; an agent target for them (an App automation row with `target_kind: agent`) and
the picker's `events` on the App installation summary are specified in `NOTIF_PLAN.md`. The
installation summary already carries `events: [{name, label}]`; the agent-target App variant
of `CreateAgentTrigger` is refused until it lands.

### Run-scoped subscriptions — design only (MP-08 / MP-10 / MP-11)

A future run-scoped subscription is owned by a workflow and created by one of its running
runs. Delivery resumes that same waiting run and it ends with the run. This round adds
neither run-scoped creation nor agent-created subscriptions for other agents.

## AEGS subscription reconciliation

AEDS routes and AEGS subscriptions are separate durable records. AEDS owns only the
mapping from an event interest to a kernel. Each AEGS owns provider authorization,
upstream webhook/subscription resources, and the mapping from an incoming provider
event to an `event_interest_key`.

The kernel reconciles each configured AEGS through its operator endpoint, signing every
request with its identity key and naming the secret owner id of the connections it acts for
(see `AEGS_MANAGEMENT_BOOTSTRAP.md`). No token is issued by Cloud or by the operator, so
provider credentials and publisher private keys never enter the kernel, AEDS, or catalog.
`PUT /v1/subscriptions/reconcile` is
authoritative for the calling kernel's bindings under one `owner_id` and `generator_id` pair and carries
App-route binding identity, opaque connection handle, provider scope,
canonical interest key, event type/version, filter, revision, and active state.
An omitted binding becomes inactive only when it is still held by that kernel.
A higher revision can transfer a logical App route to a new owner; equal or older
cross-owner writes are fenced. This is not workflow notification transfer. AEGS credentials are never reused as AEDS
credentials.

An AEGS verifies the provider request against the unmodified request body, rejects
replays according to the provider contract, normalizes a source occurrence once, and
publishes it to AEDS for every distinct matching interest key. It persists enough
source state to reconcile expiring or provider-managed webhook registrations after
restart. The version 1 request is specified by
`docs/schemas/aegs-subscription-reconcile-v1.schema.json`.

### Isolation between users

An `event_interest_key` is derivable from public names (generator, event type,
repository), and any kernel authenticated to AEDS can claim a route for it. A claim
therefore never decides who receives an occurrence. The producer does:

- `PublishEventRequest.audience` lists the routes (`kernel_id` + `binding_id`) an
  occurrence may reach. The AEGS fills it from its live, authorized subscriptions
  for the interest, through the connection that received the event. AEDS delivers
  only to active routes whose kernel and binding both appear in it, and refuses a
  publication without an `audience` array (`400 audience_required`). An empty
  audience delivers to nobody. A repeated publication of one occurrence reaches
  the routes it adds; each route is delivered to once.
- A subscription is read at publication time, so revoking a connection or
  withdrawing a subscription stops delivery at once, whatever the AEDS route
  does. A kernel that claims a route without a subscription receives nothing.
- A reconcile claim is taken only if the connection exists and is ready, and
  its provider lists the claim's `connection_scope` among the connection's
  resources. Other claims come back in `refused` with a typed `reason`
  (`connection_not_found`, `connection_not_ready`, `scope_not_in_connection`);
  the response never carries error text. The AEGS keeps the stored subscriptions
  and answers `502 scope_check_unavailable` when the provider cannot list the
  resources, so the kernel retries. A listing is cached for five minutes per
  connection and authorization generation.

One AEGS and one AEDS enforce this the same way for Cloud and self-hosted
operators: no ticket or Cloud account takes part.

Provider credentials never transit the kernel. The kernel starts authorization with
a signed request at `POST /v1/authorizations`; the response is either a
ready opaque `connection_id` or a user-action URL/device code. It then pages
provider-owned repositories, projects, workspaces, or equivalent scopes through
`POST /v1/resources/query`. The resource response supplies a display identity and the
canonical `connection_scope` used in an event interest. Both operations are bounded
shared kernel requests used unchanged by web and TUI clients. An AEGS must not return
provider access tokens, refresh tokens, webhook secrets, or raw credential material.

Management protocol version 4 added the reusable-connection lifecycle contract (still part of version 5, which authenticates every request by the kernel identity key, and version 6, which declares each event type's payload schema and examples):

- `POST /v1/connections/inspect` returns explicit lifecycle state, granted/required
  scopes, connected resources, health/event timestamps, recovery guidance, and test
  support;
- `POST /v1/connections/refresh` reconciles provider credentials/subscriptions and
  returns a fresh inspection;
- `POST /v1/connections/test-event` asks the provider adapter to construct an authentic
  test occurrence and sends it through the normal AEGS-to-AEDS delivery path;
- `POST /v1/events/query` lists the webhooks the generator stored for an owner (delivery
  ID, event type, connection scope and a short subject such as `#1026`, never the
  provider payload, received/published times, status, attempts, last
  error; never the prompt or reply context) and `POST /v1/events/replay` publishes one
  of them to AEDS again with a fresh attempt budget. AEDS dedupes on the occurrence ID,
  so replaying an event it already holds changes nothing;
- existing authorization, connection query, resource paging, reconnect, revoke, and
  subscription-reconcile endpoints remain the corresponding begin-install,
  installation-status, resource-discovery, reconnect, disconnect, and reconcile
  operations.

If an authorization callback is lost, the kernel polls connection query/inspection by
the stable opaque connection ID. Successful provider authorization is therefore
recoverable without reinstalling. A provider that cannot inspect health or emit a real
test occurrence must report that capability as unavailable; it must not fabricate a
successful health check or bypass AEDS.

## App connection lifecycle

- App inbox routes own AEDS route and AEGS subscription reconciliation.
- Route removal does not restore any retired direct workflow binding.
- Kernel restart reconnects and reasserts App route claims.
- Manifest versions are pinned; deprecation is health state, not an implicit upgrade.
- Workflow-source lifecycle and TTL are specified above and use no provider authorization.

## Security

Kernel and producer credentials are different scoped capabilities. Kernel
capabilities can only reconcile their own environments and acknowledge deliveries
addressed to them. Producer capabilities are restricted to one producer identity and
declared event types. All non-loopback deployments require TLS at the Caddy edge,
rotatable secrets, request size limits, replay-safe occurrence IDs, and logs that
exclude prompts, artifacts, and credentials.

Errors have stable codes and a `retryable` flag. Version negotiation rejects a peer
whose major protocol version is unsupported. Readiness means durable storage is
writable; liveness only means the process event loop is responsive.

MP-08 / MP-11: completion-occurrence diagnostics retain the most recent 4,096
rows across the kernel. Upgrade, capture and sweep bound this window; durable
terminal run state retains completion deduplication, and pending App receipts
are independent of diagnostic retention.

## Blocked-task decision sources

MP-08 / MP-09 / MP-10 / MP-11, local 998 / peer 437: every Room also has
an `owner_action` source named **Agent needs your action**. The kernel registers
it with the same source inventory used by notifying exits. Agents and workflow
entry points subscribe through the same notification reference, trigger, receipt,
retention and routing operations. The source's `kind` distinguishes it from a
`workflow_exit`; its workflow/exit identifiers are reserved kernel source identifiers.

Creating a blocked-task owner decision emits one occurrence per interaction ID
(task plus blocked revision). Re-projecting or rewording the same decision does
not emit it again. Its final-output text is a JSON locator with title, Room,
agent, task and interaction identifiers. Task reasons, prompts, approval choices
and secret values are excluded. Resolving the decision still uses the existing
kernel-owned interaction and its ordinary owner admission.

Delivery to an agent is always queued and may be steered from the general queued
prompt row. This source does not supply an outbound service. Outside-Chariox
email, web push or an outgoing App requires a separately configured destination
and delivery adapter; the current Cloud has no such service.

The Room tools are `chariox.notification_sources {}`,
`chariox.notification_subscribe {source,prompt?}`, and
`chariox.notification_unsubscribe {trigger_id}`. They bind to the calling agent,
reject other-agent and delivery-policy arguments, and recheck its provider epoch.
App events retain App catalog/schema admission and the App receipt pump. Their
automation summaries distinguish `workflow_endpoint` and `agent` targets; agent
subscriptions are lifetime configuration and are excluded from App-set copies.

For a browser collaborator's picker, `ListAppInstallations` accepts an optional
`session_id`. The kernel checks live Room membership and returns only installations
bound to that Room's agents, with the active release's event summaries. The owner
continues to see their kernel's installations. Room agents cannot use this field
to list another Room. Pagination remains `after` / `limit`.

The AX subscribe/unsubscribe result contains `trigger_id`, `source`, and `enabled`;
it does not include the Room snapshot. A notification manually steered into an
existing turn contributes its ancestry to that turn before the provider write.

`ListAppAutomations` and `DisableAppAutomation` accept the same optional Room
context. Collaborators may list and remove their own Room targets; naming an
owner's automation does not confer control. Both routes keep the existing App
catalog, revision and writer checks. Archiving an agent also detaches its App
subscriptions immediately, including when its App worker is stopped.


Source-side listeners (998/437): `WorkflowNotifications.listeners` reports durable subscriptions to enabled sources in the requested producer Room, including workflow and agent targets in other Rooms or kernels. `subscriptions` remains the inventory of bindings whose target is the requested Room on this kernel. The profile owner can inspect source listeners across Rooms and kernels. Room agents, collaborators and grant-bound callers receive listener details only for targets in their current Room on the current kernel. Both projections are kernel-owned; clients must not infer source listeners from target bindings.

An agent notification keeps its original deadline in kernel-owned durable prompt metadata. Ordinary prompt edits cannot extend it. Recovery restores it, and queue promotion and manual Steer expire undelivered notifications before dispatch. App admission failures defer only that receipt while independent subscribers continue; deferral preserves the original deadline.
