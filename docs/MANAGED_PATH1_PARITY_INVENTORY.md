# Managed Path-1 parity inventory (MP-11)

Historical audit ledger (2026-09-26) · scoped OSS review baseline: `d3f47513bda80ea222b6cd7e4d1e6b9d106038b9` (tree `08b8cd11bd31a69c3ed0d9b7b2066b6cb07196fd`); published OSS source baseline: `4c8b979430d2dca6662de0b478a8b75b7ac3b231`; retained prior audit OSS baseline: `dbfebe394707c7b5c85a2ee02aa5999e5e9e44b4`; Cloud source baseline last inspected: `73d82d3d3b578cb3da54dbb5a58dfcffd083b58e` (stale; not refreshed in this pass).

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

## Scoped source audit (2026-09-30)

This pass inspected selected production rules at OSS
`0e250d53977a49a73c3c3ee0f9251d9f9febe712` (tree
`2eb9c83eea61354e2b138a0f915cb63021f8ca18`) and Cloud
`a8f5ee1bc80f13f508cf950752d779399df1aa55` (tree
`a19f0ba226f49e170d9b01e122a0eac68918ac8c`). These are pinned inspection
inputs, not a claim that evolving integration heads or release artifacts
remain equivalent. The retained d3 rows below describe their historical source
only. Cloud later published `97893503c70af4997444bd1172bf46374b283f0a`
(tree `14bfdd80b6ed543b02fdadef5c1b989e4c8be6f8`) with the namespace
correction. Its combined OSS/Cloud semantics remain **pending revalidation**;
the a8 rules and defect observations below are not current-aggregate approval.

The scanner's named source-rule module binds every provisional observation to
an exact Git blob. Partial-file observations also bind inspected line ranges:
unrelated functions and inline tests receive no classification from that rule.
Changed blobs report `source_drift`. Declaration candidates cover policy
helpers and closed data shapes that the lexical selector patterns miss.
These observations never grant independent semantic disposition.

| Inspected semantic family | Rules / candidates | Concrete ordinary-versus-managed classification |
| --- | --- | --- |
| Release evidence, attempt storage and bounded extraction | 3 / 14 | Signed deployment admission: identity/digest/terminal outcome binding, exact owned attempt cleanup, bounded safe archive staging. This is the permitted deployment difference; it does not select provider runtime. |
| OSS account pairing and terminal authority | 3 / 4 | Shared Cloud account/session or machine-qualified terminal capability. No managed-profile exception or durable profile mutation is introduced by these helpers. |
| Cloud pairing control, explicit revoke and logout receipt | 4 / 7 | Shared Bearer/account operate admission; exact identity-bound ACTIVE session cleanup; receipt acknowledges only an already committed matching explicit revoke. Browser-only/unrelated sessions remain distinct. |
| Cloud machine credential, token and presence authority | 4 / 12 | Shared exact account/machine/realm/canonical-target ownership. Inner-slice subject mapping has the open collision described below; this family is not cleared for acceptance. |
| OpenShip patch0016 publication controls and co-location proof | 2 / 14 | `strict-publication-v1` selects a separate publication container's network, read-only binds and HostConfig; co-location HMAC is control-plane health proof. Neither selects the Path-1 host provider service. |
| Official provider adapters, ordinary scrub and bounded probes | 4 / 60 | Common official harness planners and cwd preflight; ordinary/Path-1 branch returns the provider program unchanged after common control-env scrub. Utility capture has shared deadline/output/read budgets. Shared-host namespace internals remain outside this scoped observation. |
| Directory discovery and exact control-state protection | 2 / 26 | Common canonical cwd/repository preflight protects kernel state and five slice-control children; slice root/development/siblings remain eligible. Explicit server repository root is a discovery input, not a provider allowlist. |
| Bootstrap topology, verified kernel launch and Path-1 unit source | 5 / 72 | Explicit deployment topology; verified kernel uses ordinary process HOME/login PATH and no provider isolation roots. Private broker handoff is a kernel slice capability. Source units have no provider `Protect*`/`Private*`/namespace restrictions; installed units/drop-ins remain unverified. |
| Waiting-room placement | 1 / 8 | Deployment-control projection of exact server machine/kernel readiness and revisions. Ready placement selects the ordinary canonical kernel; no prompt/session authority fork is added. |
| Bootstrap machine profile and persistence | 4 / 15 | Strict exchange shape has no CloudSession/operator token. Conversion clears client identity/session token/expiries; persistence replaces the profile and clears previous direct relay token/url. Worker exchange/restart validates receipt identity binding. |
| Selected provider/Extension context export | 5 / 8 | Official provider auth-file whitelist and closed Extension/dependency payload exclude automatic daemon Cloud-profile export. SourceKernel deliberately transfers an encrypted user vault; arbitrary user-authored secret contents were not inspected or certified. |
| Historical shared slice canonical-identity defect at 0e/a8 | 2 / 3 | Local `slice:<name>` defaults were used as account/realm-wide Cloud subjects; separate ordinary or managed machines with the same local name contended for one immutable owner. Cloud 978 changes admission; combined identity mapping and parity evidence remain pending. |

