# SDK storage delegate

`AppStorageBroker` accepts `state.get`, `state.transaction`, `events.emit`,
`events.status` and `events.retry` from the kernel's common worker broker. It
retains the admitted worker's trusted owner, one verified `EventCatalog` (which
owns the exact underlying `AppCatalog`), the existing durable store and
AppControl's shared eight-operation admission semaphore. Request fields cannot
replace those identities. The delegate creates no listener, acknowledges no
worker readiness and enrolls no publisher.

The decoder follows SDK0.3/protocol291 shapes. Checks and writes are required;
a check version may be null to assert absence. A write is a value (including
null) or `delete: true`, never both. Occurrences require original time, signed
event version, payload and canonical invocation. Artifact references are untrusted
metadata, never file paths to open, URLs to fetch or attachment grants to adopt.
Unknown fields, unsafe numbers, incomplete bodies and bounded collection/data
limits are rejected before blocking admission. The inherited wire parser rejects
duplicate JSON fields and invalid Unicode.

The existing writer holds one IMMEDIATE transaction for state changes and every
occurrence, including different automation IDs. It loads actual automation
revisions/targets from its owner-scoped database and repeats signed catalog and
schema fences. A later event conflict or SQL failure rolls back earlier events
and the state mutation. All acknowledgement follows the outer commit. State
transactions return `{revision, receipts}`; receipts contain only `{receiptId,
state}`. Status never reveals another owner's receipt or the stored invocation.

Retry only reconciles a due, unexpired, kernel-classified retryable receipt whose
automation revision and signed schema are still current. It preserves receipt
identity, attempts and backoff. It neither reopens terminal work nor claims a
workflow was dispatched; the separate kernel pump handles durable queue handoff.

Every operation captures the peer's monotonic deadline and cancellation, then
obtains a shared permit without another waiting queue. The blocking closure owns
the permit through writer completion even if its async caller is dropped. Budget
checks occur before enqueue, on dequeue and after SQLite writer-lock acquisition.
Cancellation after transaction admission does not promise rollback.

Tests use actual duplex peer calls, a verified signed fixture catalog and the
existing SQLite writer. They cover multi-automation atomic rollback, receipt
scope/revocation, retry reconciliation and cancellation during a real external
SQLite lock while retaining shared admission. Trusted fixture automation rows
are not evidence of user consent or workflow authorization. These tests do not
launch an App or prove production worker activation or end-to-end workflow delivery.
