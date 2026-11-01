# Release pipeline — MP-08 / MP-09 / MP-10 / MP-11

## One product version, exact component identities

OSS owns the kernel, CLI and relay; Cloud owns the hosted application. Cloud's
`releases/version.json` is the release intent, initially `0.1.0`. It is not a
release announcement: v0.1.0 identifies the first landing whose staging passes;
production approval promotes that same version later. Each later source pair
gets the next accepted patch version automatically. Raise the stable intent for
an incompatible minor release before landing it. An accepted version is never
rebound; an already accepted source pair is skipped. Versions follow pre-1.0
SemVer. A candidate's exact Git commits and content hashes
are its identity even while several candidates target the same version.

Cloud main is the coordinator. All three release workflows are manual-only
(`workflow_dispatch`); pushes, pull requests, schedules and repository events do
not start them. Nothing in this port configures credentials, registers a runner,
activates an environment or publishes an artifact. The first staging run requires
a separate operator session. Production remains separately gated.

When explicitly dispatched from main, Cloud builds its exact workflow commit with
the selected exact OSS commit (`.github/chariox-revision` remains the independent
existing test-fixture compatibility pin). The optional OSS main workflow manually
requests that same Cloud staging workflow with its exact revision. A single
concurrency group serializes staging and production, without cancelling an
in-flight deployment. Staging unions require explicit main workflow inputs.

## Manifest and storage

`releases/staging.json` and `releases/prod.json` on Cloud's dedicated
`release-manifests` branch are the small manifest directory. That branch is
created once by the owner; main's code protection is never bypassed for manifests.
The manifest files are deliberately absent until the first successful deployment;
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
Buildx builds, and digest-addressed GHCR outputs. Unsigned preparation compiles inside the shared build slot and copies binaries
before releasing it; CI consumes the Mac-signed handoff.
Neither source tests nor a local container build establishes live acceptance.

The staging workflow also accepts `workflow_dispatch` inputs `ref`, `oss_ref`
and optional exact `oss_revision`. Run the workflow from main. Only main,
`stagenew/*` and `staging/*` source branches are admitted, and an OSS revision
must belong to the selected branch. Build inputs are checked out separately;
workflow/deployment code remains main's. `chariox.release/v2` records each source
ref plus full SHA and binds the generated deployment kit's SHA-256. Unknown/v1
manifests fail closed; no previous automatic release was accepted under v1.
A validated branch staging cannot be promoted, deployed or published to prod.
A subsequent landing of the same source bytes still creates a main release.

For MP-08 / MP-09 / MP-10 / MP-11, the OSS builder also checks the Cloud archive
helper against reviewed SHA-256 bytes before executing any packaging code. The
accepted helper is Cloud `369e65eee7`'s `create-cloud-staging-release.sh`, which
runs the digest script from its own trusted directory. Older Cloud main tooling
that runs a digest script from the selected source fails with
`ERR_UNSUPPORTED_CLOUD_ARCHIVE_HELPER`; pin Cloud tooling containing the reviewed
helper before retrying. A clean trusted checkout and exact workflow revision
remain required. Any helper byte change requires a host-execution-chain review
and an explicit OSS hash update. The regression includes verbatim unsafe and
accepted Cloud helpers, their real extractor and digest, and malicious selected
source scripts for both Cloud components.

Cloud's `prepare-kit.mjs` invokes the manual stagenew kit builders for SQL
rehearsal, relay/native activation and rollback. The exact previous validated
manifest supplies each later migration/rollback base; initial bootstrap pins
come from public operator configuration. The runner imports the exact native
version/SHA handoff signed on the owner's Mac, verifies the approved public pin,
inventory signature and every hash, and publishes artifact carriers without
holding a signing key. The signed CLI must report the shared product version.
The image builder bounds Node only in build stages and preserves the deployed
runtime environment. Image/version labels and registry transport intentionally
differ from saved manual image config IDs; those IDs are never called registry
digests.

Before activation the shipped staging integration verifies artifacts and target
identity, invokes the existing retained-runtime automatic-shutdown preflight,
and runs the installed target backup wrapper. It retains the manual recovery
kit on the VPS, submits image-only input through the existing OpenShip adapter,
uses the normal web Prisma startup path, conditionally activates native/relay,
and runs the existing hosted smoke. Exact native rollback records are retained
before activation; failures invoke those scripts plus the pinned Cloud recovery
and never publish a manifest. A real live drill receipt with all checks remains
required. The production target integration is separate and still needs its
real rehearsal; staging-only recipes do not authorize production.

Cloud `scripts/release/rehearse.mjs` verifies a retained manual kit and plans
build/manifest/native/deploy work without SSH, pushes, deployment or secrets.
It executes schema rehearsal only against its own disposable Postgres. A
layout-only candidate comparison is explicitly labeled; absent final artifact
bytes, registry digests or signed inputs stay unresolved. Inventories cannot
receive acceptance or promotion. See Cloud `deploy/release/README.md` for the
handoff format and operator target fields.

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
release/atomic activation contract. The unsigned build tool prepares kernel/relay/CLI inputs for Mac signing; the full managed release also
requires the existing supervisor/App helpers and builder attestation. The ordinary
native bundle is never passed directly to a managed installer. Miguel's Mac
verifies inputs and invokes the existing signing/packaging step. Only public signature inventories and signed output hashes return to the
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
