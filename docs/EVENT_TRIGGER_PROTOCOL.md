# Event trigger protocol

Status: implementation contract; local daemon protocol 437 (MP-08 / MP-10).

## Boundaries

An enabled `event_based` workflow publication accepts two source kinds: App
outgoing events through App automations, and the owning user's workflow completion
notifications. Both use source-neutral kernel target admission and ordinary workflow
queues. App signing, event schema and capability checks remain in the App adapter.
Agents never own subscriptions or emit workflow notifications.

Protocol 365 retired direct AEGS-to-workflow bindings and their create/list/status/
transfer/test requests, exported binding templates, `event_context`, `event_action`
and reply tools. Protocol 437 explicitly amends the Apps-only rule to admit private
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

## Private workflow notifications (protocol 437; MP-08 / MP-10)

`RegisterWorkflowNotificationSource {session_id, workflow_ref, enabled}` registers
one stable source for a workflow. Workflows currently have no independent owner
field; their session owner is the source owner. A source ID survives enable/disable.
Registration has no historical replay. An explicit registration after ownership
transfer creates a new owner-scoped source ID; old subscriptions and pending rows
are left alone. Completion selects only the current session owner’s source. Only successful `Completed` runs with a
valid final output emit, once per source/run occurrence. Completion and pending
outbox records commit in the same normal workflow transaction, so a restart between
completion and routing recovers the original occurrence. Failed and intermediate
outputs never emit. No emitter tool or workflow node exists.

`AttachWorkflowNotification {session_id, source_id, publication_ref, queue_ref?,
ttl_days?}` is same-user only. The target session, publication and endpoint must
belong to that owner, and the publication must be enabled and `event_based`.
Attach resolves the ordinary workflow queue and rejects a visible local graph
cycle. Default TTL is seven days, configurable from one to thirty days. Reattaching
the same target is idempotent; changing TTL affects future occurrences only.
`ListWorkflowNotifications {session_id}` projects the owner's source inventory,
target subscriptions and bounded diagnostics. A deleted/transferred source yields
`available=false` / `source_available=false`: clients show **source not available**.
No heartbeat, ping, registry registration, AEGS or AEDS call participates.

The envelope contains `source_id`, `occurrence_id` (kernel run ID), final `output`,
kernel-derived `ancestry`, and immutable `deadline_ms`. An ancestry entry is a
JSON tuple `[kernel_id, session_id, workflow_id]`, globally qualified for the future
peer transport. Client configuration requests reject owner, output and ancestry
fields. Ancestry is retained in the inbox and linked to the actual queued prompt;
completion reads that durable link rather than trusting invocation text or caller
metadata. A completed workflow already present in its inherited ancestry drops
its outbound emission with `workflow_notification_loop_dropped`. A defensive
256-entry ceiling drops with a separate diagnostic. A->B->A and A->B->C->A are
therefore denied even if a graph changed after attach or spans future kernels.

The named kernel notification router chooses local delivery for a same-kernel
subscription. The peer route is a deferred adapter; this round adds no relay peer
shape or Cloud directory. The source persists before send and retries until durable
inbox ACK or expiry. ACKs are `accepted`, `duplicate`, or `expired`; acceptance does
not mean the target run has finished or started. Inbox deduplication prevents repeat
queue admission. Queue/receipt handoff commits on the existing single writer, then
publishes the session projection. An uncertain commit fences the writer and
requires authoritative restart before dispatch.

The local inventory admits at most 1,024 sources and subscriptions per owner,
with at most 32 targets per source and 1,024 accepted inbox items kernel-wide.
Busy, paused and full targets retain accepted inbox work until its original deadline.
Retries rotate bounded batches to avoid head-of-line blocking. A fixed template
wraps JSON final output as untrusted data in an ordinary workflow prompt. This path
uses the AEDS 1 MiB prompt ceiling: encoded output is bounded to 1 MiB minus 1 KiB
for the fixed wrapper, with at most 32 artifact references. Source-neutral target
admission does not reuse the narrower App payload validator. Oversize output drops
with `workflow_notification_output_limit`, without failing successful completion.
No artifact is fetched, opened or promoted into a provider attachment. Ordinary
queue dispatch checks the deadline too; expired queued work is visibly cancelled.
Ancestry and dedupe identities remain after payload expiry to prevent replay.

Source delete or transfer does not migrate, revoke or delete pending notification
records. Local routing stops treating the source as available; pending deliveries
expire at their existing deadlines. Notification records are local kernel state,
not exported deployment packages. UI picker and cross-kernel route-directory work
are later rounds; the picker will offer **My workflows** beside Apps on the same
attach flow.

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
