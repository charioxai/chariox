# App occurrence persistence

This component borrows the existing kernel SQLite connection. It
does not run a workflow, grant an automation, invoke an App, acknowledge AEDS,
or open another database. Its receipt values remain provisional until the
enclosing kernel writer transaction commits.

`EventCatalog::compile` retains payload schemas from an offline verified package
whose digest exactly matches the existing `AppCatalog`. `VerifiedAutomation`
loads an owner/installation-scoped durable binding and checks its exact revision,
event name, signed schema version, and schema digest. Each acceptance or delivery
mutation repeats the current catalog's active installation, generation and publisher revision checks
inside the same transaction. Changing any retained binding field invalidates a
previous snapshot, even if an erroneous configuration writer omitted its revision
bump. Loading a record checks existing authority; it grants none.

The automation table intentionally has no public enrollment API. The SDK 0.2
event declaration supplies a signed `schemaVersion`; both the durable binding
and each occurrence must match it. Target selection and automation authorization
remain kernel responsibilities. Test fixtures seed target records directly;
they are not installation or consent evidence.

`apply_in` accepts at most 16 occurrences for one admitted automation in a
savepoint. A larger state transaction may compose multiple such batches, but its
kernel decoder must enforce a total request bound. The occurrence key is owner,
installation, automation, event version, App occurrence ID and optional schedule
revision. Scheduled bindings require that revision; ordinary bindings reject it.
Generation and automation revision fence admission and delivery; they are excluded from dedupe
identity. An exact retry returns the original receipt. Changed payload or signed
schema conflicts with the existing receipt. The canonical content digest includes
the internal envelope contract, event name, event version, schema digest,
original occurrence timestamp, schedule revision and payload. The original
digest remains after payload compaction.

The recorded acceptance generation is audit data. After an installation update,
its currently admitted catalog may continue unchanged automation bindings and
their pending receipts. Current signer, catalog, schema and binding checks still
apply; the older worker generation itself has no continuing authority.

The caller composes `ManagedStateStore::apply_in` and outbox acceptance inside one
writer transaction, propagates either error, and acknowledges only after commit.
The savepoint prevents an outbox batch from leaking its earlier occurrences if a
later occurrence conflicts. It does not roll back a state mutation outside that
savepoint when a caller deliberately ignores the error and commits anyway.

Payloads are closed-schema JSON with at most 32 container levels, 16,384 nodes
and 64 KiB encoded bytes. One batch has a 512 KiB payload ceiling. Per installation
there are at most 256 retained automation records, 4,096 retained receipts,
1,024 pending occurrences and 16 MiB of retained payload. These are logical
record limits, not a total SQLite/WAL/filesystem quota or aggregate host budget.

Pending candidates are returned in acceptance order, at most 64 per read. The
consumer rechecks admission, binding revision and receipt revision at transition.
Retry times must advance, remain inside the accepted occurrence's seven-day
pending lifetime, and allow at most eight attempted queue admissions. Exhaustion
requires the kernel to classify the receipt failed rather than loop. Expiration
uses original kernel acceptance time; it makes no claim about App schedule time.

`mark_queued_in` must run in the transaction that persists the existing workflow
queue item and its delivery receipt. The outbox never constructs the workflow
prompt or chooses another endpoint. Its tests use a small SQL queue fixture to
exercise rollback composition; actual workflow integration remains untested.
Only after that combined commit may the existing workflow dispatcher run.
`queued` means that durable queue handoff; `delivered` records a later trusted
delivery acknowledgement, not semantic success of an App task. Terminal receipt
states cannot return to pending. Updates and uninstall must preserve these rows;
uninstall disables automations and does not delete workflows.

New occurrences must have a JavaScript-safe original `occurredAtMs`, no more
than 30 days old and no more than five minutes ahead of kernel time. Exact known
duplicates return their original receipt even after that window; changing the
timestamp for a retained identity conflicts. Schedule revisions distinguish
successive scheduled occurrences; the kernel's binding revision and generation
checks do not infer App-domain schedule cancellation.

This module never deletes receipt/digest tombstones, including expired ones.
At the receipt limit, new occurrences get backpressure while exact retries
continue to resolve. A future retention policy must establish that a pruned
occurrence cannot be replayed before reclaiming those identities, and recheck
admission ordering around the timestamp cutoff before introducing deletion.
Capacity-limited permanent retention is a foundation, not a complete production
retention policy.

The existing kernel event adapter and normalized workflow writer are the intended
downstream authority. Broker registration, automation management, per-kernel pump,
App inbox/source generation fencing, schedule supersession, and the AEGS/AEDS
migration remain separate integration work.
