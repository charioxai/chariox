# Managed Path-1 parity inventory (MP-11)

Audit date: 2026-09-26 · published OSS source baseline: `4c8b979430d2dca6662de0b478a8b75b7ac3b231`; retained audit OSS baseline: `dbfebe394707c7b5c85a2ee02aa5999e5e9e44b4`; Cloud source baseline last inspected: `73d82d3d3b578cb3da54dbb5a58dfcffd083b58e` (stale; not refreshed in this pass).

This is a source inventory for the canonical gate in
`docs/BROWSER_COMPUTER_USE_END_TO_END_PLAN.md`; it does not change that plan.
Static source and test assertions are not fresh-machine acceptance. `MP-01`
through `MP-11` all remain open.

The plan permits two Path-1 differences: signed release deployment/activation
and mandatory managed automatic shutdown. The 2026-09-21 Hetzner decision
requires a provider-supported destructive rebuild from the approved clean
image. Fresh-equivalent evidence must bind that rebuild to the same allocation,
new boot/machine/enrollment/relay identities, reviewed release, absence of old
runtime residue, and retirement of the prior identity before the parity matrix
or remaining acceptance gates run.

## Retained scoped OSS source audit (2026-09-26)

This section carries forward the source-family audit recorded in commit
`b221f2c626df8c55a07003cd491b0bdce58e5220` and its follow-up
`bcfa97e6953ece7086d15a23bd6047418857b51e`. That audit used OSS baseline
`dbfebe394707c7b5c85a2ee02aa5999e5e9e44b4`; other source families were not
re-audited at the published baseline above. Cloud findings retain their older
baseline and are stale. No MP gate is closed by this source inventory.
`managed_bootstrap/`, `provider/`, `runtime/`, and `git_worktree_placement.rs`
abbreviate `apps/kernel/src/`. Deployment scripts, verifier, and unit names
abbreviate `deploy/managed-kernel/`; `provision-linux-docker-slice.sh`,
`docker/`, and the AppArmor profile abbreviate
`apps/kernel/slice-linux-docker/`. `waiting-room-*` and
`cli-waiting-room-composition.ts` abbreviate `apps/cli/src/`; the full
`packages/kernel-client/src/` paths are shown. Swift kernel models/requests
abbreviate `apps/ios/CharioxPackage/Sources/CharioxFeature/Kernel/`, and Swift
command/state files abbreviate `apps/ios/CharioxPackage/Sources/CharioxFeature/State/`.

### Source fixes and remaining acceptance

1. The local placement fix protects only five slice control subdirectories.
   The retained audit found the configured Path-1 slice root was not filtered.
   Agent commit `347b823c4f55855a7d91fc127fc53314d98cdd3b` adds only
   `runtime`, `states`, `backups`, `defaults`, and `logs` below
   `CHARIOX_SLICE_ROOT` through the shared cwd and managed repository preflights
   (`git_worktree_placement.rs:23-25,96-153,175-193,225-233`). Its regressions
   allow the configured root, development project and unrelated siblings,
   reject the control directories and descendants, and reject a symlink alias
   to `backups` (`git_worktree_placement.rs:1141-1200`). Do not impose a blanket
   slice-root ban. Root integrated this patch locally and corrected an existing
   macOS fixture to expect canonical `/private/var` instead of `/var`.
   Focused Rust execution then passed 15/15, including both new regressions and
   the corrected fixture. Placement publication, aggregate validation,
   approval, deployment and fresh-worker evidence remain pending.

2. The provider PATH fix is published in root commit
   `aa4fa10831bbc3902fde005b0cf8f930c6224b7e`, integrated byte-identically from
   `0832289de8feaf3ba30068c378d66cec965b6900`. Both Path-1 role units use
   `/home/chariox/.local/bin:/usr/local/bin:/usr/bin:/bin`
   (`chariox-path1-managed-bootstrap.service:25`,
   `chariox-disposable-worker-bootstrap.service:30`). Codex, Claude and
   OpenCode resolve commands in PATH order (`provider/codex.rs:164-176`,
   `provider/claude.rs:508-524`, `provider/opencode.rs:163-176`); image setup
   places pinned binaries under `/usr/local/bin`
   (`prepare-hetzner-image.sh:197-199`). Root ran 17/17 focused source-contract
   tests successfully. Actual effective units and resolved binaries on the
   fresh worker, exact-head review and deployed parity remain unproven.

