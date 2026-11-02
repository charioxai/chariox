# Owned kernels and optional control-plane machines

MP-08 / MP-09 / MP-10 / MP-11. Design amendment, 2026-11-02; no protocol allocation or product implementation in this change.

A kernel can reach another kernel the user owns, transfer selected environment and accounts, and run ordinary Room commands there. A connected hosted control plane may additionally offer machine acquisition using the owner's session. Machine rental, its images, shutdown policies, provider selection and metering belong to Cloud. A self-hosted installation retains owned-kernel tools and exposes no rental or automatic-offload option.

Miguel's 2026-11-02 decisions replace the earlier separate managed-environment, disposable-worker and offload-machine designs with one managed-VM product. The coordinator's 10:30Z clarification removes dedicated offload machinery: an agent rents/uses an ordinary managed VM, copies files with existing tools, runs commands and retrieves results itself. No job tool, `offload_job` purpose, files-back/conflict service, queued-command VM runner or leased-process runner is added. Generic execution leases remain useful for placing a home-owned agent on any owned kernel; they do not constitute another rental product.

The Cloud implementation/acceptance design is `chariox-cloud/docs/MANAGED_VM_RENTAL_PLAN.md` and its companion `MANAGED_VM_ACCEPTANCE_ROWS.md`. This note defines the OSS side and supersedes the old rental architecture/matrix in the managed-parity ledger. It preserves ordinary/managed runtime parity and narrowed semantic review of changed security-critical anchors. Historical evidence remains bound to its original source; this design grants no acceptance PASS.

## Ownership

| Piece | Owner | Contract |
| --- | --- | --- |
| Sessions, agents, workspaces, provider runs, history, terminal/exec/files, resource admission | OSS kernel | Same runtime on local, owned remote and rented machines. Official provider CLI/server/hooks only. |
| Canonical remote targets, gateway access, caller grants and revocation | OSS kernel | Access decisions remain kernel-owned. Machine enrollment or execution lease is not a grant of owner authority. |
| Selected environment, workspace, SCM and provider-account transfer | OSS kernel | Direct encrypted kernel-to-kernel transfer. Provider tools own account format/refresh. No Cloud runtime payload proxy. |
| Execution leases and leased agents | OSS kernel | Generic owned-kernel capability; no compute-provider allocation requirement. |
| Activity reports and admission/quiescence fence | OSS kernel | Report actual work and enforce an accepted fence; do not decide rental prices, limits or shutdown policy. |
| Optional control-plane capability and authenticated resource-request transport | OSS kernel | Enabled only by the owner's connected session and its advertised account-scoped capability. No vendor or rental rule implementation. |
| Native release package, artifact verification and neutral boot installer | OSS | Verify before execution against pinned public trust and signed manifest. Provider image recipes and provisioning wrappers live in Cloud. |
| Rental aggregate/API, rules/reconciler, images, quotes, usage, provider adapters/accounts/limits | Cloud | One ManagedEnvironment path; no provider-specific business code in OSS kernel, relay, CLI, packages or deploy. |
| Rental/rule/cost chooser and account/operator usage | Cloud web | Hosted feature. OSS clients may display validated capability data using generic actions, never compiled provider catalogs/policies. |
| Encrypted transport | Relay | No runtime, allocation, spend, admission or shutdown decisions. |

## Proposed neutral protocol surface

These are shape names, not assigned protocol numbers. Implementation must request the appropriate local/peer version from the coordinator and update snapshots, hashes and focused drills only when serialized contracts change.

