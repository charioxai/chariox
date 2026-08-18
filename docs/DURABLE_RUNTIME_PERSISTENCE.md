# Durable Runtime Persistence

## Purpose

Kernel readiness must depend on current runnable state, not on the number of
workflow runs accumulated over the lifetime of a session. Durable persistence is
therefore split into three classes:

| Class | Examples | Startup behavior |
| --- | --- | --- |
| Hot runtime state | Workflow definitions, active runs, queues, bindings, schedules, agents | Restored before `runtime_ready` |
| Delivery safety state | Pending deliveries and unexpired idempotency receipts | Restored before `event_ready` |
| Historical state | Completed runs, node-run history, transcripts and recall indexes | Queried by indexed, paginated read paths |

The operational transcript/history databases remain lazy read models. They are
not scanned to reconstruct runtime state or allocate prompt IDs.

## Storage contract

`durable_workflow_hot_entities` stores independently keyed workflow entities.
Ordinary workflow transitions upsert changed values and remove entities that are
no longer present. The accompanying `workflow.runtime.updated` journal entry is
metadata only and never embeds a `RuntimeSession`.

`durable_workflow_runs` stores active and terminal workflow runs by owner,
session, workflow, status and creation time. Only non-terminal runs are restored
into `RuntimeSession`. Terminal runs remain available through the existing Runs
list/get APIs, which merge hot runs with keyset-paginated durable history.

`durable_event_delivery_receipts` stores delivery/idempotency receipts by unique
delivery ID. Expired rows are removed during bounded receipt writes and are not
restored.

Checkpoint payloads contain hot entities only. Checkpoint policy is governed by
all of:

- changed event/entity count;
- encoded event-tail bytes;
- elapsed time since the last checkpoint;
- a hard post-checkpoint tail-byte limit.

Writing a checkpoint never serializes completed workflow history back into the
hot session aggregate.

## Readiness phases

The kernel binds its configured TCP listener immediately after configuration is
loaded. Until the runtime router can adopt that same socket, HTTP probes receive
`503 Service Unavailable` with:

```json
{"phase":"booting","runtime_ready":false,"event_ready":false}
```

`runtime_ready` means execution state, queues and active runs are safe to use.
`event_ready` is separate: it is emitted only after the configured AEDS connector
has reconciled event routes and delivery recovery. A kernel without configured
AEDS reports event delivery as not configured rather than claiming readiness.

## Legacy migration

On the first start against legacy snapshots:

1. restore the legacy checkpoint/tail under the normal owner boundary;
2. materialize workflow definitions, queues, bindings, receipts and active runs
   into normalized hot tables;
3. archive terminal runs out of the live `RuntimeSession`;
4. accept runtime traffic on the already-bound listener;
5. copy terminal history in idempotent 256-run background chunks;
6. mark `workflow_history_migration_status=verified` only after the final chunk.

Chunk inserts use stable run IDs and `ON CONFLICT DO NOTHING`, so interruption and
replay are safe. Legacy session events and snapshots are retained until the
verified marker exists. The workflow runtime storage version is recorded
separately, allowing a failed upgrade to retain its rollback source.

## Disk maintenance

Kernel startup never runs `VACUUM`, `VACUUM INTO`, or any operation that requires
a second copy of the database. New state databases enable SQLite incremental
auto-vacuum before schema creation. After a background history migration, the
kernel may reclaim at most 512 already-free pages. The operation is skipped for
legacy databases that were not created in incremental mode and never blocks
listener readiness.

For an older large database, logical compaction still bounds future restore and
growth even when physical file size cannot shrink automatically. Operators should
not run a full vacuum without a backup, an explicit maintenance window and enough
free space for SQLite's copy requirements.

## Configuration

The `[state]` checkpoint controls are:

- `snapshot_interval_events` (default `1000`)
- `snapshot_interval_bytes` (default `4194304`, 4 MiB)
- `snapshot_interval_seconds` (default `300`)
- `snapshot_max_tail_bytes` (default `16777216`, 16 MiB)

The maximum tail must be greater than or equal to the normal byte interval. Prompt
IDs use a small durable counter file under the kernel state root and reserve IDs
in blocks, avoiding an operational-history maximum scan at startup.

## Operational checks

Startup logs use structured `daemon.startup` and `durable_state.restore` records.
Track at least:

- `phase` (`booting`, `runtime_ready`, `event_ready`)
- process elapsed and bootstrap time
- snapshot restore, event replay and reconciliation time
- replayed event count
- migrated historical run count and migration status
- checkpoint tail count/bytes and post-checkpoint bytes
- incremental-reclaim support and free pages before/after

Investigate growth when a workflow transition payload contains a full `session`
field, when terminal runs appear in restored hot sessions, or when post-checkpoint
tail bytes exceed the configured hard limit.