3. The public Project setup regression found the generated repeatable
   definition is persisted after validation. Agent75 is correcting the local
   and authenticated leased-worker paths in `project_environment_setup.rs`,
   preserving home authority, attempt/cancellation fencing and the validation
   gate for Ready. The fix and its lifecycle tests are not yet integrated or
   executed by root. This remains an MP-08 implementation requirement.

### Source branch inventory

| Family | Source classification and remaining evidence |
| --- | --- |
| Topology and provider launch — `managed_bootstrap/mod.rs:43-85`; `supervisor.rs:182-248`; `worker.rs:518-562`; `provider/managed_isolation.rs:157-165,908-936`; `provider/registry.rs:150-160,191-201,241-251`; catalog endpoints `codex/catalog_endpoint.rs:61-73`, `opencode/catalog_endpoint.rs:61-73` | Bootstrap topology is explicit and fail-closed. Path-1 removes shared-host isolation selectors; each official adapter uses the common unwrapped branch unless `CHARIOX_MANAGED_PROVIDER_ISOLATION` is truthy. Account utility endpoints use the same selector and command builder. This is source flow, not provider ancestry proof. |
| Environment scrub, retry, restore, reconnect — `provider/managed_isolation.rs:107-143,168-188,908-936,1324-1351`; `provider/service/run_lifecycle.rs:11-64`; `runtime/state/provider_relaunch_runtime.rs:55-85`; `runtime/state/provider_launch_failure_runtime.rs:27-45,120-174`; `runtime/state/project_environment_setup.rs:2061-2099`; `provider/run_actor/command_execution.rs:25-70` | The fixed list removes managed topology, slice, bootstrap, broker and release controls; ambient secret-like and numbered workspace-root names are also removed. Selected account paths and resolved credentials are intentional launch inputs. Initial starts and policy relaunches resolve through the common adapter; setup-recovery snapshots preserve that launch. Prompt submit/abort restore live runtime slots without selecting a new topology. Launch-failure retry here retries durable cleanup, not provider execution. Live retry/reconnect comparison remains open. |
| Supervisor restart and systemd — `managed_bootstrap/supervisor.rs:110-172`; `managed_bootstrap/worker.rs:278-304,463-489`; Path-1 units `chariox-path1-managed-bootstrap.service:9-32`, `chariox-disposable-worker-bootstrap.service:9-39`; `verify-image-release.mjs:369-445`; `upgrade-image.sh:121-134,654-660` | Kernel respawn uses the same Path-1 helper; worker preparation/restart remains Path-1. Role units contain no provider-restricting `Protect*`, `Private*`, `NoNewPrivileges`, namespace, address-family, `ReadWritePaths`, or `UMask` directives; release verification rejects those on the home unit and upgrade rejects Path-1 drop-ins. Worker `StateDirectory=chariox`/mode `0700` allocates service state, not a filesystem/process sandbox. Hardened shared-host, rootless Docker and broker services are separate. Effective installed units remain uninspected. |
| Image, shell, container and AppArmor selectors — `prepare-hetzner-image.sh:36-54,137-149,239-241`; `install-image.sh:16-31,456-476,563-568`; `upgrade-image.sh:24-40,121-142`; `provision-linux-docker-slice.sh:766-784,860-883`; `docker/start-runtime.sh:146-174`; `chariox-slice-provider.apparmor:1-7` | Preparation/install/upgrade choose Path-1 explicitly and verify its unit. Bubblewrap/AppArmor/seccomp compatibility flags apply to the separate inner Docker slice; `start-runtime.sh` launches that slice kernel with managed isolation. The locked plan allows that distinct inner topology; it is not selected for host Path-1 provider children. No image build or effective policy inspection proves this on the live machine. |
| Exact protected paths and Swift/TUI projections — `git_worktree_placement.rs:18-33,42-157,193-242`; `runtime/workspace_search.rs:198-253`; `packages/kernel-client/src/waiting-room-runtime-placement.ts:54-90,104-125`; CLI `waiting-room-controller.ts:320-411`, `waiting-room-managed-environments.ts:43-68,95-124`, `waiting-room-managed-environment-launch-controller.ts:88-120`, `waiting-room-managed-environment-reimage-controller.ts:78-145`, `waiting-room-start-rows.ts:136-147`, `cli-waiting-room-composition.ts:547-625,752-885`; `packages/kernel-client/src/ipc-managed-environment-requests.ts:24-140,183-218,234-305`; Swift `KernelProtocolModels.swift:3-89`, `KernelProtocolRequests.swift` | Exact entry and session/provider launch share cwd preflight. The local placement fix protects five control subdirectories, not the root/development/siblings. CLI projects catalog/readiness, trusted-root/context setup, lifecycle/auto-stop and reimage confirmation. After preparation it strips managed-only fields before ordinary session launch (`cli-waiting-room-composition.ts:598-625`). These are setup/control-plane projections, not an execution exception. Inspected Swift session/request models have no managed-environment projection; Swift managed commands concern live sync (`CharioxAppModelCommands.swift:85-98,157-189`; `CommandCenter.swift:164-167,464-495`). No Swift build or end-to-end client parity was run. |

