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

A changed large frame first gets VP9. The next identical protected capture gets
an exact full PNG. Once the viewer acknowledges exact pixels, small changes may
use 128-pixel PNG tiles against that exact sequence. Compare serialized patch
size against full PNG; use patches only below 48 KB. Larger moving changes
return to video. Unchanged exact frames emit no event. Lost acknowledgements,
new documents or changed protection policy discard the patch base. A fresh
independent video frame and then full PNG rebuild it. PNG-only mode is exact
on every emitted frame.

Negotiate 0.5–8 Mbps; default client budget 2 Mbps. Each frame includes base64 and
metadata. Pacing additionally reserves the relay encrypted payload's base64
expansion plus 1 KB of envelope overhead, including bootstrap and repairs.
There is no burst credit. Frames above 1 MiB serialized/encrypted estimate fail
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
   ID. Registration expires after 60 idle seconds; pixels are never replayed.
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
   frames and exposes `next`, `input`, and `close`. Call `next` at the chosen
   cadence, awaiting each call. Presentation failure requires a fresh attach;
   do not acknowledge a lost patch base. Closing clears the display and
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
bounds before committing its back buffer. It closes bitmaps and video frames.
This first transport uses per-display sequences; do not feed them into session
terminal replay cursors. Desktop clients can implement the same contract later.

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
`/root/.codex/evidence/browser-resume-20260930/display/phase3/final-typed-relay-2mbps/results.json`.
`phase3/provenance.json` binds this source, the test binary SHA-256, 118 source
file hashes, 28 exact embedded controller assets, commands, exits and receipt.
This document/handoff may follow in a documentation-only commit; execution file
hashes must still match that manifest. Earlier receipts and research results
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

**Latency remains RED against the owner's comparable-latency goal.** Historical
Phase-2 docs at DPR2/960×600 measured Selkies CBR 2 Mbps p95 74.72 ms (46.17 dB)
and paced exact PNG p95 115.81 ms. The new kernel authority/actor/capture path is
substantially slower; changed geometry, docs content and request/observation
cost mean these are not bandwidth-fair curves. Phase-2 source identities and
limits remain in `docs/MULTIDOMAIN_DISPLAY_TRANSPORT.md` on `agent/display`.
The bootstrap VP9 quality also does not beat that Selkies fixture. Exact settled
text is proven here; a universally higher-quality replacement at comparable
latency is not. Keep the feature off while profiling and replacing redundant
PNG/serialization work behind the protected seam.

Focused checks: 33 Node tests, 14 Rust shape/event/conformance/actor/takeover/origin
checks, kernel-client TypeScript, changed-Rust formatting and local build pass.
Fail-first presenter credit and failed-encoder retry tests are in external
receipts. The kernel origin guard remains intact: a browser connecting directly
to the native local WebSocket is correctly refused. No hosted WAN, production
Cloud, native Mac/Windows, live Vault login, multi-viewer or Room migration gate
closes from these results.

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
