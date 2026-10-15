# Skills v2 plan — MP-08 / MP-10 / MP-11

Status: **design for Miguel's sign-off; no implementation or acceptance claimed**.
Area: `skills-v2`. Inspected OSS base: `49a65fbdd3963d66fd5d7c1334333fbd1c0aa23b`
(local protocol 520, peer 128). Cloud source inspected read-only:
`00658b85aa81575b7c144adc11f3904eeecf0170` (`gh/main` on the builder).
These are source identities, not deployed release identities.

## 1. Product contract — MP-08 / MP-11

Chariox owns installed skills and their selection. A skill supplies instructions
and bundled resources; it adds **no tool, credential, filesystem or App authority**.
MCP, connector, script and App grants keep their existing authorization paths.
Install/import/register use the existing ask/yolo model and RuntimeInteraction;
there is no new directory-trust feature.

There are two scopes: **Chariox** (the home kernel) and **Project** (the kernel's
Project ID, independent of its folders). A Project may contain several repositories,
plain folders, or none. Its skills live in Chariox state, never in those folders.
Collaborators inherit the host kernel's applicable skills through existing session
access; there is no team registry or sharing service. Projects travel through
Environments, using the ordinary context export/import path.

Names are unique within each catalog. A Chariox name cannot occur in **any** Project
catalog on that kernel, including archived Projects; global install checks them
all. Separate Projects may reuse a name because their catalogs never resolve
together. Install/import rejects an occupied name even when bytes match, and
offers **Rename** or **Replace**. Replace updates the existing record in its existing
scope after normal authorization; it cannot leave a second copy in the requested
scope. A global-to-Project move must atomically remove the original binding.
Use one validated, case-folded collision key everywhere; preserve existing valid
legacy spellings for invocation and reject new case-only aliases.

| Effective state | Model discovery | Load behavior |
| --- | --- | --- |
| Auto | Name + description within the metadata budget | Model may request the body when relevant; `$name` also loads it. |
| Manual | Absent from model discovery | Only an explicit user `$name` selection admits a load. |
| Off | Absent | Chariox rejects selection/loading with an actionable state message. |

The frontmatter default is Manual for `disable-model-invocation: true`, otherwise
Auto. A stored skill override wins; a Project override of a Chariox skill wins
over that. **Reset to inherited** removes an override. Frontmatter changes on update
affect inherited state, never an explicit override. Off is an override, not an
invented frontmatter field. Parse real YAML, including multiline descriptions and
typed booleans; reject invalid invocation metadata instead of guessing.

State controls Chariox delivery. Provider-native skills remain independent,
read-only entries: if a native copy is still exposed, the screen must say so
rather than imply that Chariox Off disabled it. Chariox `$name` never falls through
to a native same-name copy when its installed record is Off. Instructions already
seen in a conversation cannot be erased; Off/remove stop future delivery and
reloads, while loaded-session history remains truthful.

## 2. What changes from today — MP-08 / MP-11

| Inspected source seam | Current behavior and required replacement |
| --- | --- |
| `apps/kernel/src/skill/{mod.rs,package.rs,provider_import.rs}` | Global `~/.chariox/skills`, path-based registry and simple frontmatter parser. Production lookup is user-only; a repo Project root is still used by tests/other consumers. Replace with Project-ID catalogs and immutable package revisions. |
| `extension.rs`, `agent/{types.rs,service.rs}` | `ExtensionKind::Skill`, `extension_grants`, `skill_grants()`, grant/revoke methods. Remove skills from the live grant model. |
| `runtime/state/tool_dispatch/{extension_list_tool.rs,extension_request_tool.rs}` | Lists skills as grantable extensions; requesting one can return its body and persist a grant. Replace the skill branch with instruction discovery/load operations without grants. |
| `provider/skill_context_instructions.md`, `format_granted_skill_prompt_context`, `runtime/state/prompt_skill_context_state.rs` | Granted summaries; broad name/phrase matching includes explicit bodies without a catalog budget. Replace with one state-aware selector and context assembler. Skill text is guidance beneath system/permission policy, not a new authority. |
| `runtime/state/tool_dispatch/remote_capability_sync.rs`, `app/remote_agent_binding.rs`, `app/remote_lease/skill_sync.rs`, `EnsureRemoteSkillPackages` | Ordinary remote context can include all granted bodies; native standard workers require matching local packages, while slices may receive them. Use one home-selected revision set and transfer path for every execution placement. |
| `runtime/provider_capability_import.rs` | Unified import groups candidates by name/recency and may update a differing installed package. Skill import must require explicit source and rename/replace; mtime cannot choose a winner. Leave MCP import policy separate. |
| `managed_context/kernel/{export.rs,import.rs}`, workflow requirement/code consumers | Skills still travel as Extensions or grant requirements. Move skill package/selection records to their own context and workflow instruction fields. |
| Cloud `apps/web/src/terminal/{extensions-state.ts,extensions-controller.ts,extensions-panel-dom-controller.ts,command-center.ts,command-center-controller.ts}` | Skills share an agent-focused Extensions panel; command completion handles `/` and agent aliases. Add a dedicated Skills screen and `$` picker backed by shared kernel state. |

