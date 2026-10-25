# Move an agent to another kernel

## MP-08 / MP-09 / MP-10 / MP-11 — scope and decision

Design and build plan, 2026-10-25. Source baseline:
`186c31efccc5104cef3fefde6660e84db6d42c50` (`movekernel/plan`).
This round changes documentation only. Every acceptance row below is **NOT RUN**;
neither the feature nor an MP item is accepted by this document.

The agent keeps its home kernel, Room, identity and history. Between turns, its
provider execution moves to an authorized kernel or slice through the existing
execution lease. The Prompt bar identifies both execution and home after the
move commits, in the prototype's existing `<execution> via <home>` order.
Moving back uses the same operation, with the home kernel as destination.
There is one agent and one prompt queue throughout; no copied conversation,
replacement Room or archived original.

All kernels are equal. “Home” identifies the kernel that owns this Room and
agent; “execution” identifies where its provider runs. These are roles, not
classes of kernel. A slice selects execution through its owning kernel's
existing worker path. Cloud discovers machines and coordinates managed start
and shutdown; the relay carries encrypted packets. Neither owns this move.

MP-08 requires the same move service on ordinary and managed kernels. MP-09
preserves mandatory managed shutdown and last-agent-finished accounting.
MP-10 requires real client/kernel/relay/provider drills. MP-11 requires the
behavioral comparison and current reviews of affected security anchors, not
exact-blob review of unrelated source. Read with the [agent model plan](AGENT_MODEL_OVERHAUL_PLAN.md),
[Path-1 requirements](BROWSER_COMPUTER_USE_END_TO_END_PLAN.md),
[parity inventory](MANAGED_PATH1_PARITY_INVENTORY.md) and
[M20 slice persistence contract](M20_DOCKER_SLICE_BROWSER_STATE_VALIDATION_PLAN.md).

## MP-08 / MP-10 — the person's flow

The read-only UI specification is `webterm/os-prototype-spec` at
`a697679c7a3189f83bc68161646b96c29c6440f8`, under
`apps/web/prototypes/os-shell/` in the prototype checkout. Changed picker
content or status meaning is a **proposal**, not permission to ship UI.
The r1 Move dropdown is superseded: it invented a component. The r2 proposal
changes only sample labels/status and destination rows in the existing Prompt kernel
route picker. It adds no Move button, provider-menu or details-flyout action,
custom popover or confirmation dialog. Selection requests a move of the focused
agent, preserving home; it must not inherit the prototype's sample Room-kernel
mutation.

The full menu inventory with files/selectors, capture index and proposal live
outside git in `/root/.chariox/dev/browser-resume-20260930/agents/movekernel/`
(`MENU_INVENTORY.md`, `PROPOSALS.md`). Evidence is under
`/root/.codex/evidence/browser-resume-20260930/movekernel/r2/`:
`01-move-entry.png`, `02-move-destination.png`, `03-move-route.png`, their
`baseline-*` counterparts and `side-by-side-*` comparisons. Each scene is
1440×900 at DPR1; comparisons place two scenes beside each other at native
resolution. `content-only.diff`, `style-diff.txt` and `source-hashes.json`
prove no CSS, class names, row markup, positioning code or component styles
changed. These are design evidence from copies, not product acceptance.

### MP-08 / MP-10 — component choice

The route picker already draws three kernel choices using
`openPromptConfigurationChoices` in `app.js`, selector
`.provider-menu.agent-option-submenu.prompt-option-submenu`, with
`.agent-option-choices > button.agent-option-choice > span` rows. Reuse this
function and its exact rows, selection treatment, status glyph, automatic size
and placement. Proposed slice labels name both slice and owning kernel in the
same text span, for example `Review intake · b3`. The mock's green glyph is
sample content, not proof of readiness; use actual destination status and kernel
preflight in implementation.

This adds the least because the route control exists, its list already contains
kernels, and its committed display expresses execution via home. Pane provider
menus configure providers; the Kernel App picker launches Apps. New Room's 15
setup rows only show `Configure …` toasts and have no destination subpicker.
The inventory also covers pane inline/collapsed menus, Prompt worktree/settings,
sidebar nested flyouts, Room card ⋮ and footer settings, workflow/schema/endpoint
pickers, System bar selectors and native file inputs.

1. Focus the agent, then open the existing **Kernel** route control in the
   Prompt bar (mock 01). It stays beside Worktree when provider settings
   collapse. The pane stays in its Room and desk. The real TUI's existing agent
   action surface submits the same home-kernel operation; its detailed key flow
   remains an implementation acceptance requirement.
