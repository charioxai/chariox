Protocol 410 exposes `RestoreAppDataSnapshot { installation_id,
expected_generation, snapshot_id }` through the shared kernel command path.
The authenticated owner supplies a saved snapshot identity returned by
`files.snapshot`, never a filesystem path, structured payload or authority.
The shared TypeScript binding is `restoreAppDataSnapshotRequest`.

The kernel checks ownership, active generation, package digest, data schema,
snapshot schema and file inventory before stopping the worker. Snapshots use
`chariox.app-snapshot.v2`, which records the owner and data schema; legacy v1
copies lack those fields and are refused. A snapshot from an older generation,
another installation, an incompatible schema or a corrupt tree is refused.
Both quiescent and crash-consistent copies retain their original consistency;
restoring a crash-consistent copy does not make it quiescent retroactively.

The lifecycle installation gate excludes start, update-worker replacement and
another restore. The worker is stopped and fully drained, including admitted
SDK operations, before any private bytes are overwritten. The same gate stays
held through journal publication, mutation and recovery. Success returns the
typed `AppDataSnapshotRestored { installation_id, generation, snapshot_id }`
response and leaves the installation stopped until the owner explicitly starts
it. Refusals use existing typed `AppRequestFailed` outcomes. A late storage
failure can leave the App stopped with a recovery journal; it cannot restart
past unresolved recovery.

The generation-update helper journal cannot restore a named saved tree: it
only maintains volume rollback around a generation switch. That helper remains
unchanged. The bounded restore journal instead persists synced prior files and
an immutable pinned target in the kernel's private `app-restores` directory.
The existing SQLite writer fences the current catalog/signer, replaces private
values and pending wakes, writes the unique restore receipt in the same state
transaction, then commits. The gate-held recovery applies target bytes after
the commit, outside the SQLite writer, so tree copying does not block unrelated
durable writes. A missing receipt selects
prior bytes; a committed receipt selects target bytes. An uncertain COMMIT
fences the writer and retains the journal for SQLite restart recovery. Reusing
the same saved snapshot creates a fresh restore identity each time.

Recovery validates the selected inventory and can be replayed after interruption.
It retires the journal atomically only after the entire selected tree is synced.
Linux worker preparation pins the existing storage domain and resolves recovery
before spawning App code. A staged generation cannot adopt another generation's
unfinished journal. Corrupt or incompatible recovery blocks startup. Two pinned
tree copies are bounded by the existing private-data limits and admission keeps
two GiB of host reserve beyond those copies.

Only private state values and pending wakes are replaced. State versions advance
past both the live and snapshot heads, so old compare-and-swap versions cannot
revive stale writes. Occurrence/inbox/outbox/delivery receipts, approval records,
publisher trust, bindings, automations, file grants and connections are untouched.
Restore does not regrant authority or export credentials. Imported copies are
ordinary private data: revoke retains them, while an explicit restore replaces
them according to the selected snapshot.

Run `bash scripts/app-saved-snapshot-restore-drill.sh` on the admitted Linux
builder. It exercises the protocol shape/hash, real private fixed-libc workers,
foreign/stale/incompatible refusals, replacement/deletion, table preservation,
worker drain/concurrent start, host-space refusal before stop, pre/post-commit
faults, interrupted replay and
repeated restore identities. These are kernel integration checks, not signed
production-runtime, real Room/browser, macOS or power-loss qualification.

The assigned protocol-367 base has no production macOS worker preparation in
`app_lifecycle/start.rs` and no stopped-volume macOS preparation API. A stopped
macOS installation therefore reports `storage_unavailable`; that existing
platform dependency must be integrated and qualified separately. This branch
does not import later candidate implementations to hide the dependency.

The fixed-native fixture branch returns before production Linux preparation.
Neither stopped `prepare_linux` nor the production pre-spawn recovery hook is
executed by these tests. Direct recovery checks and the mock pre-spawn storage
visitor test are functional contracts; signed enrolled Linux/VM qualification
is still required and is blocked without an authorized runtime artifact.

Uninstall holds the same installation gate until journal retirement. A retained-
data uninstall refuses an unfinished restore before stopping the worker: first
complete recovery, or explicitly choose data deletion. Successful uninstall
invalidates restore receipts; completed data deletion also retires unfinished
or corrupt journal trees. A failed deletion retains the journal and can be
retried through the existing delete-data uninstall path. Reinstall cannot adopt
an earlier restore generation. Saved snapshots retain their existing retention
policy; this change does not add a snapshot history/deletion browser.