### Uninspected and live requirements

- Finish Project persistence correction, including remote pre-validation home
  acknowledgment and fenced writes, and run exact-head lifecycle tests. The
  15/15 placement and 17/17 PATH source tests do not prove effective units.
- Inspect systemd/drop-ins, provider ancestry, resolved binaries, environment,
  mounts, permissions, `/home`, `/tmp` and actual slice-root behavior.
- Exercise retry, policy relaunch, restart/reconnect, account selection,
  Swift/client flows, errors, history and every shutdown trigger on the rebuilt
  image. Preserve ordinary behavior except signed deployment and shutdown.
- Re-audit deployed Cloud; bind allocation/rebuild, boot/machine/enrollment,
  signed release, relay retirement, residue, cleanup, resources and soak.
  MP-01 through MP-11 remain open pending full acceptance and review.

## Previously reconciled source inventory (OSS `294ff610d03ddf57a367334a4da29bfad8e106cf`; Cloud `73d82d3d3b578cb3da54dbb5a58dfcffd083b58e`)

| Surface | Exact-head finding and remaining evidence |
| --- | --- |
| **Provider-child environment** — `apps/kernel/src/managed_bootstrap/worker.rs:518-559`; `apps/kernel/src/provider/managed_isolation.rs:107-142,167-200,907-935,1323-1350` | The previously reported Path-1 leak of repository root, kernel host/port, remote-lease role/capacity, home-caller binding, topology, and release manifest/binary is corrected at the provider-launch scrub: both ordinary and isolated branches add these controls to `pty_env_remove`, and `command_from_provider_launch` applies removals to inherited and adapter-supplied values. The subsequently identified `CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE` marker is also scrubbed at this boundary. Focused assertions for the control list, ordinary Path-1 launch, and account-utility child passed 3/3 locally; actual provider-turn, retry, restart, and reconnect evidence remains required. |
| **Isolation selector and ordinary provider path** — `apps/kernel/src/provider/managed_isolation.rs:156-165,907-935`; `apps/kernel/src/provider/registry.rs:16-25,142-156,188-199,234-244`; `deploy/managed-kernel/chariox-path1-managed-bootstrap.service:9-26` | The Path-1 service selects `path1`, not `CHARIOX_MANAGED_PROVIDER_ISOLATION`; the ordinary branch preserves the provider executable/cwd and does not insert Bubblewrap. Provider adapters share that branch. `path1_ordinary_provider_launch_is_unwrapped_and_scrubs_managed_controls` at `managed_isolation.rs:5151-5240` is the focused assertion. No source-only adapter path was found that selects the shared-host isolation branch for Path 1. Live provider ancestry, mount, privilege, network, tool-installation, retry, and reconnect comparisons remain open. |
| **Workspace discovery, cwd, and protected controls** — `apps/kernel/src/git_worktree_placement.rs:18-33,42-160,169-216`; `apps/kernel/src/runtime/workspace_search.rs:9-95,260-321` | Ordinary and Path-1 exact-path admission share metadata/access checks and do not enumerate children; protection targets Chariox state and configured control roots/files. The regression assertions at `apps/kernel/src/git_worktree_placement.rs:719-789,792-843,987-1050` cover `/home`, `/tmp`, new paths, provider-home descendants, siblings, and exact control state. Workspace child enumeration remains best-effort on `PermissionDenied`. These are source assertions; live `/home`, `/tmp`, permissions, and control-file acceptance remain MP-02/03 gates. |
| **Managed home and mutable state** — `deploy/managed-kernel/chariox-path1-managed-bootstrap.service:9-26`; `deploy/managed-kernel/chariox-disposable-worker-bootstrap.service:9-40`; `apps/kernel/src/managed_bootstrap/worker.rs:531-572` | The managed-home unit and allocation-worker unit are now distinct. Both use `/home/chariox` and `/home/chariox/.chariox`; the worker receives its root from the confirmed receipt and starts the ordinary Path-1 kernel. Release data remains under the digest-named `/usr/lib/chariox` tree. The disposable-worker unit still allocates `/var/lib/chariox/provider-home`; Path-1 launches do not select Bubblewrap, but the no-use claim and mutable-state placement still need fresh-worker evidence. |
| **MP-06 trusted root create/projection** — `apps/kernel/src/local/api/types/managed_environment.rs:33-45,203-226`; `packages/kernel-client/src/ipc-managed-environment-requests.ts:93-115,258-268`; `apps/cli/src/waiting-room-managed-environments.ts:16,70-105`; `apps/cli/src/waiting-room-managed-environment-launch-controller.ts:46-55,100-120`; `apps/cli/src/waiting-room-start-rows.ts:136-147,349-365` | The OSS create API accepts optional `managed_repository_root`/`managedRepositoryRoot` (omission keeps `/home/chariox`); the TUI selects a root, sends it at creation, and displays the returned summary's required `managedRepositoryRoot`. The launch idempotency signature includes the selected value. Focused assertions are in `apps/cli/src/waiting-room-managed-environments.test.ts:55-68,70-119,693-705`, `apps/cli/src/waiting-room-managed-environment-launch-controller.test.ts:26-40,227-247`, and `packages/kernel-client/src/ipc-managed-environment-requests.test.ts:72-102,247-267`. Bootstrap schema v2 validates and binds the same root through exchange and receipt (`apps/kernel/src/managed_bootstrap/state.rs:417-460`; tests `state.rs:732-789`, `apps/kernel/src/managed_bootstrap/tests.rs:325-400`); the worker/supervisor pass the receipt value to the kernel (`worker.rs:531-539`, `supervisor.rs:187-216`). No distinct root generation field is needed. The Cloud source path is inventoried in the following rows. Fresh-machine behavior, including rejection of browser-supplied child overrides, remains open; MP-06 is not closed. |
| **MP-06 Cloud create and immutable projection** — Cloud `apps/api/src/managed-environments/control-service.ts:536-595,748`; `apps/api/src/managed-environments/managed-repository-root.ts:1-31`; `packages/db/prisma/models.prisma:1657`; `apps/web/src/terminal/waiting-room-managed-environment-launch-plan.ts:25-32`; `apps/web/src/terminal/managed-environment-browser-client.ts:535-550` | The create request is validated as a normalized absolute root, stored on the environment, and included in the nondefault idempotency digest. The Web form sends the selected root; its browser client reads the server summary rather than treating the form value as authoritative. Cloud tests cover default/custom validation, immutable replay, and response authority (`control-service.test.ts:34-114,927-966`; `managed-environment-browser-client.test.ts:150-195`). This establishes source-level create/projection behavior only; the deployed API and fresh-worker receipt still need comparison. |
| **MP-06 Cloud bootstrap and child-worker inheritance** — Cloud `apps/api/src/managed-environments/bootstrap-service.ts:231-235,340-349,396-412`; `apps/infrastructure-manager/src/managed-kernel-cloud-init.ts:112-145`; `apps/api/src/disposable-workers/allocation-service.ts:144-186,382-444`; `apps/api/src/disposable-workers/bootstrap-service.ts:78-82,165-177`; `apps/infrastructure-manager/src/hetzner-worker-adapter.ts:221-240` | Cloud reads the persisted environment root for the Path-1 bootstrap grant, encodes it in the cloud-init envelope, and returns it at exchange/confirm. Child allocation resolves the current ready home generation and persists that root on the allocation; stale or ambiguous managed-home binding fails closed, and idempotent replay retains the original allocation value even if the environment row changes. Child bootstrap and cloud-init use the allocation value, with no browser-supplied child-root override. Focused tests are `bootstrap-service.test.ts:95-140`, `managed-kernel-cloud-init.test.ts:90-135`, and `disposable-workers/allocation-service.test.ts:135-197`. This is source/test evidence, not a live child-worker assertion. |
| **Root materialization, basename, and collision behavior** — `apps/kernel/src/managed_bootstrap/mod.rs:88-98,323-396,518-522,758-781`; `apps/kernel/src/managed_context/empty.rs:161-208`; `apps/kernel/src/managed_context/development/export.rs`; `apps/kernel/src/managed_context/development/import.rs` | Bootstrap rejects mismatched/non-normalized roots, stores the root with the existing generation-bound receipt, and the empty-workspace/import paths use the trusted setting. Export preserves a validated source basename and rejects case-insensitive selected-repository collisions; import rejects occupied destinations instead of clobbering. Focused source assertions include `apps/kernel/src/managed_context/development/tests.rs:699-818` (basename and collision), `823-874` (trusted-root publication/retry), and `887-947` (occupied destinations), plus `apps/kernel/src/managed_bootstrap/tests.rs:325-400` (root match/mismatch). The copied context, custom directory creation, no-clobber recovery, and reconnect flow still need exact-head execution on a fresh worker. |
| **Bootstrap retry, restart, and identity restoration** — `apps/kernel/src/managed_bootstrap/worker.rs:280-510,517-571,690-719`; `apps/kernel/src/managed_bootstrap/supervisor.rs:120-270`; `apps/kernel/src/provider/run_actor/command_execution.rs:34-103`; `apps/kernel/src/runtime/state/provider_launch_owned_state.rs:180-246` | Bounded bootstrap retry and receipt recovery preserve the same allocation, release, root, and worker configuration; restart reconstructs the Path-1 child boundary. Focused source tests include `apps/kernel/src/managed_bootstrap/worker.rs:1054-1200` (`disposable_worker_spawn_uses_ordinary_kernel_and_scrubs_parent_state`), `1435-1510` (`disposable_worker_restart_reapplies_path1_contract`), and `apps/kernel/src/managed_bootstrap/supervisor.rs:1095-1255` (`path1_confirmation_restart_reapplies_ordinary_boundary`). No provider retry/history divergence is source-proven here. Provider process ancestry/environment and session/history parity across restart/reconnect remain live gates. |
| **Errors and client/runtime projections** — `apps/kernel/src/git_worktree_placement.rs:48-160`; `apps/kernel/src/provider/managed_isolation.rs:907-945`; `apps/kernel/src/transport/relay_client/peer_requests.rs:1681-1703`; `apps/cli/src/waiting-room-managed-environment-launch-controller.ts:348-408`; `apps/ios/CharioxPackage/Sources/CharioxFeature/Kernel/KernelProtocolModels.swift:3-19,54-69` | Ordinary cwd errors retain the shared local-transport mapping; isolated-provider and context-transfer failures use their scoped error surfaces. The CLI projects the server managed-root summary, while the inspected Swift `RuntimeSession` has no managed-machine projection. No iOS build or exhaustive cross-client parity review was done. Compare structured errors and post-pivot Web/TUI/Swift behavior on the same runtime before closing MP-08. |
| **Managed-home vs disposable-worker service selection** — `deploy/managed-kernel/prepare-hetzner-image.sh:29-42,123-124`; `deploy/managed-kernel/install-image.sh:16-24,467-476`; `deploy/managed-kernel/upgrade-image.sh:24-40,108-123,602-613`; `deploy/managed-kernel/managed-kernel-upgrade-state.mjs:331-337`; `apps/cli/scripts/live-path1-rebuild-evidence-verifier.mjs:362-374` | Fresh-image preparation requires explicit topology and installs the Path-1 managed-home service; receipt kind distinguishes allocation-worker service. The rebuild verifier requires `chariox-path1-managed-bootstrap.service` and rejects the shared-host and disposable-worker units. The prior omitted-selector defect is fixed: `upgrade-image.sh` now rejects missing, empty, or invalid topology before touching an image, with focused Node 2/2. The installer also verifies a reused digest-named release under the selected topology, with focused activation 10/10 and one Linux-root signed Path-1 fixture pass. These are source/fixture checks; actual host install, upgrade, systemd selection, and rollback remain live gates. |
| **Image, containers, Bubblewrap, and host policy** — `deploy/managed-kernel/prepare-hetzner-image.sh:185-187`; `apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh:765-781,857-890`; `apps/kernel/slice-linux-docker/docker/Dockerfile:129-138,196-222`; `deploy/managed-kernel/chariox-rootless-docker.service`; `deploy/managed-kernel/chariox-slice-broker.service` | Bubblewrap runtime-bind probing and slice AppArmor/seccomp/capability policy belong to shared-host or Docker-slice topologies. Checked-in Path-1 service selection does not choose them. No image build, effective host policy, provider ancestry, or live mount/capability inspection was performed; retain those acceptance requirements. |
| **Signed release and shutdown policy** — `apps/kernel/src/managed_bootstrap/release.rs:1-65,285-345`; `deploy/managed-kernel/install-image.sh:397-476`; `deploy/managed-kernel/upgrade-image.sh:620-654`; `apps/kernel/src/runtime/managed_kernel_activity.rs:19-120`; `apps/cli/scripts/managed-ordinary-parity-matrix.mjs:108-127,277-289` | Signed digest verification, atomic activation, and rollback are deployment differences; release-ordering/recovery assertions are in `deploy/managed-kernel/managed-release-activation.test.mjs`. The stale installer-call assertion and the missing topology verification on reused releases were corrected, and the full Node suite passes 10/10. Mandatory Cloud shutdown remains an allowed policy difference. OSS reports activity and policy, but Cloud timer/reconciliation execution was not inspected here. Run all triggers, including last-agent-finish timing, disabled/keep-running, restart, manual stop, and deployment reconciliation, on the rebuilt worker before closing MP-09. |
| **Prior parity inventory** — `docs/MANAGED_ORDINARY_KERNEL_PARITY_INVENTORY.md:12-39,112-145` | Its former fixed-root statement is now correctly qualified as the default `/home/chariox/<basename>`; the machine-level trusted root may select another absolute path, and Cloud/bootstrap/receipt/worker/Web must carry one authoritative value. The existing document correctly keeps source assertions separate from live capture. |

