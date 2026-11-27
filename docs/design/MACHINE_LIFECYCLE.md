# Machine lifecycle

Design for Miguel's review, 2026-11-27. Source queue row **25**, owner decisions **D246, corrected by D249 and reviewed in D251 / D252**. Target **L15.2 / design review**; excluded from the L15.1 cut. This document records the decided shape and remaining questions. It implements no behaviour and grants no acceptance PASS. **No lifecycle implementation lane starts until Miguel answers review items 1 (orphan recovery-image retention) and 5 (activity visibility) below.** Owner design review remains the gate; this PR authorizes no implementation. D246 ends the November 20 hold on lifecycle design only. D249 limits this document to Path-1 managed machines, offloading agent-private VMs and image management. Remote sub-agents remain separate in row 770 / `sub-agents.md`.

An agent chooses where to work. The user's own machines stay visible and untouched by rental lifecycle. A managed rented machine is a visible computer with recoverable state. An agent-private VM is temporary capacity: its agent and parent tree can use it, it never becomes a machine for the user to manage, and it must have a teardown deadline before it exists.

## Decided kinds

| Kind | Visibility | State and lifetime | Stop / Delete |
| --- | --- | --- | --- |
| **OWN physical machines**: laptop, b3, nelpx, BYOM | Visible as the user's machines and kernels. | User-owned, ordinary kernel state. | Never stopped or imaged by this machine lifecycle. Rental teardown does not apply. |
| **MANAGED rented machines** | Visible, including their agents, context and state. | Ordinary independent kernel; state is saved using the existing Path-1 imaging/state-bundle shape. | Stopping or deleting saves an image. Stop permits later restoration; Delete removes the kernel and its Rooms at once. Saved images use Machines > Images and the shared saved-image start-from flow below; orphan recovery-image retention remains open. |
| **AGENT-PRIVATE VMs** for offload, build or test | **Invisible to the user and other agents**: absent from machine/kernel lists and session pickers. Only the agent and its parent tree see the VM. Home kernel keeps a log entry only. | **Ephemeral**. Boot from an immutable **SOURCE image: OS + current kernel for speed, NEVER state** (row **25a**). Acquire must specify a deadline/teardown time; absence is refused. The agent may extend it. | Agent commits/pushes or moves work home, then **deletes**. Deleted, crashed or stuck agent: VM auto-deletes at the deadline. A kernel precaution shutdown automatically saves a recovery image unless a limit is hit. Deadline deletion still wins over keeping data. |

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

## Lifetime and visibility flow

### Agent-private VM

1. **Agent chooses placement** from its context: local work, another appropriate existing machine, or a temporary VM. The kernel does not intervene in that choice. Existing runtime protection still protects the machine; this design adds no placement threshold, scheduler, resource cap or admission heuristic.
2. **Acquire with teardown time.** Missing deadline is refused before allocating. Work within the allowance needs no user question. Above the allowance the owner accepts or raises the limit. The deadline is a VM teardown obligation, distinct from a provider request timeout, a process watch lease or an agent task deadline.
3. **Boot current SOURCE image.** This contains the OS and current kernel, never a previous agent's state. Its publication and boot-version proof belong solely to row 25a. The new runtime has the ordinary kernel's authority and provider path.
4. **Transfer and work.** Git and provider tooling ship on **every machine** as core transfer components. Tool availability does not mean credentials are baked into an image. Accounts, Git authorization and selected context use existing product transfer. The agent runs commands through ordinary runtime paths.
5. **Keep work safe and extend deliberately.** Commit/push or move results home before Delete. An extension changes the teardown deadline; it is not a new acquisition or an automatic reset on reconnect. The design requires extensions, but does not invent their duration or allowance semantics.
6. **Delete.** On normal completion the agent deletes the VM. If its agent is deleted, crashes or gets stuck, the accepted deadline remains effective and auto-deletes it. Cleanup must not require that agent, its provider turn or its home kernel to remain alive. Deadline enforcement is infrastructure lifecycle, not Cloud execution of the agent's work.
7. **Precaution shutdown.** When the kernel can save state before shutdown, it automatically saves a separate recovery image unless a limit is hit. It must not keep renting the VM past its deadline to preserve data. An abrupt crash may prevent saving; do not promise a usable image without evidence. Home retains the log entry, not a lingering selectable machine/kernel/Room.

