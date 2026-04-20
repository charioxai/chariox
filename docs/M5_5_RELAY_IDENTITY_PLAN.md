# M5.5 Relay Realms, Pairing, And Scoped Tokens

## Goal

Replace the current shared relay-token assumption with a real relay identity foundation that works for both self-hosted Arroba and the later hosted Arroba Cloud control plane.

M5 delivered relay-backed transport. M5.5 defines who is allowed to connect to a relay realm, which clients and kernels are paired, and what routing actions each connection may perform.

This milestone does not make the relay a session, workflow, provider, or workspace authority. The relay admits and routes encrypted packets. The kernel remains the runtime authority.

## Current Gap

The current relay model is good enough for local and self-hosted drills, but it does not prove that a remote kernel or CLI belongs to the same user.

Current behavior:

- the relay accepts daemon/client connections that know the configured relay token
- daemons register with that token
- clients connect with that token and target a daemon id or alias
- the relay routes opaque encrypted payloads
- trust and approval are mostly handled by the home kernel after relay routing

Problem:

- if the relay is reachable and another machine knows the shared token, it can connect to the relay realm
- a remote CLI with the shared token can attempt to target a daemon
- a remote kernel with the shared token can advertise itself
- the relay cannot distinguish "same user" from "same shared credential"

M5.5 closes this by introducing realm-scoped credentials, pairing, and caller identity propagation before multi-user collaboration depends on them.

## Core Model

Separate three layers:

1. Relay admission
   - who may connect to this relay realm?
   - which routing actions may this connection perform?
   - which target daemons/kernels may this connection reach?

2. Kernel trust
   - which machines, kernels, and clients has the home kernel paired or approved?
   - which public keys are bound to those trusted devices?

3. Session authorization
   - which user may attach to a session?
   - which user owns an agent, provider, workflow node, or endpoint?
   - which workflow mutations are allowed?

The relay owns layer 1. The kernel owns layers 2 and 3. A hosted control plane, when present, can issue layer-1 credentials but does not become runtime authority.

## Realm Model

A relay realm is a routing namespace.

Self-hosted:

```text
relay realm = one user's or team's private self-hosted routing namespace
```

Hosted:

```text
relay realm = one account, organization, or project namespace managed by Arroba Cloud
```

Every relay connection belongs to exactly one realm.

Required fields:

```rust
struct RelayRealm {
    realm_id: RelayRealmId,
    issuer_id: RelayIssuerId,
    display_name: Option<String>,
    created_at_ms: u64,
}
```

Rules:

- relay metadata lists only machines/kernels in the caller's realm
- daemon ids and aliases resolve only inside a realm
- packets cannot cross realms
- a self-hosted relay may host one realm by default
- a hosted relay may host many realms

## Scoped Relay Tokens

Replace runtime reliance on a single shared token with scoped admission tokens.

Required claims:

```rust
struct RelayTokenClaims {
    issuer: String,
    subject: String,
    subject_kind: RelaySubjectKind,
    realm_id: RelayRealmId,
    allowed_actions: Vec<RelayAction>,
    allowed_targets: Option<Vec<String>>,
    issued_at_ms: u64,
    expires_at_ms: u64,
    token_id: String,
    account_id: Option<String>,
    organization_id: Option<String>,
    device_id: Option<String>,
    machine_id: Option<String>,
    client_id: Option<String>,
    public_key_thumbprint: Option<String>,
    entitlements_version: Option<String>,
}

enum RelaySubjectKind {
    Client,
    Kernel,
    Machine,
    Service,
}

enum RelayAction {
    DaemonRegister,
    DaemonHeartbeat,
    ClientMetadataRead,
    ClientConnect,
    PacketRoute,
    PeerRequest,
    PeerEvent,
}
```

Relay validates:

- token signature
- expiration
- realm
- requested action
- target constraints
- optional public-key binding

Relay does not validate:

- session membership
- workflow node ownership
- endpoint ownership
- provider access
- managed-I/O authority

Those remain kernel responsibilities.

## Token Issuers

The relay verifier must support two issuer families:

1. Self-hosted issuer
   - local/home kernel or self-hosted admin tooling
   - no external account service required
   - suitable for open-source users running their own relay

2. Hosted issuer
   - Arroba Cloud
   - lives in the separate private cloud repo
   - handles account login, subscription state, organizations, and entitlements
   - issues the same token shape consumed by the open-source relay

The open-source runtime must not depend on Arroba Cloud existing.

## Pairing Model

Pairing binds a client or machine/kernel identity to a realm and public key.

### Remote CLI Pairing

Flow:

1. Home kernel creates a remote CLI invite.
2. Remote CLI accepts the invite.
3. Remote CLI generates or presents a client keypair.
4. Home kernel or issuer records the client public key.
5. Issuer returns short-lived scoped relay credentials.
6. Future CLI connections authenticate as that paired client.