## Source checks and remaining gate

The Rust assertions in the original agent inventory were inspected only. The
subsequent topology and release changes passed focused Node suites (upgrade
2/2, activation 10/10) and a signed installer fixture as Linux root (1/1).
The provider-marker Rust selectors passed 3/3 locally. No real host
installation, provider run, service change, or VM rebuild was performed here.
The Cloud MP-06 audit ran 61 focused compiled Node tests across managed-home
create/bootstrap, child-worker allocation/bootstrap, cloud-init, Web browser
projection, and launch-plan code: 61 passed, 0 failed. These do not replace
deployed Cloud or fresh-machine evidence.

The 2026-09-26 follow-up on production source `64c8e96888a7832693f0c5e28f5a2cefeb34a15b`
requires explicit installer topology and rejects conflicting enabled or active
bootstrap roles during image preparation. The managed-image Node suite passes
16/16; signature and release-activation suites pass 30/30. The full upgrade
fixture suite passes 54/54 as Linux root in a disposable cached Node image,
with one CPU, 1 GiB memory, no network, and a read-only source mount. That suite
requires Linux-root execution; the non-root macOS invocation fails the root
guard before exercising upgrade behavior. No real host service, release, VM,
or enrollment was changed by these fixture tests.

The status/find read-action implementation is now present in
`apps/kernel/src/runtime/state/tool_dispatch/slice/controller_browser.rs`,
including inline actor/tab/history tests. The Drill E verifier's focused Node
tests pass 3/3 on the 2026-09-26 integration source. This closes neither the
exact-head Rust execution gate nor the live three-agent concurrency gate.
The next action is to verify the implementation and retain live overlapping
read and mutation evidence, then prove provider and client behavior in the
ordinary-versus-Path-1 campaign.
The remaining acceptance is a residue-free approved-image rebuild
and the full exact-head ordinary-versus-Path-1 matrix, shutdown triggers,
Browser/Computer and Web/TUI parity, cleanup, and soak. `MP-01` through `MP-11`
remain formally open until that fresh-machine evidence is reviewed.