The private VM is absent from machine/kernel inventories and session pickers; filtering one client picker alone is insufficient. **Activity visibility is proposed, awaiting Miguel's answer to item 5:** invisible inventory does not mean hidden activity. Retain the remote-work pane line, Health processes and visible billing. Those work observations must not turn the VM into a selectable machine/kernel or make its runtime addressable to unrelated agents.

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

This does not settle the retention or discovery of **orphan offload recovery images**: item 1 remains open. Source boot images are platform starting points, not user saved-state entries. Portable image storage remains existing path1images work, with its acceptance hold unchanged.

### Two image meanings

| Image | Contents and purpose | Lifecycle |
| --- | --- | --- |
| **Immutable SOURCE** | OS + current deployed kernel; reproducible starting point. No agent workspace, Rooms/history, Vault or provider accounts. | Row 25a owns deploy pipeline and manual L15.1 publish after final staging. This lane specifies no competing builder or publish step. |
| **Saved recovery/state image** | Machine state captured for restoration: existing Path-1 imaging/state bundle, with its identity and verification boundary. It can contain private state and is never a reusable SOURCE image or credential distribution route. | Managed Stop/Delete and kernel precaution shutdown use it. Orphan recovery-image retention is **OPEN for Miguel**. |

The existing stateless preinstall plus authenticated pinned release install is a deployment mechanism, not permission to boot stale kernels. Row 25a resolves how the current SOURCE is delivered and proved. A private VM's ordinary acquisition never restores a prior state bundle by accident. Ordinary managed saved images restore through the shared start-from flow. Handling orphan recovery images still needs the decisions below.

## Fit with existing designs and authority

- **October 30 offload decisions** established hidden VMs, the pane tool/task line reporting remote work, and Health process reporting. D251 / D252 propose retaining that activity visibility: invisible means absent inventories and pickers, not hidden work. Miguel must confirm item 5 before implementation. Health processes are not a VM list or session picker. The historical OF rows provide acceptance context, not a second implementation mandate.
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

**ZERO lifecycle implementation lanes until Miguel answers items 1 and 5.** Other open items remain explicit; billing and allowance implementation stay held separately.

| Item | Status | Decision or question |
| --- | --- | --- |
| **1. Orphan recovery-image retention** | **OPEN — implementation gate** | How long, who can restore/delete, and what happens when keeping an image itself incurs cost. No retention period is selected; the older paid/free retention text is not imported as an answer. This concerns orphan recovery images, not reopening ordinary Machines > Images operations. |
| **2. Ordinary saved-image management** | **DECIDED November 20, reaffirmed D251 / D252** | Machines > Images: rename, delete with confirmation, source kernel and creation time; restore via SAVED IMAGE in shared start-from. The old broad visibility/destination question is closed by this flow. Only orphan recovery-image questions remain under items 1 and 4. |
| **3. Allowance amount and scope** | **OPEN — billing later (Miguel 11-27)** | Day/month amounts, owner/account/project/tree boundaries, shared consumption and extension accounting. No implementation now. |
| **4. Recovery at limits and failure** | **OPEN** | Which limit prevents the precaution image and how to present incomplete image creation while deadline deletion proceeds. “Not billing beats keeping data” is decided; no new retry grace or paid retention is inferred. |
| **5. Activity visibility** | **PROPOSED — Miguel to confirm; implementation gate** | Invisible means absent machine/kernel lists and session pickers, **not hidden activity**. Retain the remote-work pane line, Health processes and visible billing. Work reporting does not grant unrelated agents access to the private runtime or add it to inventory. |
| **6. Assistant kernel type** | **OPEN — future scope** | Fits beside the three kinds; visibility, lifetime and authority remain undecided and outside this lane. |

