# Managed Path-1 parity inventory (MP-11)

Audit date: 2026-09-23
Reviewed OSS head: `a3d42555f0b9514d09f83e1f6ce11979fa357ee9`
Worktree: `codex/mp11-managed-parity-inventory-20260923`

This is a bounded source inventory for the Path-1 parity gate in
`docs/BROWSER_COMPUTER_USE_END_TO_END_PLAN.md`. It inventories the OSS runtime,
CLI/API projections, service and image definitions, and host policy files that
select managed execution behavior. It does not amend the canonical plan.

## Evidence boundary and classification

Only source and documentation were inspected. No tests, build, Cargo command,
container, provider, service, machine, credential, or live infrastructure was
run or inspected. Source assertions below are not evidence from a fresh Linux
worker. `MP-01` through `MP-11` all remain open; this inventory closes none of
them.

The canonical plan allows exactly two Path-1 behavior differences: signed
release deployment/atomic activation and mandatory managed automatic shutdown.
The classification column uses:

- **Signed deployment** for work confined to provisioning, transferring context,
  or installing/verifying the immutable release.
- **Mandatory auto-shutdown** for the required managed lifecycle policy.
- **Parity defect** for any other managed-only runtime branch, selector,
  projection gap, or source/live proof gap. Conditional entries are defects if
  selected on Path 1; the label does not claim that a live reproduction exists.

The source-level branch map says the Path-1 unit selects the ordinary provider
path. That still needs an exact-head, fresh-worker run of the ordinary-versus-
managed matrix and MP-01 through MP-10 acceptance.

## Locked Hetzner reuse decision

The plan's 2026-09-21 decision permits reuse of the paid allocation only by a
provider-supported destructive rebuild/reimage from the approved clean image.
Removing the old kernel, release, or unit is insufficient. The reviewed kernel
running locally is the cutover authority; the old remote kernel cannot authorize
its own replacement. The rebuild must retire old services/processes/releases,
Bubblewrap processes, `/var/lib/chariox`, `/home/chariox`, machine and kernel
identity, enrollment, relay target, runtime identity, and stale Cloud rows.
Old paths are not copied back wholesale.

Fresh-equivalent evidence must bind the provider rebuild request/completion,
server identity, approved image, new boot/machine identity, new enrollment and
relay registration, exact signed release digest/source commit, absence of old
runtime residue, and retirement of the old identity. Only after that do the
ordinary-versus-managed matrix, all MP rows, every shutdown trigger, cleanup,
Browser/Computer, Web/TUI, and soak gates run. References:
`docs/BROWSER_COMPUTER_USE_END_TO_END_PLAN.md` sections “Locked Hetzner reuse
decision for 2026-09-21” and “Managed and ordinary kernel parity”, plus the
fresh-machine acceptance list under Milestone 9.

## Inventory

