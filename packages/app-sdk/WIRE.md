# Internal App worker protocol v1

This is an internal adapter on the single inherited duplex worker channel. It
does not create an external MCP server or another terminal protocol. The kernel
App supervisor supplies the installation generation and binds the channel to one
worker. A worker-supplied generation is checked for equality, not trusted as an
authorization claim.

SDK 0.5 requires kernel protocol 293 and adds `health_check` to the declared
lifecycle names in `worker.ready` and `lifecycle.dispatch`. It uses the existing
request/response framing; the kernel dispatches this bounded local check before
acknowledging readiness, with broker effects still denied. An absent handler
returns `null`; a failed handler returns the existing redacted error shape.

`eventVersion` must match
the publisher-signed event `schemaVersion`. An outgoing occurrence contains
`automationId`, `occurrenceId`, `eventVersion`, `occurredAtMs`, `payload`, `invocation`, and
`scheduleRevision` for a scheduled automation. Preserve the original timestamp
and revision in the App's durable outbox; a retry must not mint a newer envelope.
`occurrenceId(sourceKey, occurredAtMs)` returns
`evt1.<canonical decimal timestamp>.<64 lowercase hex SHA-256 digits>`. The digest
input is UTF-8 `JSON.stringify(["chariox.app-occurrence-id.v1", sourceKey, occurredAtMs])`.
The source key is nonempty, well-formed Unicode of at most 4096 UTF-8 bytes; the
timestamp is a nonnegative safe integer. The encoded timestamp must equal
`occurredAtMs`; leading zeros, exponent notation and changed timestamps are
invalid. Persist the ID with the original envelope. It is a replay identity,
not a grant or an assertion that the kernel understands the source event.
The required invocation is `{prompt, artifacts}`. Artifact metadata uses
`{name, mediaType, reference, sizeBytes?, digest?}`; references grant no host file,
network, credential or kernel asset access. Unknown fields and explicit null
optional fields are rejected. Payload/invocation limits are 64/256 KiB encoded;
prompts are at most 64 KiB UTF-8, with 32 artifacts of bounded metadata. A batch
has at most 16 occurrences and 512 KiB combined payload/invocation bytes.
The kernel derives installation, owner and current automation target itself.
The shared payload fixture is `test/event-contract.json`. Framing remains v1.

Each frame contains an unsigned four-byte big-endian body length followed by
exactly that many UTF-8 JSON bytes. Length excludes the header. Zero-length and
bodies larger than 1,048,576 bytes are rejected before allocation. Invalid UTF-8,
truncated frames, invalid envelopes and wrong generations permanently close the
channel. A timed-out partial write must never be resumed as a new frame.

JSON has at most 64 nested objects/arrays, counting the envelope as the first
container. Duplicate keys are rejected, including escaped spellings of the same
key. Strings and keys contain valid Unicode scalars. Numbers are finite and
integer values stay within JavaScript's safe integer range; larger identifiers
must use strings. These rules prevent silent changes when Rust and JavaScript
exchange values.

| Field | Contract |
| --- | --- |
| `version` | Integer `1` |
| `generation` | The exact active generation assigned to this worker |
| `kind` | `request`, `response`, `cancel` or `event` |
| IDs/names | Nonempty strings, at most 128 UTF-8 bytes, no Unicode control, whitespace or BOM characters |
| `deadline_ms` | Absolute Unix milliseconds, integer `1..9007199254740991` |
| `params`, `result`, `data` | JSON values, including explicit `null` |
| `context` | Optional object on supervisor requests only; forbidden on worker requests even when null |

The SDK checks every value App code gives it before writing any byte, so an
App mistake fails one call and never the channel. A handler result that is not
JSON under these rules, such as an object with an `undefined` field, a
function, symbol, BigInt, NaN, a Date or a cycle, or a response over the frame
limit, is answered with the error `INVALID_OUTPUT`, whose message names where
the value is (for example `result.echoed is undefined`). Once the worker is
ready, the SDK also writes that to the App's log with `log.write`. An `AppError`
whose code the wire refuses becomes `HANDLER_FAILED`, logged the same way. Only
a value refused before any byte is written fails one call; any other transport
failure closes the channel. SDK call parameters that are not JSON
reject that call with `INVALID_ARGUMENT` before it is sent. The SDK does not
drop `undefined` fields as `JSON.stringify` would: Rust and JavaScript must
see the same value. Invalid frames from the other side still close the channel.

