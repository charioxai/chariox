# MP-04/MP-07/MP-08/MP-10/MP-11 upgrade recovery qualification

MP-07 remains open. This document separates offline transaction proof from the
owner-authorized VM campaign. It does not authorize staging, infrastructure
credentials, replacement of the reserved builder, or an update outside Cloud.

## MP-04/MP-07/MP-10/MP-11 offline scope

Run as Linux root with disposable scratch outside the checkout:

```sh
node --test --test-concurrency=1 scripts/managed-kernel-upgrade-recovery.test.mjs
node --test --test-concurrency=1 scripts/managed-kernel-upgrade.test.mjs
node --test --test-concurrency=1 \
  deploy/managed-kernel/managed-update-recovery.test.mjs \
  deploy/managed-kernel/managed-release-activation.test.mjs \
  deploy/managed-kernel/verify-image-release.test.mjs \
  apps/kernel/managed-upgrade-protocol-transitions.test.mjs \
  scripts/apps-rollback-state.test.mjs
python3 deploy/managed-kernel/extract-release.test.py
python3 deploy/managed-kernel/release-update-storage.test.py
```

MP-07 recovery re-verifies the selected release before rollback execution and
before settling either a live or tombstoned terminal journal. The independently
supplied current/next release and builder public pins remain authority. Packaged
and journaled builder pins supply correlation only. A rejected signature, digest,
ownership, or permission check preserves the pending journal and refuses service
mutation; it must not publish success or execute the selected corrupt binary.

MP-04/MP-07/MP-10 tests execute the actual upgrader, verifier, receipt helpers,
immutable publication and symlink operations. Ed25519 private fixture keys stay
in memory. Systemd, executable binaries, presence and provider history/profile
markers are synthetic. They establish transaction invariants, not real provider
resume, browser-session continuity, service policy or VM acceptance.

| MP-07 boundary | Offline observation | Remaining VM proof |
| --- | --- | --- |
| Extraction | Real bounded extractor/storage helpers; truncated archives and unsafe entries reject before activation | Interrupt the owned updater during extraction and reboot; reconcile the same attempt |
| Release publication | SIGKILL after durable release publication, before journal publication; current and state remain unchanged; normal signed retry succeeds | Durable publication through the real filesystem and boot boundary |
| Prepared/stopped/builder-pin/activated | Original incoming image is removed; recovery-only restores the previous release, pin and receipt | Normal restarted root unit reaches recovery without its original archive |
| Supervisor start/health | SIGKILL after start, plus existing failing-health and rollback-health fixtures | Actual fresh process, presence, relay and official-provider continuity |
| Commit/result/tombstone/rolled-back | Recovery-only preserves the certified outcome, publishes correlated evidence and clears journals; replay performs no service mutation | Cloud acknowledgement, identity audit preservation and successor admission |
| Tamper/foreign command/unapproved release | Selected-release corruption and foreign update fail; existing suite covers wrong pins, unsafe ownership and stale/unapproved transitions | Rejection through the authorized VM fault seam with public receipts |
| Apps boundary | Existing signed-upgrade suite denies rollback with App state; explicit CLI override warns; failed downgrade restores the Apps release | Product-created App state and owner-authorized override, if supported |

MP-04/MP-07/MP-08/MP-10's offline B→new→B→new cycle uses synthetic signed
protocol-370 and current-policy fixtures with the checkout's updater on every leg.
It is not execution of historical B's installed tooling. No source result may be
relabeled as B, F, G2, or a signed successor's live result.

## MP-04/MP-07/MP-08/MP-10/MP-11 VM prerequisites

The coordinator must supply the following public bindings and owner-side product
access before the affected live actions can run:

1. Exact independently reviewed OSS successor commit/tree and paired Cloud
   commit/tree. The local G2 base is `9334141d420f8a32393f206102c5b8b4a1b0b609`
   (411/70); a successor must include the reviewed recovery change. Do not assign
   another protocol number for this deployment-only change.
2. Signed baseline B and successor manifests, archives and builder attestations;
   verified public release/builder inventories; archive sizes/hashes; kernel
   hashes; target architecture; provider profile/image IDs and digests. Historical
   B is OSS `b37f4504e4ce040a2d6c35dc56475315defbc861`, protocol 370, release
   `sha256:b8aa034ebf9a238113da932a410ef4d0a92cd72e24fd7c69040c2404d79e959d`.
   Those historical identifiers alone do not prove available or eligible inputs.
   Baseline tooling must support native schema 3/evidence 1. An old schema-2
   updater cannot be patched or bypassed to make the drill pass.
   MP-07 source inspection of exact historical B also finds that `poll_once`
   returns Pending for a retained recovery journal instead of launching a root
   recovery unit, and its upgrader requires the incoming image before recovery.
   The fixed-tooling fixture cannot qualify reboot into that binary. Before that
   positive fault case, supply a reviewed product recovery path covering exact B
   or an explicit owner/coordinator decision selecting a different eligible signed
   baseline. Never relabel a patched artifact as historical B; these source
   observations do not identify the first stop/restart in the historical F RED.
3. A reviewed isolated local Cloud control plane with its own database,
   publication/catalog and owner session, using the supplied product bridge.
   Freeze both release placements so reversing to B uses Cloud's normal accepted
   placement, not a direct machine-side upgrade. No staging or shared Apps host.
4. A lane-scoped bridge command/interface and expiry that permits normal product
   create, status, release-update, stop/start, delete, and approved updater/reboot
   fault actions on this disposable VM only. The bridge must retain credentials
   owner-side and prohibit the reserved builder and foreign resources.
