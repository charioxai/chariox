# MD-DISPLAY-02/03 prototype harness

Research only. No production kernel, relay, Cloud or client contract is changed.
This harness owns a headed host Chrome under UID 65534, independent Xvfb
displays, disposable profiles, loopback HTTP/WebSocket listeners, and (for the
baseline) one explicitly labelled Docker container. It needs no accounts or
provider state. Use only the credential-free fixtures and public documentation;
its masking is deliberately incomplete for authenticated pages and Vault state.

MD-DISPLAY-02 uses CDP PNG screencast and, separately, 10 Hz PNG screenshot
capture. A trusted page in a second host Chrome decodes PNG and encodes WebCodecs
H.264/VP9/AV1; a browser viewer decodes the binary WebSocket chunks. This extra
encoder browser is a prototype device, not a proposed shipping dependency.
Everything is 960×600 CSS pixels / 1920×1200 physical pixels, DPR 2.

MD-DISPLAY-02 DOM reads run in a CDP isolated world with universal access
disabled. A MutationObserver invalidates snapshots, which send changed records
with stable per-document IDs and computed styles from a narrow property list.
The scriptless, sandboxed viewer never loads source scripts, URLs or stylesheets.
Canvas/video/cross-origin iframe regions use full-source PNG capture and local
crops, replacing those regions with images. Media/frame snapshots poll because
their pixels can change without a top-document mutation. This is a PNG-patch
prototype, not the proposed regional video implementation. Native CDP input
replays viewer element actions; cross-origin patches use mapped coordinates.
SPA add/router changes, Unicode form fill/submit, password placeholder masking,
and cross-origin button effects are checked against the live source.

MD-DISPLAY-03 uses the installed image's Xvfb/Chromium and existing read-only
`slice-selkies-stream.py` adapter, including viewer registration/revocation and
frame acknowledgements. It forwards existing H.264 packets to the same decoder.
The baseline container has 2 CPUs, 3 GiB memory/no extra swap, 512 PIDs, 512 MiB
shm, host networking for loopback fixtures, and no mounts or provider accounts.
It is a component baseline; it does not start a Room/kernel, prove G2 release
provenance, or cross the encrypted hosted relay. Container Chrome uses
`--no-sandbox` with credential-free fixtures; host Chrome defaults to sandboxed.
Image tag, immutable ID, embedded source labels and script hash are recorded
separately. The tag is not assumed to establish source provenance.

Install tools outside the repository (Node 22, Xvfb, Google Chrome, Docker):

```sh
export MD_TOOLS=/root/.chariox/dev/browser-resume-20260930/agents/display/tools
npm install --prefix "$MD_TOOLS" --no-audit --no-fund playwright-core@1.58.2 ws@8.18.3 pngjs@7.0.0
node --test experiments/multidomain-display/*.test.mjs
export MD_OUTPUT=/root/.codex/evidence/browser-resume-20260930/display/host-h264
node experiments/multidomain-display/run.mjs
MD_CODEC=vp09.00.10.08 MD_MODES=screencast MD_OUTPUT=/root/.codex/evidence/browser-resume-20260930/display/host-vp9 node experiments/multidomain-display/run.mjs
MD_BASELINE=1 MD_OUTPUT=/root/.codex/evidence/browser-resume-20260930/display/selkies node experiments/multidomain-display/run.mjs
```

Run comparison campaigns sequentially. Optional `MD_PAGES=docs,spa,form,media,iframe`
and `MD_MODES=screencast,capture,dom` narrow the run. `MD_CHROME` chooses an installed
host Chrome; `MD_BASELINE_IMAGE` selects an explicitly owned/approved existing
baseline image. Do not pass real profile homes or launch this against live accounts.
There is no Rust build or compile slot use. `MD_NO_SANDBOX=1` is only a diagnostic
override; the reported host evidence uses the default sandboxed launch.