Envelopes reject unknown fields. Error objects allow only `code`, `message` and
optional boolean `retryable`. `code` follows the identifier rule; `message` is
at most 4096 UTF-8 bytes. A response contains exactly one of `result` and `error`.
Examples omit no required fields:

```json
{"version":1,"generation":"generation-7","kind":"request","id":"sdk-1","method":"state.get","params":{"key":"project"},"deadline_ms":2000000000000}
{"version":1,"generation":"generation-7","kind":"response","id":"sdk-1","result":null}
{"version":1,"generation":"generation-7","kind":"response","id":"sdk-1","error":{"code":"DENIED","message":"Request denied","retryable":false}}
{"version":1,"generation":"generation-7","kind":"cancel","id":"sdk-1"}
{"version":1,"generation":"generation-7","kind":"event","name":"supervisor.stopping","data":null}
```

Every sender correlates responses against its own pending request table. SDK IDs
contain a per-process random UUID and monotonic counter. Unknown or late replies
are ignored without allocating new state. A duplicate active inbound ID is a
protocol violation. Cancellation names the peer's active request and may arrive
after completion. It sends at most one terminal response. Neither cancellation
nor timeout implies that an already-accepted external effect was undone.

Default SDK bounds are 64 outgoing requests, 16 executing inbound handlers,
2,097,152 queued write bytes, and 128 queued frames. Admission fails immediately
instead of adding an unbounded work queue. A handler that ignores cancellation
keeps its execution slot occupied. The host process supervisor enforces the
remaining execution and OS resource budget.

Per-call execution is capped at 30 seconds; the effective inbound deadline is
the earlier of `deadline_ms` and the current time plus 30 seconds. Frame assembly
and each queued write have a 30-second absolute deadline, beginning when the
first bytes arrive or the write is accepted. Additional fragments do not extend
the deadline. Tests may reduce limits. Unknown external outcomes are never
automatically retried by this adapter.

## Supervisor-to-worker requests

| Method | Parameters | Meaning |
| --- | --- | --- |
| `tools.invoke` | `{name,input}` | Invoke a package-declared tool after kernel schema/operation checks |
| `events.deliver` | `{name,occurrence_id,payload}` | Deliver one durably accepted occurrence; retries keep the same occurrence ID |
| `lifecycle.dispatch` | `{event,data?}` | `startup`, `suspend`, `resume`, `shutdown`, `prepare_update`, `configuration_change` |
| `schedule.wake` | `{id,dueAtMs,revision,overdue}` | Deliver one due kernel-owned wake at least once; `overdue` is true after sleep, shutdown or restart |

SDK 0.8 adds kernel-owned wakes. `schedule.set` `{id,dueAtMs,revision}`,
`schedule.cancel` `{id}` and `schedule.list` `{}` return the installation's
pending wakes `{wakes:[{id,dueAtMs,revision}]}`; `state.transaction` also accepts
`wakes: [{op:"set",id,dueAtMs,revision}|{op:"cancel",id}]`, committed atomically
with its writes. At most 256 wakes per installation and 16 changes per request.
The kernel starts a worker that is not running when a wake falls due, so an App
needs no resident process or long timer to wait. A user-stopped App keeps its
wakes until the user starts it again; a failed or revoked App's wakes back off.
Failed deliveries back off and are dropped after eight attempts; the App
reconstructs schedules from its state.

