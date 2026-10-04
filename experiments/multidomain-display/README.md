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
node --test experiments/multidomain-display/metrics.test.mjs
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
`requestAnimationFrame`. Source geometry calibrates marker sampling, including
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
