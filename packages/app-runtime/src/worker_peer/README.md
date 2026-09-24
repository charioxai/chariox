# Shared worker IPC peer

`WorkerPeer` multiplexes both directions of the existing inherited App wire v1
channel. It creates no listener, process or MCP server. `WorkerProcess` continues
to own the SDK descriptor, process, sandbox/resource domain and termination.
Its trusted kernel owner passes a fixed generation and an injected `Broker` that
retains the catalog, installation owner, worker identity and common policy.

The peer does not infer authority from App params. It forwards `worker.ready` to
the broker just like any other request; the kernel must check exact declared
tools/events and its lifecycle policy before acknowledging readiness. No startup
handshake or transport test is evidence of confinement or production admission.

A single persistent reader and writer serve all tools, lifecycle, state, HTTP,
acknowledged events and other broker methods. An actor handles correlation and
admission while callbacks run separately. A partially received frame is never
cancelled to service an unrelated outgoing command. A partial-frame timeout or
write failure closes the peer permanently. Only terminal shutdown cancels a
reader/writer operation. Their tasks are joined during cleanup.

`reserve(timeout)` mints a single-use request ID, absolute wire deadline,
monotonic local deadline and transport admission permit. The caller can use the
ID/deadline to prepare an `AppCatalog` call under the existing kernel writer,
then send that exact message through the slot. A different ID, generation or
deadline is rejected. No database lock is held while awaiting IPC. A dropped
unsent slot sends nothing; dropping a sent call's future causes a Cancel within
the actor's 10 ms maintenance interval. Queued calls cancelled before their
first write are skipped. Started writes finish within the framing budget before
the cancellation frame follows, preserving framing.

Outgoing IDs are never reused during the peer lifetime. Retired or unknown
responses are ignored consistently with the JavaScript SDK and never recreate a
call. No unbounded tombstone set is retained. A transport slot retires when its
local call settles; it is not a lease proving the App or an external effect has
stopped. The kernel worker supervisor and each effect broker own those leases,
revocation checks, ambiguous outcomes and any required human-validation receipt.

Incoming absolute deadlines are converted once to local monotonic deadlines and
capped at 30 seconds (or the lower configured budget). Deadline/cancellation
sends one terminal response and signals the callback. Its capacity slot stays
occupied until the callback actually returns. Duplicate active request IDs close
the peer. Shutdown signals every callback and awaits their actual completion;
it does not silently abort a transaction future or release its admission. Kernel
brokers must implement bounded completion/cleanup. `PeerTask::join` can remain
pending if a trusted callback ignores that contract, while the rest of the Tokio
runtime and the supervisor's worker termination remain available.

The default limits are 64 pending outbound calls, 16 broker callbacks and four
frames in each transport/control mailbox. The actor's outbound backlog is capped
at `pending_calls + 2 * broker_handlers + queued_frames`; this permits a normal
admitted burst to wait for the writer without spawning writer tasks. Continued
inbound overload that fills this bounded backlog closes the peer. Every frame
has the existing 1 MiB limit, plus a bounded pre-queue graph walk (262,144 nodes,
64 total wire-container levels, finite numbers and safe integers). Limits may be
lowered. Global kernel memory/rate admission across installations is the owning
supervisor's responsibility; these are per-peer bounds.

One-way `worker.*` control events go to a separate bounded receiver. They are
untrusted observations and do not acknowledge readiness, commit App occurrences,
start workflows or carry agent authority. Other event namespaces close the peer.
An unconsumed/full control mailbox also closes it. Ordinary App occurrences must
use the acknowledged broker path and its durable policy, schema and outbox.

The focused tests use Tokio duplex streams and fake trusted callbacks. They prove
transport correlation, concurrent broker/tool traffic, queue admission,
cancellation/deadline handling and framing cleanup. They do not launch Node,
exercise sandbox policy, publish tools into a provider, or validate external
effects. Production worker admission and the kernel broker dispatcher remain
separate integration requirements.