The optional context includes kernel-assigned installation, Room, operation,
actor, agent, task and turn references. It is distinct from App tool parameters.
An agent's call carries `agent_id` and, when the agent made it during a turn,
`turn_id`: the id of that turn's Chariox prompt, as session history records it.
It is captured when the call is submitted, so a later turn or focus change does
not re-attribute it.
Background work carries one too: `events.deliver` and `schedule.wake` have
`actor: {kind: "background", id: "inbox:<route>" | "schedule"}`, an operation per
delivery (`inbox-<sequence>`) or per wake firing (`wake-<id>-<dueAtMs>`, the same
across retries of that firing), and no Room, agent, task or turn. A route or wake
id that would break the context-id bounds (at most 128 bytes, no whitespace) keeps
its prefix with a digest in place of the id: `inbox:sha256-<hex>`, `wake-sha256-<hex>`.
The receiver supplies a local `AbortSignal` and effective `deadlineMs` to the
handler. Context cannot establish a human approval; the trusted kernel checks an
operation's exact approval receipt before its protected effect.

Wire `event` messages are supervisor control notifications and their names begin
with `supervisor.`. They never carry unacknowledged App business occurrences.
Occurrence acknowledgment is a normal request response. Durable receipt and
workflow-enqueue deduplication are kernel responsibilities, not in-memory SDK
deduplication.

An inbox route fed by an event generator connection (kernel protocol 358)
delivers `events.deliver` with this payload, which the App's signed incoming
schema must accept:

```json
{"source": {"generator_id": "dev.chariox.slack", "connection_id": "connection-…",
            "event_type": "app.mentioned", "event_type_version": 1},
 "occurred_at": "2026-09-26T19:00:00.000Z", "text": "…", "metadata": {},
 "artifacts": [], "reply_context": null}
```

`occurrence_id` is the source occurrence's id, so a redelivery is the same
occurrence.

`connections.list {}` returns `{connections: [{generatorId, connectionId,
actions}]}`: the event generator connections the owner granted this
installation (kernel protocol 359, `GrantAppConnection`), each with the actions
the signed manifest's `capabilities.connections` declares for that generator.
`connections.action {connectionId, action, input?, context?, idempotencyKey?}`
runs one such action through the generator's reviewed action endpoint and
replies `{accepted, result, idempotencyKey}`. `context` is a `reply_context` a
generator-fed occurrence carried; the generator binds it to the connection.
The App never holds the provider credential. A connection that was not granted
is `CONNECTION_NOT_GRANTED`; an undeclared action is `CAPABILITY_REQUIRED`.
Calls with the same `idempotencyKey` are performed once, so an App passes one to
retry safely; without it every call is a new action. When the request may have
reached the generator without a definite answer (a lost reply, or a 5xx from the
generator or a gateway in front of it), the call fails with
`APP_CONNECTION_OUTCOME_UNCERTAIN`: the kernel never replays it, and it is
retryable only when the App supplied `idempotencyKey`. Any other
`CONNECTION_ACTION_FAILED` is retryable unless the generator refused the action
itself.

## Worker-to-supervisor requests

`worker.ready` reports `{tools:string[],events:string[],lifecycle:string[]}` after
registration. The trusted bootstrap's `declarations.incomingEvents` contains
only signed `incoming`/`both` event names; `outgoing` declarations require no
handler. The supervisor compares the readiness event list to that incoming set,
and admits only `outgoing`/`both` events to automations. A readiness
request is not evidence that the OS sandbox is active; only the trusted bootstrap
can establish that fact before loading App code.

A worker started for a local update that raises the data schema version runs a
migration phase first: the bootstrap imports each pending `migrations/NNN.js`
step in order, calls its default export with `{from, to, state:{get, transaction}}`,
then sends `migration.step {to}`; the kernel answers `null` once it records that
step. Before `worker.ready` is accepted, the kernel admits only `state.get`,
`state.transaction` and `migration.step` from a migrating worker, scopes them to
the staged generation, and requires each transaction's `schemaVersion` to equal
the current step's `to` (the SDK always sends it). Every other method is denied.
Any failed step ends the worker with exit code 136 before App runtime code loads.
A developer writes steps as `migrations/001.js`, `002.js` and so on (`.mjs`
and `.cjs` are also accepted), with no gaps. `chariox app pack` then signs
them as the manifest's chain from schema 0, and the last number is the data
schema version that state transactions use. A manifest that declares
`migrations` itself is packed as written.