This deliberately amends the **skill** clauses in `AGENTS.md`,
`docs/PROTOCOL.md` §3.3.2, `docs/ARCHITECTURE.md` §5.3.2 and the M14B validation
matrix: standard leased/native workers receive Chariox-installed skill packages.
Update those clauses during implementation; MCP placement/grants do not inherit
this change. Also update the canonical Browser/Computer regression row that still
describes skills as agent grants. Keep the kernel as authority and relay as opaque
transport; no Cloud skill resolver or managed-only runtime behavior.

## 3. Records and storage — MP-08 / MP-11

All paths are below the **resolved absolute `CHARIOX_HOME`**, defaulting to
`~/.chariox`. No generated skill files go in a repository or provider account home.

| Record/path | Contents |
| --- | --- |
| `state/skills/catalog.json` | Schema version, catalog revision, Chariox skill records. |
| `state/projects/<project-id>/skills.json` | Project records and Chariox-skill state overrides keyed by stable skill ID. |
| `skills/packages/<skill-id>/<digest>/` | Immutable `SKILL.md` and bundled files for both scopes. |
| `state/skill-materializations/<home-kernel-id>/<execution-id>/<skill-id>/<digest>/` | Execution-local package paths; execution ID is a local run or authenticated lease. |
| Existing session durable state | Confirmed load records: skill ID/digest, agent/run, prompt, reason (`explicit`/`model`/confirmed native), context generation and time. |

A skill record holds ID, name, scope/Project ID, description/short description,
frontmatter default, nullable state override, current digest, optional declared
version, file manifest, and provenance (installed/content/URL/provider import,
source machine + safe locator, provider/profile reference, import/update time).
Strip URL credentials/query secrets. A content digest is the version identity;
a declared version is display metadata. Model context gets the execution path,
never an unusable home-machine path.

Use the existing durable transaction/recovery machinery for catalog publication,
scope uniqueness and migration; do not build another database. Stage and validate
the package, publish it, then atomically change catalog bindings/revision. Serialize
collision checks with mutations and require expected revisions for replace/update.
Readers pin a digest for a turn. Update affects the next admitted turn; old packages
remain only while referenced by an active run, saved Environment or rollback.
Remove invalidates future selections and clears associated Project overrides;
cleanup removes only owned, unreferenced materializations.

Reuse the bounded package format/verification, with explicit schema changes where
needed. Hash sorted relative paths, exact bytes and executable bits; compare the
whole package, not only `SKILL.md` or a provider version label. Preserve bundled
script executability without auto-running it. Retain the existing 512-file/10 MiB
package limits; validate counts/bytes on receipt too. Reject path traversal,
duplicate/nonportable paths, escaping symlinks, malformed manifests and digest
mismatches before publication. Do not export adjacent auth/config files as assets.

## 4. Selection, injection and providers — MP-08 / MP-11

