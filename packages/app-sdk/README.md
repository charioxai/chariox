# Chariox App SDK

This package supplies the Node App backend interface and the internal worker IPC
adapter. The kernel owns authorization, persistence, event receipts, networking,
human validation and installation generations. An SDK method sends a bounded
request to that authority; it never opens the kernel's public socket or reads
kernel state itself.

The implementation is a foundation for Phase 1, not a complete App platform.
The trusted sandboxed worker bootstrap must create this SDK and inject it into
the App entrypoint after OS containment is established. This package does not
launch processes, claim sandbox enforcement or implement a privileged fallback.

## App entrypoint

The worker loads the immutable package after lockdown, then calls its entrypoint
with a frozen `AppBackendSdk`. Tool and event names come from the verified package
declarations. The kernel validates the declared input and output schemas on the
operation route; App handlers can perform their own business validation.

```js
export default function register(chariox) {
  chariox.tools.register('get_project', async ({ key }, context) => {
    return chariox.state.get(key, {
      signal: context.signal,
      timeoutMs: Math.max(1, Math.min(30_000, context.deadlineMs - Date.now())),
    });
  });

  chariox.events.register('issue_created', async (event, context) => {
    // Delivery is at least once. Commit deduplication/state/outbox together
    // through chariox.state; event.occurrenceId is stable across redelivery.
    await handleIssue(event, context);
  });
}
```

The trusted bootstrap calls `ready()` after registration. Readiness reports the
registered declarations and seals registration; it does not prove containment.
The App receives neither `ready` nor `close`. Its one entry export is the
registration function: `export default register` for ESM, or
`module.exports = register` for CommonJS. Registration may return a promise;
all declared tool and incoming event handlers must exist when it settles. Signed
event declarations specify `direction: "outgoing" | "incoming" | "both"`.
Outgoing-only events require no handler. The verified
manifest's `runtime.entry` selects this module. No App server port or separate
MCP transport is required. See the [trusted bootstrap contract](../../apps/app-worker/src/bootstrap.md).
Lifecycle handlers are optional, serialized, and deadline-bound. Tool and event
handlers have separate names, and every delivered occurrence requires a reply.
Wire control events never dispatch App event handlers.

## Supplied APIs

- `tools.register`, `events.register`, `lifecycle.on`: local handler registration.
- `state.get`, `state.transaction`: structured state and atomic outgoing occurrences.
- `files.atomicReplace`, `snapshot`, `import`, `export`: managed private-file operations.
- `events.emit`, `status`, `retry`: durable occurrence operations. The kernel alone
  decides whether a receipt can be retried.
- `http.request`: bounded HTTP broker request with optional opaque connection and
  operation references. There is no direct network fallback.
- `host.notify`, `openLink`, `writeClipboard`, `pickFile`: authorized host actions.
- `log.write`: bounded structured logging through the kernel's App log policy.
- `validation.request`, `status`: a pending operation reference returns promptly;
  an App cannot approve it or claim that an App-view click was human validation.
- `outputs.request`, `cancel`: requests for declared information sets; the kernel
  must check explicit consent and capture the task/agent/turn before delivery.
- `paths`: the read-only package and private writable data/temporary roots supplied
  by the trusted worker. Ordinary private I/O continues to use `node:fs`.

The SDK does not expose transcripts, a second prompt area, provider credentials,
raw kernel requests, or an approval-resolution method. Conversation placement
belongs to the separate App-view/terminal integration.

Inline file and HTTP bodies currently support at most 512 KiB. The streaming,
multipart, SSE and global `fetch` adapter required by Phase 1 still need the
kernel stream broker contract and its transport conformance tests. This package
does not label the bounded `http.request` interface as fetch-compatible. Typed
workflow/agent asset methods and consented output callback registration also
remain integration work. No broker operation is complete merely because its
forwarding method exists here.

## Worker integration

