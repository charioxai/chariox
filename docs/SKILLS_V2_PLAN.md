# Skills v2 plan — MP-08 / MP-10 / MP-11

Status: **design for Miguel's sign-off; no implementation or acceptance claimed**.
Area: `skills-v2`. Inspected OSS base: `49a65fbdd3963d66fd5d7c1334333fbd1c0aa23b`
(local protocol 520, peer 128). Cloud source inspected read-only:
`00658b85aa81575b7c144adc11f3904eeecf0170` (`gh/main` on the builder).
These are source identities, not deployed release identities.
Revised for Miguel's 2026-10-15 decisions: clean-slate skill reset, universal
configurable 1% metadata budget, focused-provider selection labelled
`From <Provider>`, agent self-install with hot reload, skill packages and
Environment Skills management. Skills Web drills use 1440×900 DPR1 plus the TUI,
per the 18:22 UTC decision. This revision still changes documentation only.

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
Use one validated, case-folded collision key everywhere; reject case-only aliases.

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
rather than imply that Chariox Off disabled it. Plain `$name` never falls through
to a native same-name copy when its installed record is Off; an explicitly chosen
`From <Provider>` entry is a separate provider-native selection. Instructions
already seen in a conversation cannot be erased; Off/remove stop future delivery
and reloads, while loaded-session history remains truthful.

## 2. What changes from today — MP-08 / MP-11

| Inspected source seam | Current behavior and required replacement |
| --- | --- |
| `apps/kernel/src/skill/{mod.rs,package.rs,provider_import.rs}` | Global `~/.chariox/skills`, path-based registry and simple frontmatter parser. Production lookup is user-only; a repo Project root is still used by tests/other consumers. Replace with Project-ID catalogs and immutable package revisions. |
| `extension.rs`, `agent/{types.rs,service.rs}` | `ExtensionKind::Skill`, `extension_grants`, `skill_grants()`, grant/revoke methods. Remove skills from the live grant model. |
| `runtime/state/tool_dispatch/{extension_list_tool.rs,extension_request_tool.rs}` | Lists skills as grantable extensions; requesting one can return its body and persist a grant. Replace the skill branch with instruction discovery/load operations without grants. |
| `provider/skill_context_instructions.md`, `format_granted_skill_prompt_context`, `runtime/state/prompt_skill_context_state.rs` | Granted summaries; broad name/phrase matching includes explicit bodies without a catalog budget. Replace with one state-aware selector and context assembler. Skill text is guidance beneath system/permission policy, not a new authority. |
| `runtime/state/tool_dispatch/remote_capability_sync.rs`, `app/remote_agent_binding.rs`, `app/remote_lease/skill_sync.rs`, `EnsureRemoteSkillPackages` | Ordinary remote context can include all granted bodies; native standard workers require matching local packages, while slices may receive them. Use one home-selected revision set and transfer path for every execution placement. |
| `runtime/provider_capability_import.rs` | Unified import groups candidates by name/recency and may update a differing installed package. Skill import must require explicit source and rename/replace; mtime cannot choose a winner. Leave MCP import policy separate. |
| `managed_context/kernel/{export.rs,import.rs}`, workflow requirement/code consumers | Skills still travel as Extensions or grant requirements. Remove all existing skill records/references; new v2 packages/selections use their own context and workflow instruction fields. |
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
| Package records in those same catalogs | Stable package ID, display name, scope/Project ID, optional source locator + pinned revision and upstream member mapping. |
| `skills/packages/<skill-id>/<digest>/` | Immutable `SKILL.md` and bundled files for both scopes. |
| `state/skill-materializations/<home-kernel-id>/<execution-id>/<skill-id>/<digest>/` | Execution-local package paths; execution ID is a local run or authenticated lease. |
| Existing session durable state | Confirmed load records: skill ID/digest, agent/run, prompt, reason (`explicit`/`model`/confirmed native), context generation and time. |

A skill record holds ID, name, scope/Project ID, nullable package ID, description/
short description, frontmatter default, nullable state override, current digest,
optional declared version, file manifest, and provenance (installed/content/URL/provider import,
source machine + safe locator, provider/profile reference, import/update time).
Strip URL credentials/query secrets. A content digest is the version identity;
a declared version is display metadata. Model context gets the execution path,
never an unusable home-machine path.

