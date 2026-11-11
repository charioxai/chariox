# OSS boundary plan: OSB-04 / 05 / 06 / 07 / 09 and the self-hosted UI

Status: design for Miguel, 2026-11-08, lane osbdesign r1 (MP-08 / MP-09 / MP-10 / MP-11). Design only: no code moved. The sequence below is a proposal for Miguel to confirm in one read (section 6 has the six yes/no decisions).

Measured on: OSS Landing 5 head `7906ae363e` (this worktree), Landing 6 union `c324604961` (`agent-landunion6b`, 37 commits ahead, 197 files), Cloud worktree `agent-landunion5-cloud` at `8709a4a6db` (the brief named `51a6a3bc1a`; `8709a4a6db` is the "Repin OSS to the landing 5 union r16 head" commit, and `51a6a3bc1a` is its merge base with the Landing 6 Cloud union). Audit input: `/root/.codex/evidence/osbaudit/r1/FINDINGS.md` (osbaudit r1).

Rule being enforced (Miguel 11-03, AGENTS.md rule 7): provider logic (names, limits, images, accounts, managed-environment lifecycle) lives only in Cloud; OSS is provider-neutral; an enterprise self-hosted setup is company-managed and must be able to replace our providers with a Cloud-side adapter and no OSS change.

## 0. Summary in ten lines