2. Choose a reachable, authorized kernel or existing slice from that exact
   picker (mock 02). Include home for returning and select only the committed
   destination. Selecting a different row requests the move; selecting the
   current row does nothing. Slice labels include their owning kernel as plain
   text. No metadata column, badge, destination card or new dialog is proposed.
3. Preflight checks the peer version, provider and native resume support,
   account identity, workspace destination, Project setup and granted MCP/skill
   packages. If the same authenticated account is absent, offer the existing
   account-transfer flow. A label match is insufficient. Transfer uses the
   product's Vault/profile path; the person and model never handle raw exports.
   A different account is a separate context handoff, not a successful move.
   Workspace/account details stay in existing configuration surfaces. Failures
   use existing notice/history parts with a precise remedy; richer preflight UI
   needs a separate proposal, not a wider route row.
4. If the agent is running, record the pending operation at home and let the
   current turn finish normally. Do not steer, interrupt, compact or restart it
   to accelerate a move. Progress/cancellation content is **PROPOSAL: new
   element**: mock 04 reuses the existing toast and route-choice row, with a
   `Cancel move to b3` row while pending. The toast is a transition notice;
   durable phases/outcomes stay in home-owned History and reappear on reconnect.
   The Cancel row inherits the kernel row's green status glyph in this mock;
   that semantic tradeoff is part of the approval, not a new icon design.
   This content/meaning needs approval before implementation.
   New prompts and wakes remain ordered on home, behind the move boundary.
5. At the boundary, the proposed notice reports preparation using that same
   surface. The transcript remains readable. The Prompt bar continues to name
   the **committed** execution target until commit; progress never masquerades
   as the active route. The source provider is stopped and checkpointed, then
   the target verifies the worktree and native resume state before admission.
6. Commit replaces the execution binding atomically. Keep the same pane,
   transcript, agent label, selection and Room. The recommended unchanged
   prototype order is `b3 via Mac` (or `Review intake · b3 via Mac`), where Mac is home.
   A home execution has no `via` suffix. One normal home-owned history event records the move and safe outcome; no
   synthetic user prompt is added. The next queued turn resumes the session on
   the target through the official provider adapter.
7. To return, open the same Prompt route picker and choose home. Transfer the
   current execution worktree and session state back; do not resume a stale
   pre-move checkpoint. If files at home changed independently, choose another
   destination directory or resolve that divergence before retrying. Successful
   return removes `via`; the Room and history never moved.

**Route order:** the original feature brief says “via <target>”, but the prototype
`focusedPromptConfiguration`/`promptConfigurationValue` and c#451
`executionRouteView` use `<execution target> via <home>`. The recommendation
is to preserve that order, as mock 03 shows. The r1 reversed-order alternative
is withdrawn. Home/execution facts come from the committed kernel binding;
the control never reverses the renderer's meaning.

Cancel before commit releases only this move's staging and queue barrier. It
restores source dispatch once source ownership is certain. Explicit account
transfer or package installation may already have completed; do not delete
those independent target resources as cancellation cleanup. After commit,
returning is a new move. Client disconnect never cancels a move; reconnecting
clients read the same persisted status. Request retries use one idempotency
key. A second destination request while a move is pending reports the current
operation and requires its cancellation before replacement.

## MP-08 / MP-11 — what moves and what stays