5. An enforced cap for VM count, class/CPU/RAM/disk, lifetime, total cost and
   fault/recovery observation time; a validated independent cleanup watchdog;
   read-only independent server/IP/volume census after product deletion.
6. Product-linked provider selections and a human available for any receiving
   provider login. Public status/history observations must exclude auth state.
   No credential copying, handcrafted identity or raw provider-account reads.

MP-07's positive Apps override case additionally needs explicit owner approval
and a reviewed product bridge operation carrying that decision through the
same coordinated signed transaction. The current release-update RPC has no
override field. Do not invent one or invoke a second updater authority. Missing
override support blocks that positive case only; the rejection case and other
campaign work continue.

## MP-04/MP-07/MP-08/MP-10 exact VM sequence

MP-07 uses one disposable VM and retains the same environment, Machine, Kernel,
disk and user-state lineage across all three successful transitions. A hard
reboot may change boot identity; reimage, state copying and release hotpatches
are disallowed. Separate fault VMs require their own cap and deletion receipts.

1. MP-04/MP-07/MP-10: create the VM on eligible signed B through the product
   bridge and isolated Cloud placement. Observe its actual signature/digest,
   current link and facade, root release ownership, effective units, user HOME
   `/home/chariox` and `CHARIOX_HOME=/home/chariox/.chariox`. Retain public VM,
   IP, volume, enrollment, relay and source identities. Require fresh heartbeat.
2. MP-04/MP-08/MP-10: create a Room and real official-provider turns through
   normal kernel adapters; create user files/package state and browser/profile
   markers through product actions where available. Record user-state directory
   device/inode, public history/thread IDs, profile generation and file digests.
   Never retain credential contents or hash credential files.
3. MP-07: select the reviewed successor placement in isolated Cloud. Through
   the home kernel, send `RequestManagedEnvironmentReleaseUpdate` with these
   exact reviewed public fields:

   ```json
   {
     "environmentId": "<owned-environment-id>",
     "expectedProviderImageId": "<placed-provider-image-id>",
     "expectedProviderProfileId": "<placed-profile-id>",
     "expectedProviderProfileDigest": "sha256:<placed-profile-digest>",
     "expectedRuntimeReleaseDigest": "sha256:<placed-release-digest>",
     "expectedRuntimeSourceCommit": "<reviewed-commit>",
     "expectedRuntimeSourceTree": "<reviewed-tree>"
   }
   ```

   MP-07 observes `GetManagedEnvironmentReleaseUpdate` for that environment.
   The corresponding authenticated Cloud POST/GET routes are
   `/managed-environments/:environmentId/release-update`. The bridge supplies
   authentication; do not call with extracted credentials. Require the same
   update ID, from/target digest, terminal result and authenticated identity
   report to agree before declaring the leg successful.
4. MP-04/MP-07/MP-08/MP-10: verify actual successor process/binary hash,
   immutable digest directory, current/facade links, pin correlation, healthy
   fresh presence, retained environment audit, unchanged state inode and
   real resumed provider/history/profile markers. Attach/reconnect through
   normal clients and perform another turn; an online badge is insufficient.
5. MP-07: switch only this isolated placement to reviewed B, then make the same
   normal request with B's public fields. Require B's real process and identical
   state lineage. Keep the principal cycle free of App state so a pre-410 rollback
   is eligible; do not evade the Apps guard by deleting product-created state.
6. MP-07: restore successor placement and repeat the normal request. Require
   a third distinct update ID, exact successor identity, continued Room/thread,
   state continuity and settled previous attempts. Do not count a same-release
   no-op or successful reboot as either requested transition.
7. MP-07/MP-10: on separately bounded attempts, fault extraction, publication,
   activation, actual health, and result/acknowledgement settlement through the
   supplied fault seam. Record root unit invocation IDs and the initiator of
   each stop/restart. Kill only the owned updater or reboot only the disposable
   VM; no journal edits. After extraction, remove only its original lane-owned
   archive through the approved seam, then restart/reboot normally. Require
   recovery without re-downloading or copying state and no false completion.
8. MP-07/MP-11: exercise signed-artifact tamper, wrong/unapproved target,
   foreign journal/command, failed target health and failed restored health.
   Reject before unauthorized execution; uncertain recovery remains pending
   with a diagnostic. Run the App-state rejection case separately, and only run
   the positive override case with the explicit authority described above.
9. MP-04/MP-07/MP-08/MP-10: restore a healthy certified release through the
   normal signed transaction, make a final real provider turn, then product
   stop/delete. Independently prove every owned server, both primary-IP
   resources, volume, watchdog, bridge child, target and billable resource
   absent. Retain exact resource IDs and deletion timestamps without credentials.

## MP-07/MP-10 evidence and closure

MP-07 evidence goes in the external lane evidence root. For every leg/fault,
retain exact reviewed source identities, commands/exits, trigger/initiator,
update/unit/boot/resource IDs, from/target manifests and public verification
pins, actual process identity, public state continuity, Cloud acknowledgement,
recovery timeline, resource samples and separate cleanup observations. Baseline
and successor signature checks must correlate with their approved public inventory.

MP-04/MP-07/MP-08/MP-10/MP-11 remain open until independent review and the
authorized live receipts agree. Offline helper/signed-fixture passes do not
establish ordinary-versus-managed parity, browser acceptance or VM cleanup.
