# Deployed Workflows Production Services Registration Report

**Status:** Final external-services registration audit for the deployed-workflows plan

**Audit date:** 2026-07-16

**OSS implementation snapshot:** `4d503ba984c6782c4cc6c73a2acd74b0fd80cd2f`

**Cloud implementation snapshot:** `6ca6fffe05cf48177df947864d06e123949ae39e`

This report answers one narrow question: which external service registrations or
production resources are actually required before the first external production
deployment, after accounting for the implementation that exists now and the
local/designated-Hetzner acceptance paths. It does not declare the deployed-workflows
launch gate green. The current [platform plan](DEPLOYED_WORKFLOWS_AGENT_APP_PLATFORM_PLAN.html)
and [threat model](DEPLOYED_WORKFLOWS_THREAT_MODEL.md) still list implementation,
integration, recovery, privacy, and final-matrix evidence that cannot be replaced by
buying a service.

The audit uses committed implementation snapshots with a green final local web/TUI
matrix. Callback and egress integration are landed. The open production gates are live
provider, real DNS/TLS, guarded Hetzner, integration-secret, and operational evidence;
buying an external service cannot substitute for any of them.

## Decision Language

In this report, **registration** means creating or activating a vendor tenant, account,
production application, domain/zone, paid resource, or equivalent external dependency.
It does not mean setting an environment variable for a resource already controlled by
the operator.

The classifications are:

- **Required before first external production:** the capability must exist and have an
  accepted production owner, even when an existing account or self-hosted resource can
  supply it.
- **Conditional:** required only if the first launch includes the stated product mode,
  such as paid checkout, email delivery, or `hosted_container` credential custody.
- **Optional/later:** not needed for the bounded first production shape; adopt only when
  measured load, risk, or product scope crosses a recorded trigger.
- **Already abstracted:** a usable implementation boundary exists. This does not by
  itself prove that every deployed-workflows path uses it.
- **Temporary local/Hetzner substitute:** valid for implementation and acceptance
  evidence, but not automatically an accepted production operating model.

Cost categories are deliberately relative rather than quotes: **low** is normally a
small usage-based resource, **medium** has a meaningful always-on or operations baseline,
and **high** means HA, fleet, retention, or traffic can dominate platform cost.

## Executive Decision

| Candidate | Capability before first external production | Is a new external registration required? | Current disposition |
| --- | --- | --- | --- |
| Object/package storage | Durable package retention and restore are required; a separate object service is not | **No**, if production PostgreSQL blob storage is accepted for bounded volume | Optional/later; package-store boundary exists, but immutable v3 releases remain database-bound |
| Database | A production PostgreSQL resource, backups, restore, deletion, and recovery ownership are required | **No new vendor account** if an existing managed or self-hosted production PostgreSQL resource is approved; otherwise provision one | Required capability; PostgreSQL-specific implementation |
| Auth | A real production browser identity application and tenant policy are required | **Yes, a production IdP application/client must be registered or designated**; a new vendor account is needed only if the existing tenant fails isolation, locality, or commercial criteria | Required; implementation is Auth0-specific |
| Email/invitations | Copyable expiring claim and audience links are enough for a controlled first cohort | **No** for operator-delivered links; **yes** before promised transactional delivery or self-serve invitations | Conditional; no mail adapter exists |
| Custom hostnames/CDN/DNS/TLS | A controlled public default hostname, DNS, TLS, and separate publication origin are required | **A domain/zone and TLS path must be designated**, but no managed custom-hostname/CDN vendor is required | DNS verification is implemented; edge provisioning is not |
| Billing/metering | Admission budgets and a usage ledger are required; automated charging is not | **No** for a free or manually contracted pilot; **yes** before card checkout/subscription collection | Conditional charging; Stripe-shaped adapter, PostgreSQL usage ledger |
| Logs/metrics/traces | Retained operational logs, durable metrics/alerts, paging, and incident ownership are required | **No specific vendor** if an existing/self-hosted stack meets the operating requirements; otherwise provision one | Required operational capability; current built-ins are insufficient alone; managed tracing is optional/later |
| Secrets/KMS | Production custody and rotation for Arroba platform secrets are required | **No separate KMS registration** if approved PaaS secret injection and protected host storage meet the launch scope; **conditional yes** for hosted customer credentials if that model requires managed KMS/vault custody | Platform env/file secrets exist; no deployed-workflows KMS/vault adapter exists |
| Relay/compute | Public WSS relay, publication ingress, and mode-appropriate runner capacity are required | **No new infrastructure vendor account** if existing Hetzner capacity is production-qualified; dedicated resources may still need provisioning | Required; relay pools are abstracted, hosted compute is Docker/host-specific |

Therefore, no new external signup blocks local or designated-Hetzner acceptance. The
minimum first external production resource set is production PostgreSQL, production
browser identity, public DNS/TLS on a publication origin separate from Cloud, an
operated logs/metrics/alerting path, protected platform secrets, and relay/compute
capacity appropriate to the enabled runtime mode. Each can be supplied through an
existing operator account or self-hosted resource if the criteria below are met.

## Credential Boundary That Must Not Change

User-owned Codex, Claude, and OpenCode credentials remain in the official provider
CLI's native runtime stores. They are created or imported under isolated runner homes,
represented in Cloud only by metadata and `runtimeRef`, and mounted read-only into the
runtime. Cloud secret storage is for Arroba platform secrets, not for provider login
payloads.

