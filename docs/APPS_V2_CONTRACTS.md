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
- Optional `displayName` and `icon` give the App a human name and picture for
  lists and windows; `appId` stays the identity, and clients fall back to it
  when `displayName` is absent. `displayName` is 1 to 64 characters, already
  trimmed, with no control characters. `icon` is the package path of a `.png`
  or `.svg` file (for example `ui/icon.svg`) of at most 32 KiB, which must be
  listed in the signed inventory, so both fields are covered by the signature.
  A PNG must be well formed and at most 512 by 512 pixels. An SVG must start
  with an `svg` element and carry no `script`, `foreignObject`, `iframe`,
  `embed`, `object`, `audio`, `video` or `canvas` element, no `on*` attribute,
  no `javascript:`, doctype, entity or `@import`, and every `href`, `src` and
  `url(...)` must point at a `#fragment` of the same file. `pack` and `verify`
  both refuse a package that breaks these rules; a package without the fields
  stays valid. The kernel reads both from the verified package only and sends
  them as optional `display_name` and `icon` (a `data:image/png;base64,` or
  `data:image/svg+xml;base64,` URL) on `AppReleaseSummary`, and as the same
  names on `AppInstallationSummary` for the active release (or the retained one
  of an uninstalled installation that keeps its data). Both fields are absent
  from the JSON when the App declares none.
  The Desktop shows each installed App with an active release as a tile with this
  icon and name, or a letter tile in a colour taken from the app id when it
  declares no icon; the text terminal shows names only.
- Optional `idempotentTools` names sorted, unique declared backend tools that
  the worker may replay once after instance loss. Omission permits no automatic
  replay; effects and action descriptions never imply idempotence.
- `schemas/ui-actions.json` (`UiActionDeclaration`): name, description, input
  and optional output schema, `effect` `read | write | approval |
  person_confirmed`, an `approvalReason` for, and only for, `approval`, and a
  `confirmationView` for, and only for, `person_confirmed` (APP-CONFIRM-01: the
  name of the view the App registers with `chariox.view.confirmation(name,
  render)`; also `data-chariox-confirmation-view` on an annotated form; for an App
  whose backend serves its view, the name of the backend tool that answers with
  the preview rows). An optional `standing` `{class, repo, account}` on an `approval` or
  `person_confirmed` action lets the owner decide once, per repository, that this
  class (`merge | close | delete_branch | review`, a closed set) needs no card;
  `repo` names the required text input property that holds `owner/name`, `account` the optional text
  input property that names the account the call runs as. Backend `actions` keep their
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
- `AuthoritySet`: sorted atoms for network (origin+method), credential routes
  (`credential-route:` origin, label, method, path, safety), clipboard, files,
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
  `registrations()`, `invoke(name, params, invocationToken)`, `state()`, `changes(since, waitMs)`.
  Answers are `{ok, value}` or `{ok:false, code, message}`; bounds are 64 KiB
  state, 256 KiB result, 16 KiB per change, 256 buffered changes.
- Form markup derives its schema from simple controls (text-like, number,
  checkbox, radio, select, textarea, password, and fields the product masks);
  hidden, file and multi-select controls are not agent input. An agent may
  enter a value it knows into a password or masked field; the product keeps
  masking what a Vault fill put there in model-bound observations. Anything
  richer uses `chariox.action`, whose schema the developer writes.
- **Updates and unsaved work.** When an update commits, the kernel reloads
  every open view onto the new release. A page that holds work in memory
  registers `chariox.view.beforeUpdate(handler)` (up to 8 handlers). The
  controller runs them in the old page and reloads when they have all settled,
  or after 5 s; a page without a handler reloads at once. The handlers save to
  the view's own web storage, which the reloaded page reads: the old page's
  kernel calls are refused once the new generation is active. Fields inside a
  `<form>` are kept by the product without any handler.

## 9. The agent bridge and the runtime MCP catalog (A2)

- **Tools.** An agent holding an App grant gets `chariox.app_view_catalog`,
  `_invoke`, `_state`, `_changes` and `_batch` on the existing runtime MCP. They
  address the open views of the agent's own Room by the id the catalog lists.
  A view's actions come only from the signed declarations of the generation it
  runs (`AppCatalog::ui_actions()`; empty for v1 packages, filled by R4).
- **Order of gates for `invoke`.** Grant (`AppViewAuthority`, rechecked at every
  step, cancelling the wait when revoked) → generation (a view of an older one
  is `APP_VIEW_STALE`) → declared (`find_declared`) → input against the signed
  schema (`check_input`) → approval for an `approval` action → the view's one
  queue → the person's confirmation for a `person_confirmed` action → the page's
  registration against the declaration (`check_registered`) → the call.
  Nothing that runs the action reaches the page before the last step.