The actual scans completed: OSS enumerated 3,598 files and 5,958 candidates;
Cloud enumerated 2,708 files and 2,848 candidates. The 39 scoped rules classify
243 candidates provisionally; 8,563 candidates remain outside this inspection.
All 8,806 candidates still have `semanticDisposition=unreviewed`, and both
reports exit 1 with `status=fail`. Each report retains three historical
predicates pending source review. Zero `removal_required` means no accepted
independent findings were recorded; it is not acceptance.

Cloud unified patches, Caddy/SQL files and assembled TypeScript/JavaScript
fragments are now inventoried explicitly. Added/removed/context lines retain
physical patch and embedded source identities. A patch hunk can begin inside
a literal/comment whose opener is omitted, so partial patch source retains
unstripped lexical candidates with `unknown_patch_fragment` context.
Apparent comments are not silently omitted or treated as production findings.
Exact inspected assembler/caller blobs reconstruct four Cloud fragment families
with sorted `join("")` semantics before lexical processing, preserving physical
path/blob/line/column and split-match segments. Unknown/changed consumers and
missing expected declarations produce explicit unresolved source-audit gaps.
Both actual pinned reports have zero such gaps; this does not review their
semantic candidates.
The tool identity binds the entrypoint, parser, fragment-assembly and source-rule module hashes;
this scan used bundle SHA-256
`a508912ea82d5c676d41d1ecfd6827efb155792b5b989fbe7de89e1710a9b543`.

Focused scanner execution passes 49/49, including exact-range/test separation,
generic helpers, data-shape candidates, source drift, patch gaps and unknown
starting template context. Both added and removed patch views reproduced the
independent inter-hunk finding RED before correction. Complete synthetic old
and new JavaScript also proved the same-hunk template selectors were executable.
Retained external evidence is under
`/Users/miguel/.codex/evidence/browser-computer-use/resume-20260930/mp11-source-audit/`;
full pinned scan reports remain under the matching Hetzner evidence directory.
No Cargo build, Cloud image build, live host inspection or fresh-machine
acceptance was performed by this source audit.

The historical slice gap at these pinned inputs spans `SliceStore::create` in `slice/store.rs`,
`hosted_cloud_slice_relay_token` and
`activate_hosted_slice_relay_token` in
`runtime/slice_command_executor/lifecycle.rs`, and
`authorizeMachineRuntimeToken`/`claimKernelOwnership` in Cloud
`runtime/machine-scope-repository.ts`. Cloud 978 publishes canonical machine-qualified admission and owned-parent
tests; OSS mapping, persisted-ref compatibility and relay projection still need
the exact combined source/test checkpoint and reviewed A/B release pair.
This inventory does not claim the historical Cloud finding remains unfixed at
Cloud 978 or that the full parity gap has been closed. Old e1/b375 artifacts remain historical and cannot
automatically serve as final parity baselines.

Uninspected work includes the remaining shared-host/provider namespace body,
runtime retry/relaunch/prompt/permission/history flows, broader worktree
operations, current Cloud managed lifecycle/shutdown, and web/native client
behavior outside these named regions. The imported user vault is an explicit
opaque secret boundary. Effective service/drop-in policy, provider ancestry,
runtime resources, shutdown triggers, and fresh ordinary-versus-Path-1
comparison remain live gates. Independent semantic disposition and MP-01
through MP-11 remain open.

## Retained scoped OSS source audit (2026-09-26)