1. Product code in OSS has **no compute-provider names left** (0 hits for the guard's provider pattern outside tests/docs). The remaining leaks are Cloud *business vocabulary and flows*: reimage, compute class, auto-stop, release-update A/B, data-volume admission, the TUI rental chooser, and the hosted trigger ingress.
2. The guard is green only because of the 41-entry leak list (288 matching lines in 41 files) plus 11 path entries whose seam says "none; the path goes away" (the audit counted 5; there are 11).
3. The guard cannot see: `apps/server` (not in the path scan roots; and its Cloud path `/browser/...` is not in the path pattern), `apps/setup`, `deploy/`, `docker/`, `packages/aegs-sdk`, and the neutral-name debt (OSB-05: `managed-*` / `managed_*` words, hundreds of lines, no rule at all).
4. Ten slices, B0 to B8 (B4 is split in two). **B0 (guard + four script entries) is small, independent, and makes the guard stricter.** B1, B2, B4a, B5 and the UI step are independent of Landing 6 and can start now on the Landing 5 head.
5. B3 (remove the reimage local-API variants, plus any wire renames) is the only slice that changes a serialized shape, so it takes one protocol bump and **must wait for Landing 6** (Landing 6 takes local 1001 / relay peer 440 and edits the same three shared files).
6. B6 (kernel release-update removal) must be rolled out **after** the updater unit exists in Cloud (B5), or managed machines cannot update.
7. The OSS TUI must acquire machines through `LocalDaemonRequest::ControlPlane` (`Capabilities` / `Acquire` / `List` / `Start` / `Stop`), which already exists. No new OSS capability is needed. Presets, region/compute-class cycling and the "Auto-stop policy" row are deleted (the Cloud offer carries the terms).
8. Total estimate: about 90 to 155 engineer-hours across ten slices (table in section 4).
9. Self-hosted UI: the self-hosted web bundle (built from the Cloud repo, `apps/web`) carries 9 Cloud-only modules and the strings "Cloud machine", "Ask before agents rent", "staging.chariox", "No saved images". Cheapest first step is a build-time assertion (RED today), then host injection. Section 5.
10. Six decisions for Miguel: section 6.

## 1. The leak list

### 1.1 The guard, run as is

Command and output (this worktree, `7906ae363e`):

```
$ node scripts/oss-boundary-guard.mjs
oss-boundary-guard: ok
$ node --test scripts/oss-boundary-guard.test.mjs
# tests 15  # pass 15  # fail 0
```

Green does not mean the rows pass. What exempts the code: `scripts/oss-boundary-leaks.allow.json` (41 entries), `scripts/control-plane-paths.allow.json` (47 entries, 11 marked "none; the path goes away"), `scripts/control-plane-requests.allow.json` (12 entries, 2 marked Cloud-only), and `scripts/oss-boundary-files.allow.json` (28 whole files, justified as AI-provider billing text, upstream benchmark data, fixtures).

### 1.2 Exact method

The counts below use the guard's own functions (`providerNameFindings`, `vocabularyFindings`, `isCheckedPath`, `controlPlanePaths` from `scripts/oss-boundary-guard.mjs`) over `git ls-files`, so they are exactly what the guard sees, with Rust `#[cfg(test)]` items blanked. The scratch script lives in the lane directory, not in the repo. A raw cross-check with `git grep -c -i -P` over non-test, non-doc files (it also counts `ALLOWED_FILES` and Rust test blocks, so it is higher):

| pattern | raw `git grep` files / lines (non-test, non-doc) |
|---|---|
| `reimage` | 30 / 226 |
| `compute[-_ ]?class` | 13 / 38 |
| `auto[-_ ]?stop` | 12 / 61 |
| `(billing\|billable\|invoices?)` as a whole word | 23 / 52 (mostly `ALLOWED_FILES`: AI-provider billing) |
| provider names (`hetzner\|hcloud\|\bovh\|boat\.dev\|digitalocean\|linode\|vultr\|scaleway\|aws\|azure\|gcp`) in product code | **0** (test sample data only: `kernel_browser_host.rs:1493,1502`, `kernel_gateway/directory.rs`, `self_hosted.rs:208`, all inside `#[cfg(test)]`) |

The Hetzner leaks named in the 10-30 rule (`ManagedEnvironmentReimageProviderId::Hetzner`, volume limits in `state.rs`, `prepare-hetzner-image.sh`, `hetzner-acceptance.mjs`, drill presets) are **gone** from OSS at this head. What is left is the flows around them.

### 1.3 The 41 leak-list entries, grouped by slice and OSB row

Guard-visible matching lines in parentheses (`reimage` = R, `compute class` = C, `auto-stop` = A, `billing` = B).

**B0: scripts and package (4 entries, 7 lines; OSB-04/09 evidence scripts)**

| file | lines |
|---|---|
| `apps/cli/package.json` | 1 R (:130, a test-list token `cli-waiting-room-reimage-composition.bun-test.ts`) |
| `apps/cli/scripts/managed-parity-source-inventory.mjs` | 4 A (:355,360,368; inventory rows naming files that no longer exist) |
| `apps/cli/scripts/path1-host-cloud-correlation.mjs` | 1 R (:70) |
| `apps/cli/scripts/path1-provider-cloud-correlation.mjs` | 1 R (:11) |

**B2: OSB-09 + the TUI half of OSB-04 (20 entries, 203 lines)**

`apps/cli/src/`: `cli-app-root.tsx` (2 R), `cli-automation-snapshot.ts` (1 C), `cli-command-action-composition.ts` (3 R; also named for "neutral default-host source"), `cli-input-routing-composition.ts` (1 C), `cli-primary-transcript-composition.ts` (1 C), `cli-waiting-room-composition.ts` (31 R, 1 C), `managed-environment-api.ts` (21 R, 1 A, 1 C), `managed-environment-reimage-stop.ts` (17 R), `remote-machine-command-handlers.ts` (9 R), `waiting-room-controller.ts` (2 C), `waiting-room-focus-targets.ts` (3 A), `waiting-room-kernel-client.ts` (2 R), `waiting-room-launch-ownership.ts` (1 C, 1 A), `waiting-room-managed-environment-launch-controller.ts` (2 C), `waiting-room-managed-environment-reimage-controller.ts` (63 R), `waiting-room-managed-environments.ts` (16 C, 8 A), `waiting-room-start-rows.ts` (3 A, 2 C), `waiting-room-state.ts` (1 A), `waiting-room-types.ts` (3 C, 2 A), `waiting-room-value-cycling.ts` (3 A, 2 C).

**B3: OSB-04 reimage in the kernel and kernel-client (13 entries, 51 lines)**

`packages/kernel-client/src/ipc-managed-environment-requests.ts` (11 R, 5 A, 5 C, 1 B), `apps/kernel/src/local/api/types.rs` (6 R), `.../types/managed_environment.rs` (2 R), `.../types/request.rs:254` (1 R), `.../types/response.rs:264` (1 R), `apps/kernel/src/local/mod.rs` (2 R), `runtime/command/local_request_metadata.rs` (3 R), `runtime/external_response.rs:342` (1 R), `runtime/router/pre_lane_request.rs:36` (1 R), `runtime/session_membership/scope.rs:826` (1 R), `transport/relay_peer.rs:162` (1 R), `runtime/managed_bootstrap_observation_control.rs` (9 R), and (with B3) `managed_bootstrap/cloud.rs:139` (1 R), `managed_bootstrap/mod.rs` (21 R), `managed_bootstrap/supervisor.rs:1209` (1 R).

Note: the 3 `managed_bootstrap/*` entries total 23 lines and are reimage-observation code inside the bootstrap module; they leave with B3, and the rest of that module is B4a/B4b/B6/B7 work.

**Held by decision, not by a slice (2 entries, 4 lines)**

`apps/cli/src/deployed-workflow-command.ts` (3 B, :1260 `.../billing|maintainer>` usage text) and `deployed-workflow-types.ts` (1 B, :4). "billing" here is a Cloud *account role name* in `/cloud deployments` (`owner|admin|deployer|operator|viewer|billing|maintainer`). Miguel kept `/cloud deployments` in the TUI on 11-03 ("revisit"). Decision 6.

`packages/kernel-client/src/default-control-plane.ts:7` (`https://chariox.com`) is *not* a leak entry: the guard special-cases it as the one file naming the default connected service.

### 1.4 Path entries marked "none; the path goes away" (11), and who calls them

| path | caller | slice |
|---|---|---|
| `/managed-environments`, `/{}`, `/{}/context-transfer`, `/{}/lifecycle`, `/{}/reimage`, `/{}/reimage/preflight`, `/options` (7) | `apps/cli/src/managed-environment-api.ts` | B2 |
| `/managed-environments/{}/reimage/stop` | `apps/cli/src/managed-environment-reimage-stop.ts` | B2 |
| `/v1/managed-kernels/reimage/runtime-identity` | `apps/kernel/src/managed_bootstrap/cloud.rs:139` | B3 |
| `/v1/managed-kernels/release-update/poll` and `/artifact` | `apps/kernel/src/runtime/managed_release_update.rs:32-33` | B6 |

Request entries: `LocalDaemonRequest::ObserveManagedEnvironmentPreReimage` (Cloud-only, B3) and `ipc-managed-environment-requests.ts` (Cloud-only, B3).

### 1.5 What the guard does not see

| blind spot | evidence | slice |
|---|---|---|
| `apps/server` is not a path-scan root | `apps/server/src/hosted-agent-trigger-server.ts:32` builds `new URL("/browser/agent-trigger-deployments/${id}/admission", controlOrigin)`; neither the root nor the `/browser/` prefix is in `PATH_SCAN_ROOTS` / `PATH_LITERAL` | B0 detects, B1 removes |
| other unscanned roots call control-plane-shaped paths | `packages/aegs-sdk/src/server.rs` (16 literals: `/v1/subscriptions`, `/v1/owners/kernels`, `/v1/owners/kernels/revoke`), 6 drill scripts under `apps/cli/scripts` (`/relay/token`, `/kernels/presence`, `/v1/managed-kernels/context/ticket`, `/v1/owner-managed-context-tickets`), `scripts/e2e-stack/**` | B0 adds roots; classify each |
| neutral-name debt (OSB-05) is not a rule | `managed_bootstrap` / `managed-bootstrap`: 78 files, 441 lines; `ManagedEnvironment` / `managed_environment`: 60 files, 584 lines; `data_volume`: 25 files, 382 lines; `disposable.worker`: 28 files, 140 lines; `release-update`: 11 files, 54 lines; `managed-kernel`: 49 files, 205 lines (non-test, `git grep -c -i`) | B7 ratchet |
| deploy and docker files | `docker/agent-trigger-ingress/Dockerfile:14` runs `apps/server/dist/hosted-agent-trigger-main.js`; `deploy/managed-kernel/*` (38 files) | B1, B5 |
| test files, docs | exempt by design (sample data) | none |

## 2. Per-row slices

Common to every slice: delete the old path in the same change (AGENTS.md "Development stage"); no compatibility shim; the managed-upgrade predecessor list stays "current plus one before". A slice that touches only OSS removes its leak-list and allowlist entries in the same commit (the guard fails on stale entries).

### B0 (guard + scripts), first slice

- **What**: (a) guard rule: every leak-list entry needs `row` (one of OSB-04/05/06/07/09) and `slice` (`B0`..`B8` as numbered in this doc); an entry without them fails. (b) Add `apps/server/src/`, `apps/setup/`, `deploy/`, `docker/`, `packages/aegs-sdk/src/`, `scripts/` to the path-scan roots and `/browser/` to the path prefixes; every hit gets an allowlist entry with a seam or a slice (expected: 1 hit in `apps/server`, 16 in aegs-sdk, 6 drill scripts, 3 under `scripts/`; the aegs-sdk and drill literals are expected to be classified "generic seam"). (c) A count ratchet for neutral-name vocabulary (OSB-05) as one JSON file with the current totals; it may only go down. (d) Move or delete the four script entries: `path1-host-cloud-correlation.mjs` + `path1-provider-cloud-correlation.mjs` (and their `.test.mjs`) go to Cloud `deploy/providers/<provider>/drills/` (they correlate Cloud reimage captures); `managed-parity-source-inventory.mjs` rows that name deleted files (`runtime/managed_environment_control/cloud_contract.rs`) are rewritten or the script is retired (its `lib/managed-parity-*` record is pinned to commit `9334141d4` by its own test, which is why it is on `ALLOWED_FILES`); `apps/cli/package.json:130` drops the reimage token once B2 deletes that test (until then it stays, so that entry is the only one B0 may leave on the list with `slice: B2`).
- **To where**: Cloud `deploy/providers/hetzner/drills/` (it already holds `hetzner-preset.test.mjs`, `drills/`, `path1-rebuild-capture`) for the correlation scripts.
- **Interface staying in OSS**: none.
- **Protocol bump**: no.
- **Tests that move**: `path1-*-cloud-correlation.test.mjs` with the scripts; new guard tests: (1) a planted `/browser/x/admission` literal in `apps/server` must fail (RED today: passes), (2) a leak entry without `row`/`slice` fails, (3) ratchet above the recorded total fails.
- **Estimate**: 4 to 6 h. **Risk**: low. The only judgement is classifying the 16 aegs-sdk and 6 drill literals; unknown ones get a `slice` entry rather than silent approval.

### B1 OSB-07: hosted trigger ingress out of `apps/server`

- **What moves**: `apps/server/src/hosted-agent-trigger-{server,main,invoke}.ts` (+ `hosted-agent-trigger-server.test.ts`), `apps/server/HOSTED_AGENT_TRIGGERS.md`, `docker/agent-trigger-ingress/Dockerfile`, the `chariox-hosted-agent-trigger` bin and the test-list token in `apps/server/package.json`. 171 source lines + 98 test lines.
- **To where**: Cloud, a new small service (suggested `apps/agent-trigger-ingress/` next to `apps/api`; the Dockerfile moves to `deploy/production/`). Finding: **the Cloud route `/browser/agent-trigger-deployments/{id}/admission` does not exist at Cloud `8709a4a6db`** (grep over `apps`, `packages`, `deploy` finds no `agent-trigger-deployments`), so the Cloud side of the move must add that route (or the ingress is built against whatever Cloud owns today). This is the unknown in the estimate.
- **Stays in OSS**: the non-hosted trigger server `agent-trigger-{server,client,live,page,...}.ts` and `packages/agent-trigger-gateway` (generic; `hosted-agent-trigger-invoke.ts` imports from them, so Cloud depends on the published package, not a copy).
- **Interface**: none new. The OSS gateway package is the seam.
- **Protocol bump**: no (HTTP between the ingress and Cloud; no kernel/client/peer shape).
- **Tests that move**: `hosted-agent-trigger-server.test.ts`; `apps/server` `test` script drops it; a live drill: public ingress -> admission -> OSS trigger gateway -> kernel, run in the Cloud repo.
- **Estimate**: 6 to 10 h (3 h move + 3 to 7 h Cloud route and deploy). **Risk**: medium (Cloud route may need design with Cloud owners; ingress is public-facing).

### B2 OSB-09 (+ the TUI half of OSB-04): TUI acquires machines through ControlPlane

- **What is deleted** (20 leak entries, 3,120 source lines + 4,657 test lines): `apps/cli/src/managed-environment-api.ts`, `managed-environment-reimage-stop.ts`, `remote-machine-command-handlers.ts` (reimage command, `:50`), `waiting-room-managed-environments.ts`, `waiting-room-managed-environment-launch-controller.ts`, `waiting-room-managed-environment-reimage-controller.ts`, the presets and cycling in `waiting-room-value-cycling.ts`, the "Auto-stop policy" row (`waiting-room-start-rows.ts:24..66`), compute-class/region fields in `waiting-room-types.ts`/`waiting-room-state.ts`/`waiting-room-controller.ts`/`waiting-room-focus-targets.ts`, and the reimage parts of `cli-waiting-room-composition.ts` (31 occurrences), `cli-app-root.tsx`, `cli-command-action-composition.ts`, `cli-input-routing-composition.ts`, `cli-primary-transcript-composition.ts`, `cli-automation-snapshot.ts`.
- **What stays / what is added**: the waiting room gets one section driven by `LocalDaemonRequest::ControlPlane` through `packages/kernel-client/src/ipc-control-plane-requests.ts` (already exists):
  - `Capabilities {}` -> `ControlPlaneResponse::Capabilities { machine_allocation: Option<MachineAllocationCapability> }`; `None` means no machine row is shown (self-hosted).
  - `MachineAllocationCapability { revision, actions: Vec<String>, offer: Option<MachineAcquisitionOffer> }`; the offer carries `machine_type, vcpu, ram_gb, disk_gb, location` and `terms { amount, currency, period }` exactly as the service displays them. The TUI renders it verbatim; it never computes amounts and never offers a choice of class, region or stop policy.
  - `Acquire { capability_revision, request_id }`, `List {}` -> `Machines { machines: [ControlPlaneMachineRow], saved_states }`, `Start/Stop/Release/StartSavedState/DeleteSavedState { allocation_ref, capability_revision, request_id, expected_revision }`. A row's `available_actions: Vec<String>` is the whole menu; "reimage", if the service still offers it, is just another action string, so no kernel type is needed.
  - Context carry to a new machine uses the existing generic `StartManagedContextTransfer` / `GetManagedContextTransferStatus` / `GetManagedContextLaunchTarget` (allowlisted "generic seam"), replacing the TUI's direct `/managed-environments/{}/context-transfer`.
- **To where in Cloud**: nothing moves as code (the TUI rental chooser is deleted). The Cloud API behind it (`/managed-environments*`) is already Cloud's; the connected service answers `ControlPlane` requests.
- **Protocol bump**: **no** (the ControlPlane shapes already ship; B2 only deletes TUI code). It must land before B3 because the TUI still uses the pre-reimage observation builder.
- **Tests**: delete `waiting-room-managed-environments.test.ts` (871), `-launch-controller.test.ts` (647), `-reimage-controller.test.ts` (668), `managed-environment-api.test.ts` (265), `cli-waiting-room-reimage-composition.bun-test.ts` (1,264), `ipc-managed-environment-requests.test.ts`; add (1) a waiting-room test for the three states (no capability: no row; capability with offer: one row that shows the offer text and acquires with the offered revision; machines list with `available_actions`), (2) the real drill the audit said was NOT RUN: real TUI binary, real OSS kernel, one run signed-in with a Cloud control plane fixture and one self-hosted (no control plane): asserts the self-hosted run shows no machine row and makes zero Cloud requests.
- **Estimate**: 20 to 28 h. **Risk**: medium-high: largest deletion, and user-visible behaviour is removed (presets, auto-stop row), see decision 2. Mitigation: guard entries for the 20 files and the 8 `/managed-environments*` path entries disappear in the same commit, so the guard proves completeness.

### B3 OSB-04 (reimage in the kernel) and the only protocol bump

- **What**: delete `LocalDaemonRequest::ObserveManagedEnvironmentPreReimage` (`request.rs:254`) and its response (`response.rs:264`), `types/managed_environment.rs`, the `types.rs` re-exports (6), `local/mod.rs`, `local_request_metadata.rs`, `external_response.rs:342`, `pre_lane_request.rs:36`, `session_membership/scope.rs:826`, `relay_peer.rs:162`, `runtime/command.rs`, `transport/relay_client/daemon_requests.rs`, `runtime/managed_bootstrap_observation_control.rs` (97 lines), `managed_bootstrap/mod.rs` (`PreReimageObservationAcknowledgement`, `report_pre_reimage_runtime_identity`, `observe_managed_environment_pre_reimage`, 21 occurrences), `managed_bootstrap/cloud.rs:139` (`/v1/managed-kernels/reimage/runtime-identity`), `supervisor.rs:1209`, kernel-client `ipc-managed-environment-requests.ts` (+test).
- **To where**: Cloud keeps its own reimage flow; the pre-reimage identity report is replaced by what the Cloud host agent already knows. If Cloud still needs the kernel's runtime identity before a reimage, it reads it through the existing generic `ControlPlane` machine row or the activity/quiescence seam. Nothing is ported into OSS.
- **Interface staying in OSS**: `ControlPlaneRequest` as in B2; quiescence and activity paths (`/v1/managed-kernels/quiescence/poll|ack|release-ack`, `/activity`).
- **Protocol bump**: **yes**. A client-visible request variant and response variant disappear: bump `LOCAL_DAEMON_PROTOCOL_VERSION` (`types.rs:495`, now 1000; Landing 6 makes it 1001) and `RELAY_PEER_PROTOCOL_VERSION` (`relay_peer.rs:198`, now 439; Landing 6: 440) because `relay_peer.rs` carries the request; update `apps/kernel/managed-upgrade-protocol-transitions.json`, `scripts/lib/protocol-versions.mjs`, the protocol snapshot/hash tests and `local/api/tests/protocol_shapes/managed_environment.rs`; raise no client minimum (nothing needs new behaviour). **Fold the wire renames of B7 (`*ManagedContext*` request names) into this one bump** so the bump is paid once.
- **Tests that move/change**: protocol snapshot + hash tests; `protocol_shapes/managed_environment.rs` deleted; focused drill: a client sending the removed variant gets a clear "unknown request" refusal, and `pnpm protocol:conformance`.
- **Estimate**: 10 to 14 h (+6 h if the wire renames ride along). **Risk**: medium: shared files with Landing 6 and with every in-flight peer-protocol branch; nothing else.

### B4a OSB-04 (data-volume binding), independent

- **What**: delete `managed_bootstrap/data_volume_observation.rs` (280 + 240 test lines), the `expected_data_volume_serial/size_gb` fields (`state.rs:60,62`, used at `:266,:319`), the data-volume arm of `freshness.rs` (`read_admitted_data_volume`, `:106..131`), and the fields from the bootstrap document.
- **To where**: the Cloud host agent verifies the data volume before it starts the kernel (a root unit on the image under `deploy/providers/<provider>/`); the kernel is told only "start". No new OSS interface.
- **Protocol bump**: **no** client/peer shape. The persisted bootstrap state and the Cloud-issued bootstrap document lose fields; per the development-stage rule machines are re-provisioned, no read path for the old state. Needs the matching Cloud change to stop sending the fields (`managed-kernel-cloud-init.ts`).
- **Tests that move**: `data_volume_observation_tests.rs` (240) and the data-volume cases in `managed_bootstrap/tests.rs`; add the Cloud-side check-script test.
- **Estimate**: 6 to 9 h. **Risk**: medium: boot path of every managed machine; verify with a real managed boot before landing.

### B4b OSB-04 (disposable-worker receipt), after Landing 6

- **What**: delete `managed_bootstrap/worker.rs` (1,148 lines: `CHARIOX_DISPOSABLE_WORKER_RECEIPT`, `activity_allocation`, `confirmed_disposable_worker_home_caller`; its own header says "nothing in this repository writes it"), the `ACTIVITY_RECEIPT_ENV` scrub in `supervisor.rs:286`, `LeaseWorkerHomeCaller` (`config.rs:79,162`), and the home-caller plumbing in `runtime/router/managed_context_bridge.rs` (7 sites, e.g. `:101,:242,:968`), `runtime/router/relay_peer_bridge.rs:72-81` and `managed_context/cloud_completion.rs:84`.
- **Why it waits**: the receipt gates managed-context import on a remote lease worker, so it sits inside the kernel-to-kernel context path. `relay_peer_bridge.rs` is edited by Landing 6 (a 15-line block at :38) and by p5b. The hunks are 34 lines apart, so the merge is probably clean, but the file is shared; start it from the Landing 6 head.
- **Interface staying in OSS**: the context-transfer requests that are already allowlisted "generic seam" (`StartManagedContextTransfer`, `GetManagedContextTransferStatus`, `GetManagedContextLaunchTarget`). What the receipt proved (this machine is the worker Cloud allocated) has to be carried by the allocation identity from `/v1/managed-kernels/bootstrap/exchange|confirm`; confirm that by reading the callers first; if a gap is found, record it as an open item in the slice, do not invent a new interface here.
- **Protocol bump**: no client/peer shape if the home-caller argument stays internal (to be checked against the `managed_context_bridge.rs` signatures; if a request field goes away, it rides on B3's bump).
- **Tests that move**: the `worker.rs` cases in `managed_bootstrap/tests.rs`; keep a managed-context import test with no receipt.
- **Estimate**: 8 to 12 h. **Risk**: medium-high: managed-context transfer is a security-sensitive path (relay admission, owner authority); needs the real two-kernel context-transfer drill.

### B5 OSB-06: `deploy/managed-kernel/` split

- **What moves** (Cloud `deploy/providers/<common or per provider>/`): `install-image.sh` (663), `upgrade-image.sh` (1,110), `managed-kernel-upgrade-state.mjs`, `managed-kernel-recovery-release.sh`, `managed-kernel-builder-pin-transaction.sh`, `managed-kernel-home-migration.mjs`, `release-update-storage.py`, `chariox-vm-bootstrap.service`, `chariox-path1-managed-bootstrap.service`, `managed-app-release.sh`, `path1-service-policy.mjs`, `apps-rollback-state.py`, `managed-release-activation.test.mjs`, `managed-update-recovery.test.mjs`, `release-update-storage.test.py`, plus `verify-image-release*` if only image builds use them (38 files in the directory, 5,747 source + 1,907 test lines). Cloud `apps/infrastructure-manager/src/managed-kernel-cloud-init.ts:170` names `chariox-path1-managed-bootstrap.service` from the OSS tree; it will point at the Cloud copy.
- **What stays in OSS, renamed to a neutral directory (`deploy/worker-kernel/`)**: the neutral installer part the audit named (`boot-release.mjs` + test, `extract-release.py` + test, `chariox-rootless-docker.service`, `chariox-slice-broker.service`, `chariox-docker-admission-locks.service`, `docker-admission-install.sh`, `remove-stale-rootful-docker-socket.sh`, `verify-rootless-handle-lifecycle.sh`, `verify-slice-publication-access.sh`, `verify-provider-runtime-bind.sh`, `provider-versions.env`, `native-build-resources.md`), because a company-managed machine needs them to install the kernel.
- **Callers to repoint** (found by `git grep deploy/managed-kernel`): `.github/workflows/ci.yml` and `apps-components.yml`, root `package.json`, `apps/setup/*` tests and fixture, `apps/kernel/slice-linux-docker/{Dockerfile,runtime-source-roots.txt}`, `docker/publication/Dockerfile`, `apps/kernel/managed-upgrade-protocol-transitions.test.mjs`, `managed-ordinary-parity-collector.mjs`, `apps/kernel/src/managed_bootstrap/tests.rs`, `provider/claude/usage_probe.rs`, and `runtime/managed_release_update.rs` (comments and paths; removed in B6).
- **Protocol bump**: no. **Tests**: shell/mjs tests move with the files; a Cloud-side image build and boot drill is the real acceptance.
- **Estimate**: 8 to 12 h (mostly moves and CI). **Risk**: medium: CI and the signed release path; the kernel still names `/usr/lib/chariox/...` paths until B6, so the OSS half must keep producing the same on-image layout.

### B6 OSB-04 (release-update A/B leaves the kernel)

- **What**: delete `runtime/managed_release_update.rs` (678) and `managed_release_update_evidence.rs` (464), `managed_bootstrap/release.rs` (701; the signed-release verifier), the release-evidence arm of `freshness.rs` (1,019), `state/managed_kernel_quiescence_state.rs:558` `spawn_managed_release_update` and its call at `runtime_transport.rs:755`, the two path entries (`release-update/poll|artifact`).
- **To where**: the release poll, artifact download, signature verification and swap become a root-level updater unit shipped by Cloud (B5's `upgrade-image.sh` already runs the transaction; B6 moves the *poller* out of the kernel into that unit). The kernel keeps exactly the generic quiescence seam it has today (`/v1/managed-kernels/quiescence/poll|ack|release-ack`): the updater asks the kernel to quiesce, waits for the ack, swaps the release, releases. A company-managed machine can supply its own updater without an OSS change.
- **Protocol bump**: no client/peer shape. The managed-upgrade predecessor list is untouched.
- **Rollout order (hard)**: B5 first (updater exists in Cloud), then the Cloud release of the updater on managed images, then B6. A kernel without the poller on an image without the updater cannot update itself; with the dev-stage rule this means re-provisioning the managed machines that predate the updater, which Miguel's rule allows.
- **Tests that move**: `managed-release-activation.test.mjs`, `managed-update-recovery.test.mjs`, `release-update-storage.test.py`, `scripts/managed-kernel-upgrade.test.mjs`, kernel `release_update` evidence tests -> Cloud; keep a kernel test that quiescence ack/release still works with no poller.
- **Estimate**: 16 to 24 h. **Risk**: **high**: a failed update is a bricked managed machine. Needs the real-machine drill (rent, update A to B, forced rollback) before landing.

### B7 OSB-05: neutral names

- **What**: HTTP paths `/v1/managed-kernels/*` -> `/v1/worker-kernels/*` (activity, quiescence, bootstrap, context), module/crate names `managed_bootstrap` -> `worker_enrollment`, `managed_kernel_activity|quiescence` -> `worker_activity|quiescence`, `ManagedContext*` requests -> `ContextTransfer*`, constants and env names that contain `managed` (e.g. `CHARIOX_DISPOSABLE_WORKER_RECEIPT` is gone with B4b). `ManagedEnvironment*` types disappear in B3 (deleted, not renamed). The 11 "path goes away" entries are already gone after B2/B3/B6; the other `/v1/managed-kernels/*` and `/v1/owner-managed-context-tickets*` entries are re-keyed.
- **To where in Cloud**: Cloud serves the new paths (`apps/api`, `apps/worker`). No alias window (dev-stage rule): OSS and Cloud land together; managed machines older than "current and previous" are re-provisioned.
- **Protocol bump**: yes for the `ManagedContext*` request renames (ride on B3's bump); HTTP path renames are not a client/peer shape.
- **Tests**: the guard ratchet from B0 goes to zero; `control-plane-paths.allow.json` keys change; contract tests on both sides.
- **Estimate**: 10 to 16 h (mechanical, but 78+60 files). **Risk**: medium: cross-repo, must land in lockstep. Do it last so it renames the smaller tree.

### B8 self-hosted UI: see section 5 (Cloud repo)

## 3. Conflict surface with Landing 6

Computed with `git -C /root/work/agent-landunion6b diff --name-only 7906ae363e..c324604961` (197 files, 37 commits; the union's merge base is the Landing 5 head), `git diff --name-only $(git merge-base HEAD 7906ae363e)..HEAD` for the peers `agent-p5b` (140 files, 44 commits) and `agent-treeevent` (83 files, 16 commits) (204 distinct files in total), each intersected with the slice file sets in section 2.

| slice | files in slice | files also changed by Landing 6 / p5b / treeevent | verdict |
|---|---|---|---|
| B0 guard + scripts | 11 | none | **independent: start now** |
| B1 hosted trigger | 6 | none | **independent: start now** |
| B2 TUI | 21 (+ the cli composition files) | none (Landing 6 and its peers touch 14 files under `apps/cli/src`: `agent-command-table`, `app-command-*`, `cli-sub-agents-composition`, `cli-user-app-views-composition`, `kernel-event-dispatch-controller`, `prompt-attachment-intake-controller`, `user-app-view-*`; none is a leak-list file) | **independent: start now** |
| B3 reimage API + bump | 2 own + 11 shared | `apps/kernel/src/local/api/types.rs` (hunk at :495, the protocol constant), `runtime/external_response.rs` (:132), `transport/relay_peer.rs` (:6, :202, :868, :1394: the peer 440 request/response), `managed-upgrade-protocol-transitions.json`, `scripts/lib/protocol-versions.mjs`, protocol hash tests | **must wait for Landing 6**: same constants, and Landing 6 owns 1001/440 |
| B4a data volume | 2 | none | **independent: start now**, land any time (no shape change) |
| B4b disposable-worker receipt | 4 | `runtime/router/relay_peer_bridge.rs` (Landing 6 :38, p5b) | start from the Landing 6 head |
| B5 deploy split | 38 | none (`managed-upgrade-protocol-transitions.*` and `scripts/managed-kernel-upgrade.test.mjs` are touched by Landing 6: do not move these two in B5) | **independent: start now**, minus those two files |
| B6 release-update | 12 | `runtime_transport.rs` (Landing 6 edits :40, B6 edits :755: no overlap), `managed-upgrade-protocol-transitions.json` and `.test.mjs` only if edited | independent by file; **ordered after B5 for rollout**, not for conflicts |
| B7 renames | ~190 | `relay_peer.rs`, `types.rs` (wire names), many `managed_*` | **wait for Landing 6**, after B3 |
| B8 UI (Cloud repo) | Cloud `apps/web` | Landing 6's Cloud union (`agent-landunion6b-cloud`, 17 files vs `8709a4a6db`) edits `kernel/*`, `terminal-os/room-*`, `sub-agent-feed*`, pins; not `selfhosted-bundle.ts`, `terminal-os/machines/*` or `settings-window.tsx` | independent (Cloud repo) |

Rebase cost of B3 on Landing 6: the shared hunks are the protocol constants (`types.rs:495`, `relay_peer.rs:198`), the transitions file and the new peer request/response blocks in `relay_peer.rs`, so it is a small mechanical conflict, but the protocol numbers are Landing 6's to give.

## 4. Recommended order, estimates, and the guard rule

Order, with the reason for each position:

| # | slice | rows | start | hours | bump | why here |
|---|---|---|---|---|---|---|
| 1 | **B0** guard + 4 script entries | all | now | 4 to 6 | no | small, independent, shrinks the leak list by 3 to 4 and makes `apps/server` visible; every later slice is proved by it |
| 2 | B1 hosted trigger ingress | OSB-07 | now (parallel) | 6 to 10 | no | independent, Cloud route needs design lead time |
| 3 | B2 TUI on ControlPlane | OSB-09, 04, 05 | now (parallel) | 20 to 28 | no | removes 20 leak entries and 8 path entries, no protocol change; unblocks B3 |
| 4 | B4a data volume | OSB-04 | now (parallel) | 6 to 9 | no | kernel-internal, no shared files |
| 5 | B5 deploy split | OSB-06 | now (parallel) | 8 to 12 | no | prerequisite of B6 |
| 6 | B3 reimage API + bump (+ wire renames) | OSB-04, 05 | after Landing 6 | 10 to 20 | **yes, once** | the only shape change; ride on Landing 6's numbers |
| 7 | B6 release-update out of kernel | OSB-04 | after B5 and the Cloud updater release | 16 to 24 | no | rollout-ordered, high risk |
| 8 | B4b disposable-worker receipt | OSB-04 | after Landing 6 | 8 to 12 | no (maybe B3's) | shares `relay_peer_bridge.rs` with Landing 6 |
| 9 | B7 neutral names | OSB-05 | after B3 | 10 to 16 | with B3 | last, smallest tree |
| 10 | B8 UI | UI | now (Cloud repo) | 4 to 6 first step; 12 to 16 full | no | independent |

Total: 92 to 153 h (sum of the table; the large uncertainty is B1's Cloud route, B2's TUI drill and B6's real-machine drill). Parallelism: B0 first by one person, then B1, B2, B4a, B5 can run as four parallel branches on the Landing 5 head (only one kernel build at a time per machine, per AGENTS.md).

### The guard rule (B0)

> The guard fails on any leak-list entry that is not on the "to be moved" list with an owner row. Concretely, `scripts/oss-boundary-leaks.allow.json` entries must be `{ path, row, slice, removedBy }` where `row` is one of `OSB-04|05|06|07|09` and `slice` is one of this document's `B0`..`B8`; an entry with a missing or unknown `row`/`slice` fails with "needs an owner row". New entries cannot be added without naming a row and a slice that exists in `docs/design/OSS_BOUNDARY_PLAN.md`, and reviewers reject additions (AGENTS.md rule 7). `apps/server/src/`, `apps/setup/`, `deploy/`, `docker/`, `packages/aegs-sdk/src/` and `scripts/` become path-scan roots, and `/browser/` is added to the path prefixes. The neutral-name vocabulary (`managed-kernel`, `managed_bootstrap`, `ManagedEnvironment`, `data_volume`, `disposable worker`, `release-update`) gets a total-count ratchet file that may only go down. The same "stale entry fails" rule already applies to each list, so landing a slice without removing its entries fails.

First-slice red tests (fail on the Landing 5 head, pass after B0): a planted `/browser/x/admission` literal under `apps/server/src/` (currently passes), a leak entry without `row`/`slice`, a ratchet total one above the recorded one.

## 5. Self-hosted UI observations

The web client is the Cloud repo's `apps/web` (OSS has no web app), so this work lives in Cloud. Self-hosted = `CHARIOX_WEB_CONTROL_PLANE=selfhosted`, entry `src/selfhosted-client.ts`, built by `src/selfhosted-bundle.ts`.

**Measured** (esbuild metafile over `selfhosted-client.ts`, Cloud `8709a4a6db`, scratch script in the lane directory): bundle 2,644,128 bytes, 7,205 inputs. It contains these Cloud-only modules: `terminal-os/machines/{rent-view,launch-form,images-table,run-on-menu,start-from-menu}.tsx`, `machine-replacements.ts`, `use-cloud-room-opener.ts`, `use-launch-environment.ts`, `cloud-launch.tsx` (9 modules, 31,692 source bytes). Strings present in the shipped self-hosted script: "Cloud machine" x4, "Ask before agents rent" x2, "staging.chariox" x1, "No saved images" x1, "Rent" x6. The existing build self-check (`selfhosted-bundle.ts:18`) only rejects `terminal/api.*` and `control-plane-cloud/` inputs, so it passes today. At runtime the Cloud surfaces are hidden when no `MachinesHost` is passed (`terminal-os/index.tsx:211,277`, `settings-window.tsx:36,117`), the sidebar shows Deployments, Machines and Slices, and the Machines window has an Images tab.

| surface | belongs to | action |
|---|---|---|
| Rent view, launch form, run-on / start-from menus, machine replacements, `cloud-launch`, "Ask before agents rent machines" setting | Cloud | build-time exclusion |
| Images tab + `images-table.tsx` ("No saved images") | Cloud image concept | build-time exclusion; the Machines window keeps the process view |
| "Deployments" sidebar row | Cloud `/cloud deployments` | hide when the control plane reports no deployments capability (today: always shown) |
| Machines, Slices, Settings -> Machines & kernels | OSS (own kernels, kernel-local slices) | keep, without Cloud actions |
| `staging.chariox.com` literal | Cloud default host | ends up in the bundle through the shared default; only `default-control-plane.ts` may name it (guard) |

**Method: build-time exclusion, not runtime hiding.** Runtime hiding keeps code, strings and attack surface in a bundle that claims to be Cloud-free, and a company self-host cannot audit it. The seam is `MachinesHost` (already injected by the Cloud entry): move the 9 modules behind `platform/cloud-machines-host.ts` (the Cloud entry's host; check that `selfhosted-client.ts` never imports it), and let `terminal-os` use only the host's interface.

**Cheapest first step (about 3 h, UI-1)**: widen the regex in `selfhosted-bundle.ts:18` to forbid the 9 modules and the four strings in the minified output. It is RED immediately (fails the build today), which proves the leak; then fix by host injection until it is green. Also gate the Deployments row on a capability (second step, 1 to 2 h). No new UI is invented: this only removes things the prototype's self-hosted shell should not have.

## 6. Decisions for Miguel (yes/no, default in brackets)

1. Confirm the order B0 -> (B1, B2, B4a, B5 in parallel, now) -> B3, B4b, B7 after Landing 6 -> B6 after the Cloud updater ships, with exactly one protocol bump (B3)? [yes]
2. The TUI acquires machines only through the ControlPlane offer: delete the presets, region/compute-class cycling and the "Auto-stop policy" row (Cloud decides the terms and stop policy)? [yes]
3. A reimage becomes just another `available_actions` string on a machine row; no kernel request or type for it (B3)? [yes]
4. The release-update poller leaves the kernel for a host-level updater unit shipped by Cloud, and managed machines older than the updater are re-provisioned, no compatibility window? [yes]
5. Hosted trigger ingress moves to a Cloud service (the Cloud admission route does not exist yet at `8709a4a6db` and must be built there)? [yes]
6. Keep `/cloud deployments` in the TUI (with its "billing" role word, 2 held leak entries) until the later revisit, and hide the "Deployments" sidebar row in the self-hosted web shell? [yes, keep in TUI; hide in self-hosted web]