| Resource | Contract |
| --- | --- |
| Provider session | Move the selected native session's durable resume artifacts and account binding, not just its session ID. Keep provider, model, variant and supported interface. A target may assign a new native ID only through an official import that preserves session continuity; store the old/new mapping privately. |
| Provider process | End the old provider process at the turn boundary; start a new process through the target's ordinary official adapter. No process-memory, PTY or open-connection migration. A native TUI must detach and reconnect through its existing home attachment; it cannot keep writing to the retired run. |
| Workspace/worktree | Send an immutable, verified snapshot of the actual execution worktree once per move. Reuse the existing workspace transfer/publication mechanisms; extend their manifest where necessary. No background sync or automatic push. |
| Provider account | Reuse an independently installed matching account or explicitly transfer the selected portable profile through existing encrypted account transfer and Vault materialization. No whole-Vault copy, profile-tree copy or refresh-token handling in clients/model tools. Target profile IDs may differ. |
| MCP and skills | Transfer/reconcile the agent's exact granted package set through existing capability mechanisms. Preserve each extension's home/worker execution authority rather than relocate every MCP server; home-owned tools keep their authenticated forwarding route. Rebind host paths and target credentials through ordinary setup. Verify hashes, target compatibility and current grants before readiness. Installation does not create new authority. |
| Project environment | Materialize the existing selected Project environment and run its normal target-native setup. Do not copy compiled dependencies across macOS/Linux, provider caches or a source kernel's ambient environment. Missing services or private files produce actionable preflight/setup errors. |
| Room and agent | Home retains `session_id`, `agent_id`, immutable creator lineage, Room/workflow membership, prompt ordering, task/goal/wait state, history, capability authority and shared RuntimeInteractions. The worker backing agent is an execution detail, never a second Room member. |
| Vault | Each kernel retains its Vault and keys. Only the selected transferable credential material crosses the encrypted product transfer boundary. A handle from home is not a target Vault handle. Revoke/mask under existing rules; do not echo values in transfer status, traces or history. |
| Browser/Computer | Retain the Room's existing designated Environment and routing. Moving execution does not migrate a user-domain browser, cookies, tabs or desktop, and does not grant cross-kernel user-domain control. Slice saved browser state remains the separate M20 contract. |

### MP-08 / MP-10 — why a snapshot, not a transfer branch

A Git branch alone omits staged changes, unstaged changes, untracked files and
non-Git workspaces, and requires a writable upstream. Pushing also publishes
work unnecessarily. Choose a point-in-time snapshot using the existing
Git bundle/workspace transfer path, which already has independent index and
working-file overlay states, untracked collection, non-Git workspaces and a
verification pass. Reuse its manifest for HEAD, index, tracked working bytes
and selected untracked/private Project files. Preserve binaries and modes
through that existing mechanism. It currently
refuses dirty symlinks and submodules: retain that clear refusal for initial
support, or extend validated entry types in a bounded follow-up. Account for
submodule commit/dirty state and Git LFS objects needed by the checked-out files. Recreate worktree metadata for its new root;
never copy a linked worktree's absolute `.git` pointer blindly. Plain
directories use the same manifest without invented Git history.

The snapshot is captured after the source turn settles and source execution is
quiescent. Other agents may still share those files: use the existing snapshot
consistency mechanism, or compare manifests and refuse a changing tree with
“Workspace changed during preparation; retry when writes finish.” Do not freeze
unrelated agents. Preserve all advertised file classes or refuse explicitly;
never report success after silently excluding an unsupported dirty file.
Ignored files are not ordinary untracked files: transfer the explicitly selected
Project/private files through existing protected artifact handling; name missing
required ignored files instead of silently dropping them or copying every cache.

Import into a move-owned staging directory, verify hashes/size/path policy,
then atomically publish to an unused destination. Keep the source worktree.
An existing destination can be reused only when its recorded snapshot matches
and it has not changed; otherwise offer a fresh directory or a user-resolved
merge. Never reset, stash, overwrite or merge the person's work automatically.
After commit, source and target copies evolve independently. Returning takes
another explicit snapshot from the current execution location. Later launches
never refresh target credentials or source files implicitly.

### MP-08 / MP-11 — native resume is a release gate

`provider_session_id` and generic resume flags are references, not portable
session contents. Account transfer explicitly excludes provider session stores.
Use only official CLI/server/export/import seams already permitted by the
provider contract. Do not parse and rewrite private provider databases, copy
an entire account root, manufacture history, or substitute a fresh provider run
with a transcript prompt. Same-machine resume is not proof of cross-machine
or cross-OS portability.

Before enabling a provider/interface pair, prove export, import and next-turn
resume through its official tools, including worktree-path relocation. Record
the required artifact format/version and compatible CLI versions. If the
provider cannot do that, return “This provider session cannot move to this
target yet” with its precise reason. Keep source usable. The build plan starts
with this feasibility gate; unsupported pairs cannot be counted as accepted.
Also enforce a required-resume constraint at launch: current OpenCode resume
failure creates a fresh session, and Codex/Claude have their own missing-session
recovery. Those recovery paths must report an explicit move failure when native
continuity is required, including the first turn after commit. Do not weaken
ordinary recovery unrelated to a move. A retained checkpoint alone is not proof
that the official provider resumed it.

