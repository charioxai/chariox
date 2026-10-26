# MP-01..MP-11 — hosted acceptance and replacement rollout handoff

Preparation checkpoint: 2026-10-04/05, lane b219 (B2-19). Actual publication,
Cloud staging, provisioning, hosted acceptance and rollout belong to OWNER.
This document closes preparation only. Every MP acceptance item remains open.

## MP-01..MP-11 — authority and source freeze

Read together with the [canonical plan](BROWSER_COMPUTER_USE_END_TO_END_PLAN.md),
[parity inventory](MANAGED_PATH1_PARITY_INVENTORY.md),
[M20 contract](M20_DOCKER_SLICE_BROWSER_STATE_VALIDATION_PLAN.md) and
[replay commands](HOSTED_ACCEPTANCE_REPLAY.md).
The owner's 2026-10-04 multidomain direction supersedes the canonical plan's
Selkies default/noVNC retirement prescriptions: prepare a generic replacement,
with Chromium controlled by the kernel outside slices. Preserve Browser/CDP,
Computer, Room, Vault, provider, Docker persistence and Path-1 requirements.
Benchmark rounds and ranking do not gate functional rollout or MP closure.

| MP-07/MP-08/MP-10/MP-11 identity | Exact value and admission limit |
| --- | --- |
| Frozen lane/runtime source | OSS main `358491d662330aff538bbd6d6450391332f62ac2`, tree `f78d25308bd4cc012f129a86df8ff537699712df`; tree identical to `9334141d420f8a32393f206102c5b8b4a1b0b609` |
| Paired G2 Cloud main | `619486c693cabbcef805c014e6e0b100c54dc032`; asserted by coordinator's public release receipt, private bundle not locally available for independent verification |
| G2 protocols | Local `411`, relay `70`; later allocations must be supplied by coordinator |
| Main G2 release | `sha256:81a6a9cdca62706799a534575f1db07384ad2b9b6db6b5739b8e18fe915123a8` |
| Main G2 signed context | `sha256:acf2b4dfe25f92eaacce66120987bcad78318a9e9f48d3ffc87d5bf9dce6677d` |
| Main G2 headed image | `sha256:fa5760763797bbc4fe1f28ae64872cac8fb5b7097f2e32692cb21cd7450f1032` (local image identity; registry manifest/signature/provenance need separate admission) |
| Release receipt | `/root/.chariox/dev/browser-resume-20260930/agents/G2_MAIN_RELEASE.json`, snapshotted in b219 evidence; supersedes G2C for acceptance |
| Later source reservations | Apps queue #843/416, native steering g2fix #847/422, b207 artifacts #850/420 with peer allocation still open in inspected handoff; kbrowser 417; final aggregate/pins unprovided |

The G2 receipt is an artifact handoff, not a hosted PASS or a signed release of
the later lanes. Freeze a new exact paired aggregate after integration and
review. Do not assign that aggregate a G2 receipt, inherit old reviews, or
select a protocol number. Documentation commits in this lane do not change
runtime protocol or supply new signed runtime artifacts.

Runtime authority remains the kernel. Cloud authenticates, provisions, issues
scoped tokens and projects control-plane state. The relay carries opaque
encrypted packets. Browser, kernel, remote TUI and kernel-to-kernel hosted
connections use Caddy-fronted `wss://`; heartbeat freshness gates selection.
Local/self-hosted `ws://` remains supported through existing contracts.

## MP-01..MP-11 — four-tier acceptance map

Each cell needs exact-source evidence, focused tests, independent review and
cleanup. S means source/helper evidence only; it cannot replace a live tier.
T1 is local ordinary kernel plus Docker; T2 adds same-host home/worker/relay
and real Web/local/remote TUI conjunction. T3 is fresh OpenShip/Path-1 hosted
infrastructure after T1/T2 pass. T4 is self-hosted remote compatibility.
Managed-specific source fixtures may run locally before staging, but never
claim to establish a fresh managed VM boundary.