`events.automations {}` returns `{automations:[{automationId, event,
eventVersion, state, lastReceipt}]}` from the same kernel automation records the
TUI and web show. `state` is `active`, `paused`, `broken` or `disabled`
(`paused` is a stored state no current request sets; handle it like `disabled`);
`lastReceipt` is `{receiptId, state}` for the latest retained receipt or null.
It never names the target workflow, session or deployment. Kernels before this
call answer it as an unknown method (`METHOD_NOT_FOUND`), and an App runtime
enrolled before it has no `chariox.events.automations`; `sdkVersion` cannot tell
them apart, so feature-detect with `typeof chariox.events.automations ===
'function'`.

Other typed methods use the names and parameter shapes documented in
`src/index.d.ts` and `src/index.js`: `state.*`, `files.*`, `events.*`, `http.request`,
`log.write`, `host.*`, and `validation.*`. Unsupported operations
return a typed error. The supervisor owns capability checks and rejects unknown
or undeclared effect routes; it never dispatches arbitrary kernel method names.

`validation.request {action, parameters, operationId?}` accepts only actions
the signed package declares with `criticalValidation`; `parameters` must match
that action's `inputSchema` (at most 16 KiB) and are bound canonically (sorted
keys) with the installation, generation and action into a durable operation.
The reply `{operationId, state}` is `pending` at once; the kernel shows a
trusted approval outside App content (in the owner's most recent session, preferring one they host; one
operation per installation at a time; collaborators in a shared session see it,
but only the owner can answer; it names whom the App was working for: the
kernel's caller of each call the worker was handling when it asked, such as an
agent, the owner's view or a background delivery), and only the owner's answer
there moves it to `approved` or `denied` (undecided operations expire after 10
minutes, approvals must be used within 10 minutes and are single-use). Passing
an existing `operationId` returns it only for the identical binding.
`validation.status {operationId}` reads it for this installation only.
`connectionId` is not supported yet.
An approved operation is spent by `http.open {..., operationId}` on one of the
action's declared `POST`/`PUT`/`PATCH` effect routes (exact origin, method and
path) with `hasBody: false` and no header but `accept`. The kernel sends the
approved canonical parameters as the whole JSON body. Any other request to a protected origin fails
`VALIDATION_REQUIRED` or is denied.
Once spent, the operation no longer reads as approved: `validation.status` and a
re-request with its `operationId` fail `VALIDATION_CONSUMED` (not retryable), and
another attempt needs a new approval. After 24 hours a finished operation is gone
(`NOT_FOUND`).

The effect's outcome follows what the kernel's socket saw:
- A received response, any status (a gateway's `502` or `520` included), is the
  answer, passed through as for any request.
- Failing before any request byte could be written (DNS, connect, TLS) is a
  plain `APP_HTTP_NETWORK` or `APP_DEADLINE`: the effect was not sent, though its
  approval is spent.
- Once the request may have reached the origin, a lost reply (the connection
  reset or closed, or the kernel's inactivity or lifetime limit, before any
  response head) is `APP_HTTP_OUTCOME_UNCERTAIN`: the origin may have acted.
  Check the outcome with the service before requesting a new approval; the
  kernel never replays it.
- `APP_OPERATION_STOPPED` on an effect's stream after `http.open` (the kernel
  stopped it: a drain, an update, a stale installation) leaves the outcome just
  as unknown.
- `http.request` answers `APP_HTTP_OUTCOME_UNCERTAIN` for an effect when its own
  deadline passes, or the kernel stops the stream, after it sent `http.open` and
  before a response head: the kernel spends the approval before it answers the
  open. `validation.status` then tells: still `approved` means the kernel never
  started the effect (the same `operationId` can be used again),
  `VALIDATION_CONSUMED` means it may have reached the origin.
- The App's own abort (`CANCELLED`, through the call's `signal`) after `http.open`
  was sent is passed through as is, but it leaves an effect's outcome just as
  unknown; `validation.status` applies the same way.

