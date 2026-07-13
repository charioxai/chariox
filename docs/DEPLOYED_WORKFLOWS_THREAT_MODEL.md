# Deployed Workflows Threat Model

Status: Phase 0 normative baseline

This threat model applies to Arroba-managed workflow and Agent App deployments.
It complements `DEPLOYED_WORKFLOWS_AGENT_APP_PLATFORM_PLAN.html` and the runtime
boundaries in `ARCHITECTURE.md`. A later design may add controls, but it must not
weaken these invariants without an explicit security review and migration plan.

## Scope

In scope:

- Cloud deployment control APIs and web UI
- OSS CLI/TUI Cloud deployment commands
- publication package upload, storage, download, and materialization
- hosted runners, publication containers, and local-runtime ingress registration
- public HTTP, SSE, WebSocket, and MCP ingress
- account handoff, audience access, credential binding, domains, logs, and usage
- relay/bootstrap traffic used to reach the home kernel

Out of scope for Phase 0 approval:

- claiming that current public slug URLs provide audience authentication
- transferring provider credential bytes through Cloud
- arbitrary persistent workflow patches in managed deployments
- managed application databases or durable customer application storage
- compliance certification or an SLA not backed by implemented controls

## Security Objectives

1. An actor can control only deployments in an account where their authenticated
   identity has the required role.
2. A public hostname or canonical deployment route resolves to exactly one
   deployment. Ambiguity fails closed.
3. Cloud, relay, ingress, logs, and browser clients do not receive provider
   credential payloads.
4. A package is immutable after release verification. Configuration, credential
   bindings, and audience policy cannot broaden its declared capabilities.
5. Runtime callers cannot forge Arroba identity, runner, session, or internal
   transport headers.
6. One caller cannot read another caller's state, prompts, output, attachments,
   traces, overlays, credentials, or quota state.
7. Managed deployments cannot apply persistent patches until reviewable diffs,
   authorization, audit, rollback, expiry, and concurrency controls exist.
8. Runtime and control-plane failures fail closed without silently switching
   account, deployment, credential, release, or audience identity.

## Actors And Trust

| Actor | Trusted for | Not trusted for |
| --- | --- | --- |
| Platform operator | Operating Arroba infrastructure under audited access | Customer ownership decisions or reading provider secrets by default |
| Account owner/admin | Account policy, deployment lifecycle, handoff, billing, and credential bindings | Other accounts or platform-wide policy |
| Deployer/operator | Explicit deployment and runtime operations granted by role | Ownership, billing, or capability expansion |
| Builder | Supplying source and an immutable package for review | Customer credentials, destination ownership, or undeclared capabilities |
| Customer reviewer | Accepting a claim and selecting destination policy | Builder source account or unrelated customers |
| End user | Invoking routes granted by audience policy | Control APIs, other callers, secrets, logs, or provider state |
| Machine caller | Invoking explicit routes with scoped credentials | Interactive sessions or unrelated routes |
| Home kernel | Session, workflow, agent, interaction, queue, and execution authority | Cloud account or billing authority |
| Hosted runner | Materializing approved packages and reporting observed runtime state | Changing desired state or account policy |
| Provider CLI | Provider-native execution and provider-local credentials | Arroba account authorization or ingress identity |
| Relay | Admitting scoped connections and routing encrypted packets | Runtime authority or plaintext inspection |
| Public ingress | TLS, host routing, audience auth, quotas, and trusted claim injection | Workflow or provider execution authority |

Builders and account owners can deploy code, so they are trusted for code in
their own destination account. This does not authorize them to bypass package
review, affect another account, or make model-generated mutations persistent.

## Protected Assets

- account, organization, deployment project, environment, and billing ownership
- immutable package bytes, digest, provenance, release signature, and active pointer
- provider-native credentials and external integration credentials
- deployment configuration and credential-binding versions
- audience identities, allowlists, API keys, sessions, and invocation claims
- prompts, outputs, attachments, workflow state, traces, and overlays
- custom-domain verification and TLS state
- runtime volumes, container identity, routes, queues, and replica affinity
- audit events, logs, usage records, budgets, and incident artifacts

## Trust Boundaries

### Browser Or CLI To Cloud Control Plane

- Browser mutations require an authenticated Cloud session and CSRF token.
- Bearer clients require a valid Cloud session token; CSRF does not replace bearer
  authentication.
- Account and creator identity are derived and verified server-side. Body or query
  identifiers are lookup hints, not authorization evidence.
- Read access requires membership. Managed lifecycle mutations require owner or
  admin access until narrower deployment roles are implemented.
- Cross-account reads and mutations return no target details.

### Cloud To Runner

- Cloud expresses desired state and issues account-scoped runner work.
- Runner identity is an opaque hashed credential and is scoped to one account.
- The runner materializes the selected package and reports observed state. It does
  not decide ownership, audience, billing, or desired release.
- Managed runners inspect the materialized manifest before Docker and reject
  persistent patch capability. Reconciliation removes legacy unsafe containers.
- A successful queued job is not proof of a healthy deployment. Ingress activates
  only from ready observed state with a backend target.

### Package Boundary

- Package archives are untrusted input until size, structure, digest, contract,
  and capability verification complete.
- Extraction must prevent path traversal, symlink escapes, device files, archive
  bombs, and writes outside the deployment package directory.
- Package identity and capability declarations are immutable release data.
- Cloud stores package bytes or object references, not provider credentials.
- Reupload cannot silently activate a release; readiness and activation remain
  explicit lifecycle operations.

### Public Ingress To Runtime

- Canonical routing authority is a globally unique deployment/environment identity
  or verified hostname. Human-readable slugs are presentation only.
- Legacy slug-only routing is allowed only when exactly one globally ready match
  exists; zero or multiple matches fail closed.
