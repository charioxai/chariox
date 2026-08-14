# Chariox Event Generation Server SDK

`chariox-aegs-sdk` is the public Rust SDK for first-party and third-party AEGS
implementations. It provides:

- manifest envelope and canonical digest validation;
- canonical prompt, metadata, artifact, TTL, and event builders;
- provider-neutral webhook, OAuth, hook-reconciliation, and normalization
  primitives;
- a scoped AEDS producer client;
- a durable authoritative subscription store;
- metadata-filter matching and replay-window helpers.

Run the conformance tests with:

```sh
cargo test -p chariox-event-protocol -p chariox-aegs-sdk -p chariox-aegs-dummy
```

An AEGS must expose `GET /healthz`, `GET /readyz`, `GET /version`, and the
capability-protected `PUT /v1/subscriptions/reconcile` endpoint. It must accept
only authentic provider events, normalize a provider occurrence once, apply
the subscription filter, and publish once per distinct event-interest key.
AEDS owns route fan-out, durable retry, and kernel delivery.

The event-delivery wire contract is version 3. AEDS and every producer that sends
reply-capable events must use the same protocol revision so the opaque provider
`reply_context` cannot be silently discarded by an older peer.

Management protocol version 4 also requires the shared installed-connection
lifecycle endpoints documented in [EVENT_TRIGGER_PROTOCOL.md](EVENT_TRIGGER_PROTOCOL.md):

- inspect connection state, scopes, resources, health, and recovery guidance;
- refresh/reconcile provider state without changing the stable connection ID;
- reconnect and revoke/disconnect;
- emit an authentic test occurrence through AEDS when supported.

Implement `AegsProvider::inspect_connection`, `refresh_connection`, and
`test_event` for provider-specific behavior. The SDK supplies a conservative
baseline inspection, but it intentionally does not claim that provider health
was checked or that test events are supported. A test event must use the same
subscription match and AEDS publication path as a real webhook.

The OSS repository intentionally contains no production AEDS and no
Chariox-maintained production provider implementation. Those components live in
the private `chariox-aeds` and `chariox-aegs-<provider>` repositories, consume
this SDK, and run the same public conformance contracts. The only runnable AEGS
kept here is `chariox-aegs-dummy` for deterministic local development.

Reconciliation is authoritative only for the request's `owner_id`. An AEGS
must not deactivate subscriptions owned by another kernel when one kernel
reconciles an empty or partial set. A higher binding revision may transfer the
same logical binding to a new owner; equal or older revisions from a different
owner are fenced.

Artifacts are optional. Their reference must be durable and retrievable by the
workflow; adapter-local or synthetic references are not valid. Provider
credentials, webhook secrets, and OAuth refresh material never enter a
manifest, AEDS, the registry, the kernel, or a workflow prompt.

Publisher paths:

- `official_provider`: reserved for an implementation signed and operated by
  the named service provider;
- `chariox`: a Chariox-maintained implementation, displayed as “Provider by
  Chariox” and never presented as official;
- `verified_community` or `community`: independently published and signed;
- `self_hosted`: locally registered by an operator and clearly scoped to that
  environment.

Generator IDs are publisher-scoped. Chariox uses `dev.chariox.<provider>` and
does not claim the provider's official namespace. Registry promotion requires
signed manifests, conformance, live provider evidence, and the deployment
failure matrix described by the integration's release evidence.

Every implementation should call `verify_provider_contract` from its tests.
Provider webhook fixtures can additionally use `verify_webhook_conformance` to
prove deterministic occurrence identity, bounded canonical fields, route
ownership, event type, and connection scope.