Every case submits 20 actual viewer clicks, changes a five-bit high-contrast
source marker, and waits for its matching visual state after a viewer
`requestAnimationFrame`. Video reads decoded canvas pixels; DOM reads the
mirrored marker’s `data-seq` after rAF, a render-cycle proxy rather than a raster
presentation timestamp. Source geometry calibrates video sampling, including
scrollbars; colour quantization cannot be used as an exact sequence counter.
Results retain raw samples, histogram, nearest-rank p50/p95/p99, application
bytes/s and pixel RGB MSE/PSNR, changed-pixel fraction, screenshots and 4× error
images. Frozen canvas/video frames make image pairs comparable. Password
placeholder changes and caret/focus differences remain in the raw pixel metric.
`psnr_db: null` denotes exact pixels only when `lossless: true`; mismatched
dimensions have `comparable: false` and cannot count as lossless.

Two latency clocks are explicit: harness viewer-click submission → visual ack
(includes Playwright scheduling); and input arriving at the harness → visual ack
(excludes client uplink). Both use one monotonic Node clock. Neither measures
physical display scanout or WAN input-to-photon. Application bytes exclude CDP
base64, WebSocket/TLS, encryption and existing relay fragmentation overhead.
The media source is canvas.captureStream video, not downloaded/DRM media. Public
CDP documentation is a supplemental actual network page with a frozen-image
comparison, not a passed input suite or broad arbitrary-site acceptance.

Host memory/disk and owned process CPU/RSS are sampled every second; container
CPU/RSS are separate. The run stops at 16 GiB MemAvailable or 10 GiB free disk.
CPU uses Linux USER_HZ=100 and may undercount exited children. Capture cadence,
content, shared-builder load and software encoders affect these research numbers.
No GPU cost, macOS/Windows, HiDPI zoom matrix, IME, clipboard, drag/drop, file
chooser, real codec negotiation, encrypted relay, slow viewer, multi-user,
reconnect, Vault safety or soak acceptance is claimed.

Cleanup runs through `finally` on normal completion/failure and cooperative
SIGINT/SIGTERM; a 15-minute deadline bounds a campaign. Owned browser process
groups and Xvfb stop, disposable profiles are removed, sockets close, and the
baseline container is removed only after its exact labels match. The harness
PID exits with the command. Image/caches, shared processes and other lanes are
never pruned. A SIGKILL cannot run `finally`; use recorded exact ownership to
repair only that interrupted run. Results name the source commit, dirty state,
prototype file hashes, timestamps, exit code, resource and cleanup observations.
See `docs/MULTIDOMAIN_DISPLAY_TRANSPORT.md` for MD-DISPLAY-01/04 interpretation.

MD-DISPLAY-02/03 generate a standalone local frame explorer from cleaned runs:

```sh
node experiments/multidomain-display/report.mjs /root/.codex/evidence/browser-resume-20260930/display/report /root/.codex/evidence/browser-resume-20260930/display/review-h264 /root/.codex/evidence/browser-resume-20260930/display/review-vp9 /root/.codex/evidence/browser-resume-20260930/display/review-selkies
```

Open the output `index.html`; it binds rows to receipt hashes and lets a reviewer
overlay source/viewer images or inspect amplified RGB error. Core fixture success
and supplemental public-page failures are reported separately.

## MD-DISPLAY-02/03 Phase 2 — budget curves and exact settled tiles

From the lane worktree, run sequentially (no Rust):

```sh
MD_CAMPAIGN_OUTPUT=/root/.codex/evidence/browser-resume-20260930/display/phase2-curves node experiments/multidomain-display/campaign.mjs
node experiments/multidomain-display/curves.mjs /root/.codex/evidence/browser-resume-20260930/display/phase2-curves
MD_PUBLIC=0 MD_PAGES=docs,spa MD_MODES=dom MD_SWITCH=1 MD_BITRATE=2000000 MD_OUTPUT=/root/.codex/evidence/browser-resume-20260930/display/phase2-dom node experiments/multidomain-display/run.mjs
MD_PUBLIC=0 MD_PAGES=media,iframe MD_MODES=hybrid MD_SWITCH=1 MD_CODEC=vp09.00.10.08 MD_BITRATE=2000000 MD_OUTPUT=/root/.codex/evidence/browser-resume-20260930/display/phase2-hybrid node experiments/multidomain-display/run.mjs
```