- **Approval.** An `approval` action asks through the kernel interaction
  `app-view-action-*` (allow once / allow for session / deny, shown on every
  client of the Room; `/sudo` answers it like a provider permission). The
  existing session answer covers only that App, agent, action and active
  generation. Revocation invalidates it, including after a regrant. The App
  grant alone does not approve an action. `batch` never decides one: it
  stops before the first approval step and reports it.
- **Person confirmation (APP-CONFIRM-01).** A `person_confirmed` action runs only
  after the person confirms this exact input on one kernel card
  (`app-confirm-*`, Confirm / Cancel, 300 s, Cancel on timeout). The kernel
  holds the view's turn, asks the page to render the App's own registered
  confirmation view of the input (`prepareConfirmation`: an inert, 480 by 320
  at most, `[data-chariox-confirmation]` root; no action runs), and the browser
  host, not the App or the agent, finds that one element and projects its pixels
  as the card's protected region. The card's frame (`app_confirmation`: App,
  publisher, action) is written by the kernel from the verified release, never
  by the App. The card is the person's alone: no agent holder or owner agent and no
  session approval or earlier answer stands in for it. A live `/sudo` task of the
  calling agent is the owner's consent to the work, so the kernel raises no card
  for it, as for a standing permission, and the action still checks the state it
  acts on. When a card is raised, the
  turn, grant, release and generation are checked again after Confirm. Cancel,
  timeout, disconnect or a stale view run nothing and say so. The preview is
  removed on every path. Input an agent drove through the browser into a
  `person_confirmed` form or button is refused by the page bridge
  (`person_confirmation_required`). A page view needs the App open in an App Tab.
  An App whose backend serves the view has no page or Tab: the kernel calls the
  backend tool the signed action names as its `confirmationView` with the exact
  input, and the answer, `{rows: [{label, value}]}` (one to twelve rows of plain
  text, labels up to 48 and values up to 400 characters, nothing else), travels
  in the frame as `app_confirmation.preview`; the card has no protected region.
  The card is otherwise the same, and an answer that does not fit refuses the
  call before any card.
- **Tool calls of gated actions.** An action that needs approval or the person's
  confirmation is asked for only through `chariox.app_view_invoke`: an agent
  that calls the backend tool of the same name directly is refused with
  `USE_APP_VIEW_INVOKE` and the App never runs.
- **Standing permissions.** The owner, never an App or an agent, can give a
  standing permission with `GrantAppStanding {installation_id, account, repo,
  class, expires_at_ms?}` (CLI `/app standing grant|list|revoke`; 30 days unless
  `--days n` or `--never`). When an agent invokes an `approval` or
  `person_confirmed` action whose signed `standing` names that class and whose
  input names that repository, no card is raised and the use is counted. It
  covers only the named repository, account and class. A backend-served App
  that leaves the account to itself answers the reserved tool `view_account`
  `{repo}` with `{account}`, the one the call runs as (the repository's, the
  default, else the first known); the kernel pins that account into the input,
  so the permission is judged for, and the call runs as, the same account. Without
  an answer, the account the call names or the only one bound counts; with
  several bound and none known the card asks. A permission exists only while the account
  binding it was made under (the same Vault entry) and the installation's active
  release do. Force pushes, deleting a default branch, grants and bindings are not
  classes; refusing the default or a protected branch stays the App's own rule.
- **Batch.** Every step is checked first (`plan_batch`), then run in order. It
  stops at the first error, approval or person-confirmed action and reports
  `completed` and `stopped`; there is no rollback.