The home kernel resolves `(session Project ID, user selection, effective state,
catalog revision)` for every agent. Clients submit skill references, never paths
or bodies to trust. Typed `$name` and chips use the same resolver; queued turns
revalidate state/revision at dispatch and report changed/removed selections.
Parse standalone `$name` tokens with escaping and code-span/fence exclusions;
do not interpret shell `$HOME`, currency, attachments or quoted code as skills.
Unknown explicit names produce a bounded correction before a provider turn starts.
Remove the current `@name`, bare-name and `/skill name` implicit-body heuristics;
`/skills ...` remains a management command, never skill invocation.

Use **Option A**: Chariox injects an Auto-only metadata catalog through the existing
hidden-context bridges, then loads bodies on explicit selection or a model's
`load_skill` request. A shared `list_skills`/`load_skill` instruction surface uses
the existing runtime tool transport; it is not an Extension grant. Model listing
omits Manual/Off. Loading Manual requires a kernel-recorded explicit selection,
not a model-supplied `explicit=true`. The session UI may list all states. A load
returns the pinned body, manifest and materialized root; bundled files remain
available there through ordinary permitted file/shell tools. Loading a skill must
never install or grant an MCP/connector/script/App referenced by its instructions.
Imported `allowed-tools`, hooks, fork settings and dynamic shell substitutions
cannot grant permission or execute during loading; retain unsupported metadata
as source information, with a compatibility explanation in Details.

Proposed metadata ceilings: **2%** of the resolved model context window for Codex,
**1%** for Claude and OpenCode. These are Chariox policy defaults, not assertions
about all provider implementations. Count the rendered wrapper and metadata with
the provider tokenizer where available, otherwise a conservative UTF-8 byte upper
bound. Use the provider's known context capacity; if unknown, use a documented
conservative supported floor and show the estimate. Never assume a larger window.
Prefer supplied short descriptions, then deterministically shorten descriptions
at token boundaries. If all Auto name/description pairs cannot fit, include a
bounded discovery instruction plus a stable paged `list_skills` cursor; disclose
the omitted count in the screen. No body enters this budget or loads merely
because all packages were copied. Explicit bodies use the remaining turn budget;
an oversized request fails with size guidance before submission, never truncates
instructions or silently drops a selected skill.

On compaction/context reset, provider resume and run replacement, regenerate the
current catalog and re-attach still-active loaded skills (ID + pinned digest + body
and root) before the next task continues. Use supported provider lifecycle events;
where reset visibility is incomplete, idempotently attach at each turn boundary.
Deduplicate within a context generation; never infer retention from transcript
history. Recheck Off/remove and update revisions; no dropped active body may be
reported as restored. A load becomes **Loaded in this session** only after provider
delivery acknowledgement or an observed native load, not after metadata listing.
Show agent, digest and turn; native availability without a load signal is **load
unknown**, not Loaded. Reconnect recovers these records without re-running a task.

Provider execution continues through official harnesses only:

- **Codex:** the kernel/proxy resolves Chariox `$name` before native skill-name
  validation. Project the home-resolved available catalog through the existing
  native proxy's `skills/list` response so client-side completion/validation also
  knows Chariox names, without making them provider-installed skills. Extend
  `apps/cli/src/native-tui/{codex-proxy.ts,codex-prompt.ts,codex-turn-submission.ts}`:
  the current extractor retains text/images, not skill input items. Convert those
  items into kernel-validated references; discard client-supplied skill paths.
  Preserve the original chip/text in Chariox history; send a resolved
  invocation label in provider-facing visible text, with the body on the existing
  hidden developer-context path. Do not send an unresolved Chariox `$name` to the
  native parser. For an exact native duplicate, a supported native `skill` input
  item may carry the load instead, once, with the execution-local path. Prove both
  paths in the real Codex TUI, including a name absent from its native catalog.
- **Claude:** use the existing ordinary hidden context and native TUI
  `UserPromptSubmit.additionalContext` bridge. No visible PTY body injection.
  A selected-body delivery timeout is an actionable failed submission, not a
  successful turn with empty context; ordinary turns without skills still work.
- **OpenCode:** use the existing prompt `system` field for Chariox context and the
  normal official server/runtime tool path. Never translate Chariox `$` selection
  into a provider slash command or install a parallel agent runtime.

