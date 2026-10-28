# Apps v2 contracts (frozen by R0)

The shared seams of Apps v2 Stage 1 (`spikes/wasm/APPS_V2_PLAN.md` §4). The Rust
types are the source of truth; this page says what each one fixes and which
existing kernel mechanism it reuses. Canonical vectors live next to the tests:
`packages/app-package/tests/vectors/v2/` and `packages/app-runtime/tests/vectors/`.
A vector failing means an encoding or rule changed. Regenerate only for a
deliberate contract change (`CHARIOX_UPDATE_VECTORS=1`) and review the diff.

## 1. Package v2 (`chariox_app_package::v2`)

- Schema `chariox.app.v2`, contract version 2, RFC 8785 JSON, canonical USTAR,
  Ed25519. Signing subject: canonical `{schema:"chariox.package-signature.v2",
  manifest, inventory}`; inventory schema `chariox.integrity.v2`. A v1 and a v2
  signature can never verify as the other.
- `ManifestV2` adds `world` (exact, `chariox:app/app@1.0.0`), `runtime
  {engine:"wasm", component}`, `build {language, toolchain, sdkVersion}`,
  `uiActions` and `requiredFeatures`; it drops `sdkVersion`/`minKernelProtocol`.
  `requiredFeatures` must equal the features the content uses (`wasm-component`,
  `ui-actions`, `agent-panel`, `connections`, `directed-events`), so a package
  cannot understate them. The kernel states which features it supports; no
  protocol number is copied into a package.
- `schemas/ui-actions.json` (`UiActionDeclaration`): name, description, input
  and optional output schema, `effect` `read | write | approval`, and an
  `approvalReason` for, and only for, `approval`. Backend `actions` keep their
  protected effect routes unchanged; an effect class grants no capability.
- Signature slots: `signatures/publisher.sig` plus up to 8
  `signatures/reviews/<reviewer-key-id>.json` (`chariox.package-review.v1`, at
  most 16 KiB, canonical JSON, opaque `attestation`). Reviews are excluded from the
  inventory and returned *unverified*; they grant nothing. Any other
  `signatures/` entry or review schema is refused (`IncompatibleContract`).
  Packages without reviews are valid on every route.
- Three digests (`PackageDigests`): `content` (the signing subject, which holds
  every payload digest), `publisher_archive` (canonical archive with review
  entries removed) and `delivered` (the exact bytes). Review changes alter only
  `delivered`; changed UI alters `content` and `publisher_archive`.
- Error mapping reuses existing `ErrorCode`s: unknown world or signature
  format → `IncompatibleContract`; missing kernel feature →
  `IncompatibleProtocol`; `.cwasm`, `.so` and similar → `UnsupportedFeature`.
- Reuses: `archive.rs`, `json.rs`, inventory/signature readers and
  `verify_signature` (now shared with v1), `Declarations` (now validated against
  `Capabilities`, not a v1 manifest), `TrustedPublisher` from the enrolled store.

## 2. App ID, publisher and principals (`v2::identity`)

- `AppId` keeps the v1 reverse-DNS syntax. `AppId::namespaces()` lists the
  namespaces whose verified owner may hold it: `io.github.<login>.<app>` → that
  GitHub account only; otherwise each domain whose reversed labels start the ID
  (`com.example.tools.todo` → `tools.example.com`, `example.com`). Local IDs are
  never checked against them.
- `SignerPrincipal::{Dev, Publisher}`; the reserved publisher id
  `local_dev` (`DEV_PUBLISHER_ID`, not a possible DNS label or GitHub login) is the
  kernel dev identity.
- `registration_outcome(bound, signer)` → `Bind`, `Preserve`, or a refusal
  (`DevIdentity`, `PublisherConflict`). It never overwrites a binding.
- Reuses: `Publisher`, `publisher_trust` enrollment (the dev identity is never
  enrolled there).
- Not frozen here (I1): DNS/website challenge names, transfers, confusables.

## 3. Channels (`installation::channel`)