Use the existing durable transaction/recovery machinery for catalog publication,
scope uniqueness and the clean-slate reset; do not build another database. Stage
and validate the package, publish it, then atomically change catalog bindings and
revision. Serialize collision checks with mutations and require expected revisions
for replace/update.
Readers pin a digest for a turn. Update affects the next admitted turn; old packages
remain only while referenced by an active run or saved v2 Environment.
Remove invalidates future selections and clears associated Project overrides;
cleanup removes only owned, unreferenced materializations.

Reuse the bounded package format/verification, with explicit schema changes where
needed. Hash sorted relative paths, exact bytes and executable bits; compare the
whole package, not only `SKILL.md` or a provider version label. Preserve bundled
script executability without auto-running it. Retain the existing per-skill
512-file/10 MiB transfer-payload limits; validate counts/bytes on receipt too.
Reject path traversal, duplicate/nonportable paths, escaping symlinks, malformed manifests and digest
mismatches before publication. Do not export adjacent auth/config files as assets.

### Skill packages — MP-08 / MP-11

A **package** groups skills for management: for example, install a multi-skill
repository such as Pocock skills, Claude skills, pstack or gstack as one package.
This grouping is distinct from the existing per-skill transfer payload (also
called a package in today's code). Each skill still has its own ID, content
digest, state and load record; package membership adds no invocation namespace,
authority or context overhead. Name uniqueness applies to every member across
the two scopes, even when package names differ.

A skill belongs to **at most one package** through its nullable package ID.
Packages and their members share one scope/Project; a scope move rechecks name
collisions and clears incompatible membership. A repository install with a known
source groups its reviewed skill set under that source. Unknown provenance means
**no package**, shown as ordinary ungrouped skills; never infer a package from a
name or a neighbouring folder. Users may create custom packages and move skills
into/out of them without copying skill bytes, changing IDs or losing provenance,
state overrides or session load history. Moving into a package replaces prior
membership atomically. Keep empty custom packages until explicitly removed.

A **repository install** is owner-initiated from Skills (Web "Import a public
repository", TUI `/skills repo <url>`): the kernel fetches the public git repository
with `git`, lists every SKILL.md directory at its current commit, and the reviewed
batch installs the checked members pinned to exactly that commit into one new package.
The kernel records the commit as the members' and the package's `source_revision`
and maps each upstream directory to the member's skill ID; no client label can
attest a revision. Private or auth-required repositories are refused, and an agent
cannot start a repository install. MP-08 / MP-10 / MP-11: the bounded kernel/TUI
update path resolves a newer commit with `/skills repo-update <package>`, reviews
changed/new/removed members, and publishes the pinned batch through normal confirmation.
The Web update entry point remains outside this implementation. Members moved out
and directories left unselected at install are skipped; invalid current members
refuse the whole update. Empty repositories and removal of every remaining member
require explicit removal or ungrouping. The existing 512-action limit applies.
MP-08 / MP-10 / MP-11: when another package from the same repository has already
installed a newly added upstream directory in an overlapping name scope, skip it
and retain that member's ID, grouping and installed revision. Updating the sibling
package must not reclaim or duplicate it. Different upstream directories still
use the normal name collision rules: refusal offers Rename/Replace and publishes
nothing.

Install/update/remove/state actions work on a single skill or a whole package.
Show the affected members and collisions before authorization, and commit the
reviewed batch atomically with expected catalog revisions. A collision or failed
member validation publishes nothing. State actions batch-write the existing
member overrides (including Reset to inherited) for current members; no separate
package-state precedence layer. New members use their frontmatter defaults, shown
in the update review with any requested batch state change. Show **Mixed** when
member states differ. A Project-level batch on a Chariox package writes Project overrides, just like individual actions.

Source-backed update pins a source revision and previews changed/new/removed
members; keep stable IDs for matched members and explicit overrides. Removing an
upstream member needs the same reviewed removal action. Regrouping retains each
skill's original update locator; updating its former package must not pull that
skill back or overwrite its custom grouping. Custom-package update uses the
members' recorded sources and reports members without an update source, rather
than inventing one. **Remove package and skills** removes its current members
through normal removal; **Ungroup** removes only membership/the group. Confirm
the affected set through existing ask/yolo policy. Successful batches publish one
catalog revision and use the same next-turn hot reload and transfer checks.

## 4. Selection, injection and providers — MP-08 / MP-11

The home kernel resolves `(session Project ID, user selection, effective state,
catalog revision)` for every agent. Clients submit skill references, never paths
or bodies to trust. Typed `$name` and chips use the same resolver; queued turns
revalidate state/revision at dispatch and report changed/removed selections.
Selections distinguish `chariox` from `native`. Native references bind provider,
account, execution placement, inventory revision and a kernel-issued entry ID;
resolve paths on that execution machine, never from a client-supplied path.
Plain `$name` resolves Chariox first, then an unambiguous enabled native entry of
the target agent's provider. A chip labelled `From <Provider>` explicitly selects
that native entry even beside a Chariox namesake; it cannot replace a Chariox chip
implicitly.
Revalidate on focus/provider/account/placement changes; a stale native selection
requires reselection instead of executing another provider's same-name skill.
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

The metadata ceiling is **1% of the resolved model context window for every
provider**, configurable in the home kernel's `config.toml`:

```toml
[skills]
metadata_budget_percent = 1.0
```

This is Chariox policy, not a claim about harness defaults. Accept only finite
values greater than 0 and at most 100; normal total-context limits still apply.
Use the same configured percentage across providers and execution placements;
expose the effective setting in Skills details. Config reload uses the existing
kernel settings path and refreshes the next turn. Count Chariox's rendered wrapper
and metadata (not independent harness-native context) with
the provider tokenizer where available, otherwise a conservative UTF-8 byte upper
bound. Use the provider's known context capacity; if unknown (a run's first
turn, before the provider reports its window), use the documented supported floor
of 128,000 tokens, the smallest window of any supported provider model, and show
the estimate. Never assume a larger window.
Prefer supplied short descriptions, then deterministically shorten descriptions
at token boundaries. If all Auto name/description pairs cannot fit, include a
bounded discovery instruction plus a stable paged `list_skills` cursor; disclose
the omitted count in the screen. No body enters this budget or loads merely
because all packages were copied. Explicit bodies use the remaining turn budget;
an oversized request fails with size guidance before submission, never truncates
instructions or silently drops a selected skill.

### Agent self-install and hot reload — MP-08 / MP-11

Agents may install/import/register and update skills through the same home-owned
catalog service as Web/TUI, exposed through the existing runtime tool transport.
The request names the scope/Project, source and expected revision; normal owner
authorization and ask/yolo policy apply. Ask yields one kernel RuntimeInteraction
visible to authorized clients; denial changes nothing. Yolo follows existing
policy. Remote agents route the mutation home; a worker must not create its own
shadow catalog. Bundled scripts never run merely because an agent installs them.

On successful publication, increment the catalog revision, publish the change to
connected clients and invalidate every affected agent's context/inventory cache.
A Chariox-scope install/update is available to **all agents on their next turn**,
regardless of provider or session; Project skills reach all agents in that Project
only. Existing state overrides still apply. No agent or provider restart is allowed
or required. The installing turn stays pinned; the following turn sees the new
catalog and current body digest, including for a previously loaded skill. Do not
replay a turn to refresh metadata. At every dispatch compare the durable revision
even if a change event was missed, so disconnected/queued/resumed agents catch up.
For leased/slice execution, synchronize the changed package before that next
skill-using turn. A failed transfer reports a failed submission rather than
running stale instructions. Native TUI catalog projections refresh through their
supported existing proxy/hook seams; Chariox injection must not depend on harness
startup-time discovery or a runtime-MCP tool-schema restart.

On compaction/context reset, provider resume and run replacement, regenerate the
current catalog and re-attach still-active loaded skills (ID + pinned digest + body
and root) before the next task continues. Use supported provider lifecycle events;
where reset visibility is incomplete, idempotently attach at each turn boundary.
Deduplicate within a context generation and digest; never infer retention from
transcript history. Recheck Off/remove and update revisions; no dropped active body
may be reported as restored. A load becomes **Loaded in this session** only after provider
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

Explicit `From <Provider>` selections pass to that agent's official harness
through its supported skill invocation path, under the same kernel prompt/history/permission
ownership: Codex native skill input, Claude native skill command, OpenCode native
skill loading through its official server/tool path. OpenCode's skill tool is not
a generic slash-command API. The adapters must prove invocation on pinned harness
versions; absence of a supported seam blocks that provider dimension. Chariox
neither imports/copies a native package nor injects its own body for this action.
Import remains a separate explicit operation. Provider selection cannot auto-grant
the tools or credentials named by a native skill.

### Native inventory and dedupe — MP-08 / MP-11

Discover on the **execution machine with the selected account's resolved launch
environment and actual cwd**, before adding Chariox materializations. Cache by
provider version/profile/cwd and invalidate on Project/account/source change,
catalog publication or explicit refresh. Never inventory the implementation
agent's profile. `From <Provider>` entries stay read-only with Import; Chariox
never modifies their source packages.

| Provider | Detection seam |
| --- | --- |
| Codex | Query official app-server `skills/list` for the actual `cwds`, with `forceReload` on invalidation; respect enabled state and discovery errors. Hash packages at returned execution-local paths. Include harness-discovered user/repo/system/plugin skills instead of guessing from `CODEX_HOME` alone. |
| Claude | Enumerate skill-only roots using the effective `CLAUDE_CONFIG_DIR/skills` (including linked account-profile config dirs), ordinary `~/.claude/skills` when applicable, workspace/ancestor/nested `.claude/skills`, configured additional directories and enabled plugin skill roots. Match pinned harness visibility/precedence; exclude disabled entries from dedupe. Do not scan credential/config payloads. Built-in/remote entries without readable complete packages remain unverified. |
| OpenCode | Inspect the pinned official local server's advertised schema for an effective skill inventory; use it when supported. Otherwise enumerate the actual `OPENCODE_CONFIG_DIR`, XDG/global and project/ancestor `.opencode/skill[s]`, `.claude/skills`, `.agents/skills`, plus configured skill paths from allowlisted settings only. Apply that harness version's effective precedence/permission settings. Do not invent a stable listing endpoint or fetch configured remote catalogs as an import side effect. |

MP-08 / MP-10 / MP-11 owner decision (2026-10-18): Claude skills discovered in
the selected execution's resolved skill directories count as **verified exposure**.
Accept a `From Claude` selection through Claude's `/name` adapter on local,
leased and slice execution. Complete package verification, current inventory
binding and the same-name/full-content dedupe rule still apply; unreadable or
invalid packages remain unverified. Keep the source visible in both pickers.

The local importer is incomplete for these purposes (notably Claude profile dirs
and OpenCode compatibility roots). OpenCode directory fallback yields candidates;
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
Harness settings remain independently visible and read-only.

Official references checked 2026-10-15: [Codex app-server skills](https://learn.chatgpt.com/docs/app-server#skills)
documents listing, refresh/invalidation and skill input items;
[Claude skills](https://code.claude.com/docs/en/skills) and
[settings](https://code.claude.com/docs/en/settings) describe native discovery and
config-directory relocation; [OpenCode skills](https://opencode.ai/docs/skills/)
describes compatibility roots and native on-demand loading;
[OpenCode commands](https://opencode.ai/docs/commands/) documents its separate
command mechanism. These are detection/invocation seams to validate, not evidence
that all three providers already support Chariox's new picker. Pin supported harness
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
overrides, and the required Chariox skill dependencies, including package groups
and member bindings for the exported set. Import remaps Project/skill/package
IDs consistently, preserves declared state/provenance/grouping, and applies the
same rename/replace checks against destination catalogs. It never writes into
copied repositories or overwrites provider-native files. An independently imported
Environment evolves independently; a later export is a new reviewed snapshot.
Exercise slice save/destroy/recreate and managed restart through normal lifecycle
paths, preserving pinned package identity and provider-thread continuity. No
Selkies/noVNC work or new managed-runtime exception is part of Skills v2.

Each **Environment has a Skills section** showing the Project skills it carries,
grouped by package with state/inheritance, provenance, version/files and transfer
status. Install/import/update/remove, state changes and custom regrouping there
use the same Project catalog/service as the main Skills screen. Required Chariox
dependencies are labelled separately with their scope and Project overrides;
the section is a projection, not a third scope or independent catalog. Include
Skills in the Environment move review and transfer manifest. Package membership
in a snapshot includes only exported skills, never pulls unrelated members into
an Environment. Destination edits use its imported catalog; disconnected sources
are not silently changed. Refresh both screens after a successful mutation.

## 6. Client contract and protocol work — MP-08 / MP-11

The side panel gains **Skills**, opening a main-area screen using the Workflows
screen pattern. Tabs: Chariox, selected Project, `From <Provider>` (the focused
agent's provider, e.g. **From Claude** or **From Codex**). Show state and
inheritance, scope, source, version/digest, files, conflicts/dedupe, and loaded-session
status. Offer install, update, remove and import, including folder/content/URL
sources supported today. Provider-native rows are read-only with Import and source
details, filtered to the focused agent's provider/account/execution environment.
A Project with no folders still has a complete screen; no selected Project
means an explicit choose/create action. Existing session access governs viewing;
host catalog mutations use existing owner/ask/yolo authorization.

Within each Chariox/Project tab group skills by package, with an **Ungrouped**
section for skills without one. Package headers offer install/update/remove/state
actions with member previews and a Mixed state when needed. Offer Create package,
Move to package and Ungroup; keep individual skill actions/details available.
Show package source/revision separately from each member's version and provenance.
Provider entries remain read-only until imported; Import may join a Chariox
package through the explicit source/scope review. Environment Skills uses this
same grouping and management surface for the carried Project.

`$` at the composer cursor opens the skills command center: list applicable
Chariox Auto/Manual skills for **every provider**, plus enabled harness-native
skills **only for the focused agent's provider** and actual execution environment.
Show Chariox/Project scope badges or `From <Provider>`; search and select by
click/Enter/Tab, inserting a removable `$name` chip with a typed source identity
without submitting the prompt. Choosing `From <Provider>` passes to that harness
natively with no copy; installing requires the separate Import action. Provider
focus changes refresh native rows and invalidate stale native chips, while Chariox rows
stay available. With no focused agent, show applicable Chariox rows and an explicit
choose-agent action for native skills. Preserve surrounding text, keyboard focus,
IME and attachments. Off/unknown selections show a fix action.
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
  Agent mutation tools call the same authorization/service path, without grants.
- Package create/rename/regroup and batch install/update/remove/state requests
  carry scope, package/member IDs, expected revision and reviewed member actions.
  Catalog projections expose nullable membership, source revision and aggregate
  state. Environment schemas/projections carry these same groups and Skills state;
  no Cloud-owned package registry or new authorization path.
- Prompt selections carry Chariox skill ID/name/revision or provider-bound native
  entry references; typed names use the same resolution rules. Terminal events
  project catalog/load changes and invalidate client/provider projections. Durable
  revision checks at dispatch enforce next-turn hot reload even after missed events.
- Peer package manifests/receipts carry scope/revision/state and complete digests;
  Environment context schema carries Project skills separately from grants.
- Remove Skill from current Extension grant requests, responses and workflow grant
  shapes, with no legacy decode or compatibility adapter. The destructive reset
  removes retired persisted skill data before normal v2 state loading.

Implementation must bump the shared OSS version, update local/peer protocol
snapshot/hash tests and shared TypeScript/Swift bindings, document rejection of old
skill-grant requests, and gate only clients using the new behavior on the new
minimum. Old clients/workers must receive an upgrade-required message before a
v2 mutation/turn that they cannot represent; never silently revert to grants or
repo loading. Keep self-hosted local `ws://` compatibility. The plan itself changes
no serialized code and needs no protocol bump.

## 7. Clean-slate transition and delivery sequence — MP-08 / MP-11

Miguel's decision is a **destructive skill reset, not a skill/grant migration**.
Current installed skills are removed. There is no compatibility decoder, grant
translation, one-time migration notice, automatic re-import or restored skill
dependency. V2 starts with empty Chariox and Project catalogs; users/agents may
explicitly install/import new packages afterward under normal ask/yolo policy.

1. Before normal v2 loading, quiesce affected launches and perform a crash-safe,
   idempotent schema reset using existing durable transaction/recovery machinery.
   Remove product-owned old installed packages, isolated imported copies and
   derived worker/slice materializations. Resolve ownership and absolute roots,
   including prior process-HOME roots when applicable; never recursively delete
   a parent containing unrelated state. Preserve repo/native sources, credentials,
   Vaults, keys and reviewer state. This plan performs none of those deletions.
2. Remove every executable old skill reference from durable agents and grant lists,
   workflow definitions, requirements and publications, saved-slice/relaunch
   manifests, queued launches/prompts and Environment/context exports, including
   skill references embedded in workflow bodies/generated launch code. Do not
   convert them to v2 IDs, selections or unresolved dependencies. Preserve all
   non-skill grants and unrelated workflow content. Rewrite ordinary owned records
   transactionally; invalidate immutable/signed publications or archives requiring
   removal and regenerate through the normal publisher/exporter, never edit a
   signed artifact in place. Do not run affected work with old references while
   reset is incomplete. Past transcript text is history, never replayable selection
   authority; queued skill selections (typed `$name` markers included) must be
   cleared or cancelled, never silently fall through to native invocation.
3. Commit a reset checkpoint/schema marker before accepting v2 mutations. Retire
   replayable old skill-bearing state so restart/retry cannot resurrect it; an
   interrupted reset resumes cleanup before launches proceed. A pre-v2 saved
   slice/Environment/publication that cannot be reset without a legacy reader is
   rejected with a recreate/republish action, not decoded by a compatibility shim.
   Apply the same rule to late imports and old remote peers. Delete old package
   copies only at exact product-owned paths; preserve unrelated saved-slice data.
4. Remove `ExtensionKind::Skill`, `grant_skill`/`revoke_skill`/`skill_grants`, skill
   branches in `request_extension`/`list_extensions`, worker grant synthesis,
   schemas, agent commands, prompt templates, workflow grant requirements and
   client grant controls. Delete legacy skill decode/migration helpers and tests;
   no old grant path remains. Keep MCP/connector/script/App authorization intact.
5. Add the v2 catalog/package/resolver and shared context assembler, state-aware
   discovery/load and agent install/update tools. Repo `.agents/skills`,
   `.claude/skills`, `.codex/skills`, OpenCode and user-authored `.chariox/skills`
   directories remain explicit import sources only. Editing a source cannot
   change an installed revision. Independent harness discovery is shown as
   `From <Provider>`.
6. Wire provider inventory/dedupe, source-aware `$` selection, Skills screen,
   next-turn hot reload, universal configurable 1% budget, package management,
   Environment Skills section/transfer and compaction recovery. Update the
   standard-worker skill clauses in `AGENTS.md` and protocol/architecture/M14B docs as explicitly approved; MCP
   behavior stays unchanged. Run focused reset/security/protocol regressions and
   the real scenarios below. OSS/Cloud changes remain coordinated separate
   commits; CI remains off until authorized.

The transition must never delete a newly installed v2 skill on a later restart.
No old binary may write v2 state. Old skills/workflow references are intentionally
not recoverable through a v2 compatibility/rollback mode.

## 8. Real-path validation scenarios — MP-08 / MP-10 / MP-11

All **14 rows below are NOT RUN**. The approved `skills-v2/SV-01` through `SV-12`
IDs remain; `SV-13` covers packages and `SV-14` separately covers agent self-install
and hot reload (Miguel, 18:22/18:25 UTC decisions).
This round delivers a plan, not a feature for owner testing. Implementers register
these rows with the coordinator and report each required dimension separately.

Common execution rules:

- Capture a real red on the frozen base (missing v2 flow counts only with recorded
  UI behavior), then replay on the paired implementation heads with the built app
  entry/flags, real CLI/TUI and kernel, hosted `wss` relay, and official providers.
  Use real ordinary and managed/Environment paths; direct IPC and component tests
  supplement user-driven acceptance. Pin binary hashes, harness versions and heads.
- Skills Web cells use **1440×900 DPR1 only**, plus the TUI; no display-density
  matrix is required for this area (Miguel, 2026-10-15 18:22 UTC). Drive normal clicks,
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
| `skills-v2/SV-01` — MP-08/MP-10 | **Skills management and focused-provider picker.** Fresh hosted Web at 1440×900 DPR1 plus local/remote TUI: install/import/update/remove, inspect source/files/version, select scopes and insert/delete `$name` chips amid text/attachments. Switch focus among real Codex/Claude/OpenCode agents: Chariox rows remain; only the focused provider/account/placement's native skills appear, labelled `From Codex`, `From Claude` or `From OpenCode` (never labelled Native). Select a native-only row and complete a real native load without creating any Chariox package or copying it to another machine. Reject stale native chips after focus/account change; test an explicit `From <Provider>` versus Chariox same-name choice, plain `$` Chariox precedence and no-focused-agent guidance. `/` stays commands; selection never submits prematurely; reconnect preserves focus/state. Base red: grant panel/no source-aware `$` picker. |
| `skills-v2/SV-02` — MP-08/MP-10 | **Project identity and states.** In two Projects, one spanning two repos/plain folder and one with no folders, verify a Project skill follows the Project across cwd/session changes and stays absent from the other. Exercise Auto/Manual/Off, default true/false frontmatter, explicit overrides, Project override/reset of a Chariox skill and update of inherited defaults. Real Codex and Claude turns demonstrate Auto discovery without `$`, Manual loading only with `$`, and Off rejection; recordings plus load receipts establish the result, not an agent assertion alone. |
| `skills-v2/SV-03` — MP-08/MP-10/MP-11 | **Collision and import isolation.** Through Web and TUI attempt same/case-only names across both scopes, including an archived Project, concurrent installs and Environment import. Refusal must preserve bytes/catalogs and offer working rename/replace; retry/stale revision cannot overwrite another change. Import each provider/repo source, then edit/delete that source: installed digest stays fixed. Opening a repo alone never installs it. Explicit replacement produces one binding. Base red includes unified import's automatic differing-content update. |
| `skills-v2/SV-04` — MP-08/MP-10 | **60-skill configurable budget.** Install 60 real reusable skills with bundled references and varied long descriptions; include Auto/Manual/Off controls. Real Codex/Claude tasks over MDN and public GitHub discover a relevant Auto skill, then explicitly load a Manual skill. An OpenCode-specific budget cell verifies its adapter too within the turn cap. For all providers record default 1%, resolved capacity, rendered token count and separate native overhead; repeat with a changed `config.toml` percentage and reject invalid values. Names/descriptions or paged overflow fit the configured ceiling; Manual/Off stay out of discovery; only relevant/selected bodies load and files resolve. Measure picker/echo gates. Base red: no common configurable budget and remote eager bodies. |
| `skills-v2/SV-05` — MP-08/MP-10 | **Compaction and continuity.** A ≥4-hour Codex/Claude research session across the public-site list loads an actual skill and bundled references, then reaches/requests real provider compaction. Continue the task from Web and TUI, reconnect and resume the run. Verify new context generation restores catalog/body/path before continued work, same thread/task, no duplicate action, truthful loaded status and stable resources. Disable/remove a loaded skill and verify no re-attachment; prior delivery remains visible in history. No fake compaction event or short smoke substitutes. |
| `skills-v2/SV-06` — MP-08/MP-10/MP-11 | **Codex harness dedupe.** With real Codex and its actual app-server catalog, exercise same name/full package, same body/different bundled asset, same name/different body, disabled native copy and changed profile/cwd. Confirm one delivery for equal effective copies, conflict UI for differing bytes, Chariox `$` winner, refresh after native changes and explicit unverified status for unreadable inventory. `From <Provider>` rows remain read-only; Import uses collision rules. Check prompt/load receipts and real task output. |
| `skills-v2/SV-07` — MP-08/MP-10/MP-11 | **Claude/OpenCode harness dedupe.** Repeat equal/different-content/native-disabled cases on real Claude using an account-profile `CLAUDE_CONFIG_DIR`, additional/plugin roots, and real OpenCode using its scoped config/XDG, compatibility and configured skill paths. Exercise an OpenCode-specific native skill load in ≤3 short turns per cell using the prescribed model. Verify effective native exposure/precedence, one delivery or conflict, `$` winner and native read-only Import; no credential/config payload in evidence. Record unsupported inventory seams as blockers for that dimension. |
| `skills-v2/SV-08` — MP-08/MP-10/MP-11 | **Standard leased worker and remote self-install.** From real Web + remote TUI run Codex/Claude on an ordinary second kernel without the skill. Home transfers Auto/Manual packages; load body/helper at worker paths under normal permissions. Ask a remote agent to install/update a home-managed skill: observe the home interaction, then both existing local and remote agents use the new revision next turn with unchanged process/thread IDs. Test a missed catalog event, transfer failure, home Off/remove, relay reconnect and lease replacement: fresh state applies, stale/foreign acknowledgements fail, no duplicate task or shadow worker catalog, no generated repo/provider files. Base red: standard-native no-copy rule and grant-based context. |
| `skills-v2/SV-09` — MP-08/MP-10 | **Slice lifecycle.** Launch a real local Docker slice through the user clients; select a skill with references/helper, use it on a public site with a real provider, save with idle agents, destroy/recreate through supported lifecycle and resume. Verify digest, modes, roots, state and provider thread, plus no leaked generated repo files. Run compatibility mode on the supported default. Restart/reconnect and cleanup must pass; a package-only unit test cannot satisfy persistence. |
| `skills-v2/SV-10` — MP-08/MP-10 | **Native TUIs and hot reload.** In real `chariox codex`, `chariox claude` and `chariox opencode`, submit Chariox-only `$name` locally, on a standard worker and in a slice; observe the same session from Web/TUI. Install/update from another agent, then select/use the new revision next turn without restarting native TUI/provider. Codex never reports unknown skill; Chariox selection beats a conflicting native copy; Claude uses hidden hook context; OpenCode uses the normal system channel. Also select a provider-native skill through the Chariox picker and verify native pass-through, no copy and one history/run. Typed and Chariox-origin prompts share ordering/permissions. Selected-body delivery failure cannot silently submit empty context. Record process/thread continuity; OpenCode turn caps apply. |
| `skills-v2/SV-11` — MP-08/MP-10/MP-11 | **Environment Skills and portability/parity.** Open the Environment Skills section in Web and its TUI equivalent; install/update/remove a Project skill, change state and regroup, and verify the main Skills screen shows the same revision without a second catalog. Inspect the move review and transfer receipts. Move a multi-folder Project with Project skills and Chariox overrides through normal Environments into BYOC and managed execution; compare the same user flow on an ordinary kernel. Verify package groups/membership/state/provenance and collision review in the destination Skills section, restart/resume, a Project with no repo, and independent evolution after export. Use existing b3 VM/container isolation policy; no isolation-only rental. Signed deployment and mandatory shutdown remain the only managed differences; cleanup covers only this run's targets. |
| `skills-v2/SV-12` — MP-08/MP-10/MP-11 | **Clean slate and authorization.** Begin with real old installed/granted skills plus workflow definitions/publications, saved slices, queued launches and Environment exports containing references. Upgrade with an interrupted reset/restart: installed skills and every executable old reference disappear, no automatic import/conversion/notice, other grants/data remain; old replay/import/restore cannot resurrect them. No compatibility/rollback path is exercised or retained. Denied catalog mutations change nothing; loading instructions never grants requested tools/credentials, collaborators retain existing mutation limits, and old clients/workers get actionable rejection. Source package/security/reset regressions supplement these real flows. Base red: installed grants/references persist. Agent self-install/hot reload is independently required in SV-14. |
| `skills-v2/SV-13` — MP-08/MP-10/MP-11 | **Packages and custom regrouping.** From hosted Web at 1440×900 DPR1 and the real TUI, install a real public multi-skill repository as one source-backed package; inspect source revision, member files and grouping. Load a member with a real Codex/Claude turn. Apply package Auto/Manual/Off/reset actions and a Project override; Mixed reflects an individual change, and hot reload uses member states next turn. Update the source-backed package with a reviewed diff; one colliding/invalid member or stale revision must refuse the entire batch, with no partial install/update. Create a custom package, move members from the source package and ungroup one: each skill has at most one package, stable IDs/digests/provenance/load records; updating the former source package never reclaims a moved member. An import without known package provenance stays ungrouped. Exercise custom member-source update, package removal versus Ungroup, deny/approve under ask and the normal yolo path. Inspect next-turn discovery and transfer to a leased execution place without restarting the provider; SV-11 verifies Environment export/import preserves the resulting groups. Base red: no package grouping/batch/custom-package flow. |
| `skills-v2/SV-14` — MP-08/MP-10/MP-11 | **Agent self-install and hot reload.** In real hosted Web at 1440×900 DPR1 plus local/remote TUI, ask a real agent to install and update a reusable skill in ask (deny then approve) and yolo. Observe one home-owned interaction; denial changes nothing. All already-running applicable agents see an approved change next turn: Auto discovery, explicit Manual load, Off exclusion, Project isolation and unchanged provider PIDs/thread IDs. Include Codex/Claude and an OpenCode-specific hot-reload cell within its three-short-turn cap on the prescribed model. A remote agent's install reaches the home catalog; existing local, leased, slice and native-TUI agents receive the new revision through normal paths (SV-08/SV-10). Update a loaded body and bundled helper: the next turn uses the new digest/path without replay or restart. Disconnect a client during publication and queue a turn before an update; durable revision checks catch up even after a missed event. Failed transfer visibly fails submission rather than using stale instructions. Record resolved model IDs, receipt digests and real task results. No extra tool/credential grant, shadow worker catalog or repo/provider-home write. Base red: agent-scoped grants and no scope-wide next-turn availability contract. |

Acceptance requires every required dimension PASS or an explicit Miguel waiver,
review of recordings and cleanup, plus focused protocol/reset/security
regressions. MP-11 review is the narrowed current security-anchor scope
(authorization, relay/lease admission, package containment and credential handling),
with the behavioral ordinary/managed matrix; it does not demand exact-blob review
of every non-security file. This plan closes no MP item and does not authorize
implementation, publishing or deployment before design sign-off.