Conceptual commands:

```bash
arroba client invite create
arroba client join <invite-token>
arroba client list
arroba client revoke <client-ref>
```

### Remote Machine Pairing

Flow:

1. Home kernel creates a machine invite.
2. Remote kernel accepts the invite.
3. Remote kernel generates or presents a machine/kernel keypair.
4. Home kernel records pending machine identity.
5. User approves or rejects the machine as an execution target.
6. Approved machine receives scoped relay credentials for daemon registration and peer routing.

Conceptual commands:

```bash
arroba machine invite create
arroba machine join <invite-token>
arroba machine list
arroba machine approve <machine-ref>
arroba machine revoke <machine-ref>
```

Existing `/machine approve`, `/machine rename`, and `/machine forget` semantics should be reconciled with this pairing model rather than duplicated.

## Invite Tokens

Pairing invite tokens are not runtime relay tokens.

Invite token properties:

- short-lived
- one-time or limited-use
- scoped to one realm
- scoped to one intent: client pairing or machine pairing
- exchanged for a durable paired identity and short-lived scoped runtime tokens

Runtime relay tokens should be short-lived. Long-lived secrets should be limited to refresh credentials or self-hosted admin bootstrap secrets.

## Caller Identity Propagation

Every kernel request that arrives through relay must carry an authenticated caller identity after decryption and relay admission.

Required caller concepts:

```rust
enum KernelCallerKind {
    LocalClient,
    RemoteClient,
    RemoteKernel,
    HostedService,
}

struct KernelCaller {
    caller_id: String,
    caller_kind: KernelCallerKind,
    user_id: Option<UserId>,
    client_id: Option<ClientId>,
    machine_id: Option<MachineId>,
    realm_id: Option<RelayRealmId>,
}
```

M5.5 only needs enough identity to distinguish paired clients and machines. M6.5 builds user/session authorization on top of this identity.

## Hosted Cloud Boundary

Arroba Cloud is a control plane, not a runtime authority.

Arroba Cloud may:

- authenticate users
- manage retail and enterprise subscriptions
- manage accounts and organizations
- provision hosted relay realms
- issue scoped relay tokens
- register devices, clients, and machines
- enforce hosted entitlements

Arroba Cloud must not:

- own workflow execution
- own provider execution
- own managed-I/O conflict decisions
- require plaintext prompts or provider output
- bypass kernel authorization

The open-source relay and kernel should define the issuer/verifier contract. Arroba Cloud implements a hosted issuer for that contract in a separate repository.

## Backwards Compatibility

The current shared relay token can remain as a dev/self-hosted bootstrap mode during migration.

Migration policy:

- shared token mode is allowed for local drills and explicit self-hosted bootstrap
- production relay docs should steer users toward pairing and scoped tokens
- hosted relay must not use a realm-wide shared runtime token
- once scoped-token support is stable, shared token mode should be visibly labeled as unsafe/bootstrap-only

## Protocol Changes

Relay protocol changes:

- add `realm_id` to daemon registration and client connect flows, either directly or derived from verified token claims
- replace raw `auth_token` checks with token verifier output
- attach verified subject/action/realm metadata to each connection
- constrain metadata queries to the verified realm
- constrain daemon target resolution to the verified realm
- constrain client requests and peer requests by token action/target claims

Kernel protocol changes:

- add caller identity to relay-originated kernel commands
- preserve local-client caller identity for local transport
- expose enough caller metadata for M6.5 session membership and ownership checks

## Delivery Slices

### Implementation Status

As of 2026-04-20:

- Slices 1-3 are implemented: relay identity vocabulary, verifier abstraction, and realm-scoped relay registry/routing are in place.
- Slice 4 is partially implemented: kernel-owned paired-machine state is integrated with the existing remote-machine approval registry, and paired-client state can be recorded, listed, and revoked.
- Slice 5 is implemented for the bootstrap path: shell coverage exists for `client invite create`, `client join`, `client list`, `client record`, `client revoke`, `machine invite create`, `machine join`, `machine approve`, `machine rename`, and `machine revoke`. Invite tokens are self-contained bootstrap tokens; one-time invite redemption and signed scoped-token exchange remain slice 7 work.
- Slice 6 is implemented as a foundation: verified relay caller identity is attached to forwarded relay frames and mapped into `KernelCommand.caller` for relay-originated local API requests. Session and workflow authorization checks remain M6.5 work.
- Slice 7 is implemented for the verifier contract: the relay can verify `arroba-scoped-v1` HMAC-signed tokens against configured issuer metadata supplied by the embedding server/control plane. The open-source relay still defaults to shared-token bootstrap unless constructed with a scoped verifier.
- Slice 8 is implemented for the relay identity surface: `live-relay-identity-security-drill.mjs` starts a real scoped-token relay and verifies paired/unpaired client and machine admission, action constraints, and cross-realm metadata/routing isolation. Full remote-provider CLI/machine drills still depend on physical remote machines.

