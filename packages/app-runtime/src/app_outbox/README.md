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

The Rust configuration API accepts explicit trusted kernel operations. The kernel
resolves existing workflow publication, endpoint and queue ownership, retains its
workflow/session guards, and checks those exact normalized records in the same
writer transaction as configuration CAS. Every change increments the binding
revision; reactivation repeats target authorization. Deactivation cannot grant
active access. No public App/terminal permission request path is provided here.
The SDK 0.4 event declaration supplies signed `schemaVersion` and `direction`
(`outgoing`, `incoming`, or `both`) and requires kernel protocol 292. Only
outgoing/both declarations enter this automation catalog; incoming handlers have
a separate readiness contract. Both the durable binding and each occurrence
must match the signed version. Directly seeded target rows in storage
fixtures are not installation or consent evidence.

`apply_current_in` resolves current automation revisions on the same writer
transaction and atomically accepts at most 16 occurrences across all automation
IDs with one 512 KiB body ceiling. `validate_occurrences` performs the same
structural limits before writer admission without granting schema/target access.
The lower-level `apply_in` retains one-automation savepoint composition. The occurrence key is owner,
installation, automation, event version, App occurrence ID and optional schedule
revision. Scheduled bindings require that revision; ordinary bindings reject it.
Generation and automation revision fence admission and delivery; they are excluded from dedupe
identity. An exact retry returns the original receipt. Changed payload or signed
schema conflicts with the existing receipt. The canonical content digest includes
the internal envelope contract, event name, event version, schema digest,
original occurrence timestamp, schedule revision, payload and canonical invocation.
Changing the prompt or artifact metadata conflicts too. The original
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
and 64 KiB encoded bytes. Each required invocation supplies a nonblank prompt
of at most 64 KiB and up to 32 artifact metadata records, with a 256 KiB total
canonical invocation limit. The 512 KiB batch and 16 MiB retained body limits
count both payload and invocation bytes. Per installation
there are at most 256 retained automation records, 4,096 retained receipts,
1,024 pending occurrences and 16 MiB of retained bodies. These are logical
record limits, not a total SQLite/WAL/filesystem quota or aggregate host budget.

Pending candidates exclude paused bindings and are returned in acceptance order, at most 64 per read. The
consumer rechecks admission, binding revision and receipt revision at transition.
Retry times must advance, remain inside the accepted occurrence's seven-day
pending lifetime, and allow at most eight attempted queue admissions. Exhaustion
requires the kernel to classify the receipt failed rather than loop. Expiration
uses original kernel acceptance time; it makes no claim about App schedule time.

Artifact `reference` strings are untrusted metadata. They confer no host file,
network, credential, provider attachment, or other-installation asset access.
The handoff maps them only into the existing workflow metadata model. Actual
content delivery requires an installation-scoped export/import grant resolver;
that Phase 1 integration remains outstanding. The kernel must use `app_event`
transport, so an App automation ID cannot name a legacy event binding and inherit
its reply/context/action capabilities.

Protocol 290 pending rows have no invocation. Initialization marks those rows
failed, releases their retained payload, and preserves receipt IDs and digests.
It never infers a prompt from domain payload, and those old rows cannot starve
new pending pages. Queue/retry transitions independently reject missing bodies.

`mark_queued_in` must run in the transaction that persists the existing workflow
queue item and its delivery receipt. The outbox never constructs the workflow
prompt or chooses another endpoint. Its tests use a small SQL queue fixture to
exercise rollback composition. Kernel tests additionally compose the real normalized
workflow writer, but require the hosted kernel validation gate.
`reconcile_retry_in` only acknowledges an existing kernel-classified, due,
unexpired retry under current authority. It never resets attempts or backoff.
Only after that combined commit may the existing workflow dispatcher run.
`queued` means that durable queue handoff; `delivered` records a later trusted
WorkflowRun association, not semantic success of an App task. The normal writer
checks the exact persisted run, kernel-owned `app_event` envelope and original
`queued_session_id`/`queued_prompt_id` in the same transaction. Existing queue
remove/clear transitions classify their removed, previously Queued items failed
unless that transition contains the corresponding run. Missing records during
restart alone are not cancellation evidence. Terminal receipt
states cannot return to pending. Updates and uninstall must preserve these rows;
uninstall disables automations and does not delete workflows.