The campaign retains untouched Selkies CRF/paint-over defaults, then CBR Selkies,
H.264 high-profile, VP9 and AV1 at 0.5/1/2/4/8 Mbps targets. `MD_METHODS`,
`MD_RATES`, `MD_PAGES` narrow a campaign. Each child owns its cleanup; the parent
records exact env, commands and exits. Same-CLI/env Selkies parser fields and
initial range values are selected explicitly; private live configuration is
never read. The campaign intentionally disables supplemental public navigation.

`MD_RATE=constant|variable`, `MD_KEYMS=2000`, `MD_LATENCY=realtime` configure
WebCodecs. These targets do not cap WebSocket traffic. The report plots both
configured and observed rates, with one-second application peaks; it never
asserts bandwidth fairness from a target alone. Capability probes include
high-profile AVC, AVC/VP9 4:4:4 and HEVC. A supported probe is not a passed codec
campaign. Input latency and initial decoded fidelity are scored before refinement.

`MD_REFINE=1` stops/flushes the video pump on settled content, compares 128-pixel
RGB tiles against the decoded screen, sends only differing PNG tiles paced at
`MD_BITRATE`, with at most eight awaiting acknowledgement, then verifies exact
pixels. A further native source click tests source dirty regions against the
prior exact screen. Receipt fields include bytes, time and separate frame pairs.
This explicit settled-stage prototype proves reconstruction and budget cost;
it does not implement automatic idle detection, generation fencing, masks or
loss recovery. A later video frame can overwrite the refinement. Production
needs the epoch/revision contract proposed in MD-DISPLAY-04.

`hybrid` retains DOM text/structure and crops decoded full-page video into only
opaque regions. It encodes the whole source frame and rasterizes patches in the
client, so it proves composition/input and exposes cost, not regional bandwidth
savings. `MD_SWITCH=1` measures one DOM→first-video switch and full DOM+PNG
bootstrap back; it is not a percentile distribution. `MD_FRAME_METRICS=1` takes
up to three exact timestamp-paired CDP-frame→decoded RGB samples on moving media.
Use it as a separate diagnostic: PNG readback/metric work adds load. Main curves
use frozen pairs, not moving-frame equivalence. Capture-only comparisons isolate
CDP PNG acquisition from video loss. CPU includes refinement/metric stages where
enabled and the prototype encoder browser; container CPU remains separate.

Run the focused reconstruction/metric checks with:

```sh
node --test experiments/multidomain-display/*.test.mjs experiments/multidomain-display/tiles.test.mjs
```

MD-DISPLAY-02 Phase 2 also measures `MD_MODES=lossless`: CDP PNG frames go to
native ImageBitmap/OffscreenCanvas decode/diff/PNG encode in the trusted prototype
encoder browser, then an exact full frame or atomic dirty-tile batch crosses the
WebSocket. Full PNG wins when it is smaller than the sum of tile payloads.
Initial frames and batches are application-paced at `MD_BITRATE`; one complete
image is a burst (up to the measured initial size), not a hard instantaneous
network cap. At most eight tile sends await decode credit. Source acknowledgements
wait for viewer presentation; superseded incoming capture frames can be dropped.
Unchanged captures acknowledge a no-op instead of waiting for nonexistent paint.
This browser avoids the slow JS PNG decompression path used in the first exact
prototype; it is still a research device, not a shipping encoder dependency.

```sh
MD_PUBLIC=0 MD_MODES=lossless MD_BITRATE=2000000 MD_OUTPUT=/root/.codex/evidence/browser-resume-20260930/display/phase2-exact-2m node experiments/multidomain-display/run.mjs
MD_PUBLIC=lossless MD_PAGES=docs MD_MODES=lossless MD_BITRATE=2000000 MD_OUTPUT=/root/.codex/evidence/browser-resume-20260930/display/phase2-public-exact node experiments/multidomain-display/run.mjs
```