This section carries forward the source-family audit recorded in commit
`b221f2c626df8c55a07003cd491b0bdce58e5220` and its follow-up
`bcfa97e6953ece7086d15a23bd6047418857b51e`. That audit used OSS baseline
`dbfebe394707c7b5c85a2ee02aa5999e5e9e44b4`; other source families were not
re-audited at the published baseline above. Cloud findings retain their older
baseline and are stale. No MP gate is closed by this source inventory.
The current `d3f47513bda80ea222b6cd7e4d1e6b9d106038b9` follow-up re-read the
locked MP-01 through MP-11 ledger and refreshed only the Path-1 bootstrap,
kernel broker/provider environment boundary, release exporter/installer, and
OSS auto-stop policy projection described below. Rows marked `Current d3` are
source inspection at that exact aggregate; unmarked rows retain their earlier
audit baseline. Cloud behavior remains stale and uninspected in this pass.
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
   `0832289de8feaf3ba30068c378d66cec965b6900`. The two Path-1 role units set a
   system-only bootstrap PATH (`chariox-path1-managed-bootstrap.service:25`,
   `chariox-disposable-worker-bootstrap.service:30`); they do not prepend
   `/home/chariox/.local/bin`. After release verification, the supervisor and
   worker probe the process home's login profile in an `env_clear` subprocess
   and retain only validated absolute PATH components
   (`managed_bootstrap/provider_path.rs`). A failed probe falls back to the
   system bootstrap PATH for the home kernel and the absolute
   `<process_home>/.local/bin` followed by the system bootstrap PATH for the
   worker. Empty, relative and control-containing profile entries are
   discarded. Codex, Claude and OpenCode resolve commands in PATH order
   (`provider/codex.rs:164-176`, `provider/claude.rs:508-524`,
   `provider/opencode.rs:163-176`); image setup places pinned binaries under
   `/usr/local/bin` (`prepare-hetzner-image.sh:197-199`). Root's 17/17 focused
   Node source-contract checks do not execute Rust resolver behavior. The
   Rust resolver tests cover timeout cleanup of a background stdout writer
   and oversized output; actual effective units and resolved binaries on the
   fresh worker and deployed parity remain unproven.

3. The d3 source includes the Project setup persistence correction, so the
   earlier “not integrated” status is stale. Utility-generated repeatable
   definitions are persisted under the active attempt before target validation;
   a leased worker waits for a home acknowledgment bound to attempt, lease,
   project, and definition digest before it validates or reports Ready
   (`runtime/state/project_environment_setup.rs:1450-1529,1701-1733`; `project_environment_setup_ack.rs:466-555`). The home/worker path keeps cancellation and retry fencing, and rejects an acknowledgment that does not match the active attempt. Source regressions include local generated-definition persistence before validation (`project_environment_setup_lifecycle_tests.rs:128-132,1000-1017`), the remote status/repair/ack path (`project_environment_setup_lifecycle_tests.rs:1567-1588,2287-2307`), and acknowledgment binding/cancel/retry tests (`project_environment_setup_ack.rs:653-915`). These Rust tests were inspected, not executed in this source-only pass. Runtime behavior on a fresh worker and ordinary-versus-Path-1 Project parity remain open MP-08 evidence.

4. Path-1 kernel slice access uses a broker lease, separate from the ordinary
   provider launch boundary. In `supervisor.rs:301-317,451-516`, Path 1 drops
   inherited sandbox and broker values, derives `CHARIOX_SLICE_ROOT` only from
   the configured absolute broker socket, and gives the kernel a private
   `CHARIOX_SLICE_DOCKER_BROKER_FD`; without a lease it removes socket/FD values
   and sets the required marker. The worker uses this supervisor helper at
   `worker.rs:575`. Kernel initialization in `slice/local_docker/broker.rs:119-197`
   consumes and scrubs the transport variables, makes the stream
   close-on-exec, and configures the private connection. Provider control-env
   removal in `provider/managed_isolation.rs:110-143,168-201,908-936` excludes
   the socket, FD, required marker, and slice root from ordinary and isolated
   provider commands. Linux Rust regressions at `supervisor.rs:713-1010,1366-1449`
   and `worker.rs:1056-1293` inspect a framed round trip, the derived root,
   required-marker fallback, close-on-exec, and provider-child non-inheritance
   through the production launch seam. They were inspected but not executed in
   this source-only pass. This is a kernel slice capability, not a provider
   Bubblewrap/allowlist branch. Live slice access, effective protected-control
   paths, and fresh-worker behavior remain open MP-01/03/08/11 evidence.

### Source branch inventory