New occurrences must have a JavaScript-safe original `occurredAtMs`, no more
than 30 days old and no more than five minutes ahead of kernel time. SDK
`occurrenceId(sourceKey, occurredAtMs)` (Rust `occurrence_id`) returns
`evt1.<canonical decimal time>.<lowercase SHA-256>`. The hash covers UTF-8 JSON
`["chariox.app-occurrence-id.v1", sourceKey, occurredAtMs]`; a source key is
nonempty, well-formed Unicode and at most 4096 UTF-8 bytes. The runtime requires
the ID's embedded time to equal `occurredAtMs`. Keep the original ID/time across
retries; a different ID denotes a different occurrence and grants no authority.
Schedule revisions distinguish successive scheduled occurrences; binding and
generation checks do not infer App-domain schedule cancellation.

Each owner/installation has a durable replay floor. It advances to the maximum
of its previous value and kernel time minus 30 days. Exact existing receipts
are checked first, retaining duplicate/conflict semantics below the floor.
Absent identities older than the floor are rejected. Kernel housekeeping removes
at most 256 terminal receipts per transaction, only when original time is strictly
below that floor and at least 30 days have elapsed since acceptance. Floor and
deletions commit together. Queued work is never deleted by age alone. Pending
work expires after seven days. Unsupported protocol 291 arbitrary-ID pending
receipts become failed without inventing IDs or prompts; their immutable digest
remains until normal terminal reclamation is eligible.

Clock rollback never lowers the durable floor. A large forward clock error can
therefore reject new work until corrected time catches up; the kernel must not
silently reopen an old replay window. The 4096 receipt ceiling remains an explicit
rolling rate budget (roughly 136 accepted events/day over a full 30-day window),
with backpressure when full. Exact retained retries still resolve. This is bounded
retention, not unlimited throughput; representative Slack load must inform any
future capacity change.

The kernel pump retains one shared background pass and its App admission permit
through blocking completion. Current worker/catalog leases gate new queue work;
no foreground App view is required. Every pass has a monotonic one-second minimum
interval, including backlog and coalesced wakes. Each pass attempts at most eight
workflow dispatch sessions and preserves its rotating cursor; a blocked Ready
entry does not rewrite its unchanged turn or repeat the workspace-conflict notice. New queue retries use 1, 2, 4, 8, 16, 32 and 60-second
backoff, with at most eight admissions. Housekeeping rotates inactive installations
as well. Exact existing queued records recover the commit-to-dispatch interruption;
a durable cursor avoids ambiguous old rows starving later work. Legacy missing
queue-session links can be recovered only from one exact stored queue envelope.
The existing workflow scheduler still owns provisioning and dispatch;
its synchronous provisioning cannot be interrupted mid-call by the App admission
deadline. The common queue selector first commits the Ready run, its original queued envelope,
and a kernel-generated entry intent in the same writer transaction; only then is
that session projection published. Pending intents remain independently discoverable
after the App receipt becomes `delivered`. The first prompt uses the existing durable
operation dedupe, scoped to its exact workflow run/node. Its actual prompt-state
event atomically records submission, so prompt completion before the caller receives
its reply cannot cause a second entry. Turn preparation is persisted before prompt
admission. Before admission, failed scheduling retries the same Ready run; definite
agent validation errors retain normal terminal cleanup. A failure after prompt
acceptance fences the shared writer and scheduling/projection admission; restart
uses existing accepted/uncertain prompt recovery. An already executing commit may
finish before the fence joins the writer, and the retained intent/prompt makes
that state recoverable. Submission receipts follow actual workflow-run retention,
not a timer. These kernel fault/restart tests require the hosted gate; the 29
SQLite outbox component tests do not establish provider recovery acceptance.

The existing kernel event adapter and normalized workflow writer are the intended
downstream authority. Public automation management, production activation/pump
validation, App inbox/source generation fencing, schedule supersession, and the AEGS/AEDS
migration remain separate integration work.