Waiting tasks may move if their sources remain valid at home. Home timers,
peer completions and Room interactions stay there. A live execution-bound
process, terminal or watcher cannot become a target process by copying its
receipt. Keep it supervised on its original execution lease only if the
existing source routing supports that lifetime independently; otherwise block
the move with the exact resource and its stop/settle action. Never drop an
obligation or relaunch a command as part of the move. Pending provider-native
permission input is part of the current turn and must settle first.

## MP-08 / MP-09 / MP-11 — boundary, commit and recovery

Add a small durable move operation owned by home, not a new kind of agent,
lease, scheduler or session transfer. Its proposed states are:

```text
requested -> waiting_for_turn -> preparing -> ready -> committed -> complete
      \             \             \          \
       +-------------+-------------+-----------> cancelled / failed
```

“Committed” is the authority boundary; “complete” also confirms retirement and
cleanup receipts. Uncertain network outcomes retain their last durable phase
and a visible recovery reason until queried; they are not guessed into failure
or success. The operation binds home Room/agent, expected source execution
generation and run, destination kernel/slice, idempotency key, checkpoint and
manifest digests, target preparation receipt and phase/revision. Store account
and artifact references privately; project only safe status metadata.

1. Home serializes the request with prompt/wake/workflow dispatch. A running
   turn, completion correction, queued steer or uncertain send must finish or
   reconcile before checkpoint. Hold subsequent dispatch behind the barrier.
   An idle provider process still needs adapter flush and confirmed teardown.
2. Preflight can run while the turn finishes, but final readiness is checked
   against the settled run, account, grant and workspace revisions. A revoked
   grant or changed snapshot invalidates preparation. Pin target identity/key
   and lease generation, not its display name.
3. Fence source execution and obtain its stop receipt. A disconnected worker
   is not proof of stopping. Before activating another worker, require either
   verified source teardown or expiry of an existing execution lease that is
   enforced by the worker before provider/tool effects. If current leases lack
   this guarantee, implement and drill it before permitting such a move. A
   stale worker must stop locally when supervision expires, even in a partition.
4. Target stages the snapshot, account mapping, capability requirements and
   resume artifacts through authenticated encrypted peer operations. It returns
   a verified readiness receipt, not permission to start a turn. Probe official
   resume support without submitting user work or running two provider sessions.
5. Home atomically compare-and-swaps the expected source binding to the target
   lease/execution generation and records commit. Target dispatch requires that
   committed generation. Home sends each queued prompt once using existing
   prompt IDs and receipts; stale source output, native input and completions
   cannot settle target turns. A worker provisional completion never settles
   home by itself.
6. Replay after restart queries the same operation/lease receipts. Before
   commit, abort staging or resume preparation; restore source only after its
   stopped/retired state is known. After commit, repair target activation or
   report target unavailable; never automatically execute source's stale
   checkpoint. A lost commit reply is resolved by query, not another move.
   Clean only operation-owned staging and transferred temporary artifacts.

Use the existing peer binding and heartbeat/lease machinery. Receipt delivery
must be idempotent and bounded; lost supervision produces an actionable status.
Home being unavailable prevents new home-ordered turns on every execution
kernel. This is an existing home/worker dependency, not an independent kernel
transfer. Full kernel transfer still produces independent kernels; this feature
must not add a hidden ongoing sync obligation to that transfer.

MP-09: active transfer/setup work uses ordinary lifecycle activity accounting;
it must end or time out, not pin a managed machine indefinitely. Source last-
agent-finished follows actual provider completion/teardown, not route rendering.
The target is active only for real admitted preparation/execution work. A
queued move, idle lease or future home timer does not disable mandatory STOP.
Every existing managed shutdown trigger remains in force; target shutdown
during a move is handled as target loss. Normal restart reconciles persisted
state without a new Cloud wake promise.

## MP-08 / MP-10 — failures the person can recover from

All clients show the same home-owned operation and reason. Keep the committed
route truthful and preserve queued prompts and readable history.

