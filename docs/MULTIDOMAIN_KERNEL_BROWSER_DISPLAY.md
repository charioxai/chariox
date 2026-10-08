# MD-DISPLAY-02/04: kernel browser display implementation and history

MD-DISPLAY-02/04: the current kernel speaks local protocol 474 and relay peer
protocol 96. The presenter requires local protocol 466 with CXD1 binary display
events (474 for desktop display); relay peer 96 additionally enables binary relay
frames (`chariox-relay-binary-v96`). Dependent video requires the explicit
`chariox-video-dependencies-v1` codec capability; legacy offers retain
independent frames.

Current Phase 10 native motion prefers negotiated H.264, with realtime VP9 fallback,
three default display credits, bounded reference recovery and exact PNG damage
repair. PyAV needs libx264/libvpx; optional FFmpeg plus an accessible render device
can attempt VAAPI, falling back on actual failure. Builder evidence is software
only. The portable [Linux LAN kit](../apps/browser-display/LAN_KIT.md) supplies the
coordinator-run laptop matrix, still unmeasured here. Sparse native damage piping
and reduced software motion scale are implemented; exact settled pixels stay
native DPR. Current results remain RED and the flag stays off. See the source-bound
Phase 10 matrix and integration limits in the performance doc.

MP-08/MP-10/MP-11 phase 28 (since protocol 466 only the contention fallback
below): unprotected, whole-frame software motion at
1920×1080 and 2560×1600 uses 1280×720 and 1280×800 video respectively.
Resizing precedes color conversion. Reduced video certifies no native pixels;
lossless repair restores native resolution. Protected, striped and hardware
frames retain native geometry. Capture manually redirects the owned window on
its private X server, avoiding composition onto an unused root, and falls back
to automatic redirection when another capture owns the manual redirect.
These changes do not establish performance or live acceptance.

MP-08/MP-10 protocol 466 (relay unchanged): display frame events are binary.
The encrypted plaintext is `CXD1`, a big-endian u32 header length, the JSON
header `{"event":"kernel_browser_frame","subscription_id","frame"}`, then raw
payload bytes. Each `data_base64` of the frame, its tiles and stripes becomes
`data:[offset,length]`; segments are contiguous in header order and cover the
payload exactly. JSON plaintexts never start with `CXD1`. Browser clients use
`decryptRelayEvent` from `browser-relay-crypto`; the local kernel socket sends
the same bytes as a binary WebSocket message. Native packet files are
`u32 header length + JSON segment headers (with length) + raw segments`; the
kernel binds the headers to the frame before projecting raw bytes.

Tiles carry `format` `png` or lossless `webp`, up to 2560×256 pixels. Native
exact repair merges each 128-row band into WebP strips (about half the bytes of
tile PNG on text). A `tiles` frame may carry `moves`: `[x,y,w,h,dy]`
destination rectangles copied from one snapshot of the previous canvas at
`y-dy`, applied before tiles. The capture proves every moved and unchanged
cell byte-for-byte against the previous readback or the admitted base; WebP
tiles cover the rest. Only an exact, unprotected canvas whose base is the
delivered frame may receive moves, so scrolling text stays lossless and exact.
Larger residuals use native-resolution video. Reduced 720p/800p whole-frame
software motion engages only while host CPU idle stays below 10% (held at
least 10 s) and is reported as `motion_reduced_contention`.

When a readback skips the delivered frame, the worker plans moves (or a
static residual) against its committed exact canvas instead; a refused plan
replies normally and holds lossless frames until exactness returns. Mouse
wheel notches (multiples of 120) go through XTest on the kernel's private X
server after the same document fence and actor ledger, so Chromium applies
its native smooth scrolling (CDP wheel deltas are precise and unanimated);
other deltas stay on CDP. Wheel dispatch does not wait for the renderer's
frame-aligned ack. x264 runs on its own worker thread, and capture plans are
requested only while a viewer canvas is exact and unprotected.

The historical427/74 configuration, pipeline, client and Phase7 evidence are
in [MULTIDOMAIN_DISPLAY_PERFORMANCE.md](MULTIDOMAIN_DISPLAY_PERFORMANCE.md).
The sections below record earlier419-era implementation and receipts; their
codec, source, pacing and credit descriptions are historical, not current
configuration. Their source identities and limitations remain unchanged.

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

