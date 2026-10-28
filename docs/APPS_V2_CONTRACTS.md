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
  fixed names; the import allowlist and window text are R1.
- Compatibility for the Library is this same check plus `kernel_features`.

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

## 8. View actions and the signed declarations (A1)

- The view registers what the signed `schemas/ui-actions.json` declares, through
  `chariox.action(declaration, handler)` or `chariox.form(formElement, handler)`
  on `<form data-chariox-action>` markup
  (`browser-app-view-actions.mjs`, injected by the App bridge). A registration
  has the `UiActionDeclaration` shape; there is no second format. The page
  grants nothing: the kernel compares and decides.
- `check_registration(declared, registered)` (A2: at registration and before each
  invocation) refuses an undeclared action or one whose effect, approval reason,
  input/output schema or description differs. `check_agreement(declared,
  registered)` (A4: `/app actions`) also refuses duplicates and declared actions
  the view never registered. Both return a typed `UiActionMismatch` naming the
  action and the differing part. Schemas compare in RFC 8785 form, so key order
  and number spelling are not differences. Vectors: `ui-action-agreement.json`.
- A2 reads the page only through `__charioxViewAgent` (non-writable global):
  `registrations()`, `invoke(name, params)`, `state()`, `changes(since, waitMs)`.
  Answers are `{ok, value}` or `{ok:false, code, message}`; bounds are 64 KiB
  state, 256 KiB result, 16 KiB per change, 256 buffered changes.
- Form markup derives its schema from simple controls (text-like, number,
  checkbox, radio, select, textarea, password, and fields the product masks);
  hidden, file and multi-select controls are not agent input. An agent may
  enter a value it knows into a password or masked field; the product keeps
  masking what a Vault fill put there in model-bound observations. Anything
  richer uses `chariox.action`, whose schema the developer writes.


### Caller-scoped view handlers

An action handler receives `(input, caller)`. Use `await caller.call(method, input)`
for backend calls belonging to that invocation. The function returned by
`chariox.action` runs the same handler with a human caller when the UI calls it.
Ambient `chariox.call` always acts as the human, including while an agent handler
waits. Page code never supplies an actor identity. An invocation token travels
inside the existing call id; unknown or retired tokens fail with
`APP_INVOCATION_EXPIRED`, without falling back to a human call.

Register asynchronous forms explicitly:

```js
chariox.form(document.querySelector('form[data-chariox-action]'), async (input, caller) => {
  await caller.call('save', input);
  await caller.call('refresh');
});
```

Human submit and agent invoke enter this handler with their own caller. The agent
path applies controls, waits for component updates, checks validity, then awaits
the handler. A successful form returns `{submitted: true}` after completion;
handler errors are returned as action errors. An unregistered annotated form
fails with `form_unregistered`. Native submit listeners cannot provide this
completion contract. One agent action runs per Tab: another action or state read
gets `view_busy` until the running handler settles, even after its deadline.
Human input and registration/change-log reads remain available during the wait.

Forms used only by humans can register without action markup. They do not appear
in the agent catalog and need no signed action declaration.