| Failure | Kernel outcome and visible recovery |
| --- | --- |
| Target disappears before commit | Time out preparation; show “Move to b3 failed: target unavailable. Retry or choose another kernel.” Keep old route. Resume source only when its execution ownership is proven; otherwise show that source recovery is pending. |
| Target disappears after commit | Show `b3 via Mac` (recommended order) with unavailable status and keep prompts queued. Reconnect/query the same target run and receipt. Offer retry or Move back only using the latest durable target checkpoint; if it cannot be obtained, say so and require an explicit separate recovery decision. No automatic replay of uncertain work. |
| Account missing, locked or not portable | Preflight identifies the selected account and opens the existing transfer/unlock/login path. No raw secret leaves product surfaces. A rejected transfer leaves source binding unchanged. Do not overwrite an independently changed target profile. |
| Dirty/conflicting target worktree | Preserve both copies. Show the conflicting path/revision safely; offer a new directory or resolution outside the move, then Retry. A source tree changing during capture likewise refuses with a retry reason. |
| Older or unsupported peer | Reject before transfer/spawn with “Update/restart <kernel> to move this session.” Recheck negotiated version on restored leases. Never allocate a partial worker and fall back to a copied conversation. |
| Unsupported native resume / target architecture | Explain the provider/interface or Project dependency that cannot relocate. Retain source. Do not change providers/accounts/models to make the move appear successful. |
| Missing MCP/skill, package mismatch or setup failure | Name the missing requirement and reuse normal install/grant/setup actions. Readiness remains false; retry checks the new versions. Never silently drop a granted tool or import its authority from a display label. |
| Source partition / home restart / lost acknowledgement | Show the persisted phase and recovery reason. Query exact run/lease/move receipts; fence stale effects before replacement. Cancel cannot bypass uncertain source ownership. |
| Disk full, invalid archive or expired transfer | Fail with the staging/import reason and retry action. Preserve source and unrelated target data; remove only owned temporary material. Never expose credential or session payloads in the error. |
| Source-bound open obligation | Show the exact live resource and how to finish/stop it or keep it supervised through its existing route. A move never marks a task done or silently cancels a watcher. |

## MP-08 / MP-11 — implementation seams and protocol

The source inventory below distinguishes existing foundations from proposed
extensions. The older agent-model plan is design context; current code and
`PROTOCOL.md` establish what actually exists at this baseline.

