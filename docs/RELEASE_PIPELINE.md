# Release pipeline — MP-07 / MP-11

## One product version, exact component identities

OSS owns the kernel, CLI and relay; Cloud owns the hosted application. Cloud's
`releases/version.json` is the release intent, initially `0.1.0`. It is not a
release announcement: v0.1.0 becomes released only after its staging validation
and production approval. Versions follow pre-1.0 SemVer (incompatible changes
increase the minor version). A candidate's exact Git commits and content hashes
are its identity even while several candidates target the same version.

Cloud main is the coordinator. A Cloud main push builds its own exact commit
with its reviewed `.github/chariox-revision` OSS pin. An OSS main push dispatches
an `oss-main` event to Cloud; the receiver verifies that the supplied full commit
belongs to OSS main. Both paths build a complete candidate with one version.
A single concurrency group serializes staging and production, without cancelling
an in-flight deployment. Lane branches and `[skip ci]` commits never publish.

## Manifest and storage

`releases/staging.json` and `releases/prod.json` in Cloud are the small manifest
directory. They are deliberately absent until the first successful deployment;
a placeholder must never look deployable. Candidate manifests are retained as
workflow artifacts. Git history retains every deployed manifest for rollback.
Every manifest records the version, full OSS/Cloud/relay commits, registry
**manifest** digests for web, services, OSS and relay images, the Cloud archive
SHA-256/source digest, native bundle SHA-256 and relay binary SHA-256. An image
config ID, mutable tag or abbreviated commit is not an immutable deployment pin.
OCI version/revision labels identify each component. Registry caches accelerate
builds but do not supply release identity. Never put credentials into contexts,
Docker build arguments, caches, manifests, artifacts or logs.

The manifest tool distinguishes inventory (possibly unresolved), candidate
(complete immutable pins), and validated staging (candidate plus a matching live
validation receipt). Production promotion accepts only the exact SHA-256 of a
validated staging manifest selected from an immutable Cloud Git commit. It copies
the same pins and version; it never rebuilds or resolves a tag. Main-head
changes after approval cannot change the selected candidate.

## Build and staging

The new workflows are dormant until `RELEASE_PIPELINE_ENABLED=true`,
`RELEASE_RUNNER_READY=true` and the documented credentials/settings are present.
An inexpensive hosted preflight prints missing **names**, never values, and
skips work before scheduling a missing self-hosted runner. No PR workflow runs
on the builder. A dedicated non-root b3 runner uses rootless Docker, bounded
Node memory and the coordinator's Cargo-slot wrapper. The installer defaults to
a plan; registration and service start are separate operator actions.

Builds use exact Git archives and the existing Cloud Dockerfiles/markers, cached
Buildx builds, and digest-addressed GHCR outputs. Native compilation happens
inside the shared build slot; binaries are copied before the slot is released.
Neither source tests nor a local container build establishes live acceptance.

The staging deployment plan preserves the manual prep order: Cloud archive and
web/services images; migration comparison/rehearsal; native bundle and relay;
verification; Cloud activation; conditional relay/native activation; health,
revision and fresh-target validation. The previous manifest is retained before
activation. Downloaded artifacts are verified before use. Deployment submits
prebuilt image digests to an existing OpenShip project, with its already configured
runtime environment, and checks the scan contains those exact images without
build instructions. Services-image migrations run through the target's configured
OpenShip pre-deploy command; no database/provider credentials go to the runner.

The older `deploy-cloud-staging-release.sh` builds the web image from its source
archive. Calling that unchanged is **not** digest promotion. The new OpenShip
adapter uploads an image-only Compose input instead. Its plan comparison records
this deliberate difference, registry pulls replacing tar transfer/load, and the
retained activation/validation gates. The target projects must have pre-deploy
migration, persistent volumes, routing, rollback and deployment health checks
configured before enablement. These are one-time settings, not secrets to invent.
The existing native relay service is not silently converted to a container:
relay/native activation remains an explicit configured target integration gate.

A successful OpenShip status alone does not update staging.json: a separate live
receipt must bind all observed digests/hashes and required acceptance checks to
the exact candidate hash. The workflow stops before manifest publication without
that receipt. Local dry runs cannot mint it.

## Production, rollback and signing

The manual production workflow selects a validated staging manifest by Cloud Git
commit and SHA-256. Its deploy job uses the `production` GitHub environment,
required reviewers, main-only deployment rules and no bypass. Required reviewers
for a private repository require a supporting GitHub plan; if unavailable, keep
production disabled until an approved protected public orchestrator or equivalent
approval mechanism exists. Approval occurs before environment secrets are exposed.
See [GitHub environment protection](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments).

Production reuses all staged digests and immutable artifacts, runs target health
and identity checks, then records prod.json. Rollback selects a prior validated
manifest through the same approval and deploy path. Code rollback does not undo
Prisma migrations: require backward-compatible schema changes and a verified
backup/restore plan before destructive migrations. Retain previous images,
archives, native bundles and signed release material; never prune shared caches.

MP-07 managed kernel installation still uses the existing signed immutable
release/atomic activation contract. CI produces unsigned build inputs; Miguel's
Mac verifies their hashes/provenance and invokes the existing signing/packaging
step. Only public signature inventories and signed output hashes return to the
manifest. Durable release/builder signing keys never enter GitHub, b3, images,
runner state or deployment targets. A pending signature cannot authorize managed
kernel activation. A product version never changes an existing signed artifact.

## Validation rows (MP-07 / MP-11)

- `cicd/RP-01`: local manifest/build/guard/plan regression and command comparison.
- `cicd/RP-02`: main-to-staging real image deployment, migrations, native/relay
  integration and matching live validation receipt, followed by manifest update.
- `cicd/RP-03`: reviewer-gated production promotion of identical digests and a
  successful rollback to a retained validated manifest.

RP-01 is preparation evidence only. RP-02/RP-03 require configured real targets,
GitHub credentials/settings and explicit enablement; this round must run no CI,
register no runner and contact no staging or relay target. No MP item is closed
by this preparation round.