## Uninspected scope

### Read-only Cloud receipt capture

The normal home-kernel receipt read requires protocol 345. After the selected
Cloud reimage operation has finalized, capture its binding with the reviewed
local home kernel, using non-secret identifiers and a new external output file:

```sh
node apps/cli/scripts/path1-cloud-reimage-capture.mjs \
  --kernel ws://127.0.0.1:<port> \
  --environment <environment-id> --operation <operation-id> \
  --generation <new-generation> --release sha256:<reviewed-release-digest> \
  --commit <reviewed-40-character-commit> --tree <reviewed-40-character-tree> \
  --output /Users/miguel/.codex/evidence/browser-computer-use/<campaign>/cloud-reimage.json
```

Build `@chariox/kernel-client` first. The output parent must already exist and
must resolve outside the repository. The command never starts or authorizes
reimage. It rejects pending receipts, wrong operation/generation/release
bindings, unchanged identities and incomplete retirement observations. It
retains only allowlisted data in a mode-0600 file and refuses overwrite.
Cloud's receipt digest is retained, not independently recomputed or verified.
This capture alone cannot close MP-07 or MP-10. Host, provider, relay, signed
release and cleanup observations plus the ordinary-kernel comparison are still
required. Focused fixture tests are not an executed Cloud or VM capture.

