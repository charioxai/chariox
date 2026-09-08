# SDK structured state delegate

`AppStateBroker` accepts only `state.get` and `state.transaction` from the
kernel's common worker broker. It retains the admitted worker's trusted owner,
verified catalog, existing durable store and AppControl's shared eight-operation
admission semaphore. Request fields cannot supply or replace those identities.
It does not create a listener, acknowledge worker readiness, enroll a publisher,
or implement another runtime authority.

The decoder follows the current SDK declarations exactly. `checks` and `writes`
are required; a check's `version` is required and may be null to assert absence.
A write is exactly a value (including null) or `delete: true`, never both. Unknown
fields and methods, duplicate keys in change sets, invalid versions, and managed
state size/count limits are rejected. The inherited wire decoder already rejects
duplicate JSON fields, invalid Unicode and unsafe/nonfinite numbers.

An absent or empty `occurrences` array is accepted. Any nonempty array returns
`UNSUPPORTED_OPERATION` before mutation until the structured transaction and
durable outbox are integrated. No occurrence is silently discarded or reported
accepted. Successful state-only transactions return `{revision, receipts: []}`;
reads return null or `{value, version}`. Errors contain fixed, redacted messages.

Each operation captures its monotonic deadline and cancellation from the peer's
trusted `BrokerRequest`, then obtains a shared permit without an extra waiting
queue. The blocking closure owns the permit through durable-writer completion,
even if its async caller is dropped. The writer checks the budget before queuing,
on dequeue and after SQLite writer-lock acquisition, and checks the catalog's
current signer/owner/generation in the same transaction as the state operation.
Cancellation after transaction admission does not promise rollback.

Tests include strict decoding and real duplex peer calls using a signed fixture
catalog, enrolled publisher and kernel SQLite writer. The contention test holds
a real external SQLite writer lock; a test-only observer preserves the production
budget while locating the kernel immediately before BEGIN. It cancels the actual
IPC call, verifies shared admission remains held, releases the lock and verifies
the expired operation did not mutate state. These tests do not launch an App or
prove production worker startup, complete broker policy or durable event delivery.