| Family | Source classification and remaining evidence |
| --- | --- |
| **Current d3** · Topology and ordinary provider launch — `managed_bootstrap/mod.rs:43-59,76-89`; `supervisor.rs:182-284`; `worker.rs:308-355,519-575`; `provider/managed_isolation.rs:157-165,908-936`; `provider/registry.rs:150-160,191-201,241-251`; catalog endpoints `codex/catalog_endpoint.rs:61-73`, `opencode/catalog_endpoint.rs:61-73` | Bootstrap topology is explicit and fail-closed. The supervisor receives a `VerifiedRelease`; Path 1 resolves the profile PATH before preparing local-auth state, starts with no shared-host isolation roots, and removes shared-host selectors. The worker starts the ordinary Path-1 kernel. Official adapters and account utilities use the common unwrapped provider branch unless the explicit shared-host isolation selector is active. This source path does not prove provider ancestry or effective installed units. |
| **Current d3** · Kernel-only slice broker lease vs provider inheritance — `managed_bootstrap/mod.rs:44-59`; `supervisor.rs:301-317,451-516,713-1010,1366-1449`; `worker.rs:575,1056-1293`; `slice/local_docker/broker.rs:119-197,215-235`; `provider/managed_isolation.rs:110-143,168-201,908-936` | Path-1 clears inherited broker controls and restores only a slice root derived from the absolute broker socket. The private lease is passed to the kernel by FD; without a lease the kernel gets a required marker and no socket/FD. Kernel startup consumes and scrubs the controls and sets close-on-exec. The provider launch scrub removes all broker controls and the slice root, while the Linux launch regressions use the production provider command seam to check the child has no broker environment or FD. This supports ordinary provider launch while retaining kernel-owned slice operations; it does not establish live access or fresh-worker behavior. The Rust tests were inspected, not run here. |
| **Current d3** · Environment scrub, retry, restore, reconnect — `provider/managed_isolation.rs:107-143,168-201,908-936,1324-1351`; `provider/service/run_lifecycle.rs:11-64`; `runtime/state/provider_relaunch_runtime.rs:55-85`; `runtime/state/provider_launch_failure_runtime.rs:27-45,120-174`; `runtime/state/project_environment_setup.rs:2061-2099`; `provider/run_actor/command_execution.rs:25-70` | The fixed list removes managed topology, slice, bootstrap, broker and release controls; ambient secret-like and numbered workspace-root names are also removed. Selected account paths and resolved credentials are intentional launch inputs. Initial starts and policy relaunches resolve through the common adapter; setup-recovery snapshots preserve that launch. Prompt submit/abort restore live runtime slots without selecting a new topology. Launch-failure retry here retries durable cleanup, not provider execution. Live retry/reconnect comparison remains open. |
| Supervisor restart and systemd — `managed_bootstrap/supervisor.rs:110-172`; `managed_bootstrap/worker.rs:278-304,463-489`; Path-1 units `chariox-path1-managed-bootstrap.service:9-32`, `chariox-disposable-worker-bootstrap.service:9-39`; `verify-image-release.mjs:369-445`; `upgrade-image.sh:121-134,654-660` | Kernel respawn uses the same Path-1 helper; worker preparation/restart remains Path-1. Role units contain no provider-restricting `Protect*`, `Private*`, `NoNewPrivileges`, namespace, address-family, `ReadWritePaths`, or `UMask` directives; release verification rejects those on the home unit and upgrade rejects Path-1 drop-ins. Worker `StateDirectory=chariox`/mode `0700` allocates service state, not a filesystem/process sandbox. Hardened shared-host, rootless Docker and broker services are separate. Effective installed units remain uninspected. |
| **Current d3** · Image, shell, container and AppArmor selectors — `prepare-hetzner-image.sh:36-54,137-149,239-241`; `install-image.sh:16-31,104-113,445-537,563-568`; `upgrade-image.sh:24-40,121-142`; builder `scripts/build-managed-kernel-release.mjs:11-16,121-182`; guard `scripts/managed-kernel-release.test.mjs:915-940,1013-1050`; slice `docker/Dockerfile:2,5,53-56,58,67`; `provision-linux-docker-slice.sh:766-784,860-883`; `docker/start-runtime.sh` (`start_slice_kernel`); `chariox-slice-provider.apparmor:1-7` | Preparation/install/upgrade choose Path-1 explicitly and verify its unit. The builder selects the named `scratch` target, which exports exactly the kernel, managed-bootstrap, and relay binaries; it then creates and signs the build attestation outside that image stage. The other Dockerfile base images are digest-pinned. The exporter is an artifact stage, not an installed runtime/state image. The installer separately verifies the signed release/attestation and publishes it under the digest-named root-owned release tree. Bubblewrap/AppArmor/seccomp compatibility flags apply to the separate inner Docker slice; they are not selected for host Path-1 provider children. The image was not built or installed here; effective host policy remains uninspected. |
| **Current d3** · Allowed managed differences and mandatory shutdown — `managed_bootstrap/release.rs:81-190,285-345`; `install-image.sh:104-113,445-537`; `upgrade-image.sh:620-654`; `local/api/types/managed_environment.rs:92-103,220-237`; `runtime/managed_environment_control/cloud_contract.rs:153-170,206-211,362-368`; CLI `waiting-room-managed-environments.ts:312-381` | The only approved differences remain signed release deployment/atomic activation and mandatory managed-machine automatic shutdown. The ordinary Path-1 provider launch and kernel/provider protocol remain shared. OSS defines and projects minimum-runtime/idle-delay policy; the Cloud timer, reconciliation, and last-agent-finished execution are outside this refreshed OSS source pass and the retained Cloud baseline is stale. Inspect current Cloud and run every required shutdown trigger on the rebuilt machine before closing MP-09. |
| Exact protected paths and Swift/TUI projections — `git_worktree_placement.rs:18-33,42-157,193-242`; `runtime/workspace_search.rs:198-253`; `packages/kernel-client/src/waiting-room-runtime-placement.ts:54-90,104-125`; CLI `waiting-room-controller.ts:320-411`, `waiting-room-managed-environments.ts:43-68,95-124`, `waiting-room-managed-environment-launch-controller.ts:88-120`, `waiting-room-managed-environment-reimage-controller.ts:78-145`, `waiting-room-start-rows.ts:136-147`, `cli-waiting-room-composition.ts:547-625,752-885`; `packages/kernel-client/src/ipc-managed-environment-requests.ts:24-140,183-218,234-305`; Swift `KernelProtocolModels.swift:3-89`, `KernelProtocolRequests.swift` | Exact entry and session/provider launch share cwd preflight. The local placement fix protects five control subdirectories, not the root/development/siblings. CLI projects catalog/readiness, trusted-root/context setup, lifecycle/auto-stop and reimage confirmation. After preparation it strips managed-only fields before ordinary session launch (`cli-waiting-room-composition.ts:598-625`). These are setup/control-plane projections, not an execution exception. Inspected Swift session/request models have no managed-environment projection; Swift managed commands concern live sync (`CharioxAppModelCommands.swift:85-98,157-189`; `CommandCenter.swift:164-167,464-495`). No Swift build or end-to-end client parity was run. |

