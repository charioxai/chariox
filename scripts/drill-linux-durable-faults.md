# Linux durable component and private storage drill

This drill supplements live kernel qualification. It does not attest a release
runtime, execute an App handler, observe a browser view or prove hard power-loss
survival. Its six installation boundary names refer to upload, actual signature
verification, extraction, pending approval, supervisor preparation and SQLite
activation. Preparation uses a test-supplied policy decision; real worker health
and human approval must be exercised separately on the qualification candidate.

Build on the Linux builder with Rust 1.88.0 and admission:

```sh
slot-run cargo +1.88.0 test -p chariox-app-runtime --test durable_faults --no-run
slot-run cargo +1.88.0 test -p chariox-app-runtime --lib --no-run
slot-run cargo +1.88.0 build -p chariox-app-runtime --bin chariox-app-storage
```

Run `scripts/drill-linux-durable-faults.sh TEST_BINARY SCRATCH KEY EVIDENCE` as an
ordinary user. `SCRATCH` must be empty, private and on an independently bounded
filesystem of at most 128 MiB. Evidence must be outside that filesystem. `KEY`
is a fresh 32-byte throwaway Ed25519 seed in a regular mode-600 file. No provider
account, credentials or existing kernel state is used. The runner does not
mount filesystems or provision users. Provision those only in an owned VM or
private mount namespace. Preserve the logs even when an acceptance test fails.

The process parent kills only its own child, waits for SIGKILL and then reopens
WAL/FULL databases and anchored upload/release stores. It verifies acknowledged
raw bytes, accepted upload prefixes, retry identity, coherent activation,
generation fences, snapshot restoration and coupled state/inbox/outbox/wake
commit. Poison is checked through the owner's count query, not a UI notice.
The disk leg obtains real ENOSPC and SQLITE_FULL, checks existing reads and
neighbour bytes, then retries through the same connection/store after removing
only its ballast. This is bounded state-filesystem evidence, not a shared-host
full-disk experiment. Import, log, snapshot/rollback disk exhaustion and real
view recovery remain separate live-kernel legs.

`removed_route_keeps_accepted_occurrence` is an acceptance check. Older stack
bases discard accepted occurrences on removal and therefore fail it; retain that
failure rather than changing the expected result or duplicating a later fix.
The three tests are ignored in ordinary test runs and the child entrypoint is
never selected by the runner on its own.

For real fixed-capacity ext4 storage and helper leases, adapt VM mechanics into a
fresh QEMU guest named `chariox-private-storage-drill`, with systemd and cgroup
v2. Keep source/binaries, VM disks and generated keys private to the task. Run
QEMU itself under `slot-run`. Inside that guest, invoke the existing fixture:

```sh
CHARIOX_STORAGE_PRIVATE_VM=1 CHARIOX_STORAGE_DRILL_KEY=/path/to/throwaway.seed \
  bash scripts/app-storage-linux-fixture.sh REPO /tmp/chariox-storage.XXXXXXXX \
  /tmp/chariox-storage.XXXXXXXX/build/tests \
  /tmp/chariox-storage.XXXXXXXX/build/helper
```

The scratch directory must already have `build` and `evidence` directories. The
fixture refuses existing enrollment/helper/service/storage paths. Both shell
and test admission require the explicit mode, dedicated hostname and actual
QEMU virtualization, and reject any `GITHUB_ACTIONS` variable. Hosted CI retains
its original environment contract. The guest enrolls tiny publisher-signed
validation bytes for readonly code-mount mechanics; those bytes are never
executed and are not a production native runtime. It tests actual data/tmp
ENOSPC, noexec, generation rollback, committed data persistence, mount identity,
helper SIGKILL recovery and lease/mount/loop cleanup. It stops only its own guest
services. Do not run this fixture on a shared builder or borrow another lane's
enrollment, keys, cgroups, helper, runtime or storage volumes.

After the guest exits, copy bounded evidence, record source/protocol, compiler,
image and binary hashes, and delete only owned scratch, VM images and build
outputs. Actual machine reboot, final Mac crash windows and real update power
loss require the separately authorized owner/lead sitting.