| Item | S and T1 | T2 | T3 OWNER | T4 |
| --- | --- | --- | --- | --- |
| MP-01 ordinary launch | Inspect every launch/retry/restore branch and source units; collect ordinary ancestry, mounts, privilege flags, umask, network and installs for all three harnesses | Repeat leased/native provider paths; inner-slice isolation remains a distinct topology | Same signed build, no bwrap ancestor/managed marker/equivalent inherited systemd sandbox; effective units/drop-ins; `sudo -n true` and agent-run apt install | Same ordinary control; do not impose managed sandbox on self-hosted workers |
| MP-02 paths | `/home`, `/tmp`, nested/new directories, unlistable children, new repos, exact cwd/session/error controls | Retain explicit Project/worktree/worker choices across ready/stopped machine selection and reconnect | Matched Unix-user results; investigate rootless/broker Room Start first error with safe diagnostics | Same cwd/session/result/error behavior |
| MP-03 controls | Exact protected file/service directories denied; parents and unrelated siblings admitted; symlink aliases denied | Same preflight and remote error projection | Real product-owned control counterpart and effective policy; no blanket parent ban | Same control protection and siblings |
| MP-04 home | Ordinary HOME/state ownership and immutable release exclusion | Utilities, retries, provider/native relaunch and reconnect share home model | `HOME=/home/chariox`, `CHARIOX_HOME=/home/chariox/.chariox`; user-owned mutable state stays outside releases after update/reboot | Normal configured user home/state preserved |
| MP-05 repository | Default basename, safe names, collisions, no clobber, worktrees, retry and rollback | Transferred repository and session choices agree | `/home/chariox/<basename>`, occupied destination fails safely, default/custom comparison | Same transfer/basename/no-clobber semantics |
| MP-06 trusted root | Create/bootstrap/receipt/child source fixtures | Web/TUI read server-authoritative value; no client worker override | Default/custom create, idempotency, child inheritance/override denial; root never becomes workspace allowlist | Common worker transfer behavior; managed machine-creation setting itself is N/A |
| MP-07 release | Signed admission/activation/recovery fixtures; ordinary same-artifact control | Protocol compatibility, interrupted state reconciliation | Cloud-authorized in-place old→new→old→new where rollback is admissible; interruption/reboot/tamper/unapproved tests; immutable releases and state intact | Supported install/compatibility; Cloud authorization is N/A on an unmanaged host |
| MP-08 runtime | All providers, Browser/Computer, M20, Vault, setup/Adjust, history, permissions, artifacts and target-owned export | Same Room/Web/TUIs/native clients, MCP, takeover/concurrency, reconnect, queued prompts/messages/liveness | Full local contract replay with selected context/credentials, remote Git and real worker setup validation | Same shared protocol and runtime; no hosted-only behavior fork |
| MP-09 shutdown | Source policy/timer/transaction fixtures; ordinary runtime provides activity control | Shared activity/idempotency/freshness projections | Every trigger including last-agent finish, minimum, disabled, keep-running, restart/deployment reconciliation, disconnected/manual/custom/explicit lifecycle and stale no-ACK cutoff; STOP preserves disk | Mandatory managed auto-stop is N/A; unmanaged workers must not acquire it |
| MP-10 matrix | Executable ordinary control and explicit per-cell applicability | Same-host relay proof; all provider/client intersections | Admitted paired collector, fresh boot/enrollment/relay/release, all live acceptance, costs, deletion and independent review | Repeat applicable remote functional/security/fault/soak rows |
| MP-11 audit | Exact commit/tree/blob semantic dispositions across Rust/TS/Swift/shell/units/images/AppArmor/patches/fragments | Client and relay projections; negative scoped-account/lease tests | Effective host policy and credentials scoped only to owner machine/account; historical predicates need authentic provenance | Reinspect common/self-host behavior; only managed deployment and mandatory shutdown are exceptions |

MP-08/MP-10 required conjunctions are the three official harnesses (Codex,
OpenCode, Claude) × six provider columns (Browser, Computer, thread save/restart,
permissions, attachments, Web+TUIs). Retain a per-cell table, rather than one
provider smoke replacing eighteen cells. Native TUIs attach through the home
kernel and ordinary leased run; Claude hidden context uses hook additionalContext.
Standard home-worker does not copy MCP/skills; slice-backed transfer is distinct.