The source is the existing `KernelBrowserHost.screenshot` protected PNG seam.
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
codecs are ignored. This first swappable implementation uses independent VP9
keyframes, software libvpx, realtime deadline, CPU-used 6, two encoder threads,
CRF18 and a five-frame-per-second rate-control timebase. It does not claim the
Phase-2 persistent encoder's throughput. Independent frames make loss/reconnect
safe but cost more than inter prediction. H.264/AV1/hardware encoders are later
adapters; neither VideoToolbox nor Media Foundation has been measured here.
[FFmpeg's codec documentation](https://ffmpeg.org/ffmpeg-codecs.html) describes
libvpx controls; [WebCodecs](https://www.w3.org/TR/webcodecs/) defines the browser
decoder/support-query contract. Codec support must be queried per viewer.

A changed large frame first gets VP9. The next full protected capture gets
an exact full PNG, including when the source changed slightly during refinement.
Once the viewer acknowledges exact pixels, small changes may
use 128-pixel PNG tiles against that exact sequence. Compare serialized patch
size against full PNG; use patches only below 48 KB. Larger moving changes
return to video. Unchanged exact frames emit no event. Lost acknowledgements,
new documents or changed protection policy discard the patch base. A fresh
independent video frame and then full PNG rebuild it. PNG-only mode is exact
on every full emitted frame. Phase 4 adds private, protected thumbnail-guided
native crops for small changes. A thumbnail never supplies displayed pixels or
crosses the relay. It locates a padded CSS rectangle (at most 15% of the viewport)
that is captured at native DPR and merged into the protected pixel base. It is
a damage hint: fine changes elsewhere can be missed in the first paint. The
next poll without new input forces full protected readback to verify settled
detail. Large changes, a lost base, a new document or policy changes require
full capture. Exactness claims refer to that verified settled frame.

Negotiate 0.5–8 Mbps; default client budget 2 Mbps. Each frame includes base64 and
metadata. Pacing additionally reserves the relay encrypted payload's base64
expansion plus 1 KB of envelope overhead, including bootstrap and repairs.
Up to 16 KiB of unused budget accrues while idle/capturing, starting at zero.
Large bootstrap and repair frames still wait for the remaining budget.
Frames above 1 MiB serialized/encrypted estimate fail
loudly. This is a conservative application budget, not a wire/TLS or multi-viewer
aggregate cap. A request grants one frame credit, and its next credit follows
presentation. No unbounded source/encode/viewer queue exists. Host limits eight
display subscriptions per user browser; streams expire after 60 seconds.

The display uses lightweight CDP loader checks for frame capture. Input routes
through kbrowser's `KernelBrowserDisplayRequest::Input` with its source document
binding, shared actor ledger, cancellation and state reconciliation. It preserves
that reconciliation rather than using the prototype's cheaper input receipt.
Document/URL changes still update the kernel-owned tab registry.

PNG encode/decode and software keyframes add CPU and latency; dense scroll and
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
   frames and exposes `next`, `input`, and `close`. Events may be presented before
   their receipt; the single credit remains held through both presentation and
   receipt. Call `next` at the chosen
   cadence, awaiting each call. Presentation failure requires a fresh attach;
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
colour `srgb`, and one payload: independent VP9, full PNG, or bounded PNG tiles
with `base_sequence`. The presenter validates bindings, dimensions and tile
bounds before committing. Full frames use a back buffer; all tiles decode and
validate before synchronous patch drawing, avoiding a full canvas copy/reset.
It closes bitmaps and video frames.
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

The Node tools directory needs `playwright-core` and `pngjs`; PyAV tools need
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

The metrics and fixture modules are copied without modification from Phase 2.
Phase-2 CSS viewport was 960×600; this kernel seam is 1280×800. Source docs text
also differs. Do not plot these as a bandwidth-fair replacement baseline or
claim unchanged Phase-2 execution files. Compare exactness and latency only as
scoped component observations. Production Cloud, hosted WAN, multi-viewer load,
Vault end-to-end display, Mac/Windows/GPU and Room desktop replacement remain
open. Native secrets/masking source tests are narrower than those live gates.

## MD-DISPLAY-02/04: measured Phase-3 result

Clean implementation source `836e64630bc53d42e488dc97142416fdb0c92271`, rebased
onto kbrowser `d6d03751ffea37198fb33530829f4cd76ae30fbf`. Final receipt:
`<lane evidence>/phase3/final-typed-relay-2mbps/results.json`.
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


## MD-DISPLAY-02/04: current Cloud integration handoff, 466/474

Copy `apps/browser-display/presenter.mjs`, `stripe-presenter.mjs`,
`decoder-worker.mjs` and `scroll-prediction.mjs` together at their relative URLs. Serve worker modules
with the application's restrictive CSP and correct JavaScript MIME; no page
scripts or provider/account state are added to the client. The self-contained
`harness.html` demonstrates the transport adapter; coordinator wires private
Cloud separately. Do not use the historical419 instructions as a version bump.

`attachBrowserDisplay(canvas, transport, {tab_id,generation}, options)` needs the
existing encrypted kernel `request`, event listener and scoped
`subscribeDisplay`/`unsubscribeDisplay` adapter. Preserve shared-client sender
pinning, display-next no-replay and durable session replay-cursor separation.
Use native `deviceScaleFactor`, negotiated bitrate and default `creditWindow:3`;
`start()` maintains bounded continuous credits and `stop()` awaits them. For
manual polls use `next()` while stopped. Input/takeover/release/actors route
through the same kernel actor/document seam; `close()` releases local presenter,
scoped relay registration and kernel subscription. Polling unchanged frames
still renews display admission. Default-off feature and minimum 466 (474 for
desktop display) remain.

Worker decode adds no jitter buffer; authoritative frames retain dependency
order. Keep `scrollPredictionRegion` unset unless product geometry is explicitly
trusted; prediction does not count as source acknowledgement. Surface stream
errors and actor takeover to users through existing flows. The presenter canvas
is an image surface, so IME/clipboard/file chooser/drag-drop remain existing
kernel input capability questions rather than DOM replay inferred from pixels.
See `MULTIDOMAIN_DISPLAY_PERFORMANCE.md` for RED performance targets and exact
execution/binary evidence; this module is reviewable, not rollout acceptance.

## MD-DISPLAY-04: protocol441 dependent-video compatibility

Phase9 uses reservation local 441 / relay 84 for the dependent-video transport
contract. A display subscription explicitly offers
`chariox-video-dependencies-v1` alongside its real codecs and PNG. The kernel
retains independent keyframes for older offers using the same VP9/H.264 names.
Only admitted modern offers may receive persistent key/delta chains. Sequence,
document, source and actor fences still apply; a missing dependency needs an
independent recovery frame. H.264 intra-refresh recovery points are delta
packets, not independent IDRs; explicit resets force an IDR.

Cloud should use the supplied441-minimum presenter and its capability offer.
The local/relay versions and shared client constant are bumped together; the
focused subscription snapshot/hash now pins the capability value, and the
frame-contract drill covers both legacy independent offers and negotiated
dependencies. Historical427/74 receipts retain their own source identities.


## MP-08/MP-10/MP-11 peer-96 binary relay events

Local protocol 466's `CXD1` display plaintext is unchanged. Peer 96 adds an
optional `CXR1` WebSocket binary envelope: four magic bytes, a big-endian u32
JSON-header length, a bounded routing header, then contiguous opaque ciphertext.
The header contains event direction, subscription ID, event ID, sender public
key and nonce. Its maximum size is 4096 bytes; ciphertext is 16 through 1048576
bytes. No display metadata or pixels are exposed to the relay.

The kernel offers `x-chariox-relay-protocol: 96` in the WebSocket handshake and
enables binary writes only after the relay echoes that response header. The
configured URL, authority, path and query remain unchanged.
Browsers use the `chariox-relay-binary-v96` WebSocket subprotocol; if a legacy
relay cannot select it, they reconnect using the existing connection contract.
The display presenter offers the matching codec-list capability only after
successful negotiation. This changes internal byte accounting, without changing
any local request, response or display-frame JSON shape. Legacy offers retain
base64 byte accounting and pacing.

The relay applies the existing daemon/realm/subscription route and backpressure
cleanup to both encodings. A receiver that did not negotiate binary events gets
the original JSON/base64 envelope. A peer-94 relay therefore continues to work,
and an older client can attach to a newer kernel and relay. The existing peer90
runtime/security admission floor remains unchanged: advertising optional
transport96 does not revoke compatible peer94 workers. Binary-event routing
never decrypts ciphertext or parses a display payload. Kernel and browser bounds,
wrong-daemon/realm refusal, sender-key pinning, ciphertext authentication,
legacy negotiation and wire-size accounting have focused regression coverage.

These source and component checks do not establish MP-08/MP-10 live acceptance.
Hosted realistic-network, public-site, real-app/provider, DPR1/DPR2, Intel/iHD
and multi-hour validation remain separate required gates.

### MP-08/MP-10/MP-11 adapter and native-input review gates

Before enabling `CHARIOX_KERNEL_BROWSER_DISPLAY`, the Cloud adapter must use
`decryptRelayEvent` for authenticated CXD1 events, pass the connected kernel's
version as `kernelProtocolVersion`, and declare `displayEventEncoding: 'CXD1'`.
The presenter refuses missing/older versions and JSON-only adapters before any
subscription request. These are adapter properties, not serialized protocol
fields; the presenter minimum remains 466 (474 for desktop display). Peer-96
negotiation is independent.

Only the kernel-admitted human display input path with a live caller-owned
viewer lease may use asynchronous or native wheel dispatch. Agent wheel calls
retain the awaited CDP acknowledgement and hidden-target capture lifetime.
A native wheel that reports no dispatch falls back to fenced CDP input.

Identical readbacks may advance source bookkeeping, but overlay plans and
adjacent commits use the last actual worker commit. Codec jobs retain an owned
mapping lease: releasing a client lease cannot recycle its pixels while encoding,
and dropping the supervisor cannot unmap a codec's input. Scroll votes are local
to each planner invocation. Focused checks are supplementary; real app/provider,
hosted-network and public-site acceptance remain required.

### MP-08/MP-10 typing measurement conditions

The original component campaign's `type_latency` probes run after motion has
settled. They do not establish typing latency during scrolling. A separate run
with `MD_TYPE_DURING_MOTION=1 MD_WINDOW=1` keeps scroll/wheel active through20
admitted typing actions and measures their presented-pixel echoes under
`motion.typing`. This extra input changes the workload, so its CPU/fps results
must be labelled separately from the unchanged phase29 matrix. Both are
supplementary fixture checks, not real live acceptance.