## Gap map and review evidence

This is a design/evidence inventory, not a report that the symptoms are fixed. Owner-reported current examples: agent VMs clutter Miguel's Machines tab (**Cloud 62 Failed, Cloud 42 Stopped, Falkenstein 14/15 Failed**; D251 / D252 add private farm **Cloud 64–68** and internal **agent-managed-686** class exposed in the person's list); kernels linger after deletion. These are machine labels, **not queue row numbers**. No live machine operation or billing inspection was performed by this lane.

Mechanical dedupe is requested from orch15 for each distinct symptom. Keyword/source comparisons below are context only until its workflow verdict is recorded. SAME merges new evidence into the existing row and never creates another row. Existing statuses are audit inputs; some lag current owner decisions.

| Gap | Level | Existing/new row and overlap | Dedupe verdict | Status / evidence |
| --- | --- | --- | --- | --- |
| Private agent VMs clutter user machine/kernel lists and pickers | **P1**: Miguel sees clutter today | Row 25 design; check 62/73/436 and 11/393 without starting another directory design | Pending orch15 D246 workflow | Owner examples above, including Cloud 64–68; inventory filtering remains required. Activity reporting awaits item 5; no implementation before items 1 and 5. |
| Internal `agent-managed-686` class appears in the person's machine list | **P1**: current owner-visible clutter | Row 25; compare private-VM clutter before assigning a destination | Pending orch15 per-symptom D251 / D252 workflow | Owner review evidence; no live repro by this lane. Merge into the same clutter row if workflow returns SAME. |
| Rent is fresh-only in the deployed owner flow; copy and environments are missing | **P2** | Compare 271 Path-1, 277 New Room, 279 copy and row 25 | Pending orch15 per-symptom D251 / D252 workflow | Owner report. Main source has copy/environment helpers; source alone does not explain or disprove the deployed symptom. |
| SAVED IMAGE is absent from Rent start-from | **P2** | Compare Path-1 saved-image work, 287, 296/path1images and row 25 | Pending orch15 per-symptom D251 / D252 workflow | Existing start-from source renders fresh/copy; ordinary saved-image restoration is decided. Do not conflate it with portable storage implementation. |
| Machines > Rent, New Room and agent parity lack proof of the same visible managed flow | **P2** | Compare 271/277/279 and 245; retain existing acceptance evidence | Pending orch15 per-symptom D251 / D252 workflow | Shared components exist; live verification of every start-from choice, environments, auto-stop and the different endpoints remains outstanding. |
| Delete leaves kernel/Rooms or billable orphan infrastructure | **P1** for today's owner impact | Check 62/73/436, 11/393 account parity, 12 retirement, 500 removal UI, 287/368 and 641/642 cleanup | Pending orch15 D246 workflow | Owner report; do not equate an offline directory record with proven live billing. Row 11/393 is unified P1, in lane acctkernels2, target L15.2. Row 12/73 record L14 landings; new evidence needs root-cause confirmation. |
| Mandatory VM teardown time, extension, orphan deadline deletion and precaution recovery image | **P2** absent separate live billing harm | Row 25; compare hard-stop row 368, OF lifecycle and Path-1 state imaging | Pending orch15 D246 workflow | Existing hard-stop work is partial overlap, not proof of the whole D246 path. |
| Git/provider transfer tooling guaranteed everywhere | **P2** | Check row 25, transfer/toolchain and OF rows | Pending orch15 D246 workflow | D246 identifies Git guarantee gap; tooling installed does not prove usable account transfer. |
| Product says Release rather than Delete | **P2** | Check row 245's CLI path and row 25 | Pending orch15 D246 workflow | Main `machine-command.ts` help/dispatch uses `release`. |
| Rented machine boots old kernel / SOURCE image freshness | **P1**, existing deployment hygiene | **25a**, split from 25; bootimage owns pipeline and manual publish | **SAME 25 -> 25a** (d243-bootimage-25a, owner-supplied verdict) | In lane bootimage; [OSS #1836](https://github.com/charioxai/chariox/pull/1836) is a signing prerequisite, not proof of a current deployed image. No overlap launch. |
| Farm needs root SSH rather than product work path | **P2**, existing migration | **131** | **SAME 131** (existing d243 verdict supplied in D246 brief) | Reopened by D243; source queue still says done at initial read, audit correction requested. No new row. |
| Portable saved images/state bundles | Existing level **P3** | **296 / path1images**, using 287 kept-image record | Existing work; no new symptom row requested | Held on managed-machine acceptance/dependencies. Preserve that hold; D246 does not start portable-image implementation. |
| Allowance, billing/accounts/tiers | Held; no new implementation priority | Existing 569/406 and relevant billing rows; coordinator decides exact ownership after dedupe | No implementation launch | **billing later (Miguel 11-27)**; do not mark an existing distinct security/parity fix held merely because it touches acquisition. |

All-status Cloud PR search also found [#802](https://github.com/charioxai/chariox-cloud/pull/802) (closed, Stop saves image/Start restores), [#882](https://github.com/charioxai/chariox-cloud/pull/882) (open, snapshot quota), [#872](https://github.com/charioxai/chariox-cloud/pull/872) (open, failed Stop/Delete retry and orphan volume cleanup, rows 641/642), [#893](https://github.com/charioxai/chariox-cloud/pull/893) (open, unreachable-kernel removal, row 500), and [#867](https://github.com/charioxai/chariox-cloud/pull/867) (open, allocation listing, row 272). These statuses were read on November 27; closed alone does not prove landed. Cloud main was verified through GitHub as `f6f3584177a10e5c2995082114dd5d3d52daa598`; its rental plan and control-service still describe existing purposes and cleanup deadlines rather than D246's full lifecycle.

Read all-status OSS PR searches before drafting: OSS [#1277](https://github.com/charioxai/chariox/pull/1277) (Path-1 lifecycle), [#1683](https://github.com/charioxai/chariox/pull/1683) (hard stop input), [#1660](https://github.com/charioxai/chariox/pull/1660) and [#1830](https://github.com/charioxai/chariox/pull/1830) (account transfer), [#1779](https://github.com/charioxai/chariox/pull/1779) (allocation listing), [#1739](https://github.com/charioxai/chariox/pull/1739) and [#1808](https://github.com/charioxai/chariox/pull/1808) (limits/authority), and #1836 (25a). A PR search hit establishes overlap to inspect, not merged/implemented behaviour. Exact queue statuses and dedupe receipts belong in the lane evidence, outside the repository.

## Review checks and later acceptance

Docs-only checks: references resolve, claims are tied to current source or labelled owner/historical evidence, D246 machine/image points, D249 scope correction and D251 / D252 review are covered, no unresolved question becomes policy, diff contains documentation only. Runtime cells are **NOT RUN — design only**, not PASS.

After owner review and answers to items 1 and 5, implementation acceptance must cover: Machines > Rent reaches a ready visible managed machine without creating a Room; New Room uses the same size/provider, fresh/COPY/SAVED IMAGE, environments and auto-stop flow plus the first Room and agent; agent parity returns the same visible managed result and private offload acquisition stays explicit; Machines > Images rename, confirmed delete, source kernel/time and saved-image start-from restoration; private VM absent from every machine/kernel list and session picker, including farm Cloud 64–68 and internal-class exposure; parent-tree access and log receipt, with pane/Health activity reporting per the answer to item 5; no-deadline refusal before resource creation; extension and deletion at the accepted deadline with deleted/crashed/stuck agent and unavailable home; precaution image success/limit/failure without continued billing; managed Stop/restore/Delete with kernel/Rooms removal and stale-operation fencing; Git/provider transfer and work returned home through real product paths. Repeat applicable cells after restart and from another client/machine. Row 25a separately proves the current deployed boot release. This list queues no implementation lane and makes no live product claim.