`host.pick_file {multiple?, accept?}` (Apps whose signed manifest declares
`capabilities.externalFiles: ["user_selected"]`; others get
`CAPABILITY_REQUIRED`) replies `{operationId, state: "pending", grantIds: [],
expiresAtMs}` at once. `accept` holds up to 16 file name suffixes such as `.md`.
The kernel shows the owner a trusted prompt, like a validation. The owner
either declines or chooses files (at most 8, 512 KiB together) in their
terminal. The chosen bytes become grants; the App never learns a host path.
`host.pick_file_status {operationId}` returns the same shape. The state is
`pending`, `granted` (with `grantIds`), `declined` or `expired`: unanswered
after 10 minutes, or unimported 30 minutes after the grant, or when the App
updates. `files.import {grantId, destination}` copies one grant into private
data, like `files.atomic_replace`, and spends it. It replies `{bytesWritten,
name}`, where `name` is the file name the owner chose. The SDK's
`host.pickFile` polls the status and resolves with `{grantIds}`.
`files.export {path}` (same capability) copies one private regular file of at
most 512 KiB. It follows no links. The kernel offers it to the owner in the
same kind of trusted prompt, and the reply is `{operationId}` at once. The
owner saves it from their terminal, which chooses the location, or declines.
The owner can take an offer again until it ends: 10 minutes after the App made
it, or when the App is updated or uninstalled. A file name with control or
invisible format characters (such as bidi overrides) is refused with
`INVALID_ARGUMENT`.

`files.atomic_replace {path, contentsBase64}` writes at most 512 KiB to a new
private file and renames it over `path`, replying `{bytesWritten}`. When the
installation's private data volume has no space left (a fixed 512 MiB quota per
installation on Linux and macOS), it and `files.import` fail `APP_STORAGE_FULL`,
not retryable, and the message names the quota. Nothing is written; the App must
delete data before writing again. Other failures to write stay
`APP_FILE_UNAVAILABLE`, and `APP_FILE_OUTCOME_UNCERTAIN` means the rename may
have completed. An App's own `node:fs` writes to the full volume fail `ENOSPC`.

`files.snapshot {name, consistency}` (no capability needed) copies the
installation's private files, structured state and pending wakes into the
kernel's snapshot store, outside App data. The reply is `{snapshotId,
consistency, files, bytes}`. `name` is 1 to 64 characters from `A-Za-z0-9._-`,
and `consistency` is the label the snapshot keeps:
- `quiescent`: the kernel holds this worker's other state, event, schedule
  and file SDK calls (`state.*`, `events.*`, `schedule.*`,
  `files.atomic_replace`, `files.import`), reads included, until the copy is
  done; a held call can reach its deadline. The App must pause its own `node:fs` writes while it awaits the
  call.
- `crash_consistent`: nothing is held, so files may be copied mid-write.

Only the active generation can take one (`CONFLICT` while an update is
staged), and an installation's snapshots run one at a time. The copy follows
no links and skips special and hard-linked files, and entries that vanish
while it walks. A failed copy is `STORAGE_UNAVAILABLE` (retryable when it may
pass on retry) or `LIMIT_EXCEEDED`.
It is bounded like the data volume (10,000 files, 512 MiB) and needs host free
space beyond that (`LIMIT_EXCEEDED`). The kernel keeps the newest two per
installation. Outbox, inbox and validation receipts are never part of a
snapshot, so restoring one cannot replay or forget delivered work. Restoring
a snapshot into an installation is a kernel operation, not an App call.

`validation.request` and `host.pick_file` return durable pending references when waiting for human
input. They must not retain a worker request
slot for the duration of that wait. App-specific meaning stays in App handlers;
the kernel performs generic authorization, correlation and schema checks.

The shared JSON vectors in `test/wire-vectors.json` contain `generation` and
`cases` with `name`, `sender`, `valid` and `message`. Both implementations run this
corpus, while their stream tests separately exercise binary framing and bounds.
`rawCases` use a `json` string instead of `message` to retain duplicates, number
spellings and invalid surrogate escapes that object parsing would erase.
