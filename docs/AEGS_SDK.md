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

## Start a provider

Copy [`packages/aegs-sdk/examples/starter_aegs.rs`](../packages/aegs-sdk/examples/starter_aegs.rs)
into a provider repository, implement provider authorization and webhook
normalization, then run the public contract suite before submitting the
implementation. The starter is deliberately not a production adapter: it
shows the boundary while making missing provider authentication explicit. It
also executes one deterministic local fixture through the same public
`verify_webhook_conformance` harness used by provider tests:

```sh
cargo run -p chariox-aegs-sdk --example starter_aegs
# starter AEGS conformance passed: example.created (demo-occurrence)
```

Replace the fixture and normalization with the provider's authenticated
webhook path; do not treat this example as a substitute for signature
verification or credential storage.

Publisher manifests are signed locally. The SDK signs canonical unsigned JSON
and records its digest in the envelope; it accepts only raw 32-byte Ed25519 keys in hexadecimal or
base64. Keep the key in a secret manager or a file with restricted permissions;
never put it in a manifest, Cloud, AEDS, a kernel, or a command-line argument:

```sh
export CHARIOX_AEGS_SIGNING_KEY='...raw-key-in-base64...'
cargo run -p chariox-aegs-sdk --bin chariox-aegs-manifest -- \
  sign --input manifest.json --output manifest.signed.json \
  --key-id com.example.publisher.2026-01
cargo run -p chariox-aegs-sdk --bin chariox-aegs-manifest -- \
  validate --input manifest.signed.json
```

The registry verifies the declared digest and the signature against the
publisher key registered for the publisher namespace. A signed version is
immutable: changing event definitions, scopes, endpoints, or display metadata
requires a new version and digest.

Run the conformance tests with:

```sh
cargo test -p chariox-event-protocol -p chariox-aegs-sdk -p chariox-aegs-dummy
```

The SDK is intentionally developed in this repository as a workspace package:
the local `chariox-event-protocol` path keeps protocol and SDK changes tested
together. For a distributable release, publish the matching protocol crate
before publishing `chariox-aegs-sdk`; the SDK dependency is versioned so an
external provider can then depend on the public crates without this repository
or any private AEDS/AEGS source. Until that release sequence is complete,
external examples should use the public repository workspace (or a pinned
source checkout) rather than an unpublished registry package.

The repository contains a manual `Release public AEGS SDK` workflow for this
ordered release. It checks out one immutable commit, verifies that the SDK's
protocol requirement matches the protocol crate version, and queries the exact
protocol and SDK versions through the crates.io API. It publishes only versions
that are not already present, then resolves the exact protocol version through
Cargo's index before publishing `chariox-aegs-sdk`; it does not rely on a REST
record or an unrelated search result. This makes a rerun safe after a failure
between the two publishes. To exercise that recovery
path, run it with `confirm=publish` and `fail_after_protocol=true`, then rerun
with the flag disabled; the existing protocol version is skipped and the SDK
continues. It requires the repository's `CARGO_REGISTRY_TOKEN` secret and runs
only when an operator explicitly types `publish`; normal pushes and pull
requests never publish packages.

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

Mutation actions such as `notification.reply` are idempotent and are retained
as durable action receipts so a retry cannot repeat the provider side effect.
`event.context` (and the compatibility alias `notification.context`) is
different: it is a bounded, read-only query. The SDK serializes requests with
the same idempotency key while they are in flight within one AEGS process, but
deliberately does not persist the response or conversation data in
`action_receipts`; a retry after completion performs a fresh provider query.
This in-memory coordination is not a cross-replica deduplication guarantee.
AEGS implementations must keep context responses bounded and must not turn this
path into a general provider API proxy.

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

Reconciliation is authoritative only for the request's `owner_id`. A signed
management capability may authorize the exact kernel owner and one or more
kernel-user connection owners needed by the same kernel; the request header and
body must still name one of those exact identities. An AEGS
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