The public supplement replays ten viewer wheel inputs on actual CDP documentation,
checks source scrolling and first subsequent viewer output, then compares the
settled exact projection. That proxy is not timestamp-paired physical scanout.
Full-frame frozen comparisons in exact mode include a final convergence refresh;
video curves retain their original pre-refinement score. Media frame dropping and
scroll/large-region bandwidth are separate from settled pixel fidelity. Exact
fixture pixels alone do not establish that moving media beats Selkies at the same
latency, that this obeys a WAN hard cap, or that any authenticated site is safe.

## MD-DISPLAY-02 native RGB H.264 profile probe

The WebCodecs encoder rejects AVC High 4:4:4 here; the decoder accepts it.
`native-encoder.py` uses PyAV 16.0.1 / Pillow 11.3.0, software `libx264rgb`,
RGB24, five threads, ultrafast/zerolatency, CRF18 plus a 2-second VBV at the
stated target. This is quality-driven constrained VBR, not CBR or an application
byte cap. Full-range GBR/sRGB encoder VUI and decoder colour metadata are
explicit: default decoder conversion produced a pink-background defect.
Supported decoder config alone does not prove correct decoded pixels.

Install these pinned binary wheels in a lane-owned external `MD_PY_TOOLS`
directory using pip's `--target`; never use a provider profile or repository.
The default is `/root/.chariox/dev/browser-resume-20260930/agents/display/tools-py`.
Then run sequentially:

```sh
MD_MODES=native MD_NATIVE_CRF=18 MD_BITRATE=2000000 MD_PUBLIC=native MD_OUTPUT=/root/.codex/evidence/browser-resume-20260930/display/native-2m node experiments/multidomain-display/run.mjs
```

Repeat `MD_BITRATE` at 500000/1000000/2000000/4000000/8000000 for the ladder.
`MD_FRAME_METRICS=1` adds timestamp-paired moving-frame PNG diagnostics; it also
adds readback overhead and should be a separate run. Local PNG→Python base64
IPC is a prototype adapter; it does not cross the measured WebSocket. Codec
packets are forwarded to the same browser decoder/latency/pixel harness.
The worker is reset per document and killed in `finally`; sampled owned CPU/RSS
includes it. Mac/Windows software portability is a proposed path, not tested;
this does not establish hardware encoder 4:4:4 support.

MD-DISPLAY-02: `MD_NATIVE_PRESET=veryfast` probes better compression at added
CPU cost; default stays ultrafast for the original ladder. `MD_NATIVE_TRACE=1`
writes credential-free fixture/public H.264 packets in the evidence directory
for independent decoder diagnosis. It must not be used against authenticated
sites. The packet trace is diagnostic, not a production recording path.

## MD-DISPLAY-02/03 reviewer and incident recovery

`owned-process.mjs` records each launched child PID and Linux start time.
Signals reject PID 0/1/-1, invalid values, unregistered children and reused IDs.
Before each group signal it inventories `/proc`, verifies the group exists and
proves every member belongs to the launched session/descendant tree or matches
an already observed owned identity. Browser shutdown snapshots membership before
CDP close. A failed ownership check leaves resources alone and reports RED;
this guard is Linux-specific, not a native-OS process manager.

Missing/non-executable Chrome/Xvfb launch errors reject an awaited operation;
failed display readiness and browser launch clean acquired resources before
returning. Selkies JSON/stripe/socket-write failures latch into the awaited
harness/stop path. Campaigns continue collecting independent cases, aggregate
nonzero/null/signal exits to parent exit 1, and report parent interruption as
130. Focused tests exercise failure routing, readiness cleanup, valid descendant
teardown, invalid PID rejection and campaign statuses. Raw RED receipts and
historical source hashes must stay unchanged after harness fixes.