- `ControlPlaneCapabilities`: issuer/account identity, revision, optional `machine_allocation` capability, action/catalog references. Authenticated transport/session binds the advertisement to its issuer/account. A capability descriptor is data, never executable client code or target-kernel authority.
- `RequestControlPlaneMachine`: idempotent request id, capability revision, generic CPU/memory/disk/architecture requirements, optional opaque class ref, quote ref, policy ref and context selection ref. `GetControlPlaneMachine`, `ReleaseControlPlaneMachine` and `UpdateControlPlaneMachinePolicy` use the same scoped resource identity. Access uses the existing kernel request route, not a dedicated offload tool.
- `ControlPlaneMachine`: allocation ref, state/reason, canonical kernel/machine/realm/key binding when enrolled, access state, display quote/policy projection and usage ref. Never return an owner session, infrastructure credential or bootstrap secret to a tool/model. Provider/profile ids, if necessary at all, remain opaque strings.
- `KernelActivityReport`: kernel id/generation, monotonic sequence, transition time and active agents/commands/transfers. `KernelQuiescence`: reservation id, expected sequence/generation and outcome. These extend the existing report/fence; Cloud interprets the facts against its policy.

Cloud's policy choices (time limit, last work finished with warning, manual), pricing tables, credit/quota checks, image refs and adapter details do not become OSS enums or constants. A generic kernel carries only opaque control-plane policy/action refs and validated projections. Generic activity can be useful to other control planes without introducing their business rules.

A self-hosted control plane advertises no Cloud allocation capability; rental actions, costs and rule choices are absent from clients and tool discovery. A forged/stale call is refused before resource dispatch. Cloud independently checks the owner session/account/entitlement on every mutation; advertising an invented capability cannot spend Chariox's money. Ordinary gateway/exec/files and leases between user-owned machines continue to work without Cloud rental entitlement.

## Authorization and access handoff

The kernel may acquire/release a machine using the owner's connected session when the effective caller already has authority for that action. This replaces the retired rental controls only. Room membership alone does not authorize spending or owner-level target access. The kernel keeps session credentials private and follows the product's session refresh/revocation path; an existing token field is not evidence that current enrollment supplies it. Implementation must verify that binding.

The new kernel enrolls to the owner, exposes its canonical identity through the normal inventory and attaches to the owner's gateway. The target still applies its ordinary access decision, including human-only operations and revocation. Reuse an existing grant/sudo window when applicable; where a decision is required, use the existing kernel-owned interaction visible to the owner's clients. Do not add SSH, an agent as an access proxy or a parallel rental grant. Rental readiness distinguishes enrolled, context-ready and access-ready rather than hiding an unusable machine behind provider “running.”

Runtime commands/files never pass through Cloud. The relay carries encrypted traffic. Cloud persists an accepted release rule and may delete its infrastructure after the source kernel disappears; it never schedules provider runs or decides Room state. The kernel's activity fence closes the race between accepting work and acknowledging idle shutdown. A hard user deadline/manual release can end active work; a last-work rule cannot treat a missing heartbeat as idle.

## General primitives already available

References below are pinned to OSS `715e6005b4efc755b5bb70803fe0ebdf95465e92`; they establish source availability, not a new hosted acceptance result.

