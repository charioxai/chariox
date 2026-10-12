# Event trigger protocol

Status: implementation contract; local daemon protocol 452 (MP-08 / MP-10).

## Boundaries

An enabled `event_based` workflow publication accepts two source kinds: App
outgoing events through App automations, and the owning user's workflow completion
notifications. Both use source-neutral kernel target admission and ordinary workflow
queues. App signing, event schema and capability checks remain in the App adapter.
Agents never own subscriptions or emit workflow notifications.

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

AEDS's wire contract permits multiple routes for an interest. The current kernel
App-route adapter rejects a second active route for the same interest; this document
does not claim live AEDS fanout qualification. App automations independently fan
out declared outgoing occurrences to workflow targets.

## Delivery

The kernel/AEDS event-delivery wire protocol is version 3. Version 3 carries the
optional provider-owned `reply_context` through `PublishEventRequest` and
`EventDeliveryEnvelope`; peers that do not advertise version 3 are rejected rather
than silently dropping reply capability.

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

## Private workflow notifications (local 452 / peer 82; MP-08 / MP-10 / MP-11)

MP-11 F8 is closed by the existing lease-scoped authorization: leased-resource
requests must match the authenticated `LeaseCallerBinding`, and a mismatch
returns typed `unauthorized`. Remote lease workers reject kernel-wide
List/Subscribe/Unsubscribe/Deliver notification requests, including requests
from authenticated same-owner hosted peers, because these requests have no
agent or lease selector. A future notification selector requires a separately
allocated protocol change; this decision changes no protocol shape or version.

Workflows are a second source kind beside Apps. `RegisterWorkflowNotificationSource
{session_id, workflow_ref, enabled, output_fields?}` controls **Send notifications
when runs finish**. The session owner owns the source. Its ID survives switches;
registration has no historical replay. After transfer, registration creates a new
owner-scoped ID. Completion selects only the current owner's enabled source.
Successful `Completed` runs with valid final output emit that output. `Failed` runs
emit status and recorded provenance, without final output, failure detail or output
fields. Intermediate outputs and cancelled runs do not emit. There is no emit tool.
Completion and source receipts commit in the normal workflow transaction.

`AttachWorkflowNotification {session_id, source_id, publication_ref, queue_ref?,
ttl_days?, events?, filters?, delivery_mode?}` is same-user only. Target session, publication and
endpoint must belong to the owner; the publication is enabled and `event_based`.
Events are `success` (default), `failure`, or `both`. TTL defaults to seven days and
must be 1–30 days. Reattach reuses the binding; TTL changes affect new occurrences.
Attach rejects cycles visible in the local graph. `DetachWorkflowNotification
{session_id, subscription_id}` disables local admission and sends a peer unsubscribe
when remote. If the peer is offline, its outstanding sends get refused by the target
and expire; no additional unsubscribe scheduler is introduced.

The envelope carries `source_id`, `occurrence_id` (run ID), `status`, optional final
`output`, optional opaque `subject`, opaque `fields`, kernel-derived `ancestry`
and immutable `deadline_ms`. The kernel copies the triggering App/generator metadata
exactly from recorded invocation provenance, including nested values. Subject is
passed through only when supplied as a string by that generator; the kernel never
constructs or parses it. Up to eight declared output fields (names at most 64 ASCII
letters/digits/underscores) are projected from the structured final-output message.
Output values are small scalars, with strings at most 512 bytes. Existing trigger
fields cannot be overwritten by agent output. Failure carries recorded provenance
and bare status, without output fields, errors, trace or node details. Omitted/null
`output_fields` preserves the declaration on a switch. All domain semantics belong
to generators/Apps; fixture-specific field names are not kernel policy.

Filters use the shared AEGS/SDK semantics: AND across dotted fields, scalar equality,
metadata-array membership and expected any-of arrays. The source evaluates filters
before persisting a delivery; the target checks the current binding again before
acceptance. Source metadata lists declared output fields and generic subject/status fields.
Opaque trigger metadata may be filtered by arbitrary dotted paths; the picker must
allow entering a field path instead of treating its suggestions as an allowlist. Attach configuration
never accepts caller-supplied owner, output, subject or ancestry.

An ancestry entry is `[kernel_id, session_id, workflow_id]` encoded as a JSON tuple.
The kernel links ancestry to the durable queue receipt and records it in the run's
kernel-created invocation context. It survives payload expiry. Admission drops a
notification whose target is already in its ancestry, with
`workflow_notification_loop_dropped`; completion also fences repeat identities.
A 256-entry bound drops with its own diagnostic. These rules apply across kernels
and arbitrary local graph changes, including A→B→A and A→B→C→A.