- **One queue per view.** `AppViews::agent_queue` serializes the agent calls
  that run App code (`invoke`, `state`, `batch`, the catalog's state read). The
  change log runs none, so `changes` waits outside the queue. The controller
  (`browser.app.agent`) adds a deadline (20 s; the transport waits that long on
  top of its round trip) and refuses `view_busy` while a handler that missed its
  deadline still runs.
- **Actor attribution (answers A1 question 2).** Each `invoke` receives a
  caller bound to the agent, view and document until its handler settles,
  including after a deadline. Only `caller.call` carries that scope. Ambient
  `chariox.call` always acts as the human, including calls caused by ordinary
  browser input while an agent handler waits. State reads and catalog discovery
  establish no ambient actor.
- **Redaction (answers A1 question 3).** State, results, changes and App error
  messages go through the Room's secret observation scrub
  (`room_secret_observations.scrub`) before reaching the model; the protection
  being unavailable withholds the answer. Values the agent supplies are its own.
- **Errors** are `{code, message}` with a stable code: `unknown_view`,
  `unknown_action`, `invalid_input`, `view_mismatch`, `action_not_registered`,
  `approval_denied`, `confirmation_declined`, `confirmation_expired`,
  `confirmation_unavailable`, `person_confirmation_required`, `APP_VIEW_STALE`, `AGENT_APP_NOT_GRANTED`,
  `view_busy`, `deadline`, `view_reloaded`, `view_unavailable`, `withheld`, or
  the App's own code.
- **Waits for R4/L1.** Declarations of an installed App exist only once a v2
  package installs; until then catalogs list views with no actions.
## 10. Credentialed HTTP (`capabilities.network`)

A destination may name a `credential` label and a signed route table:
`{origin: "https://api.example.com", methods, credential: "example", routes:
[{method, path: "/repos/{owner}/{repo}/pulls/{number}", safety: "read" | "write" |
"destructive"}]}`. Such a destination needs the required feature `credentialed-http`,
an `https` origin, at most 128 routes, `methods` equal to the routes' methods, and path
templates of literal segments and `{name}` (one non-empty segment); `AuthoritySet`
counts every route, so an update that adds a route or raises a safety class asks again.

The owner binds an account: `GrantAppAccount {installation_id, account, credential,
vault_credential_id, allow}` (`/app account grant`). Only the owner can; there is no
agent tool. The Vault entry must be created for HTTP use at exactly that origin with an
`Authorization` header (`chariox.vault.copy_for_use` makes one from an entry made for
another use). The App calls `http.fetch` with `connection-id: "account:<label>"` and no
`Authorization`; the kernel then

1. matches origin, method and path against the signed routes (`APP_HTTP_ROUTE_NOT_DECLARED`)
   and allows only the headers `accept, content-type, if-match, if-none-match,
   if-modified-since, x-github-api-version`;
2. checks the owner's binding exists for that credential label (`APP_HTTP_ACCOUNT_UNKNOWN`)
   and allows at least the route's safety (`APP_HTTP_ACCOUNT_NOT_ALLOWED`);
3. only then reads the Vault value (`APP_HTTP_ACCOUNT_UNAVAILABLE` if it cannot) and adds
   `Authorization` and a `User-Agent` to the request itself;
4. sends it (no redirects, public addresses only) and buffers the answer, at most 1 MiB
   (`APP_HTTP_LIMIT`), with the value removed from the body and the headers, and only the
   headers `content-type, etag, last-modified, link, location, retry-after,
   x-poll-interval, x-github-request-id, x-ratelimit-*`. A 304 answer carries its `etag`.

The same origin without an account (`APP_HTTP_ACCOUNT_REQUIRED`) and an account call to an
origin without credentials (`APP_HTTP_ACCOUNT_UNKNOWN`) are refused. The App never sees the
value; neither do App state, logs, history, traces or the agent's tool results.

## 11. Event routing (`capabilities.eventRouting`, world 1.1.0)

An App that routes one generator's events for itself declares
`eventRouting: {generator, incoming: [event names], maxRoutes}` (1 to 200 routes, at most 32
incoming events). The package needs the required feature `event-routing` and the world
`chariox:app/app@1.1.0` (1.0.0 plus the `event-routing` import; `chariox_app_wit::APP_WORLD_1_1`);
`AuthoritySet` counts the generator, each incoming event and the route limit, so growing
any of them asks the owner again. A v1 manifest refuses the capability.

The owner grants the connection exactly as for connection actions (`GrantAppConnection`,
`/app connection grant`, owner only: no agent tool). The grant names the generator the manifest
declares under `connections` or `eventRouting`. `ListAppConnections` then shows
`event_routing: {max_routes, routes, paused}` for it, and revoking the grant removes the routes the
App created itself (never the owner's hand-made ones).

The App calls (WIT `event-routing`, wire `routing.*`; every call is scoped to the installation and
takes no connection, token or Room):

- `ensure-route {route-id, event, repository, source-event-type, source-event-version}`: the event
  must be declared, the repository (the generator's `connection_scope`) must be one of the granted
  connection's resources, and the generator must declare the source event; the route's scope and
  filter come from the generator, not the App. Ensuring the same route again answers it
  (`ROUTE_EXISTS` if the id names another shape); beyond `maxRoutes` it is `ROUTE_LIMIT`.
  `remove-route`, `set-route-active` (a pause: the route is claimed inactive and takes nothing, and a
  deployment handover never resumes it) and `list-routes` act only on the App's own routes.
- `connection-status`, `connection-resources` and `start-authorization` read the one granted
  connection and start the generator's consent (the owner still grants the new connection).
- `bind-agent {event, agent-id?}` delivers an outgoing event to an agent through the ordinary agent
  trigger. The kernel names the caller from its own record: an agent binds only itself (alone or
  beside a person's call); a person may name any agent. `bind-workflow` needs a person's call;
  `unbind-target` ends only the caller's own agent target (a person any); `list-targets` lists them.

`PauseAppAutomation` and `SetAppInboxRoutePaused` give the owner the same pause
(`/app automation pause`, `/app inbox pause|resume`).

## A4 inspection (MP-08 / MP-10 / MP-11)

`InspectAppView` (local 933; peer 381) reads an **existing** Room App Tab.
The kernel selects the focused Tab unless the caller supplies `target_id`
(the logical Room Tab ID, also used by `--tab`). It resolves that ID to the
current controller target on every read and returns the logical ID in
`AppViewInspected.target_id`, so DEV refresh survives controller rebinding.
It checks Room membership, the kernel's App binding, DEV Room scope, current
publisher trust and generation before and after the read. It uses the existing
`AppView` home/worker command with `op: inspect`; it cannot invoke an action,
open a headless Tab or grant a capability. A2's runtime MCP surface is separate.

The controller reads `__charioxViewAgent.inspect()` from the bound App document
and refuses a document change. The SDK returns its registrations, state and
visible control mapping. State is bounded at 64 KiB with a two-second provider
wait; the observation is bounded at 256 KiB, 256 registrations and 512 controls.
Errors and missing state providers remain explicit results. No DOM field value
is read for coverage. Supported form fields reuse A1's `controlsOf`; submit
controls inherit their form's action. For JS actions annotate the visible
control with `data-chariox-action="name"`, including custom elements; event-listener closures are opaque. Unannotated ARIA controls (including tabs, menu items and options) remain visible in coverage diagnostics.
Unsupported form controls and unannotated buttons remain uncovered. Coverage traverses light DOM and open shadow roots; closed roots cannot be enumerated by the page SDK and are outside this diagnostic scan. Use explicit JS registrations for controls in shadow roots; A1's simple form registrations still use document markup.

`chariox_app_runtime::view_catalog::ViewCatalog::from_observation` compares
runtime registrations with A1's signed action declarations and reports
`declaration_mismatch` for signed agreement and `undeclared_control` for a missing or ambiguous page-action mapping. These diagnostics are independent, so v1 still distinguishes a covered control from a deliberately undeclared button. `actions` is signed authority;
`registrations` is diagnostic only. The current D1 active-release service reads
v1 packages, which cannot declare signed `uiActions`: it therefore supplies an
empty signed action list and reports registrations as mismatches. This does
not extend v1's manifest or admit its registrations as agent actions. Wiring
`VerifiedPackageV2::ui_actions()` into the read requires R4's v2 activation.

`/app actions [--tab TARGET] [--json]` and `chariox app actions --session ROOM`
share the same read. `--json` emits `catalog_json`, serialized once by the kernel; the client never reserializes it (number formatting and numeric object keys must survive). `/app actions
toggle` opens the DEV-only TUI inspector using that same JSON; A/Esc closes it,
R refreshes the bound Tab, F8 reaches the shared approval handler, and a Room/kernel change closes it. Inspection command failures keep their full recovery message in the TUI transcript so a clipped or expired footer cannot hide it. Human-readable labels escape terminal controls; `catalog_json` remains the unchanged machine-readable payload. This shared
serialization proves inspector/read equivalence; equivalence with A2's agent
catalog needs its integration drill. The web inspector remains a proposal until
Miguel approves a prototype-copy mock; no web behavior ships in A4.

### Caller-scoped view handlers

An action handler receives `(input, caller)`. Use `await caller.call(method, input)`
for backend calls belonging to that invocation. The function returned by
`chariox.action` runs the same handler with the caller of the originating UI
input. An agent's claimed browser click or registered form submission retains
that agent through awaited backend calls; a person's concurrent input gets its
own human caller. Browser input acknowledges dispatch while the view keeps its
agent handler single flight until the returned promise settles.
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

Room browser actions carry the kernel-selected agent separately from their
locator/action data (peer protocol 381). App clicks use the same scoped input
caller as native browser input. Explicit submit and fill-and-submit bind only
the synchronous submission of the requested form. Human events remain unscoped.
Execution receipts include the caller, so retrying an execution under a different
agent cannot reuse another caller's result.