| Need | Existing entry point | Remaining composition check |
| --- | --- | --- |
| Discover and address another owned kernel | `GatewayKernel` / `OnBehalfOf`, [kernel_gateway.rs](../apps/kernel/src/local/api/types/kernel_gateway.rs) lines 8, 85; CLI guidance in [agent-command-table.ts](../apps/cli/src/agent-command-table.ts) line 200 | Newly acquired kernel becomes reachable under existing owner access; no surrogate agent required. |
| Grant/revoke normal kernel access | [kernel_access.rs](../apps/kernel/src/runtime/state/kernel_access.rs) lines 170, 530; `RequestKernelAccessRequest` / `KernelAccessGrant` | Owner/caller/target key binding and revocation survive new-machine handoff. |
| Copy a file, resume and verify | [kernel-files.ts](../apps/cli/src/kernel-files.ts) lines 8, 13, 45; `StatKernelFile`, `ReadKernelFile`, `WriteKernelFile` | Execute real managed-kernel round trip. Directory input can be an ordinary archive; no new upload service needed. |
| Binary command streams and long terminal use | [room_terminal.rs](../apps/kernel/src/local/api/types/room_terminal.rs) line 26; [Room exec drill](../scripts/e2e-stack/room-exec/README.md) line 24 | Ordinary argv/stdin/stdout/stderr/exit, terminal wait and lease renewal work on new target. Exec reader lease is 300 s; draining/acknowledging remains necessary. |
| Transfer selected environment/accounts | [transfer/mod.rs](../apps/kernel/src/managed_context/transfer/mod.rs) lines 205, 331, 361, 572, 581; [project environment transfer](../apps/kernel/src/project_environment/transfer.rs) line 79 | Chain existing arm/import/commit receipts and account transfer into ready, without payloads in bootstrap/Cloud. |
| Run a home-owned agent remotely, when desired | [relay_peer.rs](../apps/kernel/src/transport/relay_peer.rs) lines 844, 855, 1203 | Same VM/bootstrap supports ordinary kernel work and generic execution leases; no new rental mode aggregate. |
| Report work and fence idle release | [managed_work.rs](../apps/kernel/src/room_terminal/managed_work.rs) lines 34, 44; [quiescence state](../apps/kernel/src/runtime/state/managed_kernel_quiescence_state.rs) line 113 | Include in-progress file transfer in shared activity/fence if absent; fix generically. |

Inference: these primitives cover rent → copy inputs → execute → copy results → release once the optional allocation route and access handoff exist. A real home-agent drill must use them with zero VM agents, a useful build/test and a large byte-exact transfer. Do not call source inspection or a local relay fixture managed-VM acceptance.

Retrieve results into a separate local destination and let the agent decide which files to apply using ordinary tools. No automatic file merge or product conflict engine. For a sequence with long idle gaps before retrieving files, choose manual or a sufficient time limit; last-command exit does not mean results were retrieved. Pending transfers prevent idle release and new work during warning cancels it. No new job-lifetime abstraction is required.

The local admission stripe remains a queue with Start now/Cancel. A VM runner or process migration button is unnecessary. A future approved pointer can tell the user's agent to use the existing rent/access action; it does not move suspended execution, add automatic placement policies or expose hosted options to self-hosted installations.

## Install and deletion responsibilities

Base images hold slow preinstalls, a small trusted verifier/installer and public trust roots. They contain no runtime identity, account state, owner session, workspace or bootstrap grant. Routine kernel releases do not rebuild images. On each allocation Cloud delivers the same protected one-time envelope and immutable signed-release reference; the generic installer authenticates every executable before installing/starting the kernel/bootstrap. Reuse signature/hash verification in [release.rs](../apps/kernel/src/managed_bootstrap/release.rs) lines 90, 168 and exchange/supervision. A failed download/signature is terminal with cleanup, never a fallback to an older runtime. Reboot retains the allocation's pinned release/identity rather than re-enrolling or upgrading implicitly.

Cloud keeps ManagedEnvironment and folds useful expiry/deletion-evidence logic into it. Delete rental-specific DisposableWorkerAllocation routes/handlers/bootstrap branches as their replacements land. The current [retirement gate](../apps/kernel/src/runtime/router/retired_cloud_controls.rs) line 5 rejects the old requests before routing; do not remove it wholesale and accidentally restore unrelated human-only controls. Retain generic execution leases, account transfer, installed release authenticity, gateway access and shared storage helpers still used elsewhere. Remove replaced rental reimage/in-place A-B update/compatibility and mandatory volume paths; the development-stage contract permits replacing VMs.

Acceptance belongs to the new fifteen-row Cloud plan: actual signed install, current changed security-anchor review, owner access, real provider turns, ordinary agentless remote work, reconnect, three shutdown rules, credential scans, metering and permanent provider census. Browser/computer/actor/fault matrices continue in Browser. No provider resource, implementation change, protocol number or acceptance status is created by this document.