MP-08/MP-10 placement proof must reflect the final aggregate: host user-domain
Browser remains outside slices; Room/agent slices retain desktop/program state.
Exercise home, different-slice and leased-worker agents only where the reviewed
contract admits them, with foreign Room/forged lease denial. Cross-kernel
user-domain Browser access is deferred by the owner; do not fabricate success
or expose it to satisfy historical slice-browser rows. Record the changed
placement applicability, approved replacement row and still-required Room
Computer/remote-environment rows before calling the matrix complete.

## MP-01..MP-11 — owner execution sequence and stop criteria

1. **Freeze and review (MP-07/MP-08/MP-10/MP-11).** Supply exact OSS/Cloud
   bundles, final protocol/minimums, Apps/display/native/artifact receipts and
   signed rollback candidate. Resolve valid findings in new commits; re-review
   changed heads. Run final CI only when implementation and exact-head review
   are clean. Keep implementation/review independence and true reviewer identity.
2. **Admit local results (MP-01..MP-11).** T1/T2 functional, security,
   persistence, provider/client, concurrency, faults, regression and cleanup
   must have admitted evidence for the candidate. Historical F/G2C results
   retain their original identities; unchanged G2 source evidence may be admitted
   through explicit coordinator equivalence review, with deltas replayed. It
   cannot substitute for missing fresh/hosted or replacement behavior.
   A replacement-aware active/idle soak runner must exist; the frozen active
   runner only accepts old display backends.
3. **Authorize an isolated campaign (MP-07/MP-09/MP-10).** Specify approved
   image, region, compute class, provider resource, lifetime/cost cap, owner
   account, explicit cleanup ownership and ordinary control. Use a fresh VM or
   separately approved provider reimage; never rebuild the reserved builder.
   Coordinator supplies reviewed OpenShip/Cloud product commands from the
   private bundle. No manual install, credential extraction or guessed API route.
4. **Provision and correlate (MP-01..MP-07/MP-10/MP-11).** Reviewed local
   home kernel authorizes cutover through product paths. Record allocation or
   reimage operation/completion, approved image, boot/machine/enrollment/relay
   identities, signed release and absence of old residue. Reimage needs retirement
   of old services, processes, state, targets and heartbeats; it is separate from
   MP-07 in-place update. An in-place binary replacement is not fresh evidence.
5. **Hosted replay (MP-01..MP-11).** Web authenticates via Cloud, then attaches
   through hosted `wss://`. Connect local/remote/native TUIs to the same home
   session. Check stale-heartbeat rejection and encrypted terminal/display traffic
   without Cloud proxying. Run the paired path matrix, all providers and Drills
   A–I with replacement-aware transport and host-profile tests. Use owner public
   service login/2FA and receiving-machine official provider login where required.
6. **Durability and faults (MP-07/MP-08/MP-10/MP-11).** Deterministic cookie,
   storage/service-worker/offline markers, OAuth popup, browser profile/keyring,
   download, real graphical editor/Unicode/preference, installed programs and
   provider threads survive full recreation and named backup restore. Test host
   Browser profile separately from Docker desktop/home archive. Sandboxing must
   remain active on normal, fallback and restored launches; Apps receipts must
   show real App-domain behavior. No page-load-only acceptance or manual repair.
7. **Shutdown, scale and cleanup (MP-09/MP-10/MP-11).** Sequential real trigger
   campaign, fresh same-profile eight-hour active and twenty-four-hour idle soaks,
   safe maximum admission, multiviewer/slow reader, two-machine selection/partition,
   reconnect/save/create-delete loops, terminal traffic under display load. STOP
   preserves user disk; DELETE needs independent VM/volume/IP/OpenShip/Cloud/relay
   absence and final cost receipt. Run T4 before final functional acceptance.
8. **Decide rollout (MP-07/MP-08/MP-10/MP-11).** Admit signed rollback and
   replacement policy below. OWNER records acceptance per MP item only after
   exact-source review, all applicable cells and verified cleanup agree.

