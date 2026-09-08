# Internal App worker protocol v1

This is an internal adapter on the single inherited duplex worker channel. It
does not create an external MCP server or another terminal protocol. The kernel
App supervisor supplies the installation generation and binds the channel to one
worker. A worker-supplied generation is checked for equality, not trusted as an
authorization claim.

SDK 0.4 event payloads require kernel protocol 292. `eventVersion` must match
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

The optional context includes kernel-assigned installation, Room, operation,
actor, agent, task and turn references. It is distinct from App tool parameters.
The receiver supplies a local `AbortSignal` and effective `deadlineMs` to the
handler. Context cannot establish a human approval; the trusted kernel checks an
operation's exact approval receipt before its protected effect.

Wire `event` messages are supervisor control notifications and their names begin
with `supervisor.`. They never carry unacknowledged App business occurrences.
Occurrence acknowledgment is a normal request response. Durable receipt and
workflow-enqueue deduplication are kernel responsibilities, not in-memory SDK
deduplication.

## Worker-to-supervisor requests

`worker.ready` reports `{tools:string[],events:string[],lifecycle:string[]}` after
registration. The trusted bootstrap's `declarations.incomingEvents` contains
only signed `incoming`/`both` event names; `outgoing` declarations require no
handler. The supervisor compares the readiness event list to that incoming set,
and admits only `outgoing`/`both` events to automations. A readiness
request is not evidence that the OS sandbox is active; only the trusted bootstrap
can establish that fact before loading App code.

Other typed methods use the names and parameter shapes documented in
`src/index.d.ts` and `src/index.js`: `state.*`, `files.*`, `events.*`, `http.request`,
`log.write`, `host.*`, `validation.*`, and `outputs.*`. Unsupported operations
return a typed error. The supervisor owns capability checks and rejects unknown
or undeclared effect routes; it never dispatches arbitrary kernel method names.

`validation.request` and `outputs.request` return durable pending references when
waiting for human input or model output. They must not retain a worker request
slot for the duration of that wait. App-specific meaning stays in App handlers
and validators; the kernel performs generic authorization, correlation, schema
and delivery checks.

The shared JSON vectors in `test/wire-vectors.json` contain `generation` and
`cases` with `name`, `sender`, `valid` and `message`. Both implementations run this
corpus, while their stream tests separately exercise binary framing and bounds.
`rawCases` use a `json` string instead of `message` to retain duplicates, number
spellings and invalid surrogate escapes that object parsing would erase.
