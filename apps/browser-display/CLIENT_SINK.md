# MP-08 / MP-10 / MP-11 client sink (Selkies audit step 1)

One shared module worker decodes eight independent repair channels and up to
three whole-video jobs. The ordered presentation queue decodes contiguous video
ahead, with document/format changes and every repair as barriers. Binding,
dependency, payload and decoded geometry checks run before native track writes.

Where supported, the worker writes the transferred MediaStreamTrackGenerator
writable directly. The video element sits behind the existing interactive canvas.
Otherwise a sibling canvas commits the latest decoded video on the next rAF.
Skipped presentations emit `client_present_drop`; decoder loss keeps the existing
rate-limited lost ACK/key recovery. Tiles, stripes and scroll moves stay on the transparent
canvas overlay until the next complete decoded video image, key or delta.
The newer whole image retires both overlay visibility and its bitmap; a codec
delta reconstructs a whole image, so it also supersedes older repairs. Scroll moves snapshot video plus repairs.
Closing or replacing a decoder freezes the composite for preserve-frame recovery,
retires frame owners, stops the track, and removes the layer and observers.
Hidden rAF jobs update the fallback canvas and its snapshot synchronously,
then settle as not visibly drawn. The presenter validates and retires their
outputs. Hidden video images retire old repairs immediately; repairs received after
that image remain visible until another whole video image arrives. Visibility return needs no further video: the acknowledged pixel base
is already ready for display, partial repairs, prediction and scroll moves.
The existing lease heartbeat continues while hidden.

The repair canvas overrides its CSS background to transparent until teardown,
then restores the original inline background. Trusted scroll prediction takes
a video-plus-repair snapshot and shows it in a temporary sibling canvas;
authoritative frames and new input remove that preview without changing the
repair bitmap. Canvas-only restoration copies transparent pixels exactly.

The second `onPresented` argument exposes `decodedAt` and `committedAt` epoch
milliseconds. These are decode/track-write or canvas-commit times, **not native
pixel timestamps**. Cloud projects them to the interactive canvas dataset.
No serialized protocol, producer, mask, admission, generation or lease changes.

Focused checks:

```sh
node --test apps/browser-display/presentation-queue.test.mjs \
  apps/browser-display/presenter-*.test.mjs apps/browser-display/presenter.test.mjs \
  apps/browser-display/decoder-*.test.mjs apps/browser-display/client-worker.test.mjs \
  apps/browser-display/stripe-presenter.test.mjs apps/browser-display/video-sink.test.mjs
```

Supplementary real-Chromium check (disk-backed TMPDIR, external evidence):

```sh
node apps/browser-display/presenter-sink-browser.mjs \
  /absolute/path/to/playwright-core/index.mjs /absolute/evidence/directory
```

It exercises real WebCodecs and streams, an 8ms busy page, DPR1/2, both sinks,
repair retirement through deltas and keys; all browsers and the local
server close in `finally`. Its synthetic producer is never live acceptance.

MP-08/MP-10/MP-11 review regressions with real transferred VideoFrames, both
transport modes and screenshot pixel assertions at DPR1/2:

```sh
node apps/browser-display/presenter-sink-regression.mjs \
  /absolute/path/to/playwright-core/index.mjs /absolute/evidence/directory
# Add a Cloud checkout to exercise its pinned assets and actual surface CSS:
node apps/browser-display/presenter-sink-regression.mjs \
  /absolute/path/to/playwright-core/index.mjs /absolute/evidence/cloud-directory \
  /absolute/path/to/chariox-cloud
```

The 16 cells cover DPR1/2, both sinks/transports and hidden key/delta sequences:
visible A with repairs → changed hidden B → newer hidden repair → quiet
visibility return → visible delta, partial repair and scroll move against B.
Hidden video clears older repairs; a post-video repair survives quiet return,
and the next visible delta retires it. Screenshots inspect the actual compositor pixels;
the fallback is hidden between worker decode and rAF. The checks also verify
repair retirement/persistence, predictions and transparent restoration.

The optional fourth argument `hidden-repair` isolates post-key repair retention
until a visible delta; `repair` isolates the partial-repair CSS check
when an earlier visibility/prediction regression blocks the combined flow.
These are supplementary checks with a synthetic producer/transport. They do
not establish real public-site, hosted-relay, provider or soak acceptance.

Step 1's live gate still requires exact-head real built Cloud/kernel/relay,
public sites and the quiet per-hop harness: decode→native pixels median ≤18ms,
P95 ≤25ms at DPR1/2. Host CPU P95 >40% invalidates latency attribution.

MP-08/MP-10/MP-11 dense-scroll compositor regression:

```sh
node apps/browser-display/presenter-dense-burst-regression.mjs \
  /absolute/path/to/playwright-core/index.mjs /absolute/evidence/directory
# Optional third argument: Cloud checkout for pinned assets and actual CSS.
```

At DPR1/2 with both sinks, it paints a partial repair, then a 120px scroll move,
then 60 complete video images at a nominal 60Hz cadence. It requires native
video callbacks with an uncovered layer, changing screenshot pixels, and a
transparent overlay bitmap after the burst. Callback counts are not a measured
public-site frame rate. This controlled producer regression supplements the
real built app/kernel/hosted WSS drill and strict quiet-host gate.