Stop the affected run on the first failed assertion and retain the first seam,
trigger, UTC/monotonic timeline, exact command/exit, recovery time/data loss,
source/artifact bindings, before/during/after resources and cleanup. RED remains
RED until reproduced and fixed; BLOCKED is never PASS; an uninjected fault is
UNRUN. Do not wait on benchmark ranking. Continue unrelated preparation while
owner-dependent cells remain blocked.

Hosted hard stops (MP-01/MP-03/MP-07/MP-08/MP-09/MP-10/MP-11): wrong or
unreviewed signed source, credential/secret exposure, Cloud runtime proxy or
relay inspection, unsafe hosted endpoint/fallback, stale target treated online,
duplicate Room/provider/action authority, unratified RTO/RPO regression, unsafe
sandbox fallback, incomplete migration rollback, failed shutdown or deletion.
Builder-specific floors are 60 GB available disk and 9 GiB MemAvailable; forecast
peak plus reserve, sample during work and settle only owned processes.

## MP-07/MP-08/MP-10/MP-11 — replacement policy proposal

This is a policy proposal for the display lane/OWNER, not an implemented flag.
Use one kernel-advertised capability/selection contract. Logical states are
`legacy`, `replacement`, and `unavailable`; final flag name/serialization and
any required protocol allocation remain with coordinator. Clients consume the
selection; Web never chooses another runtime authority or browser profile.

| Transition | Required receipts | Default, compatibility and rollback |
| --- | --- | --- |
| Opt-in local/internal | Exact replacement source, frame/input/security contract, focused tests and T1/T2 proof | Existing released default retained outside the opted-in cohort; unsupported clients receive explicit upgrade/unavailable state |
| Owner staging cohort | Paired signed release, reviewer-clean final CI, fresh OpenShip T3 and T4 applicability proof | Server-controlled cohort; one selected transport per view; preserve target/tab/generation/input and Vault policy fences |
| Replacement default | All functional/security/fault/resource/cleanup gates; ratified error/reconnect/task-success/latency/CPU/memory/bandwidth/support thresholds; 8h/24h receipts | Change default on signed images and Web/local/remote/native projections; old/new client combinations either explicitly compatible or reject before attachment/input |
| Rollback window | Successful downgrade/fallback/re-upgrade rehearsal with retained state and valid signatures | Proposed minimum seven consecutive days after default enablement, extended by any unresolved regression; OWNER must approve duration and thresholds before enablement |
| Retire old dependency | Window elapsed, no open fallback incidents, no active old-capability consumers, independent review and signed rollback retained off-device | Dedicated cleanup change removes only obsolete dependency paths after affected tests/pins/docs/image manifest updates and final CI |

Fallback is explicit, authorized and visible. It must preserve the kernel-owned
capture/input barrier, policy revision and masked pixels, including opaque
fallback for unsafe protected capture. It cannot bypass renderer sandboxing,
Vault authorization, hosted TLS, heartbeat admission or session ordering.
Ratify fallback triggers for unavailable/unsupported transport and sustained
ratified regressions. After rollback, repeat affected functional gates; no
benchmark ranking prerequisite and no sole-Selkies 377/69 patch adoption.

Retirement inventory covers old launch/process ownership, packages/images,
viewer imports/assets, endpoint/capability names, env/config flags, fixtures,
soak/fault runners, documentation/licenses and release overlays. Preserve
shared Browser Controller/CDP, Vault/Room/input services, generic encrypted
relay display channels, self-host compatibility and required desktop tools.
Never prune shared builders/caches, signed rollback artifacts, provider profiles,
reviewer state or another lane's containers/images. Record exact old/new image
contents and dependency sizes without making size a functional ranking gate.

## MP-07/MP-08/MP-10/MP-11 — signed rollback prerequisites

- OWNER verifies release/builder public fingerprints against approved inventory;
  off-builder durable private keys and protected backup/restore proof remain
  operator-local. Targets receive public verification pins only. No rotation
  or substitute key is inferred from a missing receipt.
