# Event trigger protocol

Status: implementation contract, version 1.

## Boundaries

Event-based notification is a workflow trigger. A workflow endpoint owns zero or more
event bindings; agents never own subscriptions. AEGS implementations
authorize and normalize source events. AEDS stores routes and pending deliveries but
does not inspect workflow execution state. The kernel owns the durable inbox, queue,
dispatch policy, and runtime decisions.

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

## Identities

- `generator_id` identifies a publisher-scoped AEGS implementation. It does not
  reserve an upstream provider name. Registry records separately identify publisher,
  upstream provider, operator, and verification level.
- `connection_id` is an AEGS-issued opaque authorization handle for one provider
  account or tenant. Provider credentials remain at the AEGS.
- `binding_id` is owned by a workflow notification trigger. A destination deployment
  materializes an independent trigger binding from its immutable workflow revision.
- `event_interest_key` is the SHA-256 digest of generator ID, event type and version,
  connection scope, and canonical filter.
- `environment_id` identifies one execution environment. A kernel defaults this to
  its stable daemon identity unless deployment supplies an explicit environment ID.
- `delivery_id` identifies one AEDS-to-kernel delivery. `occurrence_id` identifies the
  upstream fact after AEGS source deduplication.

Multiple active routes may exist for an
`(environment_id, event_interest_key)` pair. Each binding is an independent route,
so AEDS fans one occurrence out to every matching active binding in that environment;
intentional fan-out is preserved even when workflows share a session. A binding ID
still has one authoritative owner, and moving that binding is an explicit, atomic
transfer; deploying into a distinct environment creates independent routes.

## Delivery

The kernel/AEDS event-delivery wire protocol is version 3. Version 3 carries the
optional provider-owned `reply_context` through `PublishEventRequest` and
`EventDeliveryEnvelope`; peers that do not advertise version 3 are rejected rather
than silently dropping reply capability.

The kernel maintains one authenticated outbound WebSocket connection to AEDS. It
sends its stable kernel/environment identity, route claims, and last accepted cursor
on connect and reconciliation. AEDS durably stores pending delivery before sending
and retries until the kernel acknowledges or the delivery expires.

The kernel validates the envelope and binding, then atomically persists a delivery
receipt with the queued workflow prompt. It acknowledges immediately after that
transaction succeeds. A lost acknowledgement can cause network redelivery, but the
receipt prevents a second queued prompt. After insertion the prompt follows ordinary
workflow queue ordering and idle dispatch behavior.

Payloads use the workflow endpoint model: a required prompt plus optional artifact
references. Version 1 permits a prompt up to 1 MiB and at most 32 artifacts. AEDS may
set a lower deployment transport ceiling but cannot invent a second payload model.
Metadata that already fits in the canonical event input stays inline. An AEGS attaches
an artifact only when its reference is durable and retrievable by the target workflow;
synthetic or adapter-local references are invalid.

## AEGS subscription reconciliation

AEDS routes and AEGS subscriptions are separate durable records. AEDS owns only the
mapping from an event interest to a kernel. Each AEGS owns provider authorization,
upstream webhook/subscription resources, and the mapping from an incoming provider
event to an `event_interest_key`.

The kernel reconciles each configured AEGS through its operator endpoint with a
separate scoped management capability. `PUT /v1/subscriptions/reconcile` is
authoritative for one `owner_id` and `generator_id` pair and carries
trigger-owned binding identity, opaque connection handle, provider scope,
canonical interest key, event type/version, filter, revision, and active state.
An omitted binding becomes inactive only when it is still owned by that owner.
A higher revision transfers a logical binding to a new owner; equal or older
cross-owner writes are fenced. AEGS credentials are never reused as AEDS
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

## Lifecycle

- Editing or re-enabling a trigger in the same environment preserves binding identity
  and its upstream subscription while atomically updating the endpoint revision.
- Pause deactivates the route without deleting authorization.
- Trigger deletion tombstones its routes and asks the AEGS to reconcile upstream
  subscriptions.
- Kernel restart reconnects, reasserts route claims, and drains pending delivery.
- A route move fences the old kernel and resumes from durable receipts.
- Manifest versions are pinned; deprecation is health state, not an implicit upgrade.

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