| Existing seam at the OSS baseline | Reuse and required extension |
| --- | --- |
| [Agent types](../apps/kernel/src/agent/types.rs): `AgentInstance`, `RemoteAgentBinding`, `GitWorktreePlacement` | Home identity and existing worker/kernel/lease/run fields. Add safe optional move status; replace the binding without recreating the home agent. Keep private relay credentials out of new projections. |
| [Lease types](../apps/kernel/src/execution_lease.rs): `ExecutionLease`, `LeasedAgent`, `LeasedCompletionReplay` | Existing home/worker identity, backing-agent and prompt correlation. Add only the retirement/readiness receipt data missing for a between-turn move. |
| [Binding refresh](../apps/kernel/src/app/remote_agent_binding.rs): `prepare_remote_agent_binding_refresh`, `commit_remote_agent_binding_refresh`, `refresh_remote_agent_binding_to_worker_kernel_with_operation` | Existing prepare/commit expected-binding comparison and cleanup. Reuse this seam; it is not proof of native-session/workspace migration. Integrate durable move phase and native checkpoint readiness rather than invent another binding service. |
| [Worker lifecycle](../apps/kernel/src/app/remote_lease.rs): `destroy_execution_lease`, `reconcile_expired_execution_leases` | Existing idempotent destruction and one-hour absolute ceiling; expiration reconciliation currently applies to `RemoteLeaseWorker` role only. Prove teardown/partition fencing on ordinary execution kernels too; a heartbeat field or home CAS alone cannot stop an isolated provider process. |
| [Peer protocol](../apps/kernel/src/transport/relay_peer.rs): `CreateExecutionLease`, `SpawnLeasedAgent`, `DestroyLeasedAgent`, `DestroyExecutionLease`, `SubmitLeasedPrompt`, `ExecutionLeaseCreated` | Extend existing E2EE envelopes, protocol admission and worker receipts. `SpawnLeasedAgent` currently accepts profile/worktree configuration, not a portable provider checkpoint. |
| [Home dispatch](../apps/kernel/src/runtime/state/remote_prompt_dispatch_runtime.rs) and [worker submission](../apps/kernel/src/runtime/state/remote_prompt_worker_submission_runtime.rs) | Keep one home prompt queue, exact run/lease/home-prompt fences and existing uncertainty reconciliation. Enforce the move barrier before automatic queued/wake/workflow dispatch and native launch. |
| [Provider resume](../apps/kernel/src/provider/launch_contract.rs): `ProviderResumeState`; [Codex](../apps/kernel/src/provider/codex_client/thread_runtime.rs), [Claude](../apps/kernel/src/provider/claude_runtime/session_restart.rs), [OpenCode](../apps/kernel/src/provider/opencode_binding.rs) | Existing resume state contains only three native session IDs; adapters use `thread/resume`, `--resume` and native session snapshot respectively. Implement official portable artifact handling and required-resume failure semantics; none of these IDs is a migration package. |
| [Development transfer types](../apps/kernel/src/managed_context/development.rs): `DevelopmentContextManifest`, `DevelopmentRepositoryManifest`, `DevelopmentOverlayEntry`, `DevelopmentFileState`; [overlay](../apps/kernel/src/managed_context/development/overlay.rs) | Already models Git/non-Git, bundle digest, index and working states. Reuse export/verify/import; explicitly retain or extend current dirty symlink/submodule refusals. Preserve no-clobber target publication. |
| [Transfer selection](../apps/kernel/src/managed_context/package.rs): `ManagedContextPlanBinding`, `ManagedContextKernelSelection`, `ManagedContextDevelopmentSelection`, `ManagedContextProviderAccountSelection`; [owner-managed admission](../apps/kernel/src/managed_context/owner_managed.rs) | Existing transfer “items” are kernel context, Project development context, selected accounts and Git credentials. Select the needed resources; do not copy the full kernel context or import a new Room for an agent move. Existing owner-managed selection does not by itself enable every account selection: compose the authorized account path explicitly. |
| [Kernel context](../apps/kernel/src/managed_context/kernel.rs): `KernelContextPayload`; [ordinary publication](../apps/kernel/src/managed_context/kernel/ordinary_publication.rs) | Transfers extensions/dependencies and optionally a sealed Vault snapshot for the separate kernel transfer feature. Agent move uses selected extension materialization, never this full Vault snapshot. Keep independent target registries and guarded rollback. |
| [Accounts](../apps/kernel/src/account_profile.rs): `ProviderAccountMaterialization`, `export_managed_context_materialization`; [preflight](../apps/kernel/src/runtime/provider_account_portability.rs); [account contract](PROVIDER_ACCOUNTS.md) | Existing owner-bound credential export/materialization and portability check; reuse encrypted transfer and independent target identity. It explicitly is not provider-session migration. |
| [Extension manifest](../apps/kernel/src/extension.rs): `RemoteExtensionManifest`, authority/execution-location types; [skill admission](../apps/kernel/src/app/remote_lease/skill_sync.rs): `RequiredRemoteSkill` checks | Preserve home/worker tool execution locations. Standard workers verify local package hashes; home-managed slices can materialize packages through existing binding paths. Stage only missing authorized target-local packages; preserve revocation and credential bindings. |
| [Shared client API](../apps/kernel/src/local/api/types/request.rs), [managed transfer requests](../apps/kernel/src/local/api/types/managed_context.rs), [protocol contract](PROTOCOL.md) §3.3.2 | Add move request/status/cancel shapes and optional status events; share generated clients and native-TUI admission. Reuse transfer result/error surfaces and keep account/session bytes private. |

UI evidence is a separate checkout: prototype `app.js` lines 1813–1840
(provider/options menu), 2046–2075 (menu placement), 2116–2148 (route).
c#451's inspected local head is
`182bf7f21c338cd6a68dd9d28a1f846090f67d1e` in `agent-provusage`:
`apps/web/src/terminal-os/prompt-settings.tsx:51` renders the route and
`packages/terminal-domain/src/usage-view.ts:34` presents kernel-owned
`AgentUsageSnapshot.route` with independent target/home reachability. Reuse
that projection; do not derive route from the currently focused client kernel.

Keep orchestration in one focused home move service with typed commands for
request, status and pre-commit cancellation. Add corresponding shared API
requests/responses and an optional safe move projection on the existing agent
snapshot/event path. Clients render it; they never independently copy state,
decide readiness or switch leases. Bind every mutation at the shared boundary
to current owner/Room/agent, source generation, move revision and authenticated
target lease. Recheck after waits and before effects. User actions use existing
owner authorization; any future agent-callable action follows current Room,
creator and `/sudo` rules rather than gaining peer administration by default.

Extend existing encrypted lease/transfer envelopes with staged checkpoint
import and readiness/commit/retirement receipts. Keep private artifacts out of
client snapshots and normal transfer summaries. Import validates destination
paths, archive traversal/symlinks, hash/size bounds and target identity once at
the boundary. Target-held credentials remain target-owned after explicit
transfer. No credential refresh/overwrite on every prompt or kernel restart.

