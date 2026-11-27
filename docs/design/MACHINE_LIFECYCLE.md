# Machine lifecycle

Design for Miguel's review, 2026-11-27. Source queue row **25**, owner decisions **D246, corrected by D249 and reviewed in D251 / D252 / D253 / D254 / D255**. Target **L15.2 / design review**; excluded from the L15.1 cut. This document records the decided shape and remaining questions. It implements no behaviour and grants no acceptance PASS. **Owner design is ready for implementation after a fresh technical review of this revision.** D253 answers items 1 and 5; D254 answers item 4 and lifts the previous owner hold. The coordinator launches implementation by normal queue order and resources; this docs lane implements nothing. D246 ends the November 20 hold on lifecycle design only. D249 limits this document to Path-1 managed machines, offloading agent-private VMs and image management. Remote sub-agents remain separate in row 770 / `sub-agents.md`.

An agent chooses where to work. The user's own machines stay visible and untouched by rental lifecycle. A managed rented machine is a visible computer with recoverable state. An agent-private VM is temporary capacity: its agent and parent tree can use it, it never becomes a machine for the user to manage, and it must have a teardown deadline before it exists.

## Decided kinds

| Kind | Visibility | State and lifetime | Stop / Delete |
| --- | --- | --- | --- |
| **OWN physical machines**: laptop, b3, nelpx, BYOM | Visible as the user's machines and kernels. | User-owned, ordinary kernel state. | Never stopped or imaged by this machine lifecycle. Rental teardown does not apply. |
| **MANAGED rented machines** | Visible, including their agents, context and state. | Ordinary independent kernel; state is saved using the existing Path-1 imaging/state-bundle shape. | Stopping or deleting saves an image. Stop permits later restoration; Delete removes the kernel and its Rooms at once. Saved images use Machines > Images and the shared saved-image start-from flow below; orphan recovery images use the decided retention and rescue flow below. |
| **AGENT-PRIVATE VMs** for offload, build or test | **Invisible in inventories/pickers** to the user and other agents: absent from machine/kernel lists and session pickers. Only the agent and parent tree address the VM. Pane remote-work activity, Health processes and billing remain visible (D253). Home kernel keeps a log entry only. | **Ephemeral**. Boot from an immutable **SOURCE image: OS + current kernel for speed, NEVER state** (row **25a**). Acquire must specify a deadline/teardown time; absence is refused. The agent may extend it. | Agent commits/pushes or moves work home, then **deletes**. Deleted, crashed or stuck agent: VM auto-deletes at the deadline. A kernel precaution shutdown automatically saves a recovery image unless a limit is hit. Deadline deletion still wins over keeping data. |

A future **Assistant kernel** type fits beside these kinds. Its visibility, lifetime and authority are not decided here; this is not an Assistant implementation or a fourth invented policy.

Use **Delete**, never **Release**, in the product action. Delete removes kernel and Rooms together, rather than leaving a disconnected kernel or Room behind for the user to clear. Physical infrastructure cleanup must still be reconciled honestly: a failed provider deletion cannot be reported as stopped billing or successful cleanup.

## One visible managed acquisition flow (D251 / D252)

**Rent is Path 1 without the Room.** Machines > Rent and New Room use **one shared flow**, with the same choices, transfer and readiness result:

```text
Size / provider
    -> Start from: fresh / COPY of an owner's kernel / SAVED IMAGE
    -> Environments to copy
    -> Auto-stop
    -> Provision, transfer or restore, and verify ready
         Machines > Rent: ready visible managed machine
         New Room:       that machine + first Room and agent
```

Machines > Rent ends with a usable, visible managed machine; it does not require creating a Room or agent. New Room bundles that same machine flow with the first Room and agent. Selection of a COPY means an independent copy of the owner's kernel through the existing context/account transfer boundaries, not an ongoing dependency on that source. Selecting a SAVED IMAGE restores the chosen saved machine state; it is not an offload SOURCE image. The environments-to-copy step is shared across both entry points. This design does not add a new transfer protocol, pricing choice or provider-specific runtime.

**Agent parity uses that same visible managed flow.** An agent may rent a visible managed machine using the same size/provider, start-from, environments and auto-stop choices and receive the same ready result. An agent caller does not make that machine private. Explicit acquisition of an **agent-private offload VM** is a separate lifecycle kind with its stateless SOURCE boot and mandatory teardown time; do not route all agent acquisitions to it automatically.