The database model stores profile identity and lifecycle metadata, not credential bytes,
in [`DeploymentCredentialProfile`](https://github.com/mgutierrez09/arroba-cloud/blob/main/packages/db/prisma/models.prisma).
The worker prepares provider-specific `HOME`, `CODEX_HOME`, and `CLAUDE_CONFIG_DIR`
locations and secures profile trees in
[`deployment-credential-runner.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/worker/src/deployment-credential-runner.ts).
The publication runner mounts profiles read-only in
[`publication-runner.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/worker/src/publication-runner.ts).
The browser Vault is a client of the kernel-owned vault path, not a Cloud credential
database. This boundary also means Arroba does not need to register provider accounts on
behalf of users; users retain their provider relationship, billing, rotation, and
revocation.

## 1. Object And Package Storage

**Registration decision:** **Optional/later.** Do not register a separate object-storage
vendor before the first low-volume external production deployment if the selected
PostgreSQL service explicitly accepts the package-blob size, backup, restore, and cost
profile. Register or provision S3-compatible storage when package volume, database
growth, retention, restore time, object durability, or transfer throughput crosses an
agreed threshold.

**Why and abstraction status:**
[`package-store.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/publication/package-store.ts)
already selects S3-compatible, Prisma/database, or local-file implementations and
rejects local files in production. The S3 implementation supports signed `GET` and
`PUT`, a custom endpoint, path-style addressing, a prefix, and session credentials.
This is only a partial deployed-workflows abstraction today. Immutable v3 release
creation writes a `db://publication-releases/...` URI in
[`deployed-workflow-service.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/publication/deployed-workflow-service.ts),
and promotion requires `PublicationReleaseArtifact.archive` bytes in
[`deployed-workflows-repository.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/publication/deployed-workflows-repository.ts).
The database schema also contains both `PublicationReleaseArtifact.archive` and the
operational `PublicationPackageArchive.archive`. Selecting the S3 package store does not
move the immutable release path out of PostgreSQL.

**Decision criteria:** Decide from measured package count and bytes, database backup and
restore duration, retention/deletion requirements, regional durability, ingress/runner
throughput, egress charges, lifecycle support, and an exercised restore. Do not select a
vendor merely because it is named in the plan. The API needs private object storage with
reliable read-after-write behavior for the used key pattern; public buckets are not
required.

**Region and data residency:** Keep package objects in the initial EU service region,
preferably near the Cloud database and runner. Record backup/replica regions and prevent
unapproved cross-region replication. Packages can contain customer code and configuration,
so their residency and deletion terms are more sensitive than ordinary static assets.

**Security and credential scope:** Use a per-environment identity limited to `GetObject`
and `PutObject` on the configured bucket prefix. Do not reuse an account-wide object key
or generic AWS fallback credentials if a narrower identity is available. Keep the bucket
private, enable provider-side encryption and version/lifecycle policy, and separate
production from staging. The current interface has no delete operation, so retention and
garbage collection must be designed before object storage becomes authoritative.

**Estimated cost category:** **Low initially, usage-dependent.** Stored bytes, requests,
egress to runners, replicas, and retained versions are the cost drivers. Keeping blobs in
PostgreSQL shifts that cost into the database and its backups rather than eliminating it.

**Migration path:** Add object-backed immutable release persistence before migrating:
dual-write the verified archive, persist an object-backed `storageUri`, verify digest and
size on read, teach promotion/restart to dereference the URI, backfill existing releases
with digest verification, retain database-read fallback during rollback, then remove blob
columns only after restore and rollback drills. Telemetry exports can later use the same
object boundary, but must use a separate prefix and retention policy.

**Degraded behavior:** Object-storage failure must fail package upload, promotion,
restart, and any cold start that needs the archive. An already-materialized running
container may continue, but there must be no empty-package or local-file production
fallback. Database-backed package failure follows database failure behavior.

**Exact configuration surfaces:**

- Select database storage with `ARROBA_PUBLICATION_PACKAGE_STORE=database`; production
  also selects it implicitly when `DATABASE_URL` is present and no S3 bucket is set.
- Select object storage with `ARROBA_PUBLICATION_PACKAGE_S3_BUCKET`,
  `ARROBA_PUBLICATION_PACKAGE_S3_REGION`, `ARROBA_PUBLICATION_PACKAGE_S3_ENDPOINT`,
  `ARROBA_PUBLICATION_PACKAGE_S3_PREFIX`,
  `ARROBA_PUBLICATION_PACKAGE_S3_ACCESS_KEY_ID`,
  `ARROBA_PUBLICATION_PACKAGE_S3_SECRET_ACCESS_KEY`,
  `ARROBA_PUBLICATION_PACKAGE_S3_SESSION_TOKEN`, and
  `ARROBA_PUBLICATION_PACKAGE_S3_FORCE_PATH_STYLE`.
- The object adapter falls back to `AWS_REGION`, `AWS_DEFAULT_REGION`,
  `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `AWS_SESSION_TOKEN`.
- `ARROBA_PUBLICATION_PACKAGE_STORE_DIR` is local-only and is rejected as durable
  production storage. The local-browser counterpart is
  `ARROBA_LOCAL_PUBLICATION_PACKAGE_STORE_DIR`.
- There is no external-object configuration surface for immutable v3 release artifacts
  yet; their `storageUri` is assigned in code.

## 2. Database

**Registration decision:** **Required before first external production as a capability.**
A production PostgreSQL resource must be designated. A new database vendor account is not
required if the existing operator-controlled managed PostgreSQL or a production-qualified
self-hosted PostgreSQL instance satisfies the criteria. The currently documented Scalingo
addon is a staging resource and is not production evidence by itself.

**Why and abstraction status:**
[`packages/db/src/index.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/packages/db/src/index.ts)
requires `DATABASE_URL` and creates Prisma through the PostgreSQL adapter. The Prisma
schema declares `provider = "postgresql"`, and `/ready` executes `SELECT 1`. PostgreSQL is
the authority for Cloud identity, account, releases, deployments, domains, jobs, usage,
audit metadata, billing state, and currently package archives. This is a clean database
client boundary, but not a database-engine abstraction.

Customer workflow application data is intentionally outside this database. The first
product shape is stateless execution with customer-owned databases and connectors. A
managed per-app database product is optional/later and must not be implied by registering
the Cloud control-plane database.

**Decision criteria:** Evaluate EU region, availability target, encrypted storage and
backups, point-in-time recovery, tested logical/physical restore, deletion and backup
expiry, connection limits, maintenance windows, extension compatibility, observability,
support response, and latency to API/worker/runner. Include package-blob growth in the
capacity model while immutable releases remain database-bound.

**Region and data residency:** Place the primary in the initial EU region. Explicitly
record failover and backup locations, retention, and any support access outside the EU.
Keep customer identity, access grants, package code, audit metadata, and usage records in
the approved residency boundary.

**Security and credential scope:** Store the production connection string only in the
API/worker secret injection path and migration job. Require TLS where the network is not
private, restrict network sources, rotate credentials, and audit privileged access. The
current code exposes one `DATABASE_URL`; separate runtime and migration roles would need
deployment configuration or a code/config extension rather than pretending they already
exist.

**Estimated cost category:** **Medium baseline; potentially high with HA, PITR, and blob
growth.** Compute size, storage, I/O, backup retention, replicas, and connection pooling
drive cost.

**Migration path:** PostgreSQL portability is straightforward only after testing it:
apply migrations to the target, snapshot and restore or replicate, validate row and blob
digests, switch `DATABASE_URL`, run readiness and full workflow drills, retain a bounded
rollback window, then retire the source under the deletion policy. Moving package blobs
to object storage first can reduce later database migration time.

**Degraded behavior:** Database loss makes `/ready` return 503 and blocks Cloud API
authority, new sessions, deployment control, jobs, domain changes, and durable usage
admission. Already-running local processes or containers may survive briefly, and ingress
has bounded cached/spooled behavior, but neither is a substitute for database recovery.
Fail closed for authorization and new admissions.

**Exact configuration surfaces:**

- `DATABASE_URL` is consumed by the Cloud database client, Prisma migrations, API, worker,
  and the `Procfile` postdeploy migration.
- [`packages/db/prisma.config.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/packages/db/prisma.config.ts)
  and [`packages/db/prisma/schema.prisma`](https://github.com/mgutierrez09/arroba-cloud/blob/main/packages/db/prisma/schema.prisma)
  define the PostgreSQL migration surface.
- Local substitutes are PostgreSQL 16 in
  [`infra/docker-compose.yml`](https://github.com/mgutierrez09/arroba-cloud/blob/main/infra/docker-compose.yml),
  ephemeral PGlite in
  [`ephemeral-postgres.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/test-support/ephemeral-postgres.ts),
  and `ARROBA_LOCAL_DATABASE_URL` for the local browser service.
- Backup, PITR, replica, and restore settings are infrastructure-provider configuration;
  no repository environment variables currently define them.

## 3. Authentication And Browser Identity

**Registration decision:** **Required before first external production.** Designate a
production identity tenant and register a production web application/client with its
callback, logout, and origin policy. A new Auth0 account is not required if an existing
tenant provides acceptable production/staging isolation, EU residency, security controls,
and commercial terms. The documented staging issuer is a US Auth0 tenant, so it cannot be
silently assumed to satisfy an EU-locality decision.

**Why and abstraction status:**
[`packages/auth/src/index.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/packages/auth/src/index.ts)
loads and validates Auth0-specific web-session configuration and requires HTTPS outside
localhost. The database enum and identity mapper currently support only `AUTH0`, and
readiness explicitly requires Auth0 variables. This is not yet a provider-neutral OIDC
adapter despite using standard browser identity concepts.

**Decision criteria:** Decide whether one EU region requires EU identity-data locality,
whether staging and production need separate tenants, expected active users and
organizations, social/passwordless/enterprise federation scope, MFA and attack
protection, branded login requirements, exportability of provider subjects, audit logs,
support, DPA/subprocessors, and cost at expected user counts. Use those criteria before
considering a different broker.

**Region and data residency:** Resolve the current US-tenant exception before external
production. If EU locality is required, use an EU tenant/resource and document where
identity logs, backups, and support access reside. The Cloud database still stores the
mapped external subject, email, verification status, and profile metadata in its own EU
region.

**Security and credential scope:** Use a production-only confidential web client with
only the required callbacks, logout URLs, and web origins. Keep the client secret and
session secret in the API secret store, rotate them independently, require strong MFA for
tenant administrators, and restrict tenant admin roles. `ARROBA_CLOUD_DEV_AUTH_SECRET`
and test identity headers must be absent in production; there is no acceptable fail-open
identity mode.

**Estimated cost category:** **Low to medium at a small cohort, potentially high for
enterprise federation or higher active-user volume.** Tenant features and active users,
not API request volume alone, are the likely drivers.

**Migration path:** A vendor change requires more than swapping issuer variables. Add a
provider-neutral identity enum/mapper, establish an account-linking rule, support dual
issuers during migration, prevent subject collisions and account takeover, migrate active
sessions deliberately, and retain an audited rollback path. Never auto-link solely on an
unverified email address.

**Degraded behavior:** An IdP outage blocks new login and refresh. Existing browser and
Cloud sessions may continue only until their normal expiry/revalidation boundary. API
authorization must not accept synthetic identities or bypass verification during the
outage.

**Exact configuration surfaces:**

- Required production values: `AUTH0_ISSUER_BASE_URL`, `AUTH0_BASE_URL`,
  `AUTH0_CLIENT_ID`, `AUTH0_CLIENT_SECRET`, and `AUTH0_SESSION_SECRET`.
- Optional API audience: `AUTH0_AUDIENCE`.
- Related Cloud browser security: `ARROBA_CLOUD_WEB_URL`,
  `ARROBA_CLOUD_COOKIE_SECRET`, and `ARROBA_CLOUD_CSRF_HMAC_KEY`; the latter two must be
  configured together when overridden.
- Cohort/admin restrictions: `ARROBA_CLOUD_ALLOWED_BROWSER_EMAILS`,
  `ARROBA_CLOUD_ADMIN_ENABLED`, and `ARROBA_CLOUD_ADMIN_ROLES`.
- Nonproduction only: `ARROBA_CLOUD_DEV_AUTH_SECRET` and injected test-auth options.
- The current staging topology and US-tenant note are recorded in
  [`C5_HOSTED_DEPLOYMENT_MILESTONE.md`](https://github.com/mgutierrez09/arroba-cloud/blob/main/docs/C5_HOSTED_DEPLOYMENT_MILESTONE.md).

## 4. Email And Invitations

**Registration decision:** **Conditional.** No email service is required for a controlled
first external production cohort in which the builder or operator securely copies the
one-time claim or audience-invitation link to the recipient. Register an outbound email
service before the product promises transactional invitation delivery, resend, branded
mail, bounce handling, or self-serve invitation workflows.

**Why and abstraction status:**
[`deployment-access-service.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/publication/deployment-access-service.ts)
creates opaque claim tokens, stores only hashes, defaults to seven days, and permits a
five-minute to 30-day lifetime. Audience invitations use the same hash-and-return-once
pattern in
[`deployment-audience-service.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/publication/deployment-audience-service.ts).
The web app constructs copyable waiting-room links. There is no mail package, outbox,
provider adapter, SMTP configuration, bounce processing, or delivery status in the
current implementation.

**Decision criteria:** Require adequate delivery in launch geographies, EU data-processing
terms where needed, a DPA and subprocessor review, sender-domain verification, SPF/DKIM/
DMARC support, bounce and suppression handling, retry semantics, webhook verification,
rate limits, support, export/deletion behavior, and separation of staging and production.
The decision should follow cohort size and promised UX, not a vendor name from the plan.

**Region and data residency:** Prefer EU processing/storage where available and document
cross-border routing inherent in email. Send only the minimum recipient and invitation
context. Do not include prompts, output, package content, provider credentials, or secret
metadata beyond the bounded invitation URL.

**Security and credential scope:** Use a send-only API credential limited to one verified
sender/domain and environment. Keep account-administration and DNS credentials out of the
application. Do not log full invitation URLs or tokens. Delivery webhooks need signature
verification and replay protection.

**Estimated cost category:** **Low and usage-based** for an initial cohort; deliverability
operations and dedicated reputation features can raise it later.

**Migration path:** Add a provider-neutral notification interface backed by a PostgreSQL
outbox. Generate and hash tokens exactly as now, enqueue only the delivery request, make
send retries idempotent, persist delivery/bounce state, and keep copy-link as the degraded
fallback. A vendor migration then drains one outbox adapter and enables another without
changing claim semantics.

**Degraded behavior:** Email failure must not prevent claim or invitation creation.
Operators/builders can copy the link through the existing UI. The UI must show delivery
state honestly and must not claim that mail was sent when only a token was created.

**Exact configuration surfaces:** There are **no current email provider or SMTP environment
variables**. Current configuration is the request-level `expiresInSeconds` plus fixed
service bounds, and the web link uses the browser origin. `ARROBA_CLOUD_WEB_URL` is the
canonical Cloud web deployment URL but is not an email adapter. Any future mail variables
must be defined with the adapter rather than invented operationally.

## 5. Custom Hostnames, CDN, DNS, And TLS

**Registration decision:** **Required for the public edge, optional for a managed edge
vendor.** Before external production, designate an operator-controlled default hostname
and DNS zone, a separate publication origin, and a real TLS issuance/renewal path. The
customer controls DNS records for each custom hostname. Do not register a managed
custom-hostname/CDN service until domain count, WAF/DDoS requirements, certificate
operations, or global static latency justify it.

**Why and abstraction status:**
[`deployment-domain-policy.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/publication/deployment-domain-policy.ts)
normalizes hostnames and verifies the expected `_arroba-verification.<host>` TXT record
and CNAME target using the system DNS resolver. The API exposes a protected domain
approval endpoint suitable for a Caddy on-demand TLS ask check in
[`deployment-domains.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/routes/deployment-domains.ts).
Production also enforces a publication hostname distinct from `ARROBA_CLOUD_WEB_URL` in
[`node-server.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/node-server.ts).
There is no Caddyfile, DNS-provider API, ACME account configuration, CDN adapter, or edge
infrastructure definition in the Cloud repository. DNS-state abstraction exists; edge
provisioning does not.

**Decision criteria:** Select direct Caddy versus a managed edge from measured hostname
count, certificate issuance/renewal load, abuse controls, WAF/DDoS need, origin shielding,
HTTP/SSE/WebSocket/MCP compatibility, upload/body limits, global latency, log access,
support, and cost. Confirm the edge preserves host and transport semantics and does not
inspect or store runtime content beyond the approved metadata policy.

**Region and data residency:** DNS is globally distributed by nature; keep origin traffic
and application data in the initial EU region. Document where edge request logs and TLS
metadata are processed. Customer CNAMEs should point to a stable target that can move
between EU origins without requiring product-level hostname changes.

**Security and credential scope:** Use DNS credentials limited to the required zone and
records, or perform DNS changes manually for the first cohort. Scope ACME/DNS challenge
credentials separately from the Cloud API. Keep `ARROBA_DEPLOYMENT_DOMAIN_APPROVAL_TOKEN`
between the edge and API, restrict the approval endpoint at the network layer, rotate the
token, and do not expose it in browser-visible configuration. A domain must remain
unserved until verification and TLS state are ready.

**Estimated cost category:** **Low** for an owned domain, basic DNS, and direct ACME edge;
**medium to high and traffic-dependent** for managed custom hostnames, WAF, DDoS, or CDN
egress at scale.

**Migration path:** Keep the customer-facing hostname and CNAME target contract stable.
Add an edge provider adapter only when needed, import verified hostnames, issue and probe
certificates in parallel, switch DNS with bounded TTL, verify all supported transports,
and retain direct-origin rollback until certificate and route health are proven.

**Degraded behavior:** DNS or TLS failure must leave the affected hostname pending/failed
and must never downgrade to plaintext. A verified default hostname can remain available
when a custom hostname fails. Edge/control-plane failure must not cause unrelated hostnames
to route to another account or deployment.

**Exact configuration surfaces:**

- API/domain policy: `ARROBA_PUBLICATION_INGRESS_BASE_URL`,
  `ARROBA_PUBLICATION_DEFAULT_DOMAIN`, `ARROBA_CLOUD_WEB_URL`, and
  `ARROBA_DEPLOYMENT_DOMAIN_APPROVAL_TOKEN`.
- Nonproduction only: `ARROBA_DEPLOYMENT_DOMAIN_VERIFICATION_MODE=test`; code rejects it
  when `NODE_ENV=production`.
- Worker ingress: `ARROBA_PUBLICATION_INGRESS_PUBLIC_PROTOCOL` and
  `ARROBA_PUBLICATION_ROUTES_PATH`.
- There are no current CDN, DNS-provider, Caddy, ACME, certificate-file, or managed
  custom-hostname environment surfaces in Cloud. Those remain deployment configuration.
- The current two-surface Hetzner drill sets the ingress public protocol to `https` while
  reaching the worker through a local HTTP/tunnel path; it is not real deployed-workflow
  DNS/TLS evidence. The relay's existing Caddy/`sslip.io` staging edge is documented in
  [`C5_HOSTED_DEPLOYMENT_MILESTONE.md`](https://github.com/mgutierrez09/arroba-cloud/blob/main/docs/C5_HOSTED_DEPLOYMENT_MILESTONE.md).

## 6. Billing And Metering

**Registration decision:** **Conditional.** Do not require a payment-provider registration
for a free, internal, design-partner, or manually contracted first deployment. A live
payment account, product/price, and webhook registration are required before accepting
card payments or automatically granting subscription entitlements. The usage ledger and
admission budgets remain required even when no charge is made.

**Why and abstraction status:**
[`apps/api/src/billing/provider.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/billing/provider.ts)
has configured, disabled, and misconfigured provider modes. The package-level boundary in
[`packages/billing/src/index.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/packages/billing/src/index.ts)
defines checkout, portal, and idempotent webhook processing, but its provider type and
database enum currently support only Stripe. Deployment admission persists invocation
metadata and enforces concurrency, per-minute rate, and `daily_usage_units` in
[`deployments-repository.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/publication/deployments-repository.ts).
`usageUnits` currently defaults to one per invocation; it is an admission unit, not an
audited provider-token or provider-cost meter.

There is one production configuration inconsistency to resolve: the billing provider can
be intentionally disabled when all Stripe variables are absent, but
[`readiness.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/readiness.ts)
always marks missing Stripe variables unready. A free production pilot cannot be both
honestly unconfigured for billing and `/ready` without changing that readiness policy.
Production placeholder secrets are not an acceptable workaround.

**Decision criteria:** First decide whether the launch is free/manual or self-serve paid.
For paid launch, evaluate legal entity and merchant availability, supported countries and
currencies, tax/invoice/refund obligations, subscription versus metered pricing, webhook
reliability and reconciliation, customer portal needs, fraud/dispute handling, data
processing, support, and transaction economics. Validate the pricing model before
building a provider-cost pass-through meter.

**Region and data residency:** Keep Arroba entitlement and usage records in the EU Cloud
database. Send the payment provider only the minimum billing/customer metadata required.
Document unavoidable cross-border payment processing and retention. Never send prompts,
outputs, package data, or provider-native credentials to the billing service.

**Security and credential scope:** Keep live secret and webhook credentials in the API
secret store, separate test and live modes, verify webhook signatures against the raw
payload, preserve provider-event idempotency, restrict dashboard administrators, and
reconcile webhook state. The browser receives redirect URLs, not secret keys.

**Estimated cost category:** **Low fixed/operational baseline with variable transaction
fees** for subscription billing. A true metered-billing pipeline adds **medium** engineering
and reconciliation cost even if provider fees remain usage-based.

**Migration path:** For the existing provider, create production product/price and webhook
resources, load live credentials, test idempotency/reconciliation, and migrate no staging
customer identifiers into production. A provider change requires extending the provider
enum/model and running dual reconciliation. Usage billing requires a versioned rating
model and immutable billable events; do not repurpose the current one-unit admission
counter without that work.

**Degraded behavior:** Checkout and portal operations fail clearly when billing is disabled
or unavailable. Existing entitlements and deployment admission can continue from durable
Cloud state under an explicit grace/reconciliation policy; billing outage must not erase
usage or silently grant indefinite paid access. Daily admission budgets continue without
Stripe.

**Exact configuration surfaces:**

- `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, and `STRIPE_RETAIL_PRICE_ID` configure the
  current billing implementation and are all required by current readiness.
- Deployment usage and limits are database-backed fields, including
  `daily_usage_units`; they have no billing-vendor environment variable.
- The resilient worker usage path uses `ARROBA_PUBLICATION_USAGE_SPOOL_PATH`,
  `ARROBA_PUBLICATION_USAGE_SPOOL_SECRET`, and
  `ARROBA_PUBLICATION_USAGE_MAX_PENDING` in
  [`publication-ingress-usage-observer.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/worker/src/publication-ingress-usage-observer.ts).
- User provider consumption remains billed by the user's provider account through the
  provider-native CLI relationship; it is not a Cloud-stored credential or current Arroba
  billable meter.

## 7. Logs, Metrics, And Traces

**Registration decision:** **Required before first external production as an operational
capability, not as a particular vendor.** There must be durable collection, searchable
retention, actionable alerts/paging, access control, and an incident owner across Cloud,
relay, ingress, and runner. Existing PaaS log drains plus a self-hosted EU collector can
satisfy this if tested. Register a managed observability service only when no existing
stack meets those requirements. Distributed tracing is optional/later for the bounded
first deployment.

**Why and abstraction status:**
[`operational-telemetry.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/operational-telemetry.ts)
uses process-memory counters and a console audit sink. `/metrics` returns a JSON snapshot,
not a durable metrics backend. Fastify and workers log to stdout. Deployment invocation
metadata and deployment logs are in PostgreSQL, with a default 30-day policy in
[`deployment-operations-policy.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/publication/deployment-operations-policy.ts).
Retention pruning is opportunistic per environment and rate-limited to a 15-minute check
in
[`deployment-telemetry-retention.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/publication/deployment-telemetry-retention.ts),
not a guaranteed idle-environment scheduler. Inline telemetry exports are capped at
10,000 records per category and 10 MiB in
[`deployment-telemetry-export.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/publication/deployment-telemetry-export.ts).
No first-party OpenTelemetry exporter, managed log adapter, metrics scraper configuration,
or pager integration is wired into the deployed-workflows services.

**Decision criteria:** Evaluate EU ingestion/storage, DPA and support access, metadata
redaction, per-service scoped ingestion, searchable retention, metric cardinality,
dashboard/alert ownership, paging and escalation, uptime, exportability, ingest/retention
cost, and the ability to correlate request, deployment, revision, runner, and relay IDs
without capturing content. Add traces only when incident or latency evidence justifies
their data and cost.

**Region and data residency:** Keep logs, metrics, and traces in the approved EU region
unless an explicit transfer is accepted. Default to metadata-only capture. Prompt/output
content, attachments, provider credentials, runtime files, caller keys, and full invite
URLs are excluded from telemetry exports and must also be excluded from external sinks.

**Security and credential scope:** Give each component a write-only ingestion credential;
separate read, dashboard, retention, and admin roles. Redact authorization, cookies,
tokens, credential paths, query secrets, and provider CLI output before export. Restrict
support access and audit queries. Avoid placing a shared observability admin token on
runner hosts or runtime containers.

**Estimated cost category:** **Medium and volume/retention-dependent.** Logs are likely the
largest early driver; high-cardinality metrics and traces can raise the category quickly.

**Migration path:** Preserve structured stdout as the lowest common denominator, add a
collector/log drain, scrape or poll bounded metrics into a durable backend, wire alerts to
the on-call path, test loss/backpressure, and then add a provider-neutral OpenTelemetry
boundary if tracing is justified. Export retained data before vendor migration and keep a
dual-ingest window for alert comparison.

**Degraded behavior:** Telemetry-backend failure must not stop runtime traffic. Components
continue bounded local/stdout logging and expose health/readiness, while alerts explicitly
show an observability gap. Backpressure must drop or spool bounded metadata rather than
unboundedly consuming runner disk. Loss of paging is an incident, not a silent healthy
state.

**Exact configuration surfaces:**

- API routes: `GET /health`, `GET /ready`, and `GET /metrics`; Fastify logging is enabled
  by default in [`server.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/server.ts).
- Deployment retention and alert thresholds are persisted in each environment's
  `operationsPolicy`; there is no external telemetry-provider env surface.
- OSS kernel logging uses `ARROBA_LOG_DIR` and `ARROBA_LOG_LEVEL`; relay provenance can use
  `ARROBA_BUILD_COMMIT`.
- `ARROBA_PUBLICATION_CAPTURE_FAILED_CONTAINER_DIAGNOSTICS` is an opt-in runner diagnostic
  surface and requires a redaction review. Publication containers use hard-coded Docker
  log rotation of `max-size=10m` and `max-file=3`.
- `ARROBA_CLOUD_CSP_MODE` controls CSP report behavior but is not an observability backend.

## 8. Secrets And KMS

**Registration decision:** **Required for production secret custody; managed KMS/vault is
conditional.** Production Arroba platform secrets need controlled injection, encryption
at rest, rotation, access audit, and recovery before external production. Existing PaaS
encrypted environment storage plus restricted, encrypted runner-host files can satisfy a
bounded launch if the threat model and recovery drill accept them. For first external
`hosted_container` use with real customer provider credentials, either qualify encrypted
runner volumes and operator key custody or register/provision a KMS/vault/external-vault
path. `local_runtime` does not create hosted provider-credential custody.

**Why and abstraction status:** Cloud stores hashes for opaque access tokens and metadata
plus `runtimeRef` for deployed-workflow credential profiles. Provider-native enrollment
materializes official CLI stores only under the worker's private profile root, applies
private directory/file modes, and mounts them read-only. Platform secrets are ordinary
environment variables or private runner files. There is no Cloud deployed-workflows KMS,
envelope-encryption, external-vault, or managed-secret adapter. The OSS kernel does own a
separate local credential-vault boundary in
[`apps/kernel/src/secret/vault.rs`](../apps/kernel/src/secret/vault.rs); it must not be
reinterpreted as Cloud provider-secret custody.

**Decision criteria:** Base the decision on runtime mode, customer credential sensitivity,
host compromise model, encryption-at-rest evidence, key/volume separation, tenant
isolation, rotation/revocation latency, runner portability, backup/restore, auditability,
operator access, break-glass procedures, EU key residency, availability, and cost. A
managed KMS alone does not solve plaintext use on a compromised Docker host.

**Region and data residency:** Keep platform keys, encrypted provider profile volumes, and
backups in the approved EU region. Document whether key metadata or support access leaves
the region. Do not replicate provider profiles across runners or regions until the product
has an explicit customer-approved portability model.

**Security and credential scope:** Use separate secrets per environment and purpose,
service-scoped injection, least-privilege host access, private files, rotation overlap,
and audited break-glass. The API must not receive provider credential bytes. The worker
must not receive database, Auth0, Stripe, or DNS-admin credentials it does not use. Runtime
containers receive only the selected provider profile and narrow signed capabilities,
never the runner key or Cloud platform secret set.

**Estimated cost category:** **Low** for PaaS secret injection or a small number of KMS
keys/operations; **medium** for an operated vault, HSM-backed policy, multi-runner profile
portability, or customer-managed-key features.

**Migration path:** Inventory each platform secret, establish per-service secret
references, load and rotate through the deployment platform without logging values, then
remove direct operator copies. Keep provider-native files at the runner. If portability is
needed, add a direct runner-to-external-vault adapter with envelope encryption and
attested destination identity, dual-read during migration, verified deletion, and no
secret-byte transit through Cloud.

**Degraded behavior:** Secret/KMS/vault outage blocks new signing, enrollment, rotation,
or container starts that require unavailable material. Existing in-memory or mounted
credentials may continue only for their normal bounded lifetime and policy. Fail closed;
do not fall back to development defaults, another tenant's profile, or Cloud plaintext.

**Exact configuration surfaces:**

- Cloud platform secrets include `DATABASE_URL`, `AUTH0_CLIENT_SECRET`,
  `AUTH0_SESSION_SECRET`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`,
  `ARROBA_CLOUD_RELAY_TOKEN_SECRET`, `ARROBA_CLOUD_COOKIE_SECRET`,
  `ARROBA_CLOUD_CSRF_HMAC_KEY`, `ARROBA_PUBLICATION_PLATFORM_RUNNER_KEYS_JSON`,
  `ARROBA_PUBLICATION_MACHINE_AUTH_SECRET`,
  `ARROBA_PUBLICATION_CALLER_CLAIMS_SECRET`,
  `ARROBA_DEPLOYMENT_DOMAIN_APPROVAL_TOKEN`, and the package-store access credentials.
- Runner/platform authentication uses `ARROBA_PUBLICATION_RUNNER_KEY`; the usage spool can
  use `ARROBA_PUBLICATION_USAGE_SPOOL_SECRET`.
- Provider-native material is located by `ARROBA_PUBLICATION_CREDENTIAL_PROFILES_DIR`,
  `ARROBA_PUBLICATION_CREDENTIAL_PROFILE_SOURCES_DIR`, and
  `ARROBA_PUBLICATION_CREDENTIAL_SOURCES_JSON`. These point to runner-local paths; they are
  not credential-byte environment variables.
- OSS relay verification uses `ARROBA_RELAY_SCOPED_HMAC_SECRET`; local shared-token mode
  uses `ARROBA_RELAY_TOKEN`.
- There is no current KMS, managed-vault, or external-secret-provider env/config surface.
- Development-only bypasses such as `ARROBA_CLOUD_DEV_AUTH_SECRET`,
  `ARROBA_PUBLICATION_ENABLE_DEV_STUB`,
  `ARROBA_PUBLICATION_SELF_HOST_DEV_ALLOW_LEGACY_CREDENTIAL_SOURCE_FALLBACK`, and
  `ARROBA_RELAY_ALLOW_OPEN_ACCESS` must be absent from production.

## 9. Relay And Compute

**Registration decision:** **Required before first external production as capacity and
operations, but not necessarily as a new vendor account.** Provide a public Caddy-fronted
`wss://` relay, separate publication ingress, and runner capacity for the launch modes.
The existing Hetzner account/host may be used for acceptance and may be used for production
only after explicit capacity, isolation, location, recovery, monitoring, and ownership
qualification. Provision dedicated production resources when staging co-tenancy or
failure-domain risk is unacceptable; a new provider account is still not inherently
required.

**Why and abstraction status:** Cloud relay allocation supports a static pool or a stable
hash across configured pools in
[`relay/realm.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/api/src/relay/realm.ts).
The OSS relay refuses unauthenticated non-loopback startup unless explicitly overridden,
supports scoped HMAC verification, draining, and revocation sync in
[`apps/relay/src/main.rs`](../apps/relay/src/main.rs) and
[`apps/relay/src/config.rs`](../apps/relay/src/config.rs). The publication worker directly
operates Docker, host files, networks, credential profiles, and ports through
[`apps/worker/src/cli.ts`](https://github.com/mgutierrez09/arroba-cloud/blob/main/apps/worker/src/cli.ts)
and `publication-runner.ts`. Relay allocation is abstracted; compute scheduling/IaaS is
not. The egress orchestration is committed and passes the final local matrix, but remains
a production launch gate until its fail-closed behavior passes on designated Hetzner
capacity.

**Decision criteria:** Measure sustained connections, packets/bytes, invocation rate,
container CPU/memory/storage, image pull/start latency, build and restart time, bandwidth,
and control-plane outage tolerance. Decide dedicated versus shared hosts, single-host
versus HA, load balancer timing, private network, DDoS protection, patching, backup,
Docker isolation, egress enforcement, capacity headroom, on-call, and recovery from actual
drill data. Do not add a load balancer or second provider before the failure/capacity
evidence calls for it.

**Region and data residency:** Start with one documented EU region as the plan requires.
The existing staging record names a Hetzner IP but not its datacenter in repository
configuration; production must record the actual relay, ingress, runner, volume, image,
and backup locations. Keep Cloud-to-runner and provider-profile traffic within the
approved region where possible.

**Security and credential scope:** Cloud signing and relay verification secrets are
environment-specific; the relay receives no Cloud database, Auth0, Stripe, package, or
provider credential. Use WSS publicly, restrict relay/runner management ports, scope
runner keys and account identity, isolate staging and production, harden the Docker host,
and mount only narrow read-only provider profiles/capability files into containers. The
relay remains encrypted transport and must not inspect runtime terminal payloads.

**Estimated cost category:** **Medium to high and primarily always-on.** VM/runner size,
idle headroom, storage, images, bandwidth, load balancers, replicas, and operational labor
drive cost. Hosted containers are materially more expensive than `local_runtime` relay
coordination.

**Migration path:** Add relay pools/endpoints, drain old endpoints, and move account
allocations under the existing allocator while keeping one issuer contract. For compute,
introduce a named runner/scheduler backend before adding another IaaS, register new runners,
preload immutable images, copy only approved encrypted credential profiles, shift jobs in
a canary, drain containers, and then remove the old host. Preserve the home-kernel
authority and existing relay protocol throughout.

**Degraded behavior:** Relay loss blocks new remote attachment and routed terminal traffic;
home kernels and local sessions can continue locally. Runner loss blocks starts,
restarts, promotions, and hosted invocations on that capacity. An already-running
container may survive a bounded Cloud outage through cached routes and usage spooling,
but stale bounds must eventually fail closed and reconcile. There is no cross-provider or
unrestricted-network fallback.

**Exact configuration surfaces:**

- Cloud relay allocation/signing: `ARROBA_CLOUD_RELAY_URL`,
  `ARROBA_CLOUD_ISSUER_ID`, `ARROBA_CLOUD_RELAY_TOKEN_SECRET`,
  `ARROBA_CLOUD_RELAY_POOL_ID`, `ARROBA_CLOUD_RELAY_REGION`,
  `ARROBA_CLOUD_RELAY_SIGNING_KEY_REF`, `ARROBA_CLOUD_RELAY_POOLS`, and
  `ARROBA_CLOUD_RELAY_ENDPOINTS`.
- OSS relay process: `ARROBA_RELAY_HOST`, `ARROBA_RELAY_PORT`,
  `ARROBA_RELAY_SCOPED_ISSUER`, `ARROBA_RELAY_SCOPED_HMAC_SECRET`,
  `ARROBA_RELAY_REVOCATION_URL`, `ARROBA_RELAY_REVOCATION_REALM`,
  `ARROBA_RELAY_REVOCATION_INTERVAL_SECS`, `ARROBA_RELAY_DRAINING`, and
  `ARROBA_RELAY_OUTGOING_QUEUE_CAPACITY`. `ARROBA_RELAY_TOKEN` is the simpler
  local/self-hosted shared-token path.
- Runner control: `ARROBA_CLOUD_API_BASE_URL`, `ARROBA_PUBLICATION_ACCOUNT_ID`,
  `ARROBA_PUBLICATION_RUNNER_KEY`, `ARROBA_PUBLICATION_RUNNER_LABEL`,
  `ARROBA_PUBLICATION_RUNNER_ROOT`, `ARROBA_PUBLICATION_IMAGE`,
  `ARROBA_PUBLICATION_RUNNER_PORT_START`, and `ARROBA_PUBLICATION_DRAIN_MS`.
- Runner ingress/audit: `ARROBA_PUBLICATION_INGRESS_BASE_URL`,
  `ARROBA_PUBLICATION_AUDIT_BRIDGE_BIND_HOST`,
  `ARROBA_PUBLICATION_AUDIT_BRIDGE_PORT`,
  `ARROBA_PUBLICATION_AUDIT_BRIDGE_ADVERTISED_BASE_URL`,
  `ARROBA_PUBLICATION_ROUTES_PATH`,
  `ARROBA_PUBLICATION_CONTROL_PLANE_STALE_MS`, and the usage-spool variables listed above.
- Hosted egress consumes `ARROBA_PUBLICATION_EGRESS_IMAGE`,
  `ARROBA_PUBLICATION_EGRESS_UPLINK_NETWORK`, and
  `ARROBA_PUBLICATION_EGRESS_HOST_FIREWALL_HELPER`; production must set, protect, and
  validate these as part of the runner configuration.
- Public TLS termination and host provisioning are outside the repository today.

## Configuration Drift Found During Audit

The provisional
[`C1_DEPLOYMENT_TOPOLOGY.md`](https://github.com/mgutierrez09/arroba-cloud/blob/main/docs/C1_DEPLOYMENT_TOPOLOGY.md)
lists `ARROBA_PUBLICATION_RUNNER_TOKEN_SECRET`, `ARROBA_PUBLICATION_RUNNER_ID`, and
`ARROBA_PUBLICATION_STAGING_CREDENTIAL_PROFILE`. Repository search finds no code consumer
for those names. The implemented runner contract instead uses
`ARROBA_PUBLICATION_RUNNER_KEY`, `ARROBA_PUBLICATION_ACCOUNT_ID`, and the credential path
surfaces listed above. Do not put values into the document-only names or treat them as a
security boundary.

Conversely, the implemented package S3, domain approval, machine/caller claims, usage
spool, credential roots, audit bridge, and current egress variables are absent or
incomplete in that topology document. Production configuration must be generated from
code-owned validation or a corrected deployment manifest before launch, not by copying
the provisional list.

The same drift exists in billing semantics: a disabled billing provider is implemented,
but readiness still requires all Stripe values. That unresolved policy decision is a code
gate for a free external pilot, not a reason to create fake production billing secrets.

## Local And Designated-Hetzner Acceptance

No external signup is a prerequisite for implementation, local acceptance, or the
designated-Hetzner acceptance matrix:

| Candidate | Nonblocking acceptance substitute |
| --- | --- |
| Object/package storage | Local file store outside production, PostgreSQL package blobs, or optional local MinIO/S3-compatible storage |
| Database | Docker PostgreSQL 16, ephemeral PGlite for focused tests, `ARROBA_LOCAL_DATABASE_URL`, or the already-provisioned staging database |
| Auth | Local signed/test identity and guarded nonproduction device approval; production-only Auth0 behavior remains a separate gate |
| Email/invitations | Copy the expiring claim or audience link; a local mail sink may test a future adapter |
| DNS/TLS/CDN | `.localhost`, hosts-file names, `sslip.io`, direct ports/tunnels, and Caddy on the designated Hetzner host; no managed CDN is needed |
| Billing/metering | Disabled/test billing path plus PostgreSQL usage and admission limits; placeholders are nonproduction only |
| Logs/metrics/traces | Structured stdout, JSON `/metrics`, PostgreSQL operational metadata, local files, and retained drill artifacts |
| Secrets/KMS | Temporary private files, local/PaaS env injection, encrypted operator-controlled volumes, and provider-native runtime profiles |
| Relay/compute | Local `ws://` relay/Docker or the existing Caddy-fronted Hetzner WSS relay and Docker host |

The final local run used its own relay, kernel, Cloud API/server, publication ingress,
probed ports, and temporary roots. It passed six transport cases, two Agent App cases,
ten disruption cases, inspected web/TUI evidence, bounded load, and two-surface lifecycle
operations with zero owned processes and all run-owned ports/roots released afterward.
The latest managed-slice Hetzner preflight correctly stopped before mutation: available
disk was 3,286,196,224 bytes against a 3,489,660,928-byte effective requirement. Cleanup
was green and preserved unrelated containers, images, processes, ports, and roots. This is
a capacity gate for that acceptance case, not a reason to register another vendor.

These substitutes prove code and protocol behavior. They do not waive the external
production requirements for backup/restore, real DNS/TLS, production identity, monitored
operations, secret custody, capacity, residency, and incident ownership.

## External Production Gate Separate From Registration

The final registration set cannot be used to claim launch readiness. The callback and
egress paths and final local matrix are committed and green. The current threat model
still requires live protocol-241 provider and proof-of-possession exercise, hosted denial
of legacy unrestricted egress, real custom-host DNS/TLS validation, designated Hetzner
hosted-container, remote-machine, managed-slice and collaborator matrices,
integration-secret enrollment through a runtime/external-vault boundary, and
backup/retention/deletion/privacy/incident/recovery evidence.

A production resource should not be purchased to mask one of those gates. Conversely,
when a required capability can be supplied by an existing account or self-hosted resource,
the absence of a new vendor signup is not a blocker.

## Unresolved Choices Requiring An Owner

| Choice | Decision needed before first external production | Registration impact |
| --- | --- | --- |
| Launch runtime modes | Is the first production offer `local_runtime` only, or does it include `hosted_container` with real customer provider credentials? | Hosted mode can trigger encrypted-volume/KMS/vault and dedicated-runner requirements |
| PostgreSQL home | Promote/provision an existing EU managed PostgreSQL resource or operate a dedicated PostgreSQL instance; define backup/PITR/restore and blob thresholds | New vendor registration only if no approved existing resource is used |
| Identity locality/isolation | Can an existing Auth0 tenant support production isolation and the chosen EU locality policy, or is a separate EU tenant required? | At least a production app registration; possibly a new tenant/account |
| Public namespace and edge | Which owned default domain/CNAME target is used, who controls DNS, and is direct Caddy the accepted first production edge? | Domain/zone resource required; CDN/custom-host vendor remains optional |
| Commercial launch | Free/manual pilot or automated paid subscription? | Paid mode triggers live payment registration; free mode requires readiness-policy correction |
| Invitation promise | Copy-link controlled cohort or transactional email delivery? | Email registration only for the latter |
| Operations stack | Which existing/self-hosted or managed EU log/metrics/paging stack owns retention and on-call? | Managed registration only if no existing stack qualifies |
| Package threshold | Maximum package bytes/count, database share, restore duration, and retention allowed before object storage | Object registration deferred until threshold or prior design decision |
| Relay/compute production shape | Reuse and qualify existing Hetzner resources or provision isolated production hosts; define HA and load-balancer trigger | Provisioning may be required without a new provider account |
| Release signing authority | Does production require an external signing authority beyond package digests, and who owns key rotation/revocation? | Can trigger KMS/signing registration; remains optional until policy says required |
| Configuration authority | Which validated manifest replaces the provisional C1 env list, and how is billing-disabled readiness represented? | Prevents unnecessary/stale registrations and fake secrets |

Once these choices are recorded, the production registration action list should contain
only resources selected by those decisions. Until then, the correct action is to continue
local/Hetzner acceptance and close implementation evidence without making any new signup a
gate.
