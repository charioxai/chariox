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
- `log.write`: bounded structured logging, kept per installation (last 1000, 50/s) and read with `app logs`.
- `schedule.set`, `cancel`, `list`, `onWake`: kernel-owned wakes. The kernel
  starts the App when a wake falls due and delivers it at least once, so the App
  does not stay running to wait. A user stop holds wakes until the App starts.
  State transactions may commit wake changes.
- `validation.request`, `status`: a pending operation reference returns promptly;
  an App cannot approve it or claim that an App-view click was human validation.
- `paths`: the read-only package and private writable data/temporary roots supplied
  by the trusted worker. Ordinary private I/O continues to use `node:fs`.

The SDK does not expose transcripts, a second prompt area, provider credentials,
raw kernel requests, or an approval-resolution method.

## Agent panel placement (kernel protocol 371)

Chariox draws the session focus agent's pane beside every App view, in the
trusted terminal. The App's page never sees its content; it only lays out in the
space left for it. The App chooses where the panel sits:

- Default, in the signed manifest: `"ui": {"entry": "ui/index.html",
  "agentPanel": {"placement": "right" | "bottom" | "none", "size": 320}}`.
  `size` is the width at the right or the height at the bottom, 120 to 1200 CSS
  pixels, at most half the page; `none` shows no panel and takes no size.
  Declaring it needs `minKernelProtocol` 371. Omitted: at the right.
- At runtime, from the page: `await window.chariox.panel.set({placement:
  "bottom", size: 280})` answers the resulting layout, `{placement, minimized}`.
  `window.chariox.panel.get()` reads it. The page is resized to the space left
  (a normal `resize` event).
- The user may move the panel or minimize it to a bar at the bottom, and that
  choice wins over the App's. Only the user can minimize it, and an App with no
  panel cannot hide pending approvals: the terminal shows them in its chrome.

Inline file and buffered HTTP bodies support at most 512 KiB. SDK 0.7 adds a
separate streaming Fetch adapter over the kernel stream broker, including
multipart, SSE consumption, redirects and bounded decompression. Its supported
contract and evidence limits are described in [FETCH.md](FETCH.md). The bounded
`http.request` helper remains a separate convenience interface. Typed
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
`events.automations()` returns the owner's automations of the App's events,
each `{automationId, event, eventVersion, state, lastReceipt}` with `state`
`active`, `paused`, `broken` or `disabled` and the latest retained receipt or
null. It is read-only and never names the target workflow, so an App can tell
its user to fix a broken or turned-off automation in Chariox. It is additive to SDK
0.8.0: check `typeof chariox.events.automations === 'function'` and treat
`METHOD_NOT_FOUND` as unknown, for runtimes and kernels that predate it.
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

## HTTP streaming (SDK 0.6)

`http.open`, `write`, `headers`, `read`, and `cancel` use the kernel's existing
worker channel. Production currently supports anonymous HTTPS to the signed
package's exact approved origins/methods. A declared critical effect route is
reached only with the `operationId` of an approved `validation.request`; the
kernel consumes that single-use approval before sending, sends the approved
parameters as the JSON body (the App passes none), and otherwise returns
`VALIDATION_REQUIRED`. Effect routes use `POST`, `PUT` or `PATCH`, and the request
may carry no header but `accept`. Opaque connections return explicit unsupported responses.
No provider account credentials, redirects, cookies, automatic decompression,
raw sockets, or body retries are supplied by these methods.

The kernel limits streams to four per installation and 32 per kernel, each with
two 64 KiB queued chunks per direction, a 64 MiB upload, a 256 MiB response, one
hour total lifetime and 120 seconds without socket activity. These are initial
policy limits. Keep at most one read (or headers request) and one write in flight.
A pull can return `pending: true`; later pulls retain the same stream and do not
reset its lifetime. Headers can be requested again. A consumed body chunk or
write with unknown completion must never be retried. Always cancel a stream when
finished or abandoning it. `http.request` is a 512 KiB buffered convenience using
one original deadline, at most 30 seconds, across its component operations.
This subset does not claim Fetch conformance.

SDK 0.7 supplies `chariox.http.fetch` and the worker-global Fetch adapter above
these operations. See [the supported Fetch contract](FETCH.md) for streaming,
redirect/decompression behavior, pinned client tests and remaining exclusions.