Source reuse is not verification: Cloud main already has Rent/New Room using common machine rows, start-from and environment helpers. That does not prove the deployed flow exposes every choice or that the entry points behave identically. D251 / D252 report Rent as fresh-only today; the table below records that live owner symptom separately from what source components can render.

## Git and provider defaults on new kernels (D255)

**Every new managed Path-1/Rent/New Room kernel and every new private offload kernel includes Git and `gh`.** During provisioning, the **default GitHub credential** and **default travel-capable provider accounts** move through the **existing selected transfer**, including when Start from is **Fresh**. Fresh controls kernel context; it does not mean an implicit credential opt-out. This is a one-time authorized transfer into an independent kernel, never credentials forwarded for each provider launch, prompt or tool call.

Use the selected copy kernel as the source for COPY. For Fresh, use a reachable owner kernel. Keep the existing source/target identity binding, recipient authorization, encryption and account exportability checks. The UI defaults this credential/account transfer **on**, with a **per-launch None opt-out**. Non-travel-capable accounts are not made portable by this default. Owned laptop/b3/nelpx/BYOM transfer remains **opt-in and unchanged**.

**No source reachable:** launch the new machine and show a **status line with no error text**. Do not invent credentials, fetch them on every provider run or silently copy a different secret. Tool installation alone is not credential-transfer acceptance.