- Retain prior and candidate signed manifests, attestations, exact source/tree,
  kernel/bootstrap/relay hashes, headed image registry digest/provenance and
  paired Cloud/client compatibility. Public G2 pin files alone are not evidence
  of independent trust approval or of signing a later aggregate.
- Test Cloud-coordinated old→new→old→new, interruption at extraction/admission/
  journal/activation/health/migration/settlement and reboot. Bind each attempt's
  Cloud update ID, from/target digest and durable outcome; reject unapproved,
  corrupt and foreign attempts. Mutable home, profiles and repositories stay
  intact. If Apps migrations forbid an older rollback target, stop and supply
  a reviewed compatible target; never edit a signed artifact or bypass the guard.
- Prove target rejection of unsupported protocol/minimums, same-thread recovery,
  no duplicate actions/permissions and state integrity. Retain previous known-good
  save and public rotation history. Runtime identities are product-generated;
  disposable lane state is removed, durable credential assets remain protected.

## MP-01..MP-11 — resolved-versus-blocked ledger at this checkpoint

| Gate | Disposition and next concrete input |
| --- | --- |
| MP-07 G2 main artifact identity | RESOLVED as coordinator receipt only; main source/tree verified locally; no new signature/install/hosted acceptance claimed |
| MP-01..MP-11 preparation | Four-tier map, source-bound replay commands, review/CI, stop/cleanup and rollback policy prepared here |
| MP-07/MP-08/MP-10/MP-11 exact final aggregate | BLOCKED on coordinator final paired Cloud bundle, deltas, protocols, fresh signing and reviews; G2 is not the replacement aggregate |
| MP-08/MP-10 Apps/appviews/display receipts | BLOCKED: no local appsong2/appviews/display receipt at snapshot; obtain exact heads/contracts/security/model-visible/client proof; never contact their hosts |
| MP-08/MP-10/MP-11 b207 | Local `4169cf0f617297ccc5dc3e9359a94268d519eca3` iframe leak fix receipt inspected; peer allocation unresolved; provider/Web/TUI/outside-slice conjunction open |
| MP-08/MP-10/MP-11 kbrowser | Inspected `79fbe9763205cca38e12104c9b52f1fa57973145` handoff plus in-progress MD-5 status; Linux local proof has limits, Mac cross-check RED without native SDK; native Mac/secret/display/App proof still required |
| MP-01..MP-06/MP-08/MP-10/MP-11 paired matrix | BLOCKED on admitted ordinary/fresh controls, collector context and complete provider/client comparison; old managed8/F observations remain historical |
| MP-07 upgrade/rollback | BLOCKED on admissible signed baseline/candidate, live interrupted old→new→old→new and Apps migration guard proof |
| MP-09 shutdown/cost | BLOCKED on exact-final live triggers, default manual/deployment reconciliation, durable STOP and independent DELETE/cost receipts |
| MP-08/MP-10 replacement soaks/faults | BLOCKED on replacement-aware runner/contract and ratified thresholds; F full-duration soaks preserved only at F; old streamer-specific cells require explicit replacement |
| MP-08/MP-10/MP-11 human sittings | OWNER: official receiving-machine login, public-service/Vault/2FA, native Mac, authorized-recipient leakage boundary and profile/display policy decisions |
| MP-11 inventory | BLOCKED on independent exact-source semantic/effective-host audit and authentic historical predicates; historical counts do not approve the final aggregate |
| MP-07/MP-08/MP-10/MP-11 final review/CI/hosted | OWNER; local helper PASS cannot substitute. Clean exact heads and workflow run SHAs/results required, with any exception explicitly accepted by owner |

Local b219 validation: 62/62 offline host/provider-to-Cloud correlation fixture
tests passed, zero skips, and collector/comparator usage checks exited 0 at the
frozen main source. These prove replay helper behavior only. Evidence under
`/root/.codex/evidence/browser-resume-20260930/b219/` binds inputs, commands,
exit codes, resources and owned temporary cleanup. No Rust build, runtime
campaign, provider login, network contact, deployment or publication occurred.