### Host and Cloud capture correlation

The Cloud capture retains the old release digest/source and the old identity
report timestamp. An old baseline observed after the rebuild request is invalid.
Compare the retained before/after host captures with that exact operation:

```sh
node apps/cli/scripts/path1-host-cloud-correlation.mjs \
  --before /Users/miguel/.codex/evidence/browser-computer-use/<campaign>/host-before.json \
  --after /Users/miguel/.codex/evidence/browser-computer-use/<campaign>/host-after.json \
  --cloud /Users/miguel/.codex/evidence/browser-computer-use/<campaign>/cloud-reimage.json \
  --output /Users/miguel/.codex/evidence/browser-computer-use/<campaign>/host-cloud-correlation.json
```

Inputs must be distinct bounded regular files. The new mode-0600 external
output binds their exact byte hashes and checks operation generation, capture
timing, host boot/machine/source identities, dedicated Path-1 service selection,
service invocation rotation and boot-bound process identities. It does not
declare signature verification, old-state absence or MP-10 acceptance.
Device/inode numbers can recur on rebuilt filesystems; matching or different
numbers alone do not establish residue-free retirement. Independent signed
release, provider, relay and complete residue/cleanup evidence still belong in
the full campaign. This command does not provision, rebuild or modify services.

### Remaining live observations

- Deployed Cloud API, database migration, provisioning, and Web behavior on a
  newly rebuilt worker; the Cloud MP-06 source path is inventoried above but
  has not been exercised end to end on that machine.
- Fresh host identity, installed/effective service units, image/release
  provenance, process ancestry/environment/mounts/privileges, `/home` and
  `/tmp` operations, Cloud/relay retirement, cleanup, and resource samples.
- Full Swift behavior and tests, live provider behavior, structured errors,
  runtime session/history parity, and every managed auto-stop scenario.