| Surface and source | Classification | Source finding and required follow-up |
| --- | --- | --- |
| **Provider child environment:** `apps/kernel/src/managed_bootstrap/worker.rs:517-558`; `apps/kernel/src/provider/managed_isolation.rs:107-175,897-927,1313-1340`; `apps/kernel/src/app/provider_launch_policy.rs:13-24`; `apps/kernel/src/runtime/state/provider_launch_owned_state.rs:236-246` | **Parity defect** | `spawn_kernel` adds `CHARIOX_MANAGED_REPOSITORY_ROOT`, `CHARIOX_KERNEL_HOST`, `CHARIOX_KERNEL_PORT`, `CHARIOX_ACCEPT_REMOTE_LEASES`, `CHARIOX_KERNEL_RUNTIME_ROLE=remote_lease_worker`, `CHARIOX_REMOTE_LEASE_CAPACITY`, `CHARIOX_LEASE_WORKER_HOME_CALLER`, and `CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1`. The provider control-removal list does not include these names; `command_from_provider_launch` does not clear the inherited environment, so provider children retain them. It also omits `CHARIOX_MANAGED_RELEASE_MANIFEST` and `CHARIOX_MANAGED_KERNEL_BINARY`, which the Path-1 unit supplies. The worker explicitly removes several other controls, and the provider scrub already removes `CHARIOX_HOME`, bootstrap/receipt paths, vault/provider-home paths, relay values, and release signature/public-key paths. **Action:** scrub all supervisor/bootstrap-only environment names before every provider process, retaining ordinary user/provider variables. Add a child-process environment assertion through the actual Path-1 adapter for normal launch, account utility, retry, restart, and reconnect. Assert the role, home-caller binding, repository root, topology, service/release paths, receipt, local-auth, relay, vault, and isolation markers are absent. This environment mismatch repeats after every kernel restart because `spawn_kernel` reconstructs the same environment. |
| **Isolation selector and working-directory filter:** `apps/kernel/src/provider/managed_isolation.rs:12-25,146-155,897-947,951-1147,1543-1643`; `apps/kernel/src/provider/registry.rs:16-24,142-156,188-199,234-244`; `apps/kernel/src/provider/codex/catalog_endpoint.rs:52-64`; `apps/kernel/src/provider/opencode/catalog_endpoint.rs:52-64` | **Parity defect if selected on Path 1** | `CHARIOX_MANAGED_PROVIDER_ISOLATION=1` activates Bubblewrap, fixed provider `HOME`, control masking, trusted/rebound workspace roots, and managed-specific failure messages. All provider adapters and Codex/OpenCode account utilities route through the same guard. The Path-1 service file does not set the isolation selector; the shared-host unit does. Path-1 must never gain a caller, retry, account utility, restart, restore, or reconnect route around that guard. **Action:** preserve an explicit Path-1 launch matrix for all provider entry points and retries; live evidence must show no Bubblewrap ancestor, marker, Bwrap variables, or selected-root restriction. Do not use the shared-host or inner-slice probe as Path-1 proof. |
| **Path filtering and exact control protection:** `apps/kernel/src/git_worktree_placement.rs:7-33,42-160,169-216`; `apps/kernel/src/runtime/workspace_search.rs:9-95,260-321`; `apps/kernel/src/provider/managed_isolation.rs:428-460,530-660,1543-1655` | **Parity defect if managed isolation filters Path 1** | The ordinary preflight checks the requested exact directory and Unix access without enumerating its children. It protects the Chariox state directory and configured control files. Workspace search treats `PermissionDenied` while enumerating children as best-effort and preserves the exact selected directory. The Bubblewrap branch separately masks protected/private roots and only rebinds selected roots; its `/home` and private-temp handling is a managed-only filter if that branch reaches Path 1. No Path-1-specific `/home` or `/tmp` allowlist appears in the ordinary route. **Action:** compare exact entry, discovery, create, and operation on `/home`, `/tmp`, nested paths, a new directory, and a repository made after enrollment against an ordinary Linux user. Verify `/home` remains a valid exact workspace even when listing a child is denied; verify an exact protected file is blocked while unrelated siblings and its parent remain usable. |
| **Path-1 account/home and mutable state:** `deploy/managed-kernel/chariox-disposable-worker-bootstrap.service:9-30,35-37`; `apps/kernel/src/managed_bootstrap/state.rs:215-224`; `apps/kernel/src/managed_bootstrap/worker.rs:517-568` | **Signed deployment** | The worker unit runs as `chariox` with `HOME=/home/chariox` and `CHARIOX_HOME=/home/chariox/.chariox`; the worker child recreates the same values. Bootstrap validates `CHARIOX_HOME == HOME/.chariox`. This matches the locked MP-04 path contract. `CHARIOX_MANAGED_PROVIDER_HOME=/var/lib/chariox/provider-home` is also set and the worker creates/validates that directory, although the ordinary Path-1 launch does not enter Bubblewrap, whose provider-home helper is the only source consumer found. **Action:** on a fresh worker prove mutable kernel state remains in `/home/chariox/.chariox`, ordinary providers receive ordinary `HOME`/`PATH`, and no provider uses `/var/lib/chariox/provider-home`; remove or justify the unused Path-1 provider-home allocation. |
| **Custom repository root API and client projection:** `apps/kernel/src/local/api/types/managed_environment.rs:33-42,203-226`; `packages/kernel-client/src/ipc-managed-environment-requests.ts:59-113`; `apps/cli/src/waiting-room-types.ts:71-110,150-166`; `apps/cli/src/waiting-room-managed-environments.ts:67-123,440-469`; `apps/cli/src/waiting-room-start-rows.ts:336-382` | **Parity defect (MP-06)** | The managed create request, TUI draft, and returned OSS summary contain no `managedRepositoryRoot` field or row. The source does not let the OSS caller choose the locked alternate trusted root or show the server-authoritative value after creation. This is an OSS-surface finding only; Cloud's separate web flow is outside this worktree. **Action:** thread one server-authoritative root through the managed-machine create request and TUI state, validate it at creation, and project it in the returned summary/Web surface. Prove that a worker inherits it and a worker/session/browser cannot submit a second override. |
| **Repository-root bootstrap and context materialization:** `apps/kernel/src/managed_bootstrap/state.rs:20,417-480`; `apps/kernel/src/managed_bootstrap/worker.rs:306-435,517-558,690-704`; `apps/kernel/src/managed_bootstrap/supervisor.rs:175-220`; `apps/kernel/src/managed_context/empty.rs:161-208`; `apps/kernel/src/managed_context/development/import.rs:451-467,708-737,1024-1057` | **Signed deployment; parity defect until provisioned root is selectable/projected** | Bootstrap schema 2 validates a normalized absolute root, binds the Cloud exchange response to the envelope, stores it in the receipt, and sets it on the child kernel. Empty workspaces and copied-repository imports consume that setting. Materialization requires the trusted root to exist as a real directory and rejects overlap with control state. The OSS create/projection gap above leaves this path inaccessible from that client; provisioning of a custom directory is a Cloud concern and was not inspected. **Action:** create the selected root through trusted provisioning before transfer, then prove default and custom roots survive exchange, receipt persistence, child restart, empty workspace setup, copied import, reconnect, and projection; prove collision/no-clobber behavior. |
| **Repository basename and collision rules:** `apps/kernel/src/managed_context/development/export.rs:282-310,417-423,481-559`; `apps/kernel/src/managed_context/development/import.rs:510-549,609-650` | **Signed deployment** | Managed context export preserves a validated single-component source basename and rejects case-insensitive selected-repository basename collisions. Import checks destination absence before materialization and publishes without clobbering. These are context-transfer rules, not a runtime provider cwd allowlist. **Action:** retain tests for source basename preservation, reserved names, two repositories with case-only collision, pre-existing target collision, rollback, and custom-root placement; confirm the live worker permits other user-owned paths after transfer. |
| **Bootstrap retry, restart, and identity restoration:** `apps/kernel/src/managed_bootstrap/worker.rs:280-303,306-409,430-510,517-571,690-719`; `apps/kernel/src/managed_bootstrap/supervisor.rs:120-145,175-220`; `apps/kernel/src/provider/run_actor/command_execution.rs:34-103`; `apps/kernel/src/runtime/state/provider_launch_owned_state.rs:180-246` | **Signed deployment for worker recovery; parity defect for inherited provider selectors** | The disposable-worker bootstrap retries with bounded backoff, recovers exchange/confirmation state from its receipt, validates the same allocation/release/root, and respawns the same Path-1 child configuration after a kernel exit. Provider prompt resume/runtime restoration remains in the shared adapter/run-actor path; no managed-provider retry or history model was found there. The inherited-variable finding above is therefore a retry/restart finding too. **Action:** test process ancestry/environment before and after worker restart, lost confirmation response, relay reconnect, provider retry, and provider-state restore; compare session, history, active/queued turns, and error payloads with ordinary Linux. |
| **Exact error mapping:** `apps/kernel/src/git_worktree_placement.rs:48-160`; `apps/kernel/src/provider/managed_isolation.rs:901-945,2062-2065`; `apps/kernel/src/transport/relay_client/peer_requests.rs:1681-1703`; `apps/kernel/src/local/api/types/managed_environment.rs:219-226`; `apps/cli/src/waiting-room-managed-environment-launch-controller.ts:348-408` | **Parity defect for provider behavior; signed deployment for setup-transfer errors** | Ordinary cwd failures use a shared `LocalTransport` mapping. Managed-isolation failures use provider-isolation errors and relay context-import failures project a managed-transfer code/retryable pair. The latter are bootstrap/context-transfer errors; they must not replace normal provider/runtime errors after enrollment. The TUI projects managed operation `lastErrorCode`/`lastErrorMessage` and rejects transfer/launch binding mismatches. **Action:** on identical ordinary/Path-1 runtime failures compare error category, message shape, retryability, and client projection. Separately retain the managed setup/reimage binding-error tests. |
| **Managed create/reimage/launch projections:** `apps/kernel/src/runtime/managed_environment_control.rs:1-130`; `apps/cli/src/waiting-room-managed-environment-launch-controller.ts:78-205,348-408`; `apps/cli/src/waiting-room-managed-environment-reimage-controller.ts:78-205`; `apps/cli/src/waiting-room-managed-environments.ts:28-65,372-385`; `apps/ios/CharioxPackage/Sources/CharioxFeature/Kernel/KernelProtocolModels.swift:3-19,54-69` | **Signed deployment; parity defect for missing root projection** | The CLI has managed-environment catalog, launch-readiness, lifecycle, reimage-confirmation, transfer-ticket, and target-binding projections; these are placement/deployment controls. After pivot it uses the returned runtime kernel/session target. The inspected Swift `RuntimeSession` has ordinary workspace, worktree, agent, interaction, and live-sync fields and no managed-environment branch. The Swift scan found no managed-machine projection or selector. **Action:** keep managed identity/generation/context checks on deployment operations, expose the root required by MP-06, and compare post-pivot session behavior on Web/TUI/Swift using the same runtime protocol. No iOS test/build was run. |
| **Service-unit sandboxing:** `deploy/managed-kernel/chariox-disposable-worker-bootstrap.service:9-37`; `deploy/managed-kernel/chariox-managed-bootstrap.service:9-55`; `apps/kernel/src/managed_bootstrap/tests.rs:834-870`; `apps/kernel/src/managed_bootstrap/worker.rs:517-558` | **Signed deployment separation; parity defect if shared-host unit is selected for Path 1** | The Path-1 unit has no `CHARIOX_MANAGED_PROVIDER_ISOLATION`, `CHARIOX_CAPABILITY_ISOLATION_ROOT`, Bubblewrap selector, or provider-facing `NoNewPrivileges`, `PrivateTmp`, `ProtectSystem`, `ProtectHome`, `RestrictSUIDSGID`, `RestrictAddressFamilies`, `ReadWritePaths`, or `UMask`. It explicitly conflicts with the shared-host unit. The shared-host unit sets isolation and those systemd restrictions. Its supervisor strips the shared-host selector list before spawning a Path-1 child. The Path-1 child still receives other kernel-only selectors noted above. **Action:** retain a negative test for every restricted unit property on the Path-1 unit and a live `systemd-run`/process-ancestry comparison; prove every launch/restart remains in the disposable-worker unit. |
| **Image, containers, Bubblewrap, and host policy:** `deploy/managed-kernel/prepare-hetzner-image.sh:29-34,66-75,176-178`; `apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh:765-781,857-890`; `apps/kernel/slice-linux-docker/docker/Dockerfile:129-138,196-222`; `apps/kernel/slice-linux-docker/docker/managed-provider-bwrap.sh`; `apps/kernel/slice-linux-docker/chariox-slice-provider.apparmor`; `deploy/managed-kernel/chariox-rootless-docker.service:6-37`; `deploy/managed-kernel/chariox-slice-broker.service:6-33`; `apps/kernel/slice-linux-docker/chariox-rootless-engine.service:1-25` | **Signed deployment for inner-slice tooling; parity defect if selected as Path-1 provider boundary** | The managed image installs Bubblewrap for Docker-slice inner defense and shared-host images; the builder's Bubblewrap runtime-bind probe is called only for `shared_host`. Docker slice provisioning can require a nested Bubblewrap compatibility probe, mount selected workspace roots, and apply slice-only AppArmor/seccomp/capability policy. Rootless Docker, broker, and rootless-engine units are separately hardened infrastructure services. The scanned Dockerfiles, service units, sockets, and AppArmor file yielded no other Path-1 provider unit. **Action:** keep these selectors scoped to their separate topology; prove Path-1 provider PIDs never descend from the slice launcher and do not inherit its mounts, capabilities, security profile, or workspace-root environment. Never count `probe-provider-launch-ab.sh` or a nested-slice probe as Path-1 acceptance. |
| **Signed native release and activation:** `apps/kernel/src/managed_bootstrap/release.rs:1-65,285-345`; `deploy/managed-kernel/install-image.sh:397-456`; `deploy/managed-kernel/upgrade-image.sh:620-654`; `apps/kernel/src/managed_bootstrap/state.rs:158-211,218-224` | **Signed deployment** | The managed release is verified against manifest/signature/public-key and digest, published under the digest-named root-owned release tree, and activated through the `current` path and `/usr/local/bin/chariox-kernel`. Mutable kernel home/state is prepared separately. This is the allowed production deployment difference. **Action:** exact-head focused release/activation/rollback/crash-recovery tests plus live evidence for signature, digest, root ownership, active binary, and state remaining under `/home/chariox/.chariox`. |
| **Mandatory shutdown policy and all triggers:** `apps/kernel/src/local/api/types/managed_environment.rs:92-97,205-226`; `apps/kernel/src/runtime/managed_environment_control/cloud_contract.rs:145-168,300-318`; `apps/cli/src/waiting-room-managed-environments.ts:352-385`; `apps/cli/scripts/managed-ordinary-parity-matrix.mjs:108-127,277-289`; `apps/kernel/src/runtime/managed_kernel_activity.rs:19-68,70-120` | **Mandatory auto-shutdown** | OSS projects `minimumRuntimeSeconds` and nullable `idleDelaySeconds` to Cloud and reports signed worker activity. The matrix requires `agents_done`, 15-minute idle, 30-minute idle, minimum three-hour runtime, disabled, keep-running, restart reconciliation, manual stop, custom delay, explicit lifecycle reconciliation, and deployment reconciliation. Idle timing must be measured from the last agent finishing. The OSS source sends activity and policy; Cloud's timer/reconciliation executor was not inspected here. **Action:** keep every matrix scenario open until the fresh rebuilt worker proves expected stop/no-stop, timing origin/delay, cleanup, restart restore, and Cloud/provider reconciliation. |
| **Existing parity-inventory documentation:** `docs/MANAGED_ORDINARY_KERNEL_PARITY_INVENTORY.md:10-22,32-39,43-59,108-145` | **Parity defect (documentation inconsistency / evidence gap)** | The existing inventory says the managed repository destination is fixed because no trusted root exists (line 15), then later requires a machine-level trusted `repository_root` through Cloud/bootstrap/worker/Web (lines 32-37). This audit found schema-2 bootstrap and import support but no OSS create/projection field. The same document correctly says static assertions and focused Node tests are not fresh-worker evidence. **Action:** reconcile that document when MP-06's authoritative create/projection path is implemented; retain all live evidence requirements. |

