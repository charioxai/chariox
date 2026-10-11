# MP-08 / MP-10 / MP-11 client sink (Selkies audit step 1)

One shared module worker decodes eight independent repair channels and up to
three whole-video jobs. The ordered presentation queue decodes contiguous video
ahead, with document/format changes and every repair as barriers. Binding,
dependency, payload and decoded geometry checks run before native track writes.

Where supported, the worker writes the transferred MediaStreamTrackGenerator
writable directly. The video element sits behind the existing interactive canvas.
Otherwise a sibling canvas commits the latest decoded video on the next rAF.
Skipped presentations emit `client_present_drop`; decoder loss keeps the existing
rate-limited lost ACK/key recovery. Tiles and stripes remain on the transparent
canvas overlay until the next key. Scroll moves snapshot video plus repairs.
Closing or replacing a decoder freezes the composite for preserve-frame recovery,
retires frame owners, stops the track, and removes the layer and observers.
Hidden rAF jobs retire without blocking the existing lease heartbeat.

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
repair persistence through deltas and key retirement; all browsers and the local
server close in `finally`. Its synthetic producer is never live acceptance.
Step 1's live gate still requires exact-head real built Cloud/kernel/relay,
public sites and the quiet per-hop harness: decode→native pixels median ≤18ms,
P95 ≤25ms at DPR1/2. Host CPU P95 >40% invalidates latency attribution.