- `InstallationChannel {dev, release}` is the installation; `UpdateChannel
  {stable, preview}` is an optional subscription of a release installation;
  `AcquisitionSource` is provenance only.
- `resolve(installations, selector)`: owner + App ID + channel + dev Room.
  Automatic selection uses the dev copy only in its own Room, else release.
  Several matches are `Ambiguous`, never first-match. A dev installation has
  exactly one Room and no subscription (`InstallationRef::validate`).
- `check_immutable_version`: `(publisher, App ID, version)` names immutable
  content; different bytes at it fail. Dev saves are identified by digest.
- Reuses: the existing installation id as runtime/data anchor.

## 4. Activation and approval (`installation::activation`)

- `activation_entry`: no installation → `BeginAppInstall`; otherwise
  `BeginAppUpdate` with the expected generation. Every source uses it.
- `check_source`: `dev_push` ⇔ dev channel ⇔ dev principal; every other source
  ⇔ release channel ⇔ publisher principal.
- `AuthoritySet`: sorted atoms for network (origin+method), clipboard, files,
  workflows, agents, connections, effect routes, and `critical-route` /
  `verified-route` for critical routes. Action names, texts and schemas are not
  authority. `approval_requirement(approved, candidate)` → `Reuse` unless the
  candidate adds atoms (`Prompt{added}`); first installs always prompt.
  `commit_gate` repeats the check on the kernel writer against the current
  approved set and publisher-trust revision.
- Reuses: `CapabilityApproval`/`CapabilityDecision`, the stage/prepare/commit
  journal, `RuntimeInteraction` for the single prompt, `publisher_trust` revisions.

## 5. WIT versioning (`v2::world`)

- `WorldId` = `<ns>:<pkg>/<world>@<semver>` (no pre-release). `check_world`
  against `SupportedWorld {name, major, minor}`: older minor links, newer minor
  or unknown major is refused, patch is ignored; two entries for one name give
  the two-major deprecation window. `APP_WORLD` and `MIGRATION_WORLD` are the
  fixed names. R1's [`app-wit` contract](../packages/app-wit/README.md) supplies
  the exact registered worlds, binary import/export and interface-member
  allowlist, structural checks, compatibility vectors and deprecation window.
  Its migration world exports `migration-handlers` to avoid a WIT namespace
  collision with the world name.
- `v2::verify` checks actual signed backend and migration components with that
  same WIT checker after inventory verification. The generic `check_world`
  generation query does not admit an unregistered exact world or import.
  Library compatibility must additionally check kernel features, contract/resource
  policy, worker availability and platform containment, as local install does.
- The two package signature vectors use `wit:<exact-world>` payload sources
  built by the pinned 0.262.0 test toolchain, rather than component placeholders.
  Their signing rules are unchanged; inventory/signature/digest bytes change
  because the fixture payloads are now real components.

## 6. AOT cache (`release_store::aot_key`)

- `AotCacheKey` = component digest, wasmtime version, worker build, engine
  config digest, target triple, CPU-feature policy; `id()` is the digest of its
  canonical JSON. `AotProvenance` records the key, source release and artifact
  digest; `usable_by(running)` is key equality, so any worker/wasmtime update
  invalidates entries. `.cwasm` is never accepted in a package.
- Reuses: the verified release store for the retained component bytes.

## 7. Data and migrations (`managed_state::data_contract`)

- Data belongs to the installation (dev and release separate). `accept_migration
  (data_version, release_target)` → `AlreadyCurrent`, `Forward{steps}` or
  `Downgrade` (refused). `recovery(stage, compatible_migration)`: before commit
  `RestorePriorRelease`; after commit `CompatibleMigration` or
  `BlockedPreservingData`, never a discard of accepted writes.
- Migration steps are `.wasm` components in `chariox:app/migration@1.0.0`.
- Reuses: `managed_state` snapshots/journal and the private-volume snapshot;
  event identities and receipts stay outside rollbackable data.