Request protocol allocation from the coordinator at implementation time;
this document allocates none. Bump `LOCAL_DAEMON_PROTOCOL_VERSION` for shared
client shapes and `RELAY_PEER_PROTOCOL_VERSION` for changed peer shapes. Update
Rust protocol snapshots, hash tests, generated shared clients and conformance.
Raise client minimums only for clients needing the new operation. Explicitly
gate move capability before staging; old peers still support their existing
non-move operations. Preserve release deployment and shutdown as the only
managed exceptions.

## MP-08 / MP-09 / MP-10 / MP-11 — bounded build plan

Each numbered step is a separate reviewable PR (1a, 1b and 1c are separate
provider PRs), with its own failing regression through
the named real entry point, followed by the fix and passing regression. Unit
or fixture results are supplementary. Do not expose a usable Move action until
the native resume gate, retirement fence and real integrated path pass. Each
PR runs its affected full checks; protocol changes add conformance. Publish
only when the round is complete, per coordinator policy.

| PR | Bounded change and dependency | Acceptance row and fail-first check |
| --- | --- | --- |
| 1a / 1b / 1c — separate PRs | One native-resume adapter per provider: Codex, Claude, OpenCode. The first defines only the shared checkpoint contract; the others use it. Each starts with its official-tool feasibility proof, supports its own documented interfaces and adds required-resume failure semantics at the ordinary provider boundary. No UI dependency. | MK01 (separate provider/interface subcells): on the unmodified base, a real idle session ID alone cannot resume on a second machine. After: official export/import resumes the same session and context at a relocated workspace without a fresh transcript prompt. Repeat for each supported provider/interface and both OS directions; unsupported pairs return the explicit error. |
| 2 | Reuse existing dirty/index/untracked/non-Git workspace transfer and atomic staging; add only proven missing move requirements and target divergence checks. Reuse existing Project setup. Independent of provider artifacts. | MK02: through real transfer/import, compare exact HEAD/index/files including binary, modes and relevant LFS/submodules on target and return. Base loses or rejects required content; after preserves it. Divergent destination and changing source refuse without mutation. |
| 3 | Add quiescence/retirement guarantees missing from existing leases and persist the home move operation, queue barrier and restart reconciliation. Depends on the guarantees identified in PR1; no client orchestration. | MK03: request during real provider work, queue another prompt, partition source, replay old receipts and restart home at every boundary. Base cannot request the move or violates the new boundary; after there is exactly one executable generation, one next prompt and no stale completion. |
| 4 | Compose staged native checkpoint/workspace import with existing account, MCP/skill and environment transfer. Add typed readiness and commit receipts. Depends on 1–3. | MK04: real account transfer plus granted tool/skill execution after move; base lacks session migration. After: same authenticated account, selected packages and native session work on target; changed grants, missing package, locked Vault, invalid archive and old peer reject before activation with no leaks/overwrites. |
| 5 | Wire move semantics to the approved existing Prompt route picker and route projection; add only approved notice/cancellation content and equivalent real TUI action through the same API. Depends on approved proposal and 4. | MK05: record normal menu/key flow on real web and TUI, including running-turn wait, Cancel, disconnect/reconnect and return. Base has no move flow; after shows one pane/history and correct committed `via` on every client. |
| 6 | Complete live failure, managed parity and lifecycle drills on the integrated head; fix any uncovered behavior in its owning module. Depends on 1–5; no new deployment/runtime path. | MK06–MK12 below: paired ordinary/managed outcomes, every shutdown trigger, realistic failures, security-anchor review and cleanup. No aggregate PASS from focused checks alone. |

### MP-10 — live acceptance cells

New tracker area proposed: `movekernel`, rows `movekernel/MK01` through
`movekernel/MK12`. MK01–MK05 are defined above; all start NOT RUN. Each row's
manifest must enumerate provider/interface/OS combinations rather than hide
them under one successful Codex turn. Account for Codex, Claude and OpenCode
official modes; declare support only where MK01 passed. A missing official
portable resume path is a provider-specific implementation block, not a waiver
of the brief's session continuity requirement.