### Native inventory and dedupe — MP-08 / MP-11

Discover on the **execution machine with the selected account's resolved launch
environment and actual cwd**, before adding Chariox materializations. Cache by
provider version/profile/cwd and invalidate on Project/account/source change or
explicit refresh. Never inventory the implementation agent's profile. Native
entries stay read-only with Import; Chariox never modifies their source packages.

| Provider | Detection seam |
| --- | --- |
| Codex | Query official app-server `skills/list` for the actual `cwds`, with `forceReload` on invalidation; respect enabled state and discovery errors. Hash packages at returned execution-local paths. Include harness-discovered user/repo/system/plugin skills instead of guessing from `CODEX_HOME` alone. |
| Claude | Enumerate skill-only roots using the effective `CLAUDE_CONFIG_DIR/skills` (including linked account-profile config dirs), ordinary `~/.claude/skills` when applicable, workspace/ancestor/nested `.claude/skills`, configured additional directories and enabled plugin skill roots. Match pinned harness visibility/precedence; exclude disabled entries from dedupe. Do not scan credential/config payloads. Built-in/remote entries without readable complete packages remain unverified. |
| OpenCode | Inspect the pinned official local server's advertised schema for an effective skill inventory; use it when supported. Otherwise enumerate the actual `OPENCODE_CONFIG_DIR`, XDG/global and project/ancestor `.opencode/skill[s]`, `.claude/skills`, `.agents/skills`, plus configured skill paths from allowlisted settings only. Apply that harness version's effective precedence/permission settings. Do not invent a stable listing endpoint or fetch configured remote catalogs as an import side effect. |

The local importer is incomplete for these purposes (notably Claude profile dirs
and OpenCode compatibility roots). Directory discovery yields **candidates**;
only confirmed harness exposure plus a complete matching manifest permits dedupe.
If a provider cannot confirm exposure, show **unverified** and keep Chariox
delivery rather than silently suppressing it.

Same name + complete content/mode digest + compatible native invocation visibility:
skip duplicate Chariox metadata/body delivery and record **Provided by <provider>**.
`$name` must still load once; use native delivery only when it satisfies the
effective Chariox state and load acknowledgement contract. Same name + differing
content: show both sources/digests as a conflict in Skills; the Chariox revision
wins for Chariox `$name`, via kernel resolution and hidden delivery. Do not choose
by mtime or silently import. A native Manual copy cannot replace Chariox Auto
discovery; a native Auto copy cannot be presented as enforcing Chariox Manual/Off.
Native settings remain independently visible and read-only.