```js
import { createAppSdk } from '@chariox/app-sdk';
import { inheritedTransport } from '@chariox/app-sdk/internal';

// These values must come from the trusted supervisor/bootstrap, not App config.
const chariox = createAppSdk({
  transport: inheritedTransport(3),
  generation,
  paths,
  declarations: { tools: verifiedToolNames, incomingEvents: verifiedIncomingEventNames },
});
await register(chariox);
await chariox.ready();
```

`inheritedTransport` accepts an already-inherited duplex descriptor, never a
socket address. Tests can instead supply the small `AppTransport` interface.
The private descriptor is not a security boundary inside the worker: malicious
App code can bypass the SDK, so the trusted supervisor must validate and bound
every frame and infer installation/caller authority from its own operation state.

An outgoing request timeout cancels local waiting and sends cancellation. It
does not roll back a service effect and never triggers an automatic retry. A
cancelled inbound handler retains its concurrency slot until it actually ends;
the supervisor must terminate a worker that ignores its execution budget. An
infinite JavaScript loop cannot be contained by JavaScript timers.

SDK 0.5 adds the optional `health_check` lifecycle callback. The kernel runs it
before first-install activation, with a maximum three-second budget. Use it for
local checks of the loaded code and declared configuration; all kernel broker
effects remain unavailable during this check (`APP_NOT_READY`). An absent
handler succeeds with `null`, and a throwing handler fails the check. The kernel
owns readiness acknowledgment and activation. `startup` remains a separate
post-activation callback with its existing ten-second lifecycle budget.

## Validation

SDK 0.5 requires kernel protocol 293, signed event direction/schema version and
the complete `invocation: {prompt, artifacts}` alongside each occurrence's
`payload`. Persist that invocation, `occurredAtMs`, occurrence ID and scheduled
`scheduleRevision` together; exact replays preserve all of them. Artifact entries
use `{name, mediaType, reference, sizeBytes?, digest?}` and remain untrusted
metadata. They confer no file, network, credential or kernel attachment access.
The kernel validates the signed schema and current automation itself.

Create an outgoing identity with the named `occurrenceId(sourceKey, occurredAtMs)`
export. Use the original source identity and timestamp, then persist and reuse
the result; calling it with a new timestamp creates a different occurrence.
The source key is a nonempty Unicode string of at most 4096 UTF-8 bytes. The ID
has the form `evt1.<original timestamp>.<SHA-256 digest>`. The kernel rejects a
timestamp that differs from the ID, so an old ID cannot be refreshed after
receipt cleanup. This identity is not an authorization or proof of the event's
domain meaning.

Payloads have a 64 KiB encoded limit; prompts have a 64 KiB UTF-8 limit;
invocations have a 256 KiB encoded limit and at most 32 artifacts. A state
transaction has at most 16 occurrences and 512 KiB combined payload/invocation
bytes, including occurrences for different automations. State and all receipts
commit together. `events.retry` reconciles a current, due, kernel-classified
retryable receipt; it never resets attempts, advances backoff or restarts terminal
work. Actual workflow handoff remains a separate kernel operation.
The current outbox component accepts new occurrences up to 30 days old with at
most five minutes of future clock skew. Already retained exact duplicates return
their existing receipt; changed content conflicts. Terminal cleanup requires the
kernel's durable monotonic replay floor before removing receipt tombstones.
Workflow delivery and cleanup orchestration remain kernel integration work.

Run `node --test --test-concurrency=1 packages/app-sdk/test/*.test.js` from the
repository root. Tests use fake trusted transports and small in-process streams;
they do not launch Apps, providers, browsers or privileged helpers.

The shared wire corpus is `test/wire-vectors.json`; both the Rust supervisor and
JavaScript receiver must accept or reject the same cases. Tests also cover real
framing fragmentation, invalid UTF-8, truncated frames, write backpressure,
absolute frame deadlines, out-of-order replies, late replies, cancellation,
generation mismatch, handler capacity, lifecycle exclusion and event separation.