| Row | Real scenario and pass condition |
| --- | --- |
| MK06 | **Mac kernel → b3 kernel → Mac kernel.** Start real repository work and a provider session on the Mac, move while idle and again during a turn, run the next task on b3, return with changed files and context. Stable home IDs/Room/history, one ordered queue, correct `via`, native session continuity, exact workspace bytes and target-native setup. Record ordinary b3 then managed b3 placement with the same code and normalized behavior. |
| MK07 | **Kernel → slice → kernel.** Use a real existing Docker slice and its ordinary leased provider path. Move during work, resume, execute a granted MCP and skill, return with changes. Verify worker-local packages, environment and account; do not treat a slice browser snapshot as session migration. Repeat with a slice owned by another enrolled execution kernel if that combination is offered by the picker. |
| MK08 | **Execution kernel A → execution kernel B while home remains Mac.** Move an already leased agent, return home, exercise multiple clients and a real workflow claim/durable wait. No new Room member, duplicated queue, lost obligation, home authority transfer or stale completion. |
| MK09 | **Real loss/recovery.** Disconnect the hosted relay path, stop the exact owned worker, restart home/worker, lose a commit reply and cancel before commit in separate runs. Cover before/after commit and partitioned old worker. Recover from exact receipts with one effect and visible outcome; no stale native input or prompt replay. |
| MK10 | **Actionable refusal/retry.** Older real kernel, missing/nonportable account, locked Vault, changed destination, dirty source, missing package, revoked grant, incompatible native resume, failed setup and full owned staging volume. Recover through normal product UI; no silent fresh session, file reset or unrelated account replacement. |
| MK11 | **Ordinary/managed lifecycle.** Repeat success and loss paths and every existing mandatory shutdown trigger, including last-agent-finished delay, disabled/keep-running modes and restart reconciliation. Record provider finish/teardown, transfer activity and STOP/start receipts. No permanent activity from a queued move, idle lease or wait; parity differences limited to deployment/shutdown. |
| MK12 | **Security and sustained usability.** Current semantic review of changed lease admission/peer gating, Vault/account transfer, signing/trust/capability packages, sudo/permissions, process signal guards and browser observation boundaries. Run a multi-hour real work session with repeated moves, browser work and reconnects; no stuck pane, stranded task, unmasked product-injected Vault field, duplicate provider or leaked resource. |

For every live row, pin source/target/kernel/client/relay/provider CLI versions,
commits and binary hashes. Drive normal real web clicks/keys and real TUI
commands, with official providers and product-linked accounts. Record identity,
queue/turn/run/lease receipts, checkpoint and workspace digests, phase and
activity timeline, screenshots/video and sanitized logs **outside the repo**.
Capture a base failure and fixed-head pass with the same scenario. Inspect the
recording: any confusing blocked state, blank pane or wrong route is a failure.
Do not expose native session content, credentials or private artifacts in the
evidence; secret scans report booleans and classifications only.

Use the real hosted `wss://` relay and realistic network shaping (~8 Mbit/s,
measured RTT at least 60 ms), desktop DPR1 at 1920×1080 and 1440×900, and DPR2.
Any prompt/echo/pane latency gate is P95 ≤ measured RTT + 60 ms, diagnostics
off. On timing rows use the coordinator's quiet-host/I/O-control method.
During sustained browser work use the fixed real-site set: Wikipedia article
and portal, GitHub repository, DuckDuckGo/Bing search, a major news site and
MDN. Google is N/A-AUTOMATED under the later owner decision. This feature does
not alter display transport, but affected browser routes still need the
existing screenshot, coverage, smoothness and observation-protection checks.
Do not replace a missing real resource with a fixture and label it acceptance.

Use the lane's evidence root and `ACCEPTANCE_UPDATE.json` with exact IDs,
status, head, date, evidence and note; new rows include `new: true`. Missing
machines/accounts go under Coordinator asks; owner decisions under Owner
questions. Only mark product completion after all required supported cells
pass and applicable security reviews are current. This design round creates
the plan, not live acceptance results.

## MP-08 / MP-10 — questions for Miguel

1. **Route selection means move:** approve choosing another destination directly
   in the existing Prompt Kernel picker, including slice rows with owning-kernel
   text, without another Move action or confirmation dialog? Recommendation:
   yes, preserve exact prototype rows and `b3 via Mac` order (r2 mocks 01–03).
2. **Pending move content:** approve the separately labelled **PROPOSAL: new
   element** in mock 04: existing toast for transitions, existing History for
   durable phases and a plain `Cancel move to b3` route row? Recommendation:
   reuse these parts; no custom progress popover or stripe is authorized.
3. **Provider release scope:** if an official provider cannot relocate its native
   session, ship proven provider/interface pairs first? Recommendation: yes,
   with an explicit unsupported reason for the others and separate acceptance
   rows. Never label a fresh context handoff as a session move.

No answer is required to complete this documentation assignment. UI work waits
for proposal approval; provider support waits for actual native-resume proof.
