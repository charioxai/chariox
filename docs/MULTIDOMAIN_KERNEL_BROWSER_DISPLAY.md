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

The display uses lightweight CDP loader checks for capture and input, while
sharing the same typed physical input helper and cancellation checks. It avoids
constructing an accessibility snapshot for every frame/input acknowledgement.
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
5. `display_input` sends the currently displayed `document_id`, tab/generation
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
node --test apps/kernel/slice-linux-docker/docker/kernel-browser-{display,pixels,host}.test.mjs
node apps/browser-display/drill.mjs /absolute/kernel-test-binary \
  /absolute/external/evidence /absolute/node-tools /absolute/pyav-tools
```

The Node tools directory needs `playwright-core` and `pngjs`; PyAV tools need
only public `av` and `av.libs`. The drill copies public dependencies and the test
binary into a freshly allocated external scratch directory, then runs the real
kernel as a non-root user. It launches its own Xvfb, opens a host tab through the
focused runtime MCP tool (dev-stub admitted run; no provider model), and drives
the actual kernel WebSocket with a headless Chromium presenter. By default
it also boots the production relay on a dynamic loopback port, starts the normal
kernel relay connector, issues disposable scoped tokens through the product
auth API, and uses the existing browser WebCrypto implementation for requests
and event decryption. Bootstrap tokens stay in private disposable runtime state
and never enter evidence. A browser cannot connect directly to the native local
WebSocket: the existing kernel `Origin` guard rejects it. The drill uses the
scoped relay; native local clients keep their existing socket path. It measures
bootstrap video and settled PNG pixel pairs/diffs, twenty source click visual
acknowledgements, latency histograms, observed local bytes and resources. A stale
input document must fail. Exact owned process/state cleanup and failures are
recorded in a RED or PASS_LOCAL_COMPONENT receipt.

The metrics and fixture modules are copied without modification from Phase 2.
Phase-2 CSS viewport was 960×600; this kernel seam is 1280×800. Source docs text
also differs. Do not plot these as a bandwidth-fair replacement baseline or
claim unchanged Phase-2 execution files. Compare exactness and latency only as
scoped component observations. Production Cloud, hosted WAN, multi-viewer load,
Vault end-to-end display, Mac/Windows/GPU and Room desktop replacement remain
open. Native secrets/masking source tests are narrower than those live gates.

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
