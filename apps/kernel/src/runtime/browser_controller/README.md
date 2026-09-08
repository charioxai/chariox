# Exact-target Browser Controller adapter

This private kernel module connects to the page target selected by the managed
Room Environment owner. It neither launches a browser nor finds a page by URL,
position, focus, or last-opened tab. It checks the returned target identity and
retains the supplied execution lease until its CDP connection closes.

The owner and its cloneable handles share one bounded actor. It serializes CDP
requests, exposes a bounded accessibility snapshot, and retains only the latest
screencast frame. Each input consumes an already admitted Room input reservation;
the actor retains it after caller cancellation until the request completes or
the connection closes. The owner must provide the actual Room admission and
managed-process leases. These private types do not independently grant access.

`BrowserController::shutdown` requests drain and joins the task with a one-second
bound. Dropping the owner aborts that task, closing its owned socket before
releasing the active reservation and execution lease. Neither operation closes
the Chromium Tab or undoes an input already sent. Handles become unavailable.
Viewer disconnect must drop that viewer's handle; it must not drop the Room's
controller owner.

Input references bind the kernel Environment, Tab, runtime generation, and the
controller's observed main-document revision and connection epoch. A recreated
controller receives a fresh process-local epoch, so reconnecting the same target
cannot revive references from the previous connection. These references are
private, in-memory values; a future serialized projection must also bind the
kernel incarnation. The adapter checks Chromium's
current main frame and loader before transmission, and checks again afterward.
This does not make coordinate input atomic with document navigation. If navigation
is observed after transmission, or the response is lost, the result is
`OutcomeUncertain`; the adapter closes and never retries that action. A successful
reply means CDP completed without an observed intervening navigation, not that a
business operation committed. Critical-effect authorization stays in kernel
brokers. Chromium gives screencast pixels no document token; their reference is
the document observed on receipt, not an attestation of pixel provenance.

Current limits: loopback endpoint only; exact bounded target IDs; one pending CDP
request; eight queued commands; five-second command deadline; two-MiB WebSocket
messages/frames; 4,096 accessibility nodes at depth eight; one JPEG frame of at
most 1,536 KiB encoded data; viewport dimensions at most 4,096. Input currently
supports bounded inserted text, left-button coordinates, and a fixed set of
navigation keys. It is a transport foundation, not the complete viewer keyboard,
IME, clipboard, viewport, focus/takeover, or accessibility presentation contract.

The adapter is deliberately not connected to the existing clear relay display
tunnel. The next integration must retain Room/installation/Tab ownership, attach
the App origin and storage/network policies, and expose a scoped encrypted
projection through the existing kernel terminal transport. Viewers must never
receive this CDP endpoint or a generic CDP request method.

The tests use actual WebSocket framing over bounded Tokio duplex streams and a
small deterministic CDP peer. They exercise exact-target startup, shared handles,
generation rejection, navigation uncertainty, cancellation ownership, queue
admission, latest-frame acknowledgment, shutdown, and malformed payload bounds.
They do not establish live Chromium compatibility or browser containment.
The ignored live test is run by the existing hosted Chromium profile drill:
`scripts/chromium-profile-drill/controller.mjs` copies and hashes the exact Rust
source into an external harness, compiles it under a 1,536-MiB/one-CPU/128-task/
180-second systemd unit, and runs its ten ordinary tests. The live test runs
under a separate 256-MiB/one-CPU/32-task/30-second unit. Only that fixture's
network namespace is entered; UID/GID and supplemental groups are dropped before
executing the test with a clean environment. No CDP port or proxy is exposed.
The owned Chromium container retains its existing sandbox and hard limits.

The live test creates two disposable targets, controls the selected target while
checking the other stays unchanged, receives accessibility and screencast data,
checks controller shutdown preserves both Tabs, and removes its own targets.
Whole-fixture cleanup removes its container and all scratch/build outputs even
when a test fails. This actual Chromium test remains a required hosted gate;
compiling its ignored body locally is not evidence of successful execution.

Protocol references: Chromium's [Target domain](https://chromedevtools.github.io/devtools-protocol/tot/Target/),
[Page domain](https://chromedevtools.github.io/devtools-protocol/tot/Page/), and
[Accessibility domain](https://chromedevtools.github.io/devtools-protocol/tot/Accessibility/).