Official references checked 2026-10-15: [Codex app-server skills](https://learn.chatgpt.com/docs/app-server#skills)
documents listing, refresh/invalidation and skill input items;
[Claude skills](https://code.claude.com/docs/en/skills) and
[settings](https://code.claude.com/docs/en/settings) describe native discovery and
config-directory relocation; [OpenCode skills](https://opencode.ai/docs/skills/)
describes compatibility roots and native on-demand loading. Pin supported harness
versions and confirm these seams in the real drills; current website docs alone
do not prove a deployed binary supports them.

## 5. Execution placement and Environments — MP-08 / MP-10 / MP-11

Before dispatch, home sends the effective Auto/Manual package set and state/revision
manifest to every execution place: local, slice, leased worker (including standard
native TUI), BYOC and managed machine. Off packages are not newly transferred.
Transfer bytes once per digest, reconcile metadata per catalog revision, and await
verified materialization before the selected skill's provider turn. Missing/broken
transfer fails visibly; no fallback to a same-name worker package. Dedupe concerns
model delivery, not transport integrity or the home's package identity.

Extend `EnsureRemoteSkillPackages` and its existing authenticated lease path;
remove the standard-worker skill no-copy exception and worker grant synthesis.
Bind acknowledgements to home/session/agent/lease/Project/revision and package
digests; reject a foreign, expired or replaced lease. Worker caches are private to
that binding and cannot publish received skills as its own global catalog or
expose them to unrelated agents. Refresh at turn/lease boundaries; this is active
home-session delivery, not background synchronization between independent kernels.
Use actual target roots in hidden context and retain normal permission checks for
bundled script execution. Reconnect must not duplicate loads or provider turns.

Environment export includes the selected Project's skill records, packages,
overrides, and the required Chariox skill dependencies. Import remaps Project/skill
IDs consistently, preserves declared state/provenance, and applies the same
rename/replace checks against destination catalogs. It never writes into copied
repositories or overwrites provider-native files. An independently imported
Environment evolves independently; a later export is a new reviewed snapshot.
Exercise slice save/destroy/recreate and managed restart through normal lifecycle
paths, preserving pinned package identity and provider-thread continuity. No
Selkies/noVNC work or new managed-runtime exception is part of Skills v2.

## 6. Client contract and protocol work — MP-08 / MP-11

The side panel gains **Skills**, opening a main-area screen using the Workflows
screen pattern. Tabs: Chariox, selected Project, Provider-native. Show state and
inheritance, scope, source, version/digest, files, conflicts/dedupe, and loaded-session
status. Offer install, update, remove and import, including folder/content/URL
sources supported today. Provider-native rows are read-only with Import and source
details. A Project with no folders still has a complete screen; no selected Project
means an explicit choose/create action. Existing session access governs viewing;
host catalog mutations use existing owner/ask/yolo authorization.

`$` at the composer cursor opens the skills command center: search available
Auto/Manual names, show scope badges, select by click/Enter/Tab, and insert a
removable `$name` chip without submitting the prompt. Preserve surrounding text,
keyboard focus, IME and attachments. Off/unknown selections show a fix action.
`/` remains the commands picker; keep management under `/skills`. Local/remote
TUI use the same list, state, management and selection semantics, with a textual
chip representation. Native TUIs keep their native UI while routing submitted
Chariox references through the same home resolver.

**Coordinator protocol request: reserve the local-daemon version and any required
relay-peer version for Skills v2 before implementation. No number is allocated here.**
Proposed wire changes, to settle in that allocation:

- Skill list/get/install/import/update/remove requests use typed scope + Project
  ID, expected revision and explicit collision action; add state/override mutation,
  native inventory, session load projection and bounded paged discovery/load.
- Prompt selections carry stable skill ID/name/revision references; plain typed
  references resolve identically. Terminal events project catalog/load changes.
- Peer package manifests/receipts carry scope/revision/state and complete digests;
  Environment context schema carries Project skills separately from grants.
- Remove Skill from current Extension grant requests, responses and workflow grant
  shapes; retain legacy decode solely at durable/import migration boundaries.

Implementation must bump the shared OSS version, update local/peer protocol
snapshot/hash tests and shared TypeScript/Swift bindings, document rejection of old
skill-grant requests, and gate only clients using the new behavior on the new
minimum. Old clients/workers must receive an upgrade-required message before a
v2 mutation/turn that they cannot represent; never silently revert to grants or
repo loading. Keep self-hosted local `ws://` compatibility. The plan itself changes
no serialized code and needs no protocol bump.

## 7. Migration, removal and delivery sequence — MP-08 / MP-11

1. Add the kernel catalog/package/resolver boundary and a crash-safe, idempotent
   schema migration. Inventory installed `~/.chariox/skills/<name>` packages and
   product-isolated imported skill roots; migrate only product-owned installed
   records, preserving bytes, executable modes, IDs/provenance where known, and
   frontmatter defaults. Unknown provenance is labeled legacy, never fabricated.
   Respect explicit `CHARIOX_HOME`; when the old code wrote via process HOME,
   inventory that legacy root as an explicit migration source, not a new live root.
2. Existing installed skills become scope-managed skills using frontmatter state,
   regardless of which agents formerly had grants. Explain this availability
   change once in Web/TUI; do not convert grants into tool permissions or turn
   previously ungranted skills Off. Known Project-owned records bind to Project
   IDs; ambiguous ownership or cross-scope/name conflicts require a rename/replace
   resolution through existing interaction policy. Quarantine the affected record
   from selection until resolved; other skills continue. Never guess from cwd.
3. Repo `.agents/skills`, `.claude/skills`, `.codex/skills`, OpenCode directories
   and legacy user-authored `.chariox/skills` are **import sources only**, never
   live Chariox roots. Offer a copy into the chosen scope. Editing/removing a source
   afterward cannot change the installed revision. Native harness discovery of
   repo skills is displayed as native; it is not Chariox auto-import.
4. Migrate durable agents, queued launch configs, relaunch/saved-slice manifests and
   workflow definitions/publications: remove only skill grants, convert workflow
   instruction dependencies to skill references, retain all other grant
   fields exactly. Missing installed packages become visible unresolved references;
   do not invent or download them. Legacy journal replay decodes and normalizes old
   skill grants before current state; no grant/revoke compatibility path remains
   callable by an agent/client. Snapshot/replay and Environment import need the
   same adapter so restart cannot resurrect grants.
   A migrated workflow dependency packages/discovers a skill; it does not fabricate
   an explicit Manual invocation. Manual use needs a saved user `$name` selection.
5. Replace local/remote/native context formatters with the shared assembler; replace
   skill-specific `request_extension`/`list_extensions` branches, agent control
   commands, schemas, prompts, grant-based workflow requirements and UI controls.
   Delete `grant_skill`/`revoke_skill`/`skill_grants`, grant filters and worker grant
   synthesis after migration is wired. Keep package verification and the lease
   transport, refactored into named skill responsibilities. Remove obsolete tests
   asserting repo materialization or grant-dependent availability.
6. Add provider native inventory/dedupe and wire shared clients, Skills screen,
   `$` picker, Environment transfer and compaction recovery. Run focused regression
   checks and all required real scenarios below on paired exact heads. OSS and
   Cloud changes are separate coordinated commits; CI remains off until authorized.

Retain an external migration backup/receipt until the upgrade is accepted. A crash
must leave either the old schema or one complete v2 transaction; repeat startup
must not duplicate packages/records. Rollback restores the backup with the old
binary only while v2 writes are stopped; never run the old writer on a v2 catalog.
Delete legacy product-owned package copies only after verified publication; never
delete repo/native sources, credential profiles or shared reviewer state.

## 8. Real-path validation scenarios — MP-08 / MP-10 / MP-11

All **12 rows below are NOT RUN**; IDs are proposed new `skills-v2` acceptance rows.
This round delivers a plan, not a feature for owner testing. Implementers register
these rows with the coordinator and report each required dimension separately.

Common execution rules:

- Capture a real red on the frozen base (missing v2 flow counts only with recorded
  UI behavior), then replay on the paired implementation heads with the built app
  entry/flags, real CLI/TUI and kernel, hosted `wss` relay, and official providers.
  Use real ordinary and managed/Environment paths; direct IPC and component tests
  supplement user-driven acceptance. Pin binary hashes, harness versions and heads.
- Web cells use DPR1 at **1920×1080 and 1440×900**, plus DPR2. Drive normal clicks,
  typing, scrolling, banners/popups, tab creation/closing and navigation. Record
  video/screenshots and watch each recording; blank frames, stuck input or confusing
  dead ends fail with a timestamp. No DOM injection, programmatic banner dismissal
  or viewer-side CDP shortcuts to complete tasks. TUI cells use the built TUI binary.
- Real provider cells: Codex `gpt-6-luna` low, Claude `claude-haiku-5-5`; verify
  resolved IDs. OpenCode only for its provider-specific cells, at most three short
  turns per cell: `opencode-go/deepseek-v4.1-flash`, then
  `opencode/deepseek-v4.1-flash`, then `opencode/glm-5.3-flash` on a refusal. Record
  actual model/turn count; no paid OpenCode soak or generic coverage loop.
- Relevant browser tasks cover `en.wikipedia.org` article + `www.wikipedia.org`
  portal, a public `github.com` repository, DuckDuckGo (Bing if needed), a major
  news site and `developer.mozilla.org`. Select harmless reading/research tasks
  and record per-site results/screenshots, including cookies/popups. Google is
  **N/A-AUTOMATED**, with any later manual check owned by Miguel.
- Hosted cells shape client uplink to about 8 Mbit/s with measured RTT ≥60 ms.
  Collect at least 100 local echo/picker samples for interaction P95 gates:
  **P95 ≤ measured wire RTT + 60 ms**. Separate model inference time from product
  overhead. Timing evidence follows the quiet-host/guest + I/O probe method;
  busy b3 fsync tails are environment failures. Provider success alone is no
  latency proof. Report screen scroll ≥30 fps where streamed; any browser mirror
  measurement first asserts protocol ≥517, `wire: 2`, first packet and paint mode.
- Evidence: screenshots/video, red/green step log and console captures, safe load
  receipts/digests/token counts, resources, exit codes and cleanup under the lane's
  external evidence directory. Never capture account/token/config contents. Keep
  state outside repos and use own Vault. Remove only run-owned processes (guard
  signal IDs >1), containers, materializations and stacks; retain protected assets.
  Missing resources become BLOCKED with exact coordinator/owner action, never PASS.

| Proposed ID / MP items | User-driven scenario and pass assertions |
| --- | --- |
| `skills-v2/SV-01` — MP-08/MP-10 | **Skills management and selection.** Fresh hosted Web at all three display conditions plus local/remote TUI: open Skills, install from folder/content/public URL, inspect files/version/source, update/remove, choose Chariox/Project, and insert/delete `$name` chips amid text/attachments. `/` still opens commands and selecting a skill never submits prematurely. Reconnect preserves screen/focus/state; a real Codex task uses the chosen revision. Base red: current grant panel/no `$` flow. |
| `skills-v2/SV-02` — MP-08/MP-10 | **Project identity and states.** In two Projects, one spanning two repos/plain folder and one with no folders, verify a Project skill follows the Project across cwd/session changes and stays absent from the other. Exercise Auto/Manual/Off, default true/false frontmatter, explicit overrides, Project override/reset of a Chariox skill and update of inherited defaults. Real Codex and Claude turns demonstrate Auto discovery without `$`, Manual loading only with `$`, and Off rejection; recordings plus load receipts establish the result, not an agent assertion alone. |
| `skills-v2/SV-03` — MP-08/MP-10/MP-11 | **Collision and import isolation.** Through Web and TUI attempt same/case-only names across both scopes, including an archived Project, concurrent installs and Environment import. Refusal must preserve bytes/catalogs and offer working rename/replace; retry/stale revision cannot overwrite another change. Import each provider/repo source, then edit/delete that source: installed digest stays fixed. Opening a repo alone never installs it. Explicit replacement produces one binding. Base red includes unified import's automatic differing-content update. |
| `skills-v2/SV-04` — MP-08/MP-10 | **60-skill budget.** Install 60 real reusable skill packages with bundled references and varied long descriptions; use Auto plus Manual/Off controls. Run real Codex and Claude tasks over MDN and a public GitHub repo without naming the appropriate Auto skill, then explicitly select a Manual skill. Record the rendered budget/context capacity/count method and pre/post-load receipts: names/descriptions or visible paged overflow fit the ceiling, Manual/Off never enter discovery, only relevant/selected bodies load, and files resolve. Measure picker/echo responsiveness under the common gates. Base red: no budget and remote eager bodies. |
| `skills-v2/SV-05` — MP-08/MP-10 | **Compaction and continuity.** A ≥4-hour Codex/Claude research session across the public-site list loads an actual skill and bundled references, then reaches/requests real provider compaction. Continue the task from Web and TUI, reconnect and resume the run. Verify new context generation restores catalog/body/path before continued work, same thread/task, no duplicate action, truthful loaded status and stable resources. Disable/remove a loaded skill and verify no re-attachment; prior delivery remains visible in history. No fake compaction event or short smoke substitutes. |
| `skills-v2/SV-06` — MP-08/MP-10/MP-11 | **Codex harness dedupe.** With real Codex and its actual app-server catalog, exercise same name/full package, same body/different bundled asset, same name/different body, disabled native copy and changed profile/cwd. Confirm one delivery for equal effective copies, conflict UI for differing bytes, Chariox `$` winner, refresh after native changes and explicit unverified status for unreadable inventory. Native rows remain read-only; Import uses collision rules. Check prompt/load receipts and real task output. |
| `skills-v2/SV-07` — MP-08/MP-10/MP-11 | **Claude/OpenCode harness dedupe.** Repeat equal/different-content/native-disabled cases on real Claude using an account-profile `CLAUDE_CONFIG_DIR`, additional/plugin roots, and real OpenCode using its scoped config/XDG, compatibility and configured skill paths. Exercise an OpenCode-specific native skill load in ≤3 short turns per cell using the prescribed model. Verify effective native exposure/precedence, one delivery or conflict, `$` winner and native read-only Import; no credential/config payload in evidence. Record unsupported inventory seams as blockers for that dimension. |
| `skills-v2/SV-08` — MP-08/MP-10/MP-11 | **Standard leased worker.** From real Web + remote TUI run Codex and Claude on an ordinary second kernel with no preinstalled Chariox skill. Home copies Auto/Manual packages; agent loads a body and executable bundled helper at the worker path under normal permissions. Change/off/remove at home between turns, drop/reconnect relay and replace a lease: exact new set applies, old/foreign acknowledgements are denied, task/thread is not duplicated and no repo/provider profile receives generated files. |
| `skills-v2/SV-09` — MP-08/MP-10 | **Slice lifecycle.** Launch a real local Docker slice through the user clients; select a skill with references/helper, use it on a public site with a real provider, save with idle agents, destroy/recreate through supported lifecycle and resume. Verify digest, modes, roots, state and provider thread, plus no leaked generated repo files. Run compatibility mode on the supported default. Restart/reconnect and cleanup must pass; a package-only unit test cannot satisfy persistence. |
| `skills-v2/SV-10` — MP-08/MP-10 | **Native TUIs.** In the real `chariox codex`, `chariox claude` and `chariox opencode` binaries, submit Chariox-only `$name` locally, on a standard leased worker and in a slice; observe the same session from Web/TUI. Codex never reports unknown skill, conflicting native copy cannot win, Claude bodies stay hidden via hook context, and OpenCode uses the normal system channel. Typed and Chariox-origin prompts share ordering/history/permissions with one run. Cover explicit selected-body delivery failure: no silent empty-context turn. OpenCode cells stay within the per-cell turn cap. |
| `skills-v2/SV-11` — MP-08/MP-10/MP-11 | **Environment portability/parity.** Move a multi-folder Project with Project skills and Chariox overrides through normal Environments into BYOC and managed execution; compare the same user flow on an ordinary kernel. Verify package/state/provenance and collision review, restart/resume, a Project with no repo, and independent evolution after export. Use existing b3 VM/container isolation policy; no isolation-only rental. Signed deployment and mandatory shutdown remain the only managed differences; cleanup covers only this run's targets. |
| `skills-v2/SV-12` — MP-08/MP-10/MP-11 | **Upgrade, authorization and rollback.** Start from real installed/granted skills with an existing workflow, queued turn and saved slice. Upgrade/restart, interrupt migration and retry; execute the workflow and resume through real clients/providers. Skill grants never return; MCP/connector/script/App grants are identical. In ask and yolo, install/import follows existing policy; deny a requested tool/credential while a skill asks for it and prove no bypass. A collaborator inherits only its authorized host session, cannot mutate the host catalog or read another Project/lease. Old client/worker rejection is actionable. Exercise the documented backup rollback before reopening writes, then re-upgrade. Source security tests add malformed packages/forged receipts; they supplement these live flows. |

Acceptance requires every required dimension PASS or an explicit Miguel waiver,
review of recordings and cleanup, plus focused protocol/migration/security
regressions. MP-11 review is the narrowed current security-anchor scope
(authorization, relay/lease admission, package containment and credential handling),
with the behavioral ordinary/managed matrix; it does not demand exact-blob review
of every non-security file. This plan closes no MP item and does not authorize
implementation, publishing or deployment before design sign-off.