The named notification router uses direct delivery locally and the existing E2EE
relay peer channel remotely. Peer 82 adds `ListWorkflowNotificationSources`,
`SubscribeWorkflowNotifications {source_workflow_ref, target_ref}`,
`UnsubscribeWorkflowNotifications` and `DeliverWorkflowNotification`. Each request
carries `protocol_version:82`; incompatible peers fail before activation. Remote
requests require an unexpired sender-key-bound kernel or machine identity, matching
realm and owner. The relay authenticates the registered sender kernel; Kernel claims
also require an exact canonical sender subject. Existing home-kernel peer requests
project to Machine identity, as execution leases already do. Shared-token/unbound peers do not authorize private streams.
The subscriber kernel owns target admission; the source records its authenticated
remote subscription locally. Relay remains ciphertext transport.

Bindings use `app_automations.source_kind=workflow_completion`; source pending and
target accepted/queued receipts use `app_outbox`, beside `app_event`. App-specific
signature, catalog, schema and capability checks remain in the App adapter. The
existing App pump and atomic queue handoff serve both kinds; no new scheduler,
provider execution path, Cloud directory, AEGS registration or AEDS call is added.
Round-1 delivery tables migrate transactionally into these shared tables, preserving
pending deadlines and queue links, and are removed. Legacy subscribers select success.

Persist-before-send and retry continue until ACK or expiry. ACKs are `accepted`,
`duplicate`, `expired`, `filtered` or `loop_dropped`; ACK never means run completion.
Terminal ACKs settle source retryable receipts on both local and peer routes,
including when an edited binding filters out an already-captured occurrence.
The target retains accepted work while busy, paused or full. Deduplication prevents
repeated queue insertion; conflicting occurrence content is refused. An uncertain
writer commit fences further transitions until restart. Ordinary dispatch checks
notification deadlines and visibly cancels expired queued work.

Every App automation and workflow-notification binding has `delivery_mode:queue|inject`
(default `queue`). Both adapters admit to the same durable receipt and workflow queue.
`inject` selects the endpoint entry agent only when its workflow has exactly one
active run and that endpoint's entry turn is active. It persists the exact run/turn
before using ordinary provider steering. The item stays durably accepted until
provider acceptance. Structured-provider mailbox enqueue is not acceptance: both
finished-submit reapers correlate the actual result with the durable injection.
Codex hidden-context failures before steering and explicit negative RPC replies
permit queue fallback. Write, read, malformed-reply and disconnect errors do not
prove non-acceptance: the exact local provider run, target prompt/turn, agent and
submit epoch remain durably held. Every local path, including native Claude,
Claude-headless and plain PTY writers, persists this same send intent before I/O.
A lost native hook acknowledgement or durable settlement cannot authorize another
write or a new workflow invocation. A restart also holds an unsettled local submit
intent, even if the original turn ended. Local providers have no authoritative
steer-receipt reconciliation API, so unknown outcomes expire without replay or
queue fallback; this can delay or expire an input that never reached the provider.
A buffered output error cannot acknowledge an injected notification. Remote send
intent pins the worker, machine, execution lease, turn and provider run before I/O. An uncertain remote outcome must reconcile
through `ReconcileLeasedPromptSteerReceipt` before replay or fallback, even when the
original turn has ended. Dispatching/unavailable/conflicting receipts remain held
until an exact accepted/rejected receipt or expiry. Idle/ended turns with no
uncertain send fall back to an ordinary queued run. Several active workflow runs fall back with
`notification_inject_multiple_runs_queued`. An ended turn during dispatch falls back
with `notification_inject_turn_ended_queued`. Provider errors retain the pending
item rather than assert acceptance. Incoming ancestry is linked to the target run
before steering, so that run's completion keeps the no-loop fence. No agent owns
any subscription. Run-scoped subscription creation remains design only below.

New payloads and prompts use the reused App path's **64 KiB** ceilings (not the AEDS
1 MiB transport ceiling). Final output reserves 8 KiB for envelope fields. Source
persistence and target acceptance both reserve the rendered prompt's 51-byte header:
the entire encoded envelope is limited to 65,485 bytes, so its prompt fits 65,536 bytes.
Refusal leaves no delivery receipt; completion retains a visible
`workflow_notification_prompt_limit` diagnostic. At most 32 artifacts remain metadata only: no fetch,
host-file read or provider attachment promotion. There are at most 1,024 local sources
and active subscriptions per owner, 32 subscribers per source, 1,024 pending receipts
and 16 MiB pending payload per subscription, measured as UTF-8 bytes for both new and
already retained envelopes. Oversize/backpressure is diagnostic and cannot
roll back successful workflow completion. New envelope output, subject and fields use the shared secret redaction policy before source forwarding and receipt persistence. Delivery also protects retained pre-redaction envelopes before local queueing or peer forwarding. Legacy accepted bytes retain their original
TTL; editing a binding's TTL affects new occurrences only, and admission of older
pending occurrences and duplicate ACKs uses the fixed 30-day protocol ceiling.
The current completion `status` is reserved while copying opaque trigger metadata,
so chained workflows with different outcomes filter on their own status.
Settled receipt payloads expire; deduplication tombstones remain for 30 days after
deadline before bounded reclamation. Causal ancestry remains in recorded runs.