### Triage of pending historical scanner predicates

The scanner keeps reviewed predicates bound to source commit
`391b38b2be15c4f49d8ea70cc14b031385cf8331` and tree
`199b565b83e9582acd38a38a76520e2ba5ac02b4`. Scanning d3 therefore reports all
three as `pending_source_review`. The source-only triage below records what the
same symbols do at d3; it does not change or repin any predicate.

| Predicate and preserved anchor | Current d3 source and triage |
| --- | --- |
| `MP-07-release-verify-release` — `apps/kernel/src/managed_bootstrap/release.rs:39`, symbol `verify_release`, anchor line `pub(super) fn verify_release(` | The function remains at `release.rs:81` and verifies the signed manifest and kernel artifact before returning `VerifiedRelease` (`release.rs:81-158`). This is the allowed signed-release deployment/activation exception for MP-07/11, not an inner-slice branch or provider-runtime divergence. |
| `MP-09-auto-stop-policy` — `apps/kernel/src/runtime/managed_environment_control/cloud_contract.rs:110`, symbol `AutoStopPolicy`, anchor line `minimum_runtime_seconds: u64,` | The field remains at `cloud_contract.rs:209` and is projected into `ManagedEnvironmentAutoStopPolicy` (`cloud_contract.rs:362-368`). It describes the required automatic-shutdown policy under MP-09/11. This field does not prove the Cloud timer or trigger execution. |
| `MP-09-auto-stop-idle-delay` — `apps/kernel/src/runtime/managed_environment_control/cloud_contract.rs:111`, symbol `AutoStopPolicy`, anchor line `idle_delay_seconds: Option<u64>,` | The field remains at `cloud_contract.rs:210` and is projected alongside the minimum runtime (`cloud_contract.rs:362-368`). It describes the required shutdown idle delay under MP-09/11; last-agent-finish timing and live Cloud execution remain unverified. |

