# Managed Path-1 parity inventory (MP-11)

Audit date: 2026-09-24 · OSS source baseline: `294ff610d03ddf57a367334a4da29bfad8e106cf`; Cloud source baseline: `73d82d3d3b578cb3da54dbb5a58dfcffd083b58e`.

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

## Reconciled source inventory

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