## MP continuity status

`MP-01` through `MP-11` remain open. The two highest-priority implementation
handoffs from this inventory are:

1. Remove all Path-1 supervisor, worker-role, repository-root, topology, and
   managed-release environment values from provider children, then test the
   real provider-grandchild environment across launch, retry, restart, and
   reconnect. This is the clearest source-level runtime parity defect found.
2. Add and prove the single trusted repository-root value in the OSS create and
   projection path, then verify its Cloud provisioning, receipt, child-worker,
   empty-workspace, copied-repository, and reconnect flow. The separate Cloud
   web/API implementation was not inspected, so this is an OSS gap, not a claim
   that the Cloud implementation is absent.

Remaining acceptance gaps are source-only versus fresh-machine proof for
`/home`, `/tmp`, exact control files, provider mounts/privileges/network/tool
installation, all structured errors, ordinary provider/session/history/client
parity, signed-release activation, every shutdown trigger, cleanup, Browser/
Computer, Web/TUI, and soak behavior.

## Uninspected scope

- `chariox-cloud`: the managed-environment web form, API request/storage schema,
  provisioning/bootstrap envelope issuer, trusted-root directory creation,
  provider-specific Hetzner request/reimage path, Cloud shutdown timers and
  reconciliation, and Web root projection. OSS source cannot establish those
  behaviors.
- Live host: provider-side destructive rebuild request and receipt, approved
  image identity, effective systemd properties, file ownership/modes after
  unit startup, host AppArmor/LSM state, provider ancestry/environment/mounts,
  `/home` and `/tmp` operations, new enrollment/relay identity, old-state
  residue, resource samples, or cleanup.
- Runtime evidence: no official provider launch, environment capture, MP matrix,
  fresh-machine comparison, reconnect/restart fault, shutdown timing run, or
  evidence-signature validation. Existing source tests and static service tests
  were read as source only and were not run.
- Swift: managed-machine selectors/projections were searched and none were
  found in the inspected iOS feature sources; no exhaustive feature-by-feature
  Swift runtime parity review or iOS build/test was performed.
- Other non-Path-1 product areas and third-party dependencies were not audited
  line by line. The service/container/AppArmor scan covered the repository's
  checked-in unit, Dockerfile, socket, and AppArmor definitions; it says nothing
  about policy injected by Cloud or the live host.
