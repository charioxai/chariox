# MD-DISPLAY-02/04: experimental kernel browser display, protocol 419

This implements the Phase-2 recommendation as an opt-in, removable adapter.
It does not close an MD or MP acceptance item. The owner still decides the final
transport. Phase-2 measurements remain on research branch `agent/display` / PR
#844 and keep their original source identities; they are not validation of this
implementation. Room/Selkies replacement and native Mac/Windows execution are
outside this first kernel-browser implementation.

## MD-DISPLAY-04: enablement and ownership

Default off. Set `CHARIOX_KERNEL_BROWSER_DISPLAY=1` on the home kernel. The
normal host browser still requires a non-root Unix user and sandbox-capable
Chromium. Its existing per-user profile, focused runtime MCP tools, tab registry,
input cancellation and Vault observation barrier remain authoritative.

The experimental software video adapter needs Python 3 and operator-installed
PyAV with `libvpx-vp9`. `CHARIOX_BROWSER_DISPLAY_PYTHON` may select an absolute
interpreter or operator-owned wrapper. This selector and the display flag cross
the host controller's explicit environment allowlist; provider credentials do
not. No Python/CDP endpoint, profile, cookie or codec service is exposed to the
viewer. Missing encoder dependencies fail the awaited operation and close the
owned child. A PNG-only viewer does not launch an encoder.

The source is the existing `KernelBrowserHost.screenshot` protected capture seam. Native exact pixels use PNG; continuous motion may use protected JPEG at CSS resolution only with an empty Vault observation registry.
An emulated canonical tab viewport stays 1280×800 CSS pixels with negotiated
DPR 1 or 2. Different DPR selections on the same live tab are refused. Screenshots
and Vault region masks follow that geometry, including DPR2 pixel conversion.
The display never consumes unmasked CDP screencast pixels. Capture checks the
observed document again after screenshot acquisition; replacement documents fail
that frame instead of attaching old input coordinates to new content.

`kernel-browser-display.mjs` owns encoding, exact refinement, dirty regions,
egress pacing and the encoder child. The host owns tab/stream lifetimes. The
Rust transport adapter owns transient event projection. `apps/browser-display/`
owns presentation and contains the client module/harness that Cloud can import.
No encoder policy lives in the relay or Cloud.

## MD-DISPLAY-02: configuration and limitations

