# Deployed Workflows Threat Model

Status: normative baseline for managed Agent App activation

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

Builders and account owners can deploy reviewed code into their own destination
account. Package code is still untrusted with respect to provider credentials,
the kernel control transport, the platform runner credential, other deployments,
and other callers. Accepting a release does not authorize package code to cross
those boundaries.

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

### Credential Enrollment Boundary

- A credential setup is a short-lived, one-time enrollment bound to account,
  profile, target version, runner, mode, and expiry. Claim and consumption are
  atomic; a wrong runner, stale version, expired enrollment, or replay fails
  closed.
- Runner jobs use expiring leases and monotonic claim attempts. Heartbeats renew
  only the current attempt; restart may reclaim an expired lease on the same
  runner, and a superseded attempt cannot report progress or completion.
- Cloud stores enrollment status, a short-lived privileged projection of a
  sanitized provider setup URL/code when needed, account label when
  provider-native verification returns one, and opaque runtime references. It
  never receives provider credential files, provider tokens, OAuth callback
  codes, or integration secret values.
- Setup URLs can contain provider PKCE state and challenge parameters. They are
  redacted from ordinary profile reads, logs, screenshots, and support bundles,
  exposed only to destination owners/admins, bounded and cleared at terminal
  state. Secret-bearing URL parameters fail closed.
- The Cloud URL/code projection is a temporary bootstrap bridge. Production
  activation requires the planned scoped direct runtime/relay setup channel.
  Provider responses that must travel back to a login process, including Claude
  OAuth callback codes, use that one-time channel and never ordinary Cloud form,
  query, log, or runner-job fields.
- Provider-native verification is an explicit runner attestation after the
  official provider harness reports ready. Copying a pre-existing file is not
  provider-native verification.
- Runner-seeded enrollment is an explicit local/self-hosted migration and drill
  mechanism. Hosted Cloud disables it by default, and every surface labels its
  identity as unverified.
- Enrollment sources are minimal, permission-restricted, symlink-free, bounded,
  manifest-bound to the enrollment, consumed once, and removed after use.
- Every credential-bearing deployment and replacement job is pinned to the one
  runner that owns all of its materialized profiles. Mixed, missing, and live
  cross-runner bindings are rejected before desired state changes.

### Hosted Runtime Process Boundary

- Kernel/provider execution, the publication gateway, and package-supplied
  actions run as distinct unprivileged identities with separate homes. Package
  actions receive an allowlisted environment and cannot traverse provider homes.
- A strong kernel-local transport credential is generated inside the container,
  never passed by Docker or Cloud, removed from the kernel environment before it
  launches provider children, and unavailable to gateway-unrelated identities.
  The kernel keeps only its private in-memory copy and rejects unauthenticated
  local WebSocket handshakes when the credential is configured.
- The platform runner key never enters the deployment container. Runtime audit
  delivery uses a deployment- and revision-scoped capability to a runner-owned
  bridge, which verifies that the runtime is active, candidate, or draining
  before forwarding bounded entries.
- Read-only root filesystems, no-new-privileges, bounded tmpfs mounts, provider
  cache separation, and explicit process cleanup remain defense in depth; none
  substitutes for the identity and transport boundaries above.

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
| Enrollment replay or runner swap | One-time account/profile/version/runner binding, expiry, atomic claim/consume, source manifest |
| Worker crash or duplicate completion | Expiring same-runner lease, monotonic claim attempt, heartbeat, stale-attempt rejection |
| Setup URL/code disclosure | Privileged transient projection, strict URL validation, ordinary-read redaction, terminal clearing |
| OAuth callback or integration secret retained by Cloud | One-time direct runtime/relay input channel; no ordinary control-plane fields or logs |
| Package action reads provider credentials | Separate UID/home, permission-restricted mounts, sanitized environment, denial probe |
| Package action controls kernel over loopback | In-container random kernel auth, authenticated handshake, token removed before provider launch |
| Runtime steals platform runner authority | Runner key absent from container; revision-scoped runner audit bridge capability only |
| Rotation deletes the serving credential | Preserve prior profile through candidate activation/drain; durable post-convergence GC |
| Revocation reports success while runtime serves | Process STOP reconciliation first and reject revoke/purge while any active/candidate/drain uses the ref |
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
validated end to end:

- typed release contract, archive verification, digest/signature verification,
  immutable promotion, rollback, and provenance
- deployment projects/environments and desired/observed reconciliation semantics
- audience authentication and route-role enforcement for every transport
- one-time handoff claims and destination-owned credential replacement
- provider-native setup for Codex, Claude, and OpenCode; one-time enrollment;
  secure provider-response input; direct runtime/relay setup transport; opaque
  bindings; runner affinity; readiness; rotation; revocation; and cleanup
- integration-secret enrollment through a direct runtime or external-vault
  adapter boundary without secret bytes entering Cloud
- package-action credential and kernel-isolation attacks against the production
  image, plus proof that the runner key is absent from container inspect/process
  state and runtime logs continue through the scoped bridge
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
| Credential runner | wrong account/runner/version/attempt, lease expiry/reclaim, heartbeat, stale completion, enrollment expiry/replay, source traversal, rotation overlap, revoke-before-stop, restart-safe GC |
| Runtime isolation | action UID cannot read sentinel credential, inspect protected process env, authenticate to kernel, or obtain runner key |
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