- Caller-supplied internal Arroba and identity headers are removed.
- Audience authentication must complete before opening HTTP streams, SSE,
  WebSockets, or MCP sessions.
- A later signed invocation envelope must bind deployment, environment, subject,
  organization, roles, audience, expiry, nonce, and invocation ID and be verified
  again at the runtime boundary.
- Dynamic/authenticated responses are never CDN cached.

### Kernel And Provider Boundary

- The home kernel remains runtime authority. Cloud is bootstrap and control plane,
  not a second runtime implementation or long-stream proxy.
- Providers run through official provider harnesses. Provider credentials remain
  in provider-native stores on the execution machine.
- Structured identity claims are not automatically inserted into model-visible
  prompt text.
- Runtime interactions, including permissions and user requests, remain
  kernel-owned and are projected consistently to web and TUI clients.

### Relay Boundary

- The relay routes encrypted packets and enforces scoped admission.
- It does not inspect or persist prompts, outputs, attachments, workspace data,
  provider payloads, or session history.
- Relay availability never changes kernel authority or account identity.

## Primary Threats And Required Controls

| Threat | Required prevention or detection |
| --- | --- |
| Forged account or creator fields | Session-derived actor plus repository membership checks |
| Browser cross-site mutation | CSRF on browser mutations, secure cookie policy, Origin/CORS tests |
| Cross-account object reference | Account-scoped service/repository lookup and denial regression tests |
| Duplicate slug hijack | Stable deployment ID or verified host; ambiguous legacy slug fails closed |
| Runner credential theft | Hashed opaque key, account scope, rotation/revocation, no logging |
| Package substitution | Digest/signature verification and immutable release pointer |
| Malicious archive | Bounded structured extraction and traversal/bomb fixtures |
| Credential exfiltration | Provider-native stores, opaque bindings, redaction, egress policy |
| Caller identity spoofing | Strip internal headers and inject/verify signed invocation claims |
| Persistent model mutation | Managed API, clients, runner, reconciliation, and ingress reject it |
| WebSocket/SSE auth bypass | Authenticate before upgrade/stream and test disconnect/reconnect paths |
| Cross-caller state leak | Caller-scoped sessions, overlays, queues, affinity, logs, and quotas |
| Replay of claim/API key | Hashed token, audience binding, expiry, nonce, single use, revocation |
| DNS/domain takeover | Proof before bind, global host uniqueness, revoke route before release |
| Log or trace leakage | Metadata-only default, redaction, scoped access, retention and deletion |
| Resource exhaustion | Body, timeout, queue, concurrency, replica, storage, egress, and budget limits |
| Stale runtime after control failure | Desired/observed reconciliation, heartbeat freshness, explicit degraded state |

## Phase 0 Enforced Baseline

- Deployment control routes authenticate Cloud sessions.
- Browser mutations require CSRF; bearer clients require valid bearer sessions.
- Account membership is verified server-side and creator identity is session-derived.
- Canonical ingress URLs include a stable deployment ID.
- Legacy slug-only lookup returns a route only for one globally unique ready match.
- Managed persistent patch controls are absent from the web UI.
- Cloud creation/start/restart and public ingress reject persistent patch metadata.
- OSS CLI/TUI Cloud deploy and reupload reject persistent patch packages before
  network access.
- Hosted runners reject unsafe packages before Docker and remove legacy unsafe
  containers during reconciliation.

These controls contain the known Phase 0 issues. They do not make unauthenticated
public deployment URLs suitable for customer data. Audience policy and signed
runtime claims remain a later activation gate.

## Residual Risks And Activation Gates

The following block a production customer-hosting claim until implemented and
validated:

- typed release contract, archive verification, digest/signature verification,
  immutable promotion, rollback, and provenance
- deployment projects/environments and desired/observed reconciliation semantics
- audience authentication and route-role enforcement for every transport
- one-time handoff claims and destination-owned credential replacement
- opaque credential bindings with runtime readiness checks and rotation
- custom-domain verification, global uniqueness, TLS lifecycle, and safe callbacks
- per-caller isolation, quotas, abuse controls, budgets, and privacy policy
- auditable lifecycle, access, credential, domain, and policy mutations
- backup, retention, deletion, incident response, and recovery drills

## Verification Matrix

Every security-sensitive change must include focused tests and an end-to-end drill
for each affected surface.

| Surface | Required verification |
| --- | --- |
| Cloud API | unauthenticated, CSRF, role, cross-account, stale session, and replay probes |
| Ingress | canonical route, duplicate slug, encoded path, headers, body, SSE, WebSocket, MCP |
| Package | malformed, traversal, symlink, bomb, digest mismatch, unsupported contract/capability |
| Runner | start, restart, reconcile, stale backend, crash, unsafe package, cleanup |
| Web terminal | create, configure, deploy, recover, inspect, stop, rollback, access denial |
| TUI/CLI | same lifecycle and denial semantics, including reupload and reconnect |
| Collaboration | owner, admin, member, invited customer, end user, and revoked actor |
| Remote | local runtime, slice, hosted runner, and Hetzner machine with resource samples |

Use isolated local relay, kernel, server, and Cloud ports for local drills. Use the
designated Hetzner machine for remote, slice, and collaboration drills. Compare
technical failures with existing drills before inventing a parallel path. Capture
browser screenshots where possible, monitor local/slice/Hetzner CPU, memory, disk,
processes, containers, and ports, and remove all temporary artifacts afterward.

## Review Rules

- Any serialized protocol change follows the protocol version/hash/drill rule.
- Any new trust assumption must be explicit in this document and in tests.
- Any control implemented only in React is incomplete.
- Security denials must be consistent in web and TUI surfaces and must not leak
  cross-account object existence.
- Commit and push every meaningful green increment so test evidence maps to code.