Negotiate `vp09.00.10.08` plus mandatory `png`, or `png` only. Other advertised
codecs are ignored. Phase 6 uses a persistent software libvpx VP9 context,
realtime deadline, CPU-used 8, four encoder threads, no lag, 30 fps rate-control
timebase and a 60-frame key interval. The encoder clears the PNG decoder's
inherited I-frame hint so changed frames can become deltas. Its media target is
45% of the negotiated application budget, reserving double base64 and metadata
expansion. Equal min/max/target rates select CBR; intra overshoot is capped at
200% of the average frame target, with a small VBV buffer. The source image
demuxer clock is explicitly replaced by the encoder clock before assigning PTS. Document/base loss, geometry/rate change or transition from exact
repair forces an independent keyframe. The persistent client decoder validates
sequence/document dependencies and never flushes between deltas. H.264/AV1/hardware encoders are later
adapters; neither VideoToolbox nor Media Foundation has been measured here.
[FFmpeg's codec documentation](https://ffmpeg.org/ffmpeg-codecs.html) describes
libvpx controls; [WebCodecs](https://www.w3.org/TR/webcodecs/) defines the browser
decoder/support-query contract. Codec support must be queried per viewer.

A changed large frame first gets VP9. Exact refinement waits for an unchanged
full protected capture. Small repairs use a full PNG; larger repairs use batches
of 128-pixel PNG tiles against the last displayed sequence (including a video
base). All tiles of that protected snapshot must arrive before the server marks
it exact. New source pixels, a lost base, document or protection changes discard
pending repair batches. Once exact, small changes use 32-pixel dirty tiles; larger regions use 128-pixel tiles. Tile batches
use half a second of negotiated budget, allowing outer base64 and 4 KB overhead,
with a 24 KB minimum / 192 KB maximum JSON allowance; one incompressible tile can
exceed that allowance but still must fit the 1 MiB event bound. Larger moving
changes use continuous inter-predicted video. Four admitted credits normally
pipeline capture/delivery; the client may select one to eight with a 1 MiB
receive reservation and bounded source recovery window. A per-user async capture
gate waits outside the blocking host mutex and yields to pending admitted input;
source and encoder remain serial. Input admission drops on completion/error/
cancellation; capture cancellation releases its slot.
Unchanged motion pixels skip encoding and emission. Empty credits back off
32–100 ms independently of receipt ordering; recent motion uses 8–33 ms.
Input or changed frames wakes all parked slots. No automatic retry or
bitrate resubscription is introduced. Unchanged exact frames emit no event. Lost acknowledgements,
new documents or changed protection policy discard the patch base. A fresh
independent video frame and then complete lossless repair rebuild it. PNG-only mode is exact
on every full emitted frame. Phase 4 adds private, protected thumbnail-guided
native crops for small changes at an unscrolled, unzoomed known viewport origin.
Scrolled/zoomed/uncertain viewports use full protected capture: native CDP clips
are page rectangles, while the damage hint is a viewport rectangle. A thumbnail never supplies displayed pixels or
crosses the relay. It locates a padded CSS rectangle (at most 15% of the viewport)
that is captured at native DPR and merged into the protected pixel base. It is
a damage hint: fine changes elsewhere can be missed in the first paint. The next poll after a crop without new input forces full protected readback
to verify settled detail. Intermediate crops can miss fine changes outside their
rectangle; they do not establish whole-source exactness. Empty-policy unchanged verified pixels can be reused for at most
250 ms while idle; thumbnail equality alone never establishes exactness. A complete
protected PNG that matches a previously decoded PNG byte for byte reuses that
immutable native pixel buffer, avoiding duplicate decoding while still verifying
the complete source. Large-motion JPEG
captures stay in video until 300 ms of stable pixels, then return to a full native
DPR readback. JPEG motion covers the complete current visual viewport using its
CDP page origin; zoomed or unknown geometry stays on native full capture. An
admitted wheel input keeps complete motion capture selected for 400 ms, even
when repeating text looks unchanged in the thumbnail. The hint is bound to the
source document and never supplies pixels or bypasses Vault protection. Large changes, a lost base, a new document or policy changes require
full capture. Exactness claims refer to that verified settled frame.

Negotiate 0.5–8 Mbps; default client budget 2 Mbps. Each frame includes base64 and
metadata. Pacing additionally reserves the relay encrypted payload's base64
expansion plus 1 KB of envelope overhead, including bootstrap and repairs.
Up to 16 KiB of unused budget accrues while idle/capturing, starting at zero.
Large bootstrap and repair frames still wait for the remaining budget.
Frames above 1 MiB serialized/encrypted estimate fail
loudly. This is a conservative application budget, not a wire/TLS or multi-viewer
aggregate cap. A request grants one frame credit. Each window slot is held through its
receipt and presentation; up to eight admitted requests may overlap. No unbounded source/encode/viewer queue exists. Host limits eight
display subscriptions per user browser; streams expire after 60 seconds.

The display uses lightweight CDP loader checks for frame capture. Input routes
through kbrowser's `KernelBrowserDisplayRequest::Input` with its source document
binding, shared actor ledger, cancellation and state reconciliation. It preserves
that reconciliation rather than using the prototype's cheaper input receipt.
Document/URL changes still update the kernel-owned tab registry.

PNG encode/decode and software video add CPU and latency; dense scroll and
media remain a known weak seam. Source capture is request-driven rather than a
continuous compositor feed. This is not a 60 Hz display. No native browser
chrome, desktop/file chooser, cursor-shape discovery, clipboard or live IME
acceptance is claimed. Input uses the same typed click/key/text/scroll service;
DPR never doubles CSS input coordinates. The harness uses a normal arrow cursor.

## MD-DISPLAY-04: shared protocol and Cloud integration

Shared local protocol **419**, with Rust/TypeScript version guards, request/event
shape snapshots and an explicit frame-contract test. Existing browser commands
still have minimum 417. The new presenter has minimum 419. Relay protocol is
unchanged because it still carries opaque encrypted terminal packets.

1. Use the existing admitted kernel `request` function to send
   `KernelBrowser.command = {op: "display_subscribe", tab_id, generation,
   codecs, bitrate, device_scale_factor}`. The kernel derives the user/caller.
   Subscribe replies with a caller-bound display ID and the chosen codec.
2. Register the display ID with the existing decrypted terminal event adapter.
   On relay, send `ClientSubscribe` with scope `kernel_browser_display`,
   session ID equal to the display ID, attachment ID equal to the generation
   string, and the same persistent client keypair used for requests.
   The kernel rechecks the display owner and maps its ID to the relay delivery
   ID. Registration expires after 60 idle seconds; each admitted display-next
   poll renews it even when there are no changed pixels. Pixels are never replayed.
   `display_next` carries the ID, generation and last presented `after_sequence`.
   A changed frame emits `event: "kernel_browser_frame"`, `subscription_id`,
   and `frame`. Its response is a small receipt with `frame_sent`; event and
   receipt may arrive in either order. No changed frame means `frame_sent=false`.
3. On a relay connection, the kernel encrypts the event to the admitted request
   sender and routes it through the existing `DaemonEvent` queue. The relay
   receives only the ordinary subscription ID/event sequence/encrypted packet.
   The ID is delivery correlation, not a new authorization capability. A
   display-next call rechecks profile, generation and exact terminal caller.
   No event-log replay or command-result cache stores display-next pixels.
   The local kernel WebSocket emits the same `KernelEvent` through its bounded
   event queue; priority control responses keep their existing queue.
4. Import `apps/browser-display/presenter.mjs`. Supply
   `transport.request(LocalDaemonRequest)` and
   `transport.onEvent(listener) -> unsubscribe`. On relay also supply
   `subscribeDisplay(binding)` and `unsubscribeDisplay(binding)` using the
   existing relay subscribe/unsubscribe envelopes. Keep one client keypair
   for admission, request encryption and event decryption during the attachment. For a relay client this adapter
   uses its existing sender-pinned decryption, never a Cloud HTTP media route.
   `attachBrowserDisplay` negotiates, correlates events/receipts, presents atomic
   frames and exposes `next`, `start`, `stop`, `input`, and `close`. Events may be
   presented before their receipt; each credit remains held through both. Call
   `start()` for a bounded continuous window, or await each `next()` for a single
   repair/test credit. `stop()` drains existing slots. The client reorders frames
   by source sequence before dependency decoding because encrypted event lanes
   and decryption may complete out of order. Stale video frames are decoded to
   retain references but may skip drawing when a newer video is queued; repairs
   are never skipped. Presentation failure requires a fresh attach;
   do not acknowledge a lost patch base. Never automatically resend uncached
   `display_next` after a stall or connection loss: its outcome is unknown and
   a fresh attachment is required. The shared IPC client handles this nested
   command explicitly. Closing clears the display and
   unsubscribes. Reconnect requires a fresh display ID.
5. The public adapter routes through
   `KernelRuntimeState::kernel_browser_display_request(&KernelCommand, ...)` from
   kbrowser Phase 4 (`d6d03751f`). `display_capture` selects its bound `Capture`;
   negotiated subscriptions and encoded captures extend that typed seam. Encoding
   runs over the same protected screenshot inside the capture/input barrier,
   including policy invalidation and egress pacing. No raw CDP feed is consumed.
   `display_takeover`, `display_release`, and `display_actors` map to its existing
   shared actor operations. The presenter exposes `takeover`, `release`, and
   `actors`; Cloud supplies the trusted controls and renders that projection.
   Deliberate takeover belongs to the authenticated terminal actor, cancels/fences
   focused agent input, and release requires that owner. Display commands remain
   terminal-only and are excluded from the provider MCP adapter.
6. `display_input` sends the currently displayed `document_id`, tab/generation
   and the existing `KernelBrowserInput`. The host refuses stale documents
   before dispatch. Preserve trusted host UI outside the canvas and map pointer
   coordinates to the canonical CSS geometry. Cloud owns no browser state,
   secrets, renderer authority or provider execution.

Frames carry sequence, document ID, tab/generation, CSS/pixel geometry, DPR,
colour `srgb`, and one payload: VP9 key/delta, full PNG, or bounded PNG tiles
with `base_sequence`. The presenter validates bindings, dimensions and tile
bounds before committing. Full frames use a back buffer; all tiles decode and
validate before synchronous patch drawing, avoiding a full canvas copy/reset.
It closes bitmaps and video frames and reuses its back buffer. Motion video may
decode to 1280×800 and is scaled to the canonical DPR canvas; native exact
repairs restore sharp settled text. The older experimental 419 presenter only
accepted independent full-resolution video: deploy the current source/client
together while the flag remains off; no mixed experimental-client compatibility
or negotiated motion-resolution extension is claimed.
This first transport uses per-display sequences; do not feed them into session
terminal replay cursors. Local transient display envelopes carry event ID zero;
the shared IPC cursor ignores them. Desktop clients can implement the contract later.

## MD-DISPLAY-02/04: focused validation

Build with the allocated Rust slot/target, then run:

```sh
node --test apps/browser-display/{contract,presenter}.test.mjs \
  apps/kernel/slice-linux-docker/docker/kernel-browser-{display,pixels,host}.test.mjs
node apps/browser-display/drill.mjs /absolute/kernel-test-binary \
  /absolute/external/evidence /absolute/node-tools /absolute/pyav-tools
```

The Node tools directory needs `playwright-core`, `pngjs` and `typescript`; PyAV tools need
only public `av` and `av.libs`. The drill copies public dependencies and the test
binary into a freshly allocated external scratch directory, then runs the real
kernel as a non-root user. It launches its own Xvfb, opens a host tab through the
focused runtime MCP tool (dev-stub admitted run; no provider model), and drives
the real kernel request/event router through the production relay with a
headless Chromium presenter. It boots that relay on a dynamic loopback port, starts the normal
kernel relay connector, issues disposable scoped tokens through the product
auth API, and uses the existing browser WebCrypto implementation for requests
and event decryption. Bootstrap tokens stay in private disposable runtime state
and never enter evidence. A browser cannot connect directly to the native local
WebSocket: the existing kernel `Origin` guard rejects it. The drill uses the
scoped relay; native local clients keep their existing socket path. It measures
bootstrap video and settled PNG pixel pairs/diffs, twenty source click visual
acknowledgements, latency histograms, observed local bytes and resources. A stale
input document must fail. Relay-driven human takeover must fence focused MCP
input while keeping observations available; owner input and release/resumption
must work through the shared actor API. Exact owned process/state cleanup and
failures are recorded in a RED or PASS_LOCAL_COMPONENT receipt.

The metric implementation originated in Phase 2; fixtures and the kernel drill
have evolved, and every receipt identifies its actual execution source.
Phase-2 CSS viewport was 960×600; this kernel seam is 1280×800. Source docs text
also differs. Do not plot these as a bandwidth-fair replacement baseline or
claim unchanged Phase-2 execution files. Compare exactness and latency only as
scoped component observations. Production Cloud, hosted WAN, multi-viewer load,
Vault end-to-end display, Mac/Windows/GPU and Room desktop replacement remain
open. Native secrets/masking source tests are narrower than those live gates.

## MD-DISPLAY-02/04: measured Phase-3 result

Clean implementation source `836e64630bc53d42e488dc97142416fdb0c92271`, rebased
onto kbrowser `d6d03751ffea37198fb33530829f4cd76ae30fbf`. Final receipt:
`/root/.codex/evidence/browser-resume-20260930/display/phase3/final-typed-relay-2mbps/results.json`.
`phase3/provenance.json` binds this source, the test binary SHA-256, 118 source
file hashes, 28 exact embedded controller assets, commands, exits and receipt.
This is historical Phase-3 coverage; Phase-4 execution files differ and the
current binary was rebuilt. Earlier receipts and research results
retain their original sources, including the pre-rebase implementation.

The real headed, sandboxed kernel Chromium runs outside slices. The focused
runtime MCP dev-stub opens the tab; a separate headless Chromium decrypts and
presents actual production kernel/relay events. No provider model or Cloud proxy
runs. Linux Chrome 154, 1280×800 CSS, DPR2 (2560×1600 pixels), VP9 + PNG repair,
2 Mbps paced frame budget, local scoped-auth production relay:

| Observation | Final Phase-3 component result |
| --- | --- |
| Bootstrap VP9 RGB PSNR | 34.04 dB; not lossless |
| Settled full PNG and final patched RGB | Exact, MSE 0 at matching dimensions |
| Input visual acknowledgements | 20/20 |
| Click p50 / p95 / p99 | 771.71 / 822.16 / 829.50 ms |
| Received encrypted application bytes/s | 7,451 during the click interval |
| Bootstrap video / exact repair envelope | 594,520 / 810,620 bytes |
| Small patch envelope | About 6.3 KB |
| Observed owned CPU | 70.5% of one core |
| Minimum sampled MemAvailable / free disk | 47.88 / 211.27 GiB |
| Stale input / takeover / release | Rejected stale input; agent fenced during takeover; resumed after release |
| Kernel exit and owned teardown | Exit 0; process inventory empty; exact disposable state removed |

Byte rate includes inbound relay control responses and events after bootstrap,
with resource sampling in the measured interval; it excludes requests, TLS and
bootstrap/repair. Frame pacing includes those bootstrap/repair envelopes. CPU is
live-process tick deltas (Linux CLK_TCK=100), including fixture/headless readback
cost and excluding already-exited workers. These are scoped observations, not
sustained video throughput, total egress caps or full input-to-photon hardware
measurements. Screenshots/diffs and raw latency histograms remain external.

**Phase-3 latency was RED against the owner's comparable-latency goal.** Historical
Phase-2 docs at DPR2/960×600 measured Selkies CBR 2 Mbps p95 74.72 ms (46.17 dB)
and paced exact PNG p95 115.81 ms. The new kernel authority/actor/capture path is
substantially slower; changed geometry, docs content and request/observation
cost mean these are not bandwidth-fair curves. Phase-2 source identities and
limits remain in `docs/MULTIDOMAIN_DISPLAY_TRANSPORT.md` on `agent/display`.
The bootstrap VP9 quality also does not beat that Selkies fixture. Exact settled
text was proven there; a universally higher-quality replacement at comparable
latency was not. Phase-4 component measurements below supersede only the local
latency result. Keep the feature off for remaining acceptance gates.

Focused checks: 33 Node tests, 14 Rust shape/event/conformance/actor/takeover/origin
checks, kernel-client TypeScript, changed-Rust formatting and local build pass.
Fail-first presenter credit and failed-encoder retry tests are in external
receipts. The kernel origin guard remains intact: a browser connecting directly
to the native local WebSocket is correctly refused. No hosted WAN, production
Cloud, native Mac/Windows, live Vault login, multi-viewer or Room migration gate
closes from these results.

## MD-DISPLAY-02/04: Phase-4 latency measurements

Execution and kernel-build source `05387e1e4`. Clean receipt:
`phase4/final-damage-2mbps/results.json` under the external display evidence root.
The same headed, sandboxed Linux source, DPR2 geometry, 2 Mbps negotiated budget,
production scoped relay and encrypted runtime events are used. Twenty clicks
change a small binary counter on the docs fixture. The endpoint is a headless
browser rAF callback **after** reading back and checking the visible counter.
This is a software presentation proxy, not physical monitor photon timing.

| Measurement | Instrumented before (`cb382fb22`) | Optimized (`05387e1e4`) |
| --- | ---: | ---: |
| Input to verified presentation p50 / p95 | 744.20 / 778.20 ms | 73.80 / 92.40 ms |
| Input request round-trip p50 | 227.70 ms | 34.80 ms |
| CDP input p50 | 3.87 ms | 3.71 ms |
| Full CDP capture p50 | 148.59 ms | Small-change preview 5.94 + native crop 2.74 ms |
| Source PNG decode p50 | 101.67 ms | Preview 1.31 + crop decode/merge 1.11 ms |
| Dirty comparison/tile encode p50 | 4.85 ms | 0.89 ms |
| Selected payload encode p50 | 1.62 ms | 0.02 ms (already encoded tiles) |
| Pacing wait p50 | 27.36 ms | 1.12 ms |
| Event serialize/encrypt p50 | 0.85 ms | 0.87 ms |
| Bounded frame-credit acquisition p50 | 0.09 ms | 0.04 ms |
| Enqueued event → viewer arrival p50 | 42.60 ms | 2.00 ms |
| Client decode p50 | 12.40 ms | 1.70 ms |
| Client synchronous presentation p50 | 1.12 ms | <0.10 ms |
| Entire next-credit round-trip p50 | 455.50 ms | 37.60 ms |
| Observed received application bytes/s | 7,551 | 27,741 |
| Observed live owned CPU (one core = 100%) | 88.9% | 162.3% |

Spans overlap and are not additive. The event arrival span includes writer queue
and both same-host relay hops; it does not isolate individual relay transit.
Raw timestamps, per-stage p50/p95/p99 and histograms are in each JSON receipt.
`phase4/provenance.json` binds 134 execution-source hashes, 30 exact embedded
controller assets, current binary SHA-256, commands, checks and historical/final
receipts. Documentation/status commits follow only with unchanged execution hashes.
CPU excludes exited encoder workers and includes fixture/viewer readback. Byte
rate excludes bootstrap, requests and TLS, includes intervening control responses
and resource-sampling time; faster interaction increases bytes/s. Bootstrap and
refinement remain charged to the budget. A bounded 16 KiB allowance permits small
idle changes without an additional serialization-duration sleep.

Full-readback intermediate source `8473bf69a` measured 437.40 / 455.70 ms, still
RED. Its process refresh cost was 13.50 ms per refresh (14 per click). Incremental
Linux PID enumeration/identity reads reduce that to 1.52 ms in the final run.
Signaling always takes a fresh full membership/start-time snapshot; unsafe,
foreign and reused groups remain rejected. The Mac `ps` path is unchanged and
unmeasured. Fast native PNG capture and row-specific PNG predictors retain RGB;
flagged Chromium disables the capture frame-rate limit. That flag can increase
animation CPU and needs moving-content tests before rollout.

Small admitted display packets bypass the existing 33 ms event batching window
through a bounded priority queue; large frames retain bounded event credit.
TCP_NODELAY removes delayed receipt coupling on the local relay sockets. The
presenter begins decoding an event while awaiting its receipt, then releases
credit only after both finish. Native damage crops avoid full readback on each
small input; tile comparison uses those private crop bounds. Full protected idle
verification scans the entire frame and preserves settled pixels.

Before the final crop-bounded tile scan, two runs passed at 78.10/106.60 and
72.80/88.60 ms. A third clean run (`final-clean-2mbps`, source `6cfcf6410`)
was RED: p50 88.50 ms, p95 100.20 ms. It retains its failing exit and receipt;
that variance motivated the tile optimization rather than being discarded.

A second clean run at `69a4897ef` (execution files unchanged from `05387e1e4`)
polled a static display for 100 seconds / 256 polls, then passed p50/p95
72.80/83.90 ms. Both final runs pass both latency thresholds, all 20 visual counter checks,
exact settled and final RGB (MSE 0), stale-document rejection, takeover fencing,
owner input and release/resumption. VP9 bootstrap remains 34.04 dB, followed by
exact PNG. This does not improve large moving-video fidelity or prove arbitrary
page interaction under 150 ms. Full refinement is about 1,023,108 encrypted
application bytes here and is intentionally slow at 2 Mbps. Keep that distinction
from settled small-input latency explicit.

Review regressions have fail-first evidence: delayed/connection-lost uncached
credits replayed in shared IPC, local transient frames overwrote session resume
cursor 500 with 1, and real 100-second static polling expired delivery before the
next change. Fixes recognize nested `display_next` as outcome-unknown/no-replay,
exclude transient display events from durable cursors, and renew delivery only
inside successful admitted polling. The registration unit also rejects another
client key and expires genuine idle. The final live static run is recorded
separately in `phase4/final-damage-static-100s/results.json`. The earlier
`final-static-100s` receipt retains its dirty-tree flag from a temporary test
dependency symlink; it was not current-file proof after the native tile fix.

Focused checks: 39 Node tests, 13 Rust ownership/registration/queue/protocol/
event/actor/takeover/origin checks, 11 shared IPC tests and TypeScript typecheck.
Build and checks have their own logs; historical RED receipts remain RED. The
initial final run observed a distinct compositor refinement after the first exact
capture; the harness now allows at most five distinct refinements before requiring
an unchanged exact poll. It does not silently discard failures.

## MD-DISPLAY-04: owner decisions and migration

First wire the flagged kernel-browser presenter in Cloud and validate the
same bytes over hosted WAN. The owned local relay drill is the first transport
proof, not hosted/client acceptance. Require source/viewer
pixel pairs and slow-viewer/reconnect/Vault policy tests before enabling it.
Then replace the software keyframe adapter with a protected persistent encoder
or compositor source behind the same frame/input contract. Preserve exact idle
repair and capture barriers. Measure 0.5/1/2/4/8 Mbps, moving text and media at
matched geometry against the retained research baseline. Test native
VideoToolbox/Media Foundation only when the kernel platform exists; retain
software/PNG negotiation fallbacks. App-native and selective DOM projections
remain measured options, not commitments in this implementation.

Owner decisions: acceptable moving and settled fidelity, p95 input latency/WAN
and total-egress budgets, required client codecs/platforms, initial page scope,
cursor/IME/file-chooser coverage, and criteria for Room desktop migration. Until
those decisions and independent review, keep the flag off by default.

## MD-DISPLAY-04: native OS adapter design (not implemented)

`apps/kernel/native-display/adapter.mjs` is an unavailable, unregistered stub.
It never opens a window, captures pixels or encodes them. Portable VP9/PNG stays
the only negotiated implementation at protocol 419. These integration points
are proposed designs, inferred from platform APIs; no native timings are claimed.

On macOS, first retain `KernelBrowserHost.screenshot` and its protected DPR2 PNG
barrier. Decode into a kernel-owned sRGB `CVPixelBuffer` pool and pass that image
to a bounded `VTCompressionSession`. Request H.264 low-latency rate control,
`RealTime`, expected frame rate and the negotiated average bitrate; force a
keyframe after a lost base/document/policy change. Callback buffers must be tied
to the admitted source binding and one outstanding credit, and released on
cancel/close. Query hardware availability rather than promising an encoder.
[Apple's low-latency sample](https://developer.apple.com/documentation/videotoolbox/encoding-video-for-low-latency-conferencing)
and [compression-session lifecycle](https://developer.apple.com/documentation/videotoolbox/vtcompressionsession-api-collection)
support those API choices. H.264 conversion can blur chroma; the existing exact
PNG/tile refinement remains required for text.

ScreenCaptureKit is a later capture adapter, using a content filter for the
kernel-owned Chromium window and backing-pixel geometry. It adds screen-recording
permission and native window/content-rectangle mapping; CDP avoids that new OS
capture dependency for the first browser-only path. See
[Apple's window-capture sample](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos).
Neither ScreenCaptureKit nor raw CDP screencast may bypass the kernel's Vault
barrier: associate the current document and protection epoch, mask before the
encoder, recheck after capture, and emit opaque pixels on uncertainty. Mask
updates fence queued images and encoder references before more frames leave.
Cursor, native file chooser, HDR and occlusion require separate acceptance.

On Windows, keep protected CDP PNG first. Copy protected sRGB BGRA into an NV12
sample and use an enumerated H.264 Media Foundation `IMFTransform`; negotiate
output before input and query `ICodecAPI` support. Request mean/max bitrate,
low latency, no reordering, GOP/keyframe control and bounded async work. Drain
and release samples on close. The
[H.264 encoder contract](https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder)
and [low-latency property](https://learn.microsoft.com/en-us/windows/win32/medfound/codecapi-avlowlatencymode)
document these controls; hardware and optional properties vary. NV12 is lossy
for colored text, so exact refinement stays separate. Later,
[Windows.Graphics.Capture](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture)
can supply a kernel-window D3D surface through a bounded frame pool after consent.
Apply the same mask/document barrier before BGRA/NV12 conversion. Handle resize,
device loss and HDR-to-sRGB explicitly; never capture the whole user desktop as
a browser fallback.

A native encoder cannot be plugged into the present VP9-only presenter merely
by changing an executable. Before native activation, the coordinator must
allocate any required protocol evolution, add AVC decoder-config/framing and
negotiation coverage, and run native protected-image/slow-viewer/reconnect/Vault
drills. Persistent inter prediction likewise needs an explicit decoder/base
recovery contract. The current independent-frame wire shape remains unchanged.

MD-DISPLAY-04 review 03:42: a document change recreates the native capture closure
before its next protected screenshot. This keeps the same display subscription
usable after full navigation while old document input remains refused. Drill
TypeScript loads from the explicitly supplied Node tools under cleanup coverage;
no shared checkout path is required. Campaign interruptions ask the owned drill
to settle its children before namespace removal; unsafe/group IDs stay guarded.

## MD-DISPLAY-02/04: Phase-5 WAN and moving-page result

Final execution/kernel-build source **933d6222d**; receipts in external
`phase5/final-v3/`. A second clean checkout at
`/root/work/agent-display-impl-replay` ran the same source with a process-local
hook making `/root/work/oss` TypeScript unavailable. Its 100-second static poll,
then input/navigation/actor checks pass in `phase5/relocated-static-100s-v3/`.
Both sources are bound to the copied ELF's SHA-256 in each receipt; external
`phase5/provenance.json` binds execution hashes, embedded assets and checks.
Documentation/status handoff commits change no measured execution file.

The same real headed sandboxed kernel Chromium, DPR2 / 1280×800 CSS geometry,
2 Mbps frame budget and production encrypted relay are used. Each campaign case
owns a fresh network namespace. Only the viewer-to-relay TCP proxy leg is shaped;
fixture HTTP, CDP control and the kernel-to-relay leg are unshaped. The proxy
forwards opaque bytes. Namespace-only loopback MTU is 1500, TSO/GSO/GRO are off;
no host default interface is touched. Delay is applied both ways, jitter is
normal, loss is random 1%, and the cap is shared by both directions. Raw netem
settings, packet/drop counters, commands and cleanup are in receipts.
This emulates one client/relay WAN leg, not two distant kernel/client legs.

| Profile | RTT / jitter / cap | Input p50 / p95 ms | Initial exact repair s | Received application KB/s during clicks |
| --- | --- | ---: | ---: | ---: |
| Local | 0 / 0 / uncapped link | 68.5 / 81.6 | 5.35 | 29.2 |
| WAN40 | 40 / ±2 ms / 5 Mbps, 1% loss | 169.6 / 432.5 | 8.39 | 20.5 |
| WAN80 | 80 / ±5 ms / 2 Mbps, 1% loss | 287.5 / 310.9 | 13.17 | 15.0 |
| WAN150 | 150 / ±10 ms / 1 Mbps, 1% loss | 469.0 / 498.1 | 18.21 | 11.0 |

Every row has 20/20 verified counter changes and exact settled/final RGB (MSE 0).
The input probes remain a software rAF/readback endpoint. Initial repair time
starts after bootstrap video and its diagnostic pair; it includes credited
capture/encode/transit/decode until an unchanged poll, not reference comparison.
Bootstrap quality remains about 34 dB. Click KB/s excludes bootstrap, repairs,
requests and TLS, and includes receipt responses/resource-sampling time.
The final 100-second repeat is 69.6 / 84.3 ms, 255 unchanged polls, followed by
fresh independent navigation, exact repair and stale-document rejection.

WAN loss tails are noisy at n=20. Historical clean source `29d3698e4` / binary
`7197ac202` measured WAN40/80/150 p95 262.9/671.8/486.4 ms. Keep both series
with their identities rather than selecting the better tail. The earlier
`740a7ffaa` campaign used default loopback MTU/offloads, so its lower tails are
exploratory and not packet-loss realism. No Selkies WAN run is claimed.

| Stage p50 ms (spans overlap) | Local docs | WAN80 docs | Local post-scroll click |
| --- | ---: | ---: | ---: |
| Input request round-trip | 28.1 | 120.1 | 32.7 |
| Capture layout metrics | 0.45 | 0.61 | 0.55 |
| Source capture | Preview 5.56 + native crop | Preview 6.44 + native crop | Full 90.16 |
| Source decode | Crop merge 1.07 | Crop merge 1.20 | Full 44.92 |
| Frame pacing wait | 1.08 | 1.13 | 1.18 |
| Entire next-credit round-trip | 35.1 | 161.6 | 167.1 |
| Enqueued frame to viewer | 2.0 | 68.6 | 2.1 |
| Browser decode | 1.6 | 1.9 | 1.8 |

Raw per-stage histograms retain capture, encoding, credit, encryption, delivery,
decode and presentation timings. Network delay and TCP retransmission dominate
WAN, while full PNG readback/decode dominates scrolled input. The harness waits
for input acknowledgement before asking for its frame, paying roughly two RTTs.
A continuously credited viewer may differ; it has not been measured here.

The new repair policy waits for an unchanged protected snapshot. Large exact
repairs are delivered as sequence-bound tile batches; a lost base or new pixels
restart independent video rather than queue stale refinement. Batches scale
with negotiated bitrate, and one outstanding credit adapts cadence to the link.
No automatic bitrate resubscription was added: steady static-input frame traffic is
below the constrained link caps, and lowering fidelity does not remove RTT/TCP
loss or PNG capture cost. This is bounded credit/repair behavior, not a claim
of a congestion-control encoder.

| Moving workload, local 2 Mbps | Before drawn fps | Final drawn fps | Frame-event Mbps | Freeze-to-exact s | Post-settle click p50 / p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Canvas animation | 0.79 | 1.18 | 0.81 | 1.95 | 104.0 / 120.7 |
| HTML video playing | 0.89 | 1.19 | 0.76 | 1.90 | 104.4 / 114.3 |
| Dense long-page scrolling | RED oversized repair | 0.25 | 1.44 | 7.73 | 198.0 / 232.0 |

Canvas/video used only dirty tiles during the final moving interval, with no
full PNG repair while motion continues. Video is an actual HTMLVideoElement
playing a 30fps canvas captureStream, not network/DRM playback. Scroll shifts
most text and uses large independent VP9 keyframes. The cadence is decoded/
drawn frames under sequential credits; two diagnostic capture/readback pairs
and resource scans reduce it. Live pixel pairs include motion age/compositor
drift and are not codec-only PSNR. Frozen first and final pairs/diffs are saved;
all workloads converge to exact RGB. The event rate counts only encrypted
frame events; diagnostic capture responses are separate and unpaced. Including
those responses yields 1.23/1.16/2.18 Mbps respectively, so the diagnostic total
must not be presented as a hard 2 Mbps egress cap.

Observed owned CPU over startup/repair/workload/click intervals is about 200%
for local docs, 263% canvas, 222% video and 102% scroll (one core = 100%). This
includes fixture/viewer readback and excludes exited workers; it is neither
steady encoder CPU nor GPU accounting. Minimum final-campaign MemAvailable is
47.1 GiB. Software capture and independent keyframes remain the moving bottleneck;
this is not yet smooth interactive media or dense scrolling.

### MD-DISPLAY-02: historical Selkies comparison for the owner

These retained Phase-1/default-path figures are from the research branch's
Phase-2 rerun, not a new baseline. Source/browser/geometry differ: Selkies uses
Chrome147 at 960×600 CSS; this kernel run uses Chrome154 at 1280×800 CSS, both
DPR2. The baseline's frozen quality is not moving codec-only fidelity.

| Observation | Historical Selkies default path | Final flagged kernel path |
| --- | --- | --- |
| Frozen docs RGB | 48.30 dB, 0.482 Mbps observed | Exact after repair; bootstrap ~34 dB |
| Frozen media RGB | 47.23 dB, 0.253 Mbps observed | Exact after freeze; live temporal drift retained separately |
| Small docs click p95 | 90.56 ms | 81.6 ms local; WAN 311–498 ms in this final series |
| Media click p95 | 90.02 ms | 114.3 ms after motion has stopped |
| Dense motion/scroll | Historical CBR2 wheel p95 86.4 ms, frozen public page 23.81 dB | 0.25 drawn fps; post-scroll click p95 232 ms; eventual exact RGB |

This supports exact settled text and a portable flag-gated seam. It does not
establish a bandwidth-fair universal replacement, higher moving fidelity than
Selkies, hosted Cloud acceptance or native-OS performance. Keep Selkies available
for Rooms and keep the new kernel-browser flag off by default.

### MD-DISPLAY-04: review, failures and next gate

Review 03:42 fixes are `7197ac202`: recreated document-bound capture closure and
portable supplied TypeScript dependency under cleanup. Corrected navigation
fail-first-v2, fixed unit and every final live navigation prove one subscription
survives a full document replacement; old document input is still rejected.
Relocated 100-second live replay proves the dependency fix without shared checkout
resolution. SIGTERM interruption exits 130 after owned cleanup; namespace inventory
is empty. An earlier interruption attempt copied a binary during linking and
exited before the signal; it remains RED, and ELF/hash checks now bind each copy.

Historical RED cases remain: early partial-repair harness comparison; dense
scroll overflow; source counter1/viewer0 under a scrolled crop; and the initial
wrong-target unit setup. Corrected fail-first tests identify the actual seams.
`933d6222d` falls back to full protected capture for scroll/zoom/unknown origins,
preserving masking and exactness at a measured latency cost. Final seven-case
campaign exits 0; all stale input, owner takeover, focused MCP fencing, release,
navigation, exactness and owned teardown assertions pass. 46 Node and 13 focused
Rust checks pass. No protocol shape/version change beyond existing 419.

Next: measure a continuously credited client before changing input/credit
semantics; replace full-PNG capture and independent encoders with a protected
persistent source/encoder before promising smooth media. Native stubs and their
API plan above remain unregistered. Cloud wiring, hosted WAN, live Vault,
slow-viewer/reconnect, native OS, cursor/IME/file chooser, multi-viewer and Room
migration are separate gates. PNG-only oversized first-frame/high-entropy pages
retain the bounded-packet limitation; this campaign negotiated VP9 + PNG.
Owner decisions remain the motion/WAN/fidelity budgets and final transport.
No MD/MP acceptance item is closed by these component receipts.

MD-DISPLAY-02 campaign replay (Linux root with `ip`, `tc`, `ethtool`, Xvfb and
sandbox-capable Chrome already installed; supplied public tools include TypeScript):

```sh
MD_CASES=local:docs,wan40:docs,wan80:docs,wan150:docs,local:canvas,local:video,local:scroll \
  MD_MOTION_MS=10000 node apps/browser-display/campaign.mjs \
  /absolute/kernel-test-binary /absolute/external/evidence \
  /absolute/node-tools /absolute/pyav-tools
```

The campaign aggregates failing/null child exits and interruptions into a failing
shell status. Evidence directories must be new per run; retain previous RED
receipts. Only exact namespace names created by that campaign may be removed.