### Slice 1. Token And Realm Types

Add shared domain types for:

- relay realms
- relay token claims
- relay subject kinds
- relay actions
- verified relay identity
- paired clients
- paired machines

Exit criteria:

- relay and kernel compile against the same identity vocabulary
- tests cover token action/target helper behavior

### Slice 2. Relay Token Verifier

Add a verifier abstraction:

```rust
trait RelayTokenVerifier {
    fn verify(&self, token: &str, action: RelayAction) -> Result<VerifiedRelayIdentity, RelayAuthError>;
}
```

Implement:

- current shared-token verifier for compatibility
- signed scoped-token verifier skeleton
- tests for expiration, action mismatch, realm mismatch, and target mismatch

Exit criteria:

- relay admission code no longer hardcodes direct shared-token equality as the only auth path

### Slice 3. Realm-Scoped Registry

Update relay registry:

- key daemon registration by `(realm_id, daemon_id)`
- resolve aliases inside a realm
- list machines/kernels inside a realm
- reject cross-realm packet routing

Exit criteria:

- two realms can contain daemons with the same alias without collision
- metadata queries return only the caller's realm

### Slice 4. Client And Machine Pairing State

Add kernel-owned pairing records:

- paired clients
- paired machines
- pending machine approvals
- public key fingerprints
- revocation state

Exit criteria:

- home kernel can record, list, approve, and revoke paired identities
- existing machine approval behavior maps onto the new paired-machine state

### Slice 5. Pairing Commands

Add TUI/slash and shell commands for:

- client invite create
- client join
- client list
- client revoke
- machine invite create
- machine join
- machine approve
- machine revoke

Exit criteria:

- shell command coverage exists for every new pairing command
- commands use kernel authorization and state, not relay-only local logic

### Slice 6. Caller Identity On Relay Requests

Thread verified relay caller identity into kernel command sources.

Exit criteria:

- relay-originated commands identify the paired client or machine
- local commands identify the local client path
- M6.5 can build session membership and ownership checks on this caller identity

### Slice 7. Hosted Issuer Compatibility

Document and test the hosted issuer contract without implementing Arroba Cloud in this repo.

Exit criteria:

- relay can verify tokens signed by configured issuer metadata
- self-hosted issuer remains available
- docs point hosted-control-plane implementation to the separate `arroba-cloud` repository

Contract shipped in this repository:

- Token format: `arroba-scoped-v1.<claims-base64url>.<signature-base64url>`.
- Claims payload: JSON-serialized `RelayTokenClaims`.
- Current verifier algorithm: HMAC-SHA256 over `<claims-base64url>`, using the configured secret for `claims.issuer`.
- Verifier behavior: after signature verification, the relay enforces expiration, allowed action, and allowed target constraints before admitting or routing.
- Integration boundary: hosted or self-hosted control planes instantiate `RelayAuthVerifier::scoped_hmac(...)` or an equivalent future verifier and pass it to `RelayServer::with_auth_verifier(...)`.
- `arroba-cloud` should issue this token shape first, then can migrate to an asymmetric verifier without changing relay/kernel authorization semantics.

### Slice 8. Live Security Drills

Run drills covering:

- valid paired remote CLI can connect
- unpaired remote CLI cannot connect
- valid paired remote machine can register
- unpaired remote machine cannot register
- revoked client cannot connect
- revoked machine cannot register
- client token cannot perform daemon registration
- daemon token cannot perform client metadata queries unless explicitly granted
- two realms cannot see each other's machines
- two realms cannot route packets to each other's daemons
- shared-token bootstrap mode remains explicit and isolated from scoped-token production mode

## Exit Criteria

M5.5 is complete when:

- relay connections are scoped to realms
- relay admission supports scoped token verification
- shared relay token mode is reduced to explicit bootstrap/dev compatibility
- remote CLI pairing exists
- remote machine pairing exists
- paired identity state is owned by the kernel or self-hosted issuer path
- relay metadata and routing are realm-scoped
- relay-originated kernel requests carry caller identity
- hosted issuer compatibility is documented and testable
- M6.5 can depend on caller identity for session membership and ownership checks

## Open Questions

- Should the self-hosted issuer live in the home kernel, a relay-admin command, or both?
- Should remote CLI pairing be user-bound immediately, or only client-bound until M6.5 user membership lands?
- Which token format should v1 use: JWT, PASETO, or macaroon-style caveated tokens?
- Should relay token public keys be configured statically in self-hosted mode, discovered from a local issuer endpoint, or both?
- How aggressively should shared-token mode be deprecated after scoped-token support lands?