Reuse [M28 §6 provider and SCM runtime](../M28_ENVIRONMENT_CONTEXT_MATERIALIZATION_PLAN.md#6-provider-and-scm-runtime), [protocol 319](../PROTOCOL.md) `PrepareManagedEnvironmentGitCredentialEnrollment` and its `gitCredentials: none | selected` contract, and the independent context/account/Git layers in [the control-plane boundary](../CONTROL_PLANE_MACHINE_BOUNDARY.md). Default-on fills the existing selected launch plan; explicit None keeps the existing opt-out. The [MP08 drill](../drills/MP08_MANAGED_ENVIRONMENT_PROVIDERS.md) already records `gh` as required in the base image for Git credential import. This changes defaults and installation/acceptance requirements, **not credential transport**, and exposes no secret to the model.

Git acceptance is **P1, existing row 270, blocking Path-1 owner flow row 271**. Provider-install implementation remains row 582; preserve its existing PRs and acceptance rather than create a new account-transfer design. D255's additional mechanical check is running; the completed d252 SAME mapping already establishes the existing Git destination.

## Lifetime and visibility flow

### Agent-private VM

1. **Agent chooses placement** from its context: local work, another appropriate existing machine, or a temporary VM. The kernel does not intervene in that choice. Existing runtime protection still protects the machine; this design adds no placement threshold, scheduler, resource cap or admission heuristic.
2. **Acquire with teardown time.** Missing deadline is refused before allocating. Work within the allowance needs no user question. Above the allowance the owner accepts or raises the limit. The deadline is a VM teardown obligation, distinct from a provider request timeout, a process watch lease or an agent task deadline.
3. **Boot current SOURCE image.** This contains the OS and current kernel, never a previous agent's state. Its publication and boot-version proof belong solely to row 25a. The new runtime has the ordinary kernel's authority and provider path.
4. **Transfer and work.** Git and provider tooling are core transfer components; new managed/offload kernels include **Git + gh**. D255 defaults GitHub and travel-capable provider accounts to the existing selected transfer even for Fresh; owned-machine transfer stays opt-in. Tool availability does not mean credentials are baked into an image. Accounts, Git authorization and selected context use existing product transfer. The agent runs commands through ordinary runtime paths.
5. **Keep work safe and extend deliberately.** Commit/push or move results home before Delete. An extension changes the teardown deadline; it is not a new acquisition or an automatic reset on reconnect. The design requires extensions, but does not invent their duration or allowance semantics.
6. **Delete.** On normal completion the agent deletes the VM. If its agent is deleted, crashes or gets stuck, the accepted deadline remains effective and auto-deletes it. Cleanup must not require that agent, its provider turn or its home kernel to remain alive. Deadline enforcement is infrastructure lifecycle, not Cloud execution of the agent's work.
7. **Precaution shutdown (D254).** Automatically save a separate recovery image subject only to existing saved/kept-image maxima. Add no new limit where none exists. A limit hit, failed save or unfinished save **never extends the VM deadline**: delete at that deadline, with no retry grace or extra rental time. Avoiding billing comes first. An abrupt crash may prevent saving; do not promise an image without evidence. The Orphaned images entry shows **Not saved** with a **kernel-authored reason class and no error text**; retain the log line. Home retains that log, not a lingering selectable VM/kernel/Room.

The private VM is absent from machine/kernel inventories and session pickers; filtering one client picker alone is insufficient. **D253 decides that invisible means inventory/picker exclusion only, not hidden activity.** The pane ran-remotely line, Health processes and billing remain visible. Those work observations do not turn the VM into a selectable machine/kernel or grant unrelated agents access to its runtime.

### Visible managed machine

```text
Visible running machine with agents/context/state
    -> Stop -> save and verify state -> stop compute -> visible saved machine
    -> Start -> restore that state -> ordinary running kernel
    -> Delete -> save image; remove kernel and Rooms together
                 -> reconcile resource cleanup; image remains in Machines > Images
```

Reuse Path-1's stable machine/saved-state identity, operation revision, run generation and image verification. Start must not quietly substitute a blank machine if recovery fails. Delete must fence an in-flight Start and old activity reports so they cannot recreate a deleted kernel or its Rooms. This does not choose an additional Undo window for rental Delete. Account unlink and its existing Undo parity remain the separate row 11/393 work.

### Saved image management (November 20 decision, reaffirmed D251 / D252)

**Machines > Images** is the ordinary saved-state inventory. It supports **rename**, **delete with confirmation**, and shows the **source kernel and creation time**. Restore uses **SAVED IMAGE** in the shared start-from flow above. Deleting the original managed machine and its Rooms does not silently recreate a selectable kernel; a later saved-image start-from is the explicit restoration path. These operations and placement are decided, not an open new directory design.

Source boot images are platform starting points, not user saved-state entries. Orphan recovery images have the additional area and rescue flow below. Portable image storage remains existing path1images work, with its acceptance hold unchanged.

### Orphaned images: retention and rescue (D253 / D254)

**Decided retention is the existing kept-image rule:** paid images expire **3 days** after retention starts; free images have **no age limit**. Reuse `keptImageRetention`, not a separate recovery-image TTL. Paid/free charge basis, retained time and expiry come from the saved-image lifecycle; do not turn “3-day recovery image” into expiry for a free image.

**Machines > Images > Orphaned images** shows the orphan image and its expiry, with an action for the owner to **Assign an agent** to inspect/rescue. Notify the original parent tree first if it is alive; it may rescue its own work. Report the orphan as a status/notification line plus its entry and expiry, **not a user card**. At expiry, delete without asking. A free image has no age expiry; do not invent one. Assignment and notification do not extend the image's expiry or a VM deadline.

**Operational fit proposed by the Mac:** restore the selected recovery image onto a **private VM with a mandatory teardown deadline**. The assigned agent inspects the work, commits/pushes or moves it home, then deletes the VM. This is explicit recovery from a selected private saved artifact, not ordinary offload acquisition silently reusing state, and it does not make the rescue VM visible in machine/kernel lists or pickers. Its activity and billing remain visible. This proposal composes the decided rescue action with existing private-VM lifecycle; it adds no retention duration, new allowance or waiting threshold.

**D254 failure outcome is decided:** a limit hit, save failure or unfinished save never buys more VM time. Delete at the deadline with no retry grace; retain the log line and show **Not saved** in Orphaned images with the **kernel-authored reason class, no error text**. This failed-save entry is an outcome, not proof that a restorable artifact exists.

Source verified at Cloud `f6f3584177a10e5c2995082114dd5d3d52daa598`: [kept-images.ts](https://github.com/charioxai/chariox-cloud/blob/f6f3584177a10e5c2995082114dd5d3d52daa598/apps/api/src/managed-environments/kept-images.ts#L49) defines `paidKeptImageRetentionDays = 3` and free `expiresAt: null`. The reviewed saved-image contract has the existing `maxSavedImagesPerJob = 4096` [job bound](https://github.com/charioxai/chariox-cloud/blob/f6f3584177a10e5c2995082114dd5d3d52daa598/packages/infrastructure-manager-contracts/src/saved-image.ts#L12), enforced by `jobSavedImages`. That is a job payload bound, not a newly chosen account allowance. No distinct precaution-image count quota was found in the reviewed save path; this design adds none and preserves applicable existing maxima.

### Two image meanings

| Image | Contents and purpose | Lifecycle |
| --- | --- | --- |
| **Immutable SOURCE** | OS + current deployed kernel; reproducible starting point. No agent workspace, Rooms/history, Vault or provider accounts. | Row 25a owns deploy pipeline and manual L15.1 publish after final staging. This lane specifies no competing builder or publish step. |
| **Saved recovery/state image** | Machine state captured for restoration: existing Path-1 imaging/state bundle, with its identity and verification boundary. It can contain private state and is never a reusable SOURCE image or credential distribution route. | Managed Stop/Delete and kernel precaution shutdown use it. Orphan images use the existing paid 3-day / free no-age-limit retention and the Orphaned images rescue flow. |

The existing stateless preinstall plus authenticated pinned release install is a deployment mechanism, not permission to boot stale kernels. Row 25a resolves how the current SOURCE is delivered and proved. A private VM's ordinary acquisition never restores a prior state bundle by accident. Ordinary managed saved images restore through the shared start-from flow. Orphan recovery follows the explicit rescue flow above, with its deadline unchanged.

## Fit with existing designs and authority

- **October 30 offload decisions** established hidden VMs, the pane tool/task line reporting remote work, and Health process reporting. D253 confirms that activity visibility: invisible means absent inventories and pickers, not hidden work. Health processes are not a VM list or session picker. The historical OF rows provide acceptance context, not a second implementation mandate.
- [Owned kernels and optional control-plane machines](../CONTROL_PLANE_MACHINE_BOUNDARY.md) and [Cloud managed-VM plan](https://github.com/charioxai/chariox-cloud/blob/main/docs/MANAGED_VM_RENTAL_PLAN.md) supply ordinary execution, transfer, activity fencing and saved-state operations. Their November 2 universal visibility, universal shutdown preservation and hidden-cost wording is superseded **for these points** by D246's three kinds, ephemeral deadline deletion and visible billing. No separate offload job service or rental-only provider runtime follows from the new kinds.
- The **November 23 ordinary rented-kernel decision** remains: rented machines are ordinary kernels, including account-copy targets. [Provider accounts](../PROVIDER_ACCOUNTS.md) defines the product account-transfer path. An independent kernel receives its own account/config copy through authorized transfer and then operates independently. No provider credential per launch, prompt or tool call; no kernel asks another kernel to fetch credentials. No credential distribution through Cloud follows from these lifecycle kinds.
- The **30-day retirement** rule remains unchanged (row 12/kernelretire): after 30 days offline with no Room activity, retire the kernel record and remove its Rooms from navigation. This is directory hygiene, not a rental lifetime or a grace period after Delete. It neither stops nor images owned physical machines. D246 Delete removes the relevant kernel/Rooms at once, not after 30 days.

The kernel remains authority for sessions, agents, workspaces, history, state, interactions and provider runs. Visibility filtering and tree authority belong in that shared runtime/protocol, not only in a client. Cloud owns compute allocation, accepted teardown obligations, images and infrastructure cleanup, with provider-specific logic outside OSS. Cloud does not carry runtime commands, files or provider traffic and does not choose work placement. The relay carries encrypted traffic. Use ordinary kernel gateway, execution, file and account-transfer paths; no Cloud runtime proxy and no new SSH runtime route.

Current source anchors on OSS `4e687b858d367c6785f42ac085ccc7fa85c2c295`:

| Existing seam | What the source establishes; not a D246 implementation claim |
| --- | --- |
| [Machine presence](../../apps/kernel/src/runtime/machine_presence.rs) | Existing directory removal/reconciliation and 30-day forget constant. |
| [Machine acquisition](../../apps/kernel/src/runtime/state/machine_acquisition.rs), [without asking](../../apps/kernel/src/runtime/state/machine_acquisition/without_asking.rs) | Existing acquisition and no-ask limits/ledger. D246's allowance replaces the per-kernel switch; new billing machinery is held. |
| [Machine CLI](../../apps/cli/src/machine-command.ts) | Current command says `machine release`; acquire exposes class/region/request id, not the mandatory D246 teardown input. |
| [Context transfer](../../apps/kernel/src/managed_context/transfer/mod.rs) | Existing transfer composition, rather than Cloud carrying runtime context. |
| [Authenticated boot](../AUTHENTICATED_KERNEL_BOOT.md), [current boot image boundary](../MANAGED_REMOTE_KERNEL_IMAGE.md) | Stateless image/authenticated release deployment boundary; row 25a owns the current-kernel gap. |

Cloud acquisition/image source context at `f6f3584177a10e5c2995082114dd5d3d52daa598`: [Rent form](https://github.com/charioxai/chariox-cloud/blob/f6f3584177a10e5c2995082114dd5d3d52daa598/apps/web/src/terminal-os/machines/rent-view.tsx) and [New Room machine sheet](https://github.com/charioxai/chariox-cloud/blob/f6f3584177a10e5c2995082114dd5d3d52daa598/apps/web/src/terminal-os/machines/new-machine-sheet.tsx) share machine/start/environment helpers; [start-from menu](https://github.com/charioxai/chariox-cloud/blob/f6f3584177a10e5c2995082114dd5d3d52daa598/apps/web/src/terminal-os/machines/start-from-menu.tsx) renders fresh/copy, and [Images table](https://github.com/charioxai/chariox-cloud/blob/f6f3584177a10e5c2995082114dd5d3d52daa598/apps/web/src/terminal-os/machines/images-table.tsx) renders image name, source association and created time. These are source observations, not live evidence that Rent exposes copying or that Images meets every decided operation.

## Allowance and interim operations

**Billing is always visible.** Accounts, tiers, billing and allowance implementation wait until the system works. Hold those rows exactly **"billing later (Miguel 11-27)"**. This document does not introduce amounts, scope, rates, estimation policy, new account screens or an allowance implementation lane.

The decided eventual shape is a per-day/per-month allowance for agent VMs, replacing “Agents may offload and rent without asking” per-kernel switching. No per-acquisition user question inside it; above it the owner accepts or raises the limit. The amount, accounting scope and enforcement details remain open. It is not a new exception to provider permission or secret handling.

Until then, **D146 / D228 / D242 standing farm rules** are the interim allowance, not a general customer pricing policy:

- Scale sequentially **through the product**, verifying that each added machine picks up a ticket before adding another.
- Staging maximum **12** machines / about **EUR 30 per day**; bring oldest ticket wait below **10 minutes**.
- Delete a machine after **30 minutes idle** once the queue has been empty **20 minutes**.
- Buy no credit; report low credit.

These are supplied owner limits, not numbers chosen by this design. No rentals, scaling or deletion is performed by this lane. Row 131 owns migration of the farm's root-SSH workaround to product execution/jobs; do not reopen a duplicate or call the workaround the final design.

## Owner review: decided and open items

**D253 / D254 lift the previous owner implementation hold.** This design becomes ready for implementation after a fresh exact-head technical review. No implementation is performed or launched by this docs lane. Allowance/billing and Assistant scope remain outside implementation readiness.

| Item | Status | Decision or question |
| --- | --- | --- |
| **1. Orphan recovery-image retention** | **DECIDED D253** | Existing kept-image retention: paid 3 days, free no age limit. Orphaned images area shows expiry; owner assigns an agent to rescue; original parent tree notified first if alive. Status/notification plus entry, no user card; expiry deletes without asking. Mac's private deadline-bound restore/rescue procedure is the operational proposal above. |
| **2. Ordinary saved-image management** | **DECIDED November 20, reaffirmed D251 / D252** | Machines > Images: rename, delete with confirmation, source kernel and creation time; restore via SAVED IMAGE in shared start-from. |
| **3. Allowance amount and scope** | **OPEN — billing later (Miguel 11-27)** | Day/month amounts, owner/account/project/tree boundaries, shared consumption and extension accounting. No implementation now. |
| **4. Recovery at limits and failure** | **DECIDED D254** | Existing saved/kept maximum only; absent limit stays absent. Limit hit, failed save or unfinished save never extends VM deadline: delete at deadline, no retry grace or extra rental time. Orphaned images shows Not saved with kernel-authored reason class, no error text; log line remains. |
| **5. Activity visibility** | **DECIDED D253** | Invisible only from machine/kernel lists and session pickers. Pane ran-remotely line, Health processes and billing remain visible; this grants no unrelated runtime access. |
| **6. Assistant kernel type** | **OPEN — future scope** | Fits beside the three kinds; visibility, lifetime and authority remain undecided and outside this lane. |

## Gap map and review evidence

This is a design/evidence inventory, not a report that the symptoms are fixed. Owner-reported current examples: agent VMs clutter Miguel's Machines tab (**Cloud 62 Failed, Cloud 42 Stopped, Falkenstein 14/15 Failed**; D251 / D252 add private farm **Cloud 64–68** and internal **agent-managed-686** class exposed in the person's list); kernels linger after deletion. These are machine labels, **not queue row numbers**. No live machine operation or billing inspection was performed by this lane.

Mechanical workflow **d252-machine-gaps** completed with **ALL SAME** (coordinator verdict 243751, exact destinations delivered in message 243795). Evidence merges into existing rows; none of the seven symptoms creates a new row. The additional **d253-orphan-image-rescue** workflow also returned **SAME 25 / D246**; its retention/rescue/Not saved acceptance belongs to row 25, not a new lifecycle row. Existing statuses below are retained unless explicitly reopened for the D253 implementation scope. Levels describe the current symptom; the shared row retains the highest applicable level.

| Gap | Level | Existing row / ownership | Dedupe verdict | Status / evidence |
| --- | --- | --- | --- | --- |
| Private agent VMs clutter user machine/kernel lists and pickers | **P1**: Miguel sees clutter today | **25** | **SAME 25 / D246**, d252-machine-gaps #1 | Implementation pending design review; owner examples Cloud 62/42, Falkenstein 14/15 and farm Cloud 64–68. Activity remains visible by D253. |
| Delete leaves kernel/Rooms or billable orphan infrastructure | **P1** for today's owner impact | **25**; stale-record coverage **62 / 73 / 385** | **SAME 25; 62/73/385 coverage**, d252-machine-gaps #2 | Implementation pending; preserve older removal evidence and acceptance status rather than duplicate directory design. Do not infer live billing from an offline record alone. Row 11/393 account parity remains acctkernels2. |
| Deadline/extend/orphan auto-delete and precaution recovery image | **P2** | **25** | **SAME 25 / D246**, d252-machine-gaps #3 | Implementation pending: mandatory acquire teardown time, extend, auto-delete even after agent loss, standard paid 3-day/free no-age retention; D254 limit/failure/unfinished save cannot extend deadline and reports Not saved reason class. |
| Orphaned images area, expiry and agent rescue | **P2** | **25**; related 287/296/368 retain their existing scope | **SAME 25 / D246**, d253-orphan-image-rescue (verdict 243814, completion 243821; coordinator confirmation 243866) | D253/D254 retention/rescue/Not saved acceptance merged into row 25. No separate lifecycle row. Mac operational proposal above; runtime acceptance not run. |
| Git + gh and default selected credentials/accounts on new managed/offload kernels | **P1 D255**, blocks **271** | **270** Git/validation; **582** provider-install implementation | **SAME 270 / 582**, d252-machine-gaps #4; additional D255 mechanical check running | Row 270 reopened for installation plus real selected-transfer/default-on/None/Fresh/unreachable-source acceptance. Preserve earlier partial validation and 582's existing priority, PRs and holds. Owned-machine opt-in unchanged. |
| Product says Release rather than Delete | **P2** | **25** | **SAME 25 / D246**, d252-machine-gaps #5 | Implementation pending; main `machine-command.ts` help/dispatch uses `release`. Row 245 historical CLI rental evidence is context. |
| Shared Rent/New Room start-from, copy, environments, images and agent parity | **P2** | **277**, including prior closed [Cloud #798](https://github.com/charioxai/chariox-cloud/pull/798) | **SAME 277**, d252-machine-gaps #6 | Not started/reopened existing row. Owner reports deployed Rent fresh-only; common source helpers do not prove usable choices. Preserve 271/279 acceptance evidence; verify both entry points and agent parity. |
| Internal unavailable `agent-managed-686` class in person's list | **P2** catalogue gap; P1 private-VM clutter remains row 25 | **442**, open [Cloud #870](https://github.com/charioxai/chariox-cloud/pull/870) | **SAME 442**, d252-machine-gaps #7 | Existing held PR / L16 status retained; current owner evidence merged rather than duplicating the catalogue work. |
| Rented machine boots old kernel / SOURCE freshness | Existing **P1** | **25a**, bootimage deployment | **SAME 25 -> 25a**, d243-bootimage-25a | In lane bootimage; [OSS #1836](https://github.com/charioxai/chariox/pull/1836) prerequisite is not deployed-image acceptance. Separate from this design lane. |
| Farm root SSH instead of product work path | Existing **P2** migration | **131** | **SAME 131**, d243 verdict | Reopened/not started, after 25a proof. No new row or implementation here. |
| Portable saved images/state bundles | Existing **P3** | **296 / path1images**, using 287 kept-image record | Existing work; no new row | Existing managed-machine acceptance/dependency hold retained. |
| Allowance, billing/accounts/tiers | Held new policy scope | Existing relevant billing/allowance rows; 569/406 context | No implementation launch | **billing later (Miguel 11-27)**. Preserve distinct existing acquisition security/parity work rather than silently changing another lane's scope. |

All-status Cloud PR search also found [#802](https://github.com/charioxai/chariox-cloud/pull/802) (closed, Stop saves image/Start restores), [#882](https://github.com/charioxai/chariox-cloud/pull/882) (open, snapshot quota), [#872](https://github.com/charioxai/chariox-cloud/pull/872) (open, failed Stop/Delete retry and orphan volume cleanup, rows 641/642), [#893](https://github.com/charioxai/chariox-cloud/pull/893) (open, unreachable-kernel removal, row 500), and [#867](https://github.com/charioxai/chariox-cloud/pull/867) (open, allocation listing, row 272). These statuses were read on November 27; closed alone does not prove landed. Cloud main was verified through GitHub as `f6f3584177a10e5c2995082114dd5d3d52daa598`; its rental plan and control-service still describe existing purposes and cleanup deadlines rather than D246's full lifecycle.

Read all-status OSS PR searches before drafting: OSS [#1277](https://github.com/charioxai/chariox/pull/1277) (Path-1 lifecycle), [#1683](https://github.com/charioxai/chariox/pull/1683) (hard stop input), [#1660](https://github.com/charioxai/chariox/pull/1660) and [#1830](https://github.com/charioxai/chariox/pull/1830) (account transfer), [#1779](https://github.com/charioxai/chariox/pull/1779) (allocation listing), [#1739](https://github.com/charioxai/chariox/pull/1739) and [#1808](https://github.com/charioxai/chariox/pull/1808) (limits/authority), and #1836 (25a). A PR search hit establishes overlap to inspect, not merged/implemented behaviour. Exact queue statuses and dedupe receipts belong in the lane evidence, outside the repository.

## Review checks and later acceptance

Docs-only checks: references resolve, claims are tied to current source or labelled owner/historical evidence, D246 machine/image points, D249 scope correction and D251 / D252 / D253 / D254 / D255 review are covered, no unresolved question becomes policy, diff contains documentation only. Runtime cells are **NOT RUN — design only**, not PASS.

After fresh exact-head design review, implementation acceptance must cover: Machines > Rent reaches a ready visible managed machine without creating a Room; New Room uses the same size/provider, fresh/COPY/SAVED IMAGE, environments and auto-stop flow plus the first Room and agent; agent parity returns the same visible managed result and private offload acquisition stays explicit; Machines > Images rename, confirmed delete, source kernel/time and saved-image start-from restoration; private VM absent from every machine/kernel list and session picker, including farm Cloud 64–68 and internal-class exposure; parent-tree access and log receipt, with visible pane/Health activity and billing; no-deadline refusal before resource creation; extension and deletion at the accepted deadline with deleted/crashed/stuck agent and unavailable home; precaution image success/limit/failure/unfinished save without extending teardown, Not saved with kernel-authored reason class and no error text, Orphaned images expiry/assignment/no-card rescue, paid 3-day expiry and free no-age retention, parent-tree notification and deadline-bound private restore with work returned home; managed Stop/restore/Delete with kernel/Rooms removal and stale-operation fencing; Git + gh installed on a fresh managed and private offload kernel; default GitHub and travel-capable provider accounts imported through existing selected transfer for COPY and Fresh, per-launch None opt-out, no reachable source still launches with status/no error text, owned-machine transfer opt-in unchanged; private Git clone/push and a real provider turn plus work returned home through product paths. Repeat applicable cells after restart and from another client/machine. Row 25a separately proves the current deployed boot release. Acceptance is queued through the existing deduped rows; only the coordinator launches implementation by queue order/resources. This docs lane launches none and makes no live product claim.