None of these predicates describes an inner-Docker-slice topology or a Path-1
provider-runtime divergence in the inspected d3 source. The two shutdown
predicates classify only policy data; Cloud enforcement and all live shutdown
triggers remain uninspected. All three scanner predicates remain pending, and
MP-01 through MP-11 remain open.

The previous v1 `managed-parity-source-inventory` scanner enumerated broker
scope/FD/required-marker variables separately from other managed selectors and
recorded the `managed-release-artifacts` stage separately from signature
verification and activation. Its counts and automatic `removal_required`
labels are historical lexical output, not reviewed semantic findings. The v2
correction below supersedes that disposition model.

### Source inventory v2 correction (2026-09-26)

The inventory now keeps three layers distinct. Each entry retains the lexical
candidate (category, source anchor, selector, and affected behavior), attaches
non-authoritative `sourceRoleHints`, and carries a separate
`semanticDisposition`. A role hint never changes a disposition or makes a
candidate pass. An unreviewed candidate, including unknown positive Path-1
behavior, remains `fail_closed`.

The scanner retains matches from inline Rust `#[cfg(test)]`/`#[test]` regions
and conventional test source paths. It marks them as test evidence candidates
without deleting or approving their lexical matches. It derives a service
topology hint only from an exact
`Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1|shared_host` declaration
inside that unit; kernel/deploy directory prefixes do not imply Path-1. The
unit declaration is source evidence, not proof of the effective installed
service. Inner Docker-slice paths receive a location hint and remain visible.

The verifier's forbidden-marker arrays can be hinted as verification guards;
an actual Bubblewrap command or restriction in an explicitly Path-1 unit is
hinted as a positive service directive. Shared-host unit restrictions and
inner-slice isolation remain separate hints. These distinctions do not grant
policy exemptions: guards, tests, shared-host behavior, and slice isolation
need an exact independent semantic review just like any other candidate.

New semantic review records must bind the exact source commit/tree and a
candidate anchor containing blob, path, line, column, symbol, category,
selector, and context hash. They also require independent reviewer identity,
review ID, review time, and rationale. Caller-supplied claims cannot add
approvals. The three historical predicate objects above are preserved exactly;
they remain pending on changed source heads and, even at their original head,
do not grant a semantic disposition because they lack independent review
metadata. `removal_required` counts only independently reviewed findings;
unreviewed candidates still fail the inventory closed. This source-only
scanner correction does not close an MP gate or replace root's exact-head full
inventory and acceptance evidence.

The v2 focused scanner suite passes 27/27 with
`node --test apps/cli/scripts/managed-parity-source-inventory.test.mjs`; both
scanner modules pass `node --check`. No full inventory scan was run here; root
owns that exact-head result.

### Uninspected and live requirements

- The d3 source contains the Project persistence correction and focused
  lifecycle/acknowledgment regressions described above. Run those exact-head
  Rust tests and validate the remote worker behavior on the rebuilt machine;
  source inspection does not prove runtime persistence or MP-08 parity.
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

The previous v1 focused inventory scanner suite passed 23/23 Node tests,
including fixtures for the broker-control and release-exporter categories and
a d3 source guard for the kernel-only FD/provider scrub boundary. Those v1
results are historical. The separate
`managed-kernel-release.test.mjs` already has an exact artifact-stage guard; it
was not rerun here. No Rust build or test, Docker build, provider run, host
installation, service change, Cloud refresh, or VM rebuild was performed in
this pass.

The previous v1 scanner's d3 source pass returned `fail`: it enumerated all
required categories, including 138 broker-control references and the four
exact artifact-exporter lines, while labeling 1,285 lexical matches
`removal_required`, 2,312 `unreviewed`, and three historical predicates
pending. Those v1 lexical counts are not a confirmed defect count and are
superseded by v2 candidate/source-role/semantic separation. Root owns the
current full inventory run and its exact results.

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
bindings, unchanged worker identities and incomplete retirement observations. It
retains only allowlisted data in a mode-0600 file and refuses overwrite.
Cloud's receipt digest is retained, not independently recomputed or verified.
If the selected source controller deliberately retains the old relay realm,
add `--shared-controller-target <selected-source-target-id>`. The capture then
requires current managed-context, Cloud account/profile, and connected local
kernel identity reads to agree on that controller, distinct from both retired
and replacement workers. Bracketing reads check identity, not physical socket
continuity. Only public bindings are retained; profile credentials are excluded.
The shared realm may remain active, but all old-worker generation, credential,
target, heartbeat and host-residue retirement checks remain mandatory. Without
that explicit verified binding, the old realm must be disabled and rotated.
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