`ListWorkflowNotifications {session_id}` is the shared TUI/web picker request. Local
sources come from kernel state. Picker open refreshes the existing waiting-room
kernel inventory, then fans out source-list requests (four concurrent, three-second
fanout deadline, at most 128 kernels). Responses contain source IDs, names, events
and filter fields, never outputs or history. Last results persist per owner/kernel.
Directory changes refresh lists without a new polling loop. Offline/missing sources
are unavailable and cannot attach; consumers show **source not available**. Source
delete/transfer does not revoke, migrate or rewrite pending deliveries; they expire.
No notification-specific heartbeat or ping is introduced.

Shared shell/TUI commands:

- `/workflow notifications on|off [workflow] [--fields verdict,review_url]`
- `/workflow trigger notification list`
- `/workflow trigger notification attach <source> [success|failure|both]
  [--publication <ref>] [--ttl <1..30>] [--filter opaque.field=value] [--delivery queue|inject]`
- `/workflow trigger notification detach <subscription-id>`

Attach resolves the selected workflow's unique enabled notification trigger, or the
explicit publication. Web uses these same requests; its **My workflows** picker next
to Apps and settings switch belong in Cloud, using the kernel as runtime authority.

### Run-scoped subscriptions — design only (MP-08 / MP-10 / MP-11)

Subscribers always belong to workflow endpoints. A future run-scoped subscription
is owned by that workflow and created by one of its running runs, using the same
generic opaque subject/field filters. Delivery resumes that same waiting run.
It ends with the run, TTL, or an incoming generator occurrence carrying the generic
`subject_closed:true` marker. A signed action manifest may declare that its result
opens a subject; the kernel does not interpret domain-specific lifecycle events.
This round adds neither run-scoped creation nor agent-owned subscriptions/tools.

## AEGS subscription reconciliation

AEDS routes and AEGS subscriptions are separate durable records. AEDS owns only the
mapping from an event interest to a kernel. Each AEGS owns provider authorization,
upstream webhook/subscription resources, and the mapping from an incoming provider
event to an `event_interest_key`.

The kernel reconciles each configured AEGS through its operator endpoint with a
separate scoped management capability. Hosted kernels obtain this capability from
the Cloud API after authenticating their persisted Cloud session or machine
credential; the registry-provided HTTPS management URL is checked against the
published generator digest before a token is issued. The capability is a short-lived
Ed25519-signed bearer token scoped to one generator, manifest digest, kernel identity,
an exact set of allowed owner identities, and management audience. AEGS SDK servers verify it locally with the configured
Chariox Cloud public key, so provider credentials and publisher private keys never
enter the kernel, AEDS, or catalog. Self-hosted deployments may continue using a
static operator token. `PUT /v1/subscriptions/reconcile` is
authoritative for one `owner_id` and `generator_id` pair and carries
App-route binding identity, opaque connection handle, provider scope,
canonical interest key, event type/version, filter, revision, and active state.
An omitted binding becomes inactive only when it is still owned by that owner.
A higher revision can transfer a logical App route to a new owner; equal or older
cross-owner writes are fenced. This is not workflow notification transfer. AEGS credentials are never reused as AEDS
credentials.

An AEGS verifies the provider request against the unmodified request body, rejects
replays according to the provider contract, normalizes a source occurrence once, and
publishes it to AEDS for every distinct matching interest key. It persists enough
source state to reconcile expiring or provider-managed webhook registrations after
restart. The version 1 request is specified by
`docs/schemas/aegs-subscription-reconcile-v1.schema.json`.

Provider credentials never transit the kernel. The kernel starts authorization with
the AEGS management capability at `POST /v1/authorizations`; the response is either a
ready opaque `connection_id` or a user-action URL/device code. It then pages
provider-owned repositories, projects, workspaces, or equivalent scopes through
`POST /v1/resources/query`. The resource response supplies a display identity and the
canonical `connection_scope` used in an event interest. Both operations are bounded
shared kernel requests used unchanged by web and TUI clients. An AEGS must not return
provider access tokens, refresh tokens, webhook secrets, or raw credential material.

Management protocol version 4 adds the reusable-connection lifecycle contract:

- `POST /v1/connections/inspect` returns explicit lifecycle state, granted/required
  scopes, connected resources, health/event timestamps, recovery guidance, and test
  support;
- `POST /v1/connections/refresh` reconciles provider credentials/subscriptions and
  returns a fresh inspection;
- `POST /v1/connections/test-event` asks the provider adapter to construct an authentic
  test occurrence and sends it through the normal AEGS-to-AEDS delivery path;
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
