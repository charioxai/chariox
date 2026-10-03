# Structured App state

This component stores structured values on the existing kernel SQLite
connection. Every read and mutation checks the installation owner, active
generation and quiescence state in the same SQLite snapshot. Mutations also
require the active release's schema version. It opens no second database and
does not expose an App RPC or authenticate a caller.

`StateChanges` bounds and validates the structured part of an SDK transaction.
Keys are exact case-sensitive Unicode strings of at most 128 UTF-8 bytes, with
no control characters. Values are JSON with at most 32 nesting levels, 16,384
nodes and 256 KiB encoded bytes. One change has at most 128 unique checks,
64 unique writes and 512 KiB of encoded key/value bytes. Checks and writes may
refer to the same key; duplicate checks or duplicate writes are rejected.

A check with no version means the key is currently absent, including after a
deletion. It does not promise there was no intervening activity. Present values
use the monotonically increasing installation revision assigned by their last
transaction. Deletion and recreation never reuse a previous present version.
Revisions stop at JavaScript's maximum safe integer rather than wrapping.

The component admits at most 4,096 current keys and 16 MiB of UTF-8 keys plus
encoded values per installation. It checks the final transaction footprint,
so replacing data at capacity is independent of write order. These are logical
payload limits, not a filesystem quota: SQLite pages, WAL, journals, ordinary
App files, browser data and aggregate host reserves need their production
storage mechanisms.

`transaction` owns an immediate transaction and acknowledges after commit.
`apply_in` uses a savepoint inside an existing mutable transaction so the
installation outbox can compose state and accepted occurrences atomically.
It does not emit, validate or silently discard occurrences. A failed state
write rolls back its savepoint even if a caller commits unrelated work. The
encompassing operation must roll back on failure and must not acknowledge
either state or an occurrence until the outer commit succeeds.

The kernel writer adapter must retain the admitted worker's verified publisher
binding and check it in that same transaction, apply the existing operation
policy, and hold bounded admission through blocking completion. `StateScope`
validates identity syntax only; it must come from kernel-owned worker identity,
not the App's arguments. This generic SQL layer alone is not the SDK broker or
a publisher-revocation boundary.

Ordinary `node:fs` writes are independent. Managed-state snapshots/migrations
must fence writers and restore values through a trusted transaction while
preserving monotonic revision allocation and non-rollbackable delivery
receipts. That lifecycle integration is not implemented here. Uninstall and
data-retention policy belong to the installer; this store does not silently
delete user data when metadata becomes inactive.

Six real SQLite tests cover competing connections, owner isolation, reopen,
delete/recreate versions, savepoint/outer transaction failure, generation and
schema changes, exact key/payload ceilings, retained data after rejected writes,
and revision exhaustion. Metadata-only fixture activation tests SQL behavior;
it does not represent signed package, runtime, or release acceptance.
