# MP-08/MP-10/MP-11: phase17 software display performance — RED

Assigned base `f5d3b1da96e06a57a6f4b7b590abbbd2bb958885`, branch
`md/display-perf`. Final runtime/test assets and every final comparison are clean
`c53693acf8434970898d15199faa6f666977160a`. Protocol 447 / relay 90 unchanged.
No MP acceptance item closes. The requested CPU, typing and all-row Selkies win
remain RED; the required real-app drill is blocked by the paired Cloud source
and shared e2e harness. This report does not turn component passes into acceptance.
Evidence: `<lane evidence>/phase17/`.

## MP-08/MP-10: cadence result and source identity

The fresh base on a verified 60 Hz scroll source presents 53.07 fps at 1.99
source-plus-pipeline cores. Capture-start pacing reached 56.50 fps; concurrent
independent row decoding and actual direct conversion reach 58.98 fps at 1.76 cores
on the final clean source. The 60 Hz source is close to its source rate; the
<=1 core target remains unmet. The 30 fps video and scroll30 fixtures generate 30
updates/s. They establish no global30Hz pipeline ceiling. Selkies presents
roughly 62 fps even on those sources; compression changes can also trigger the
thumbnail content metric. Do not manufacture duplicate packets to match that count.
Wheel30 remains 21.96 fps; this lane does not claim its scheduling gap is fixed.

The comparisons use the optimized kernel libtest executable built from
`b5df3da5c`, SHA256
`16ca02baa601441ae45e39547781e83a6e9bd229c70a82dc8ecb2cbd8109809b`,
with explicit production controller asset overrides from the final source. Each
receipt pins 57 controller assets and the client assets independently. The binary's
embedded 56 assets retain their b5df source identity. This is the production
scoped-auth encrypted local relay/kernel request and event path, sandboxed owned
Chromium, and production presenter in a fixture HTML entry. It is not the built
Cloud app, standalone kernel/relay/CLI UI or provider drill required for acceptance.

## MP-08/MP-10/MP-11: implementation

- Pace native readback from the start of a capture rather than its completion.
- Decode independent stripes concurrently; validate first and close every decoded
  output on any failure before an atomic draw.
- Optionally convert BGRx directly into cached PyAV I420 planes with libyuv;
  retain cached portable PyAV conversion. x264 is already ultrafast/zerolatency,
  single-threaded per stripe. Four credits no longer trigger normal-load bitrate
  reduction; a two-credit experiment reduced60Hz output to37.11fps and was rejected.
- Empty native credits skip redundant CDP observation only after existing admission
  and lease renewal and when source/policy/tab/document/epoch/cursor/queue/exact
  bindings match. This is a negative-only shortcut: it never emits pixels or
  authorizes capture. CDP fallback keeps its PNG verification deadline.
- Preserve sparse exact damage tiles for pooled native rasters through bounded
  region reads (32768 pixels, owner/mode/size check, no symlink traversal, descriptor
  closed before lease release). Docs now use 62 exact tile packets and 2 initial
  stripe packets. This proves selection behavior, not a typing latency win.
- Physical input retires exact refinement while preserving usable video references.
  Actual full-video fallback retirement/overflow forces a key regardless of stripe
  negotiation. Complete independent key stripes can recover a lost outer cursor;
  partial/dependent stripes still require the cursor and validated references.
- Forward the selected encoder/adapter/converter through the real env-cleared host
  spawn. Require actual encoder and converter receipts. Clear obsolete converter
  identity when switching to full-video fallback. Bind reports/LAN manifests to
  versions derived from the pinned source;447 passes and 446 is rejected.

No relay/kernel serialized shape changes. No batching protocol change. Capture,
source protection, Vault, actor authority and process signal ownership remain
below clients. No provider runtime changes and no GPL library enters the kernel.

## MP-10: invalidated historical labels and optional converter

The launcher omitted SOFTWARE_ENCODER, OPENH264_ADAPTER and LIBYUV from its
allowlist. Phase16 requested OpenH264 rows actually report x264. Initial phase17
SIMD rows also used PyAV, proved by VideoReformatter profiles. The audit retains
all original identities and raw receipts in `encoder-identity-audit.json`; neither
historical claim establishes actual OpenH264 or libyuv behavior. Fail-first real
Rust child-spawn testing proves selectors now cross the boundary and an unrelated
synthetic control setting stays excluded (no secret payload printed).

The isolated 600-frame converter experiment is 3.551 CPU seconds cached PyAV versus
0.182 libyuv; this is not a whole-pipeline speedup claim. Optional helper setting:
`CHARIOX_BROWSER_DISPLAY_LIBYUV=/absolute/path/libyuv.so.0`. The measured public
Debian library is `libyuv0 0.0.1922.20260106-1`, SHA256
`d71b97b26d8415269640bd94b88597fa1c33b6b72043d2cd4d117c2b28c2342f`;
its BSD3 text is archived in `libyuv-license.txt`. It is an externally supplied
helper dependency, not a kernel linkage or bundled default. Missing/failed direct
conversion retains portable PyAV. Encoder default remains owner-selected.

## MP-10: software head-to-head,15 rows

Same builder, 1920x1080/DPR1, 8 Mbps ceiling, local owned namespaces, MTU 1500,
offloads disabled, 20 click and 20 typing samples per case, 10 s motion and 1.5 s idle.
The baselines have their normal local software path; ours includes the encrypted
relay overhead. Rows retain their own source, commands, library hashes, screenshots,
console/kernel logs, CPU samples and cleanup in receipts. Fresh baseline retry
has 10 component passes; ours x264/OpenH264/VP8 has 17. An earlier baseline launch
supplied venv directories instead of Python executables:10 EACCES startup REDs
remain preserved separately, never relabeled as measurements.

MP-08/MP-10/MP-11 — software1080p DPR1, 8 Mbps; RED_PERFORMANCE.
|Fixture|Backend|Click P50/P95 ms|Type P50/P95 ms|FPS / content FPS|Pipeline idle/active cores|Source / viewer active cores|Source+pipeline cores|Video idle/moving Mbps|Live PSNR dB|Settled PSNR / exact|
|---|---|---|---|---|---|---|---|---|---|---|
|docs|Selkies 2.0.0|29.20/32.00|26.10/30.40|—/—|0.40/0.39|0.09/0.86|0.48|2.14/—|—|33.01/False|
|docs|Selkies legacy|31.70/55.40|28.90/49.50|—/—|0.72/0.81|0.09/0.79|0.90|4.99/—|—|31.80/False|
|docs|ours x264|37.00/51.70|21.60/36.90|—/—|0.16/0.87|0.21/0.52|1.08|0.00/—|—|exact/True|
|canvas|Selkies 2.0.0|31.20/47.50|29.30/33.90|61.91/59.82|0.34/0.40|0.26/0.86|0.66|0.19/0.64|30.58–30.67|41.67/False|
|canvas|Selkies legacy|47.80/49.20|46.00/47.60|58.26/57.66|0.64/0.68|0.26/0.82|0.94|0.58/1.05|30.38–30.38|39.04/False|
|canvas|ours x264|36.90/39.10|37.70/54.10|59.73/59.33|0.15/1.30|0.37/1.02|1.67|0.00/1.36|24.92–27.85|exact/True|
|video|Selkies 2.0.0|31.20/48.30|29.50/35.30|61.82/30.06|0.35/0.38|0.12/0.85|0.50|0.17/0.52|41.70–41.70|41.70/False|
|video|Selkies legacy|47.80/48.90|45.80/46.80|57.28/29.99|0.65/0.68|0.13/0.82|0.81|0.60/0.93|27.70–27.73|39.05/False|
|video|ours x264|23.20/39.80|36.30/55.40|30.02/30.02|0.16/1.09|0.16/0.73|1.25|0.00/0.73|41.88–42.26|exact/True|
|scroll30|Selkies 2.0.0|30.90/48.10|29.50/35.40|61.81/61.51|0.42/0.52|0.17/0.96|0.69|3.12/6.85|14.45–14.92|30.74/False|
|scroll30|Selkies legacy|47.70/48.90|46.10/47.30|45.50/45.10|0.75/0.86|0.18/0.75|1.03|5.82/7.80|14.46–14.56|25.45/False|
|scroll30|ours x264|37.50/55.00|38.50/55.50|29.76/29.76|0.17/0.80|0.25/0.70|1.05|0.00/3.69|15.24–16.34|exact/True|
|wheel30|Selkies 2.0.0|30.60/48.00|30.00/33.60|61.40/48.84|0.42/0.56|0.24/0.98|0.80|3.19/7.52|14.85–15.05|30.93/False|
|wheel30|Selkies legacy|48.00/55.00|46.20/62.00|57.81/51.22|0.72/0.99|0.27/0.95|1.26|5.80/7.02|14.82–15.22|26.14/False|
|wheel30|ours x264|37.70/53.90|37.80/53.50|21.96/21.96|0.15/0.93|0.47/0.66|1.40|0.00/2.73|16.75–16.95|exact/True|

MP-10 GPU/VAAPI: Selkies2, legacy and ours are unmeasured for all five fixtures. No MP item closes from this matrix.

MP-10 metric limits: CPU is sampled Linux task ticks; source+pipeline excludes
remote viewer and measurement harness. Idle means frozen media with continuous
credits. Media Mbps includes each transport's media/application envelope but
excludes request/socket overhead. Live PSNR compares a later viewer snapshot with
latest source and includes temporal drift; it is not frame-aligned codec PSNR.
Content FPS uses 64x36 thumbnail changes above the common threshold; presentations
can repeat content or reflect compression noise. Settled RGB exactness is measured
separately. None of these limits is a reason to call the RED comparison GREEN.

## MP-10: actual software encoders,15 rows

The corrected actual Cisco 2.6 screen-content adapter is not close to x264 on
scrolling: 4.18 versus 29.76 fps on scroll30; VP8 is 5.19 fps. OpenH264's 0.93 core
scroll result is bought with the lost frame rate and is not a CPU target win.
All three support exact settle and zero idle media; the default remains unchanged.
The larger scrolling envelopes and pacing are the first measured bottleneck:
x264 mean 21.3 KB, pacing P95 14.35 ms; OpenH264 mean 223.8 KB, pacing P95 194.48 ms;
VP8 mean 173.2 KB, pacing P95 135.66 ms. Motion encode P95 is 13.65/46.97/50.21 ms,
respectively. Some unsent work is retired. These are pipeline observations, not
claims about intrinsic codec limits. No Selkies/noVNC product engineering added.

|Fixture|Encoder|Click P50/P95 ms|Type P50/P95 ms|FPS / content FPS|Owned active / idle cores|Media moving / idle Mbps|Live PSNR dB|Settled|
|---|---|---|---|---|---|---|---|---|
|docs|x264|37.00/51.70|21.60/36.90|—/—|1.08/0.18|—/0.00|—|exact|
|docs|OpenH264|37.00/53.30|33.40/37.00|—/—|1.04/0.19|—/0.00|—|exact|
|docs|VP8|37.60/54.70|30.20/37.00|—/—|1.05/0.19|—/0.00|—|exact|
|canvas|x264|36.90/39.10|37.70/54.10|59.73/59.33|1.67/0.17|1.36/0.00|24.92–27.85|exact|
|canvas|OpenH264|37.90/53.80|38.00/55.00|59.53/59.53|1.82/0.20|1.34/0.00|24.92–27.84|exact|
|canvas|VP8|38.40/55.00|35.70/54.90|59.56/59.56|2.07/0.21|2.47/0.00|26.11–27.85|exact|
|video|x264|23.20/39.80|36.30/55.40|30.02/30.02|1.25/0.19|0.73/0.00|41.88–42.26|exact|
|video|OpenH264|35.70/38.70|37.10/55.40|29.93/29.93|1.28/0.20|0.69/0.00|40.87–40.98|exact|
|video|VP8|23.90/39.60|38.20/55.60|30.02/30.02|1.39/0.18|1.30/0.00|27.85–42.30|exact|
|scroll30|x264|37.50/55.00|38.50/55.50|29.76/29.76|1.05/0.22|3.69/0.00|15.24–16.34|exact|
|scroll30|OpenH264|37.40/55.00|35.60/54.00|4.18/4.18|0.93/0.20|6.31/0.00|13.59–14.05|exact|
|scroll30|VP8|38.50/54.70|36.20/54.70|5.19/5.19|1.12/0.22|6.00/0.00|13.73–13.76|exact|
|wheel30|x264|37.70/53.90|37.80/53.50|21.96/21.96|1.40/0.18|2.73/0.00|16.75–16.95|exact|
|wheel30|OpenH264|36.40/54.60|37.80/54.50|3.08/3.08|1.12/0.18|4.89/0.00|13.82–13.88|exact|
|wheel30|VP8|23.70/39.50|36.90/54.40|3.89/3.89|1.33/0.20|4.43/0.00|13.82–14.00|exact|

MP-10 decode support is measured only on Google Chrome 154.0.8037.97 on builder2:
actual x264 H.264, actual OpenH264 H.264 and VP8 decoded successfully. Edge,
Firefox and Safari remain unmeasured for all three; no browser support inferred.

## MP-10: GPU/VAAPI measurements

|Fixture|Selkies2 VAAPI|Legacy VAAPI|Chariox VAAPI|
|---|---|---|---|
|docs|Unmeasured|Unmeasured|Unmeasured|
|canvas|Unmeasured|Unmeasured|Unmeasured|
|video|Unmeasured|Unmeasured|Unmeasured|
|scroll30|Unmeasured|Unmeasured|Unmeasured|
|wheel30|Unmeasured|Unmeasured|Unmeasured|

MP-10 builder2 has no GPU; owner laptop measurements are required. No GPU result
from an earlier source is claimed here, and no kit repack or deployment performed.

## MP-10: stage profile on the final runtime

The separate instrumented 60 Hz run presents 58.97 fps at 1.87 owned cores. Profiling
adds overhead; the uninstrumented final comparison is 58.98 fps/1.76 cores.
Whole-run cProfile wall time blocked in read/select is not CPU time. Kernel perf
records only instruction addresses after readiness, without inherited children,
call stacks or memory payloads;499samples,0lost. The process observer remains a
kernel CPU cost (pid/proc directory scans); its 25 ms ownership cadence and verified
signal guards were not weakened to improve a score.

|MP-10 moving component|Logical cores, instrumented|
|---|---|
|Source Chromium|0.441|
|Xvfb|0.111|
|Kernel/relay/observer|0.244|
|Node controller|0.275|
|Native capture helper|0.138|
|Encoder helper|0.662|
|Viewer, excluded|1.292|
|Measurement harness, excluded|0.516|

|MP-10 stage|P50 ms|P95 ms|
|---|---|---|
|native_readback|1.459|1.923|
|native_fingerprint|0.957|1.142|
|native_xshm_capture|4.298|5.264|
|host_capture_or_control|14.382|17.705|
|motion_encode|11.220|13.399|
|pacing|11.525|14.840|
|packet_serialize|0.046|0.079|
|event_serialize_encrypt|0.052|0.070|
|event_queue_credit|0.010|0.022|
|event_socket_write|0.061|0.103|

MP-10 nested timings overlap and include waiting; do not add them as CPU or
critical-path latency. Encoder plus Node/kernel work remains the major pipeline
CPU cost. The first typing miss is the capture/credit/input scheduling path:
docs P95 36.90 ms and moving fixtures 53.50–55.50 ms, versus Selkies 30.40–35.40 ms.
The wake/reference changes and sparse tiles did not establish the requested win.
No <=1 core or <=Selkies typing claim is made.

## MP-08/MP-10/MP-11: verification and review mapping

- Fail-first row concurrency, direct planes, credit pressure, input-reference,
  empty-credit and bounded-region tests each retain RED and GREEN logs. Atomic
  decode failure closes all successful outputs before refusing the draw.
- 13:41 inbox finding1 (raw-less negotiated fallback retirement) maps to
  `b5df3da5c` and `3796eb6aa`: stale/in-flight/queued/overflow regressions RED,
  then 19 GREEN. Finding2 (lost outer cursor) maps to `b5df3da5c`:
  producer-to-presenter recovery RED/GREEN. Finding3 (441 reporting) maps to
  `b5df3da5c`: pinned447 acceptance/446 rejection RED/GREEN.
- Final Node suite 596/597 passes. The sole failure is the pre-existing managed
  isolation mode000 fixture under root; identical base reproduces it. The base
  fixture passes as UID65534. No security test or isolation check was weakened.
- Optimized b5df Rust ELF host/process suite 27 passes. Local API suite 383/385
  passes; two TypeScript cases fail because system Node 22.22.1 was built without
  TypeScript stripping. The affected three-test subset passes with official
  pinned Node 22.20.0 (`process.features.typescript=strip`). This does not relabel
  the original broader run as all-green or as a final-source recompile.
- Real SIMD/x264/VP8 planes/row chains 5 pass; encoder/fallback/signal checks 6
  pass; converter fallback transition 2 pass; PNG worker 5 pass. Final Node suite
  includes the added controller/credit/region/decoder regressions.
- 17 final Chariox plus 10 fresh baseline component cases pass; final instrumented
  profile and 65 s continuous-credit idle pass. The latter renews beyond the lease
  horizon, sends 0 idle media bytes, uses 0.15 source+pipeline cores, and retains
  exact pixels, navigation/takeover and owned-resource cleanup.

MP-11 narrowed scope: these tests support semantic guards for observation
protection, input/document binding, peer admission and owned signal handling.
They are not an assertion that every security anchor or ordinary/managed parity
matrix is closed. No non-security exact-blob backlog is invented.

## MP-08/MP-10: interface handoff for the Cloud lane

The supplied Cloud `f6cfd0066d75844dbab795cdf715789ec5fa37d6` has the older
Room `chariox-display-v1` socket client. It lacks the kernel-browser 447 presenter
integration and `scripts/e2e-stack`. The coordinator must supply the real app
integration source and harness before the required real-app red/green drill.
No Cloud checkout is modified by this lane.

Cloud integration needs only the following existing interfaces:

1. Bundle `apps/browser-display/presenter.mjs`, `stripe-presenter.mjs`,
   `decoder-worker.mjs`, `tile-cache.mjs`, and `scroll-prediction.mjs` from this
   OSS source. The decoder worker must resolve as an actual module-worker asset
   from the built app. WebCodecs requires a secure context (localhost is allowed).
2. Await `attachBrowserDisplay(canvas, transport, {tab_id,generation}, options)`
   after the real user-domain browser UI has opened/selected the kernel Tab.
   `transport.request({KernelBrowser:{command}})` resolves the normal encrypted
   kernel reply `{KernelBrowser:{result}}`. `onEvent(listener)` delivers the
   decrypted `kernel_browser_frame` event and returns its unsubscribe function.
   Keep it separate from durable terminal replay; display sequence is transient.
3. Implement `transport.subscribeDisplay(binding)` / `unsubscribeDisplay(binding)`
   with existing relay `client_subscribe` / `client_unsubscribe`,
   `subscription_scope:"kernel_browser_display"`, session_id=subscription_id,
   attachment_id=String(generation), and the existing client public key/target.
   Use the normal authenticated bootstrap and opaque encrypted relay transport.
   Cloud supplies no capture, media proxy, session state or input authority.
4. Require local protocol>= 447 for this view and offer `chariox-stripes-v1`,
   `chariox-video-dependencies-v1`, a supported codec and PNG through the helper.
   Start with `deviceScaleFactor:1`; 1920x1080 supports only DPR1. To reproduce
   this table use `creditWindow:4`, `bitrate:8000000` and the selected supported
   codec (H.264 `avc1.420033` or VP8 `vp8`). Do not mutate the
   Tab viewport on resize outside the existing kernel negotiation. Match kernel
   `CHARIOX_KERNEL_BROWSER_DISPLAY=1`; leave production feature opt-in and use
   the real app's documented feature flag in the acceptance command.
5. `stream.start()` runs bounded credits; `stream.input()` sends observed-document
   bound click/text/key/scroll in CSS coordinates. Bind actual UI mouse/keyboard
   events and human takeover/release to these methods; actors come from
   `stream.actors()`. `onPresented(frame)` fires after validated atomic draw.
   Render errors/retry to the user. Retry closes the old stream and subscribes
   afresh; never replay a lost display_next credit. Await `stream.close()` on
   navigation/unmount/disconnect. Avoid simultaneous `next()` and `start()`.
6. Supply the real app entry/built bundle URL and flags to the shared e2e harness,
   real kernel/relay/CLI binaries, and public fixture origin. Drive click/type,
   source60Hz motion, exact settle, idle, unsupportedDPR, navigation, takeover,
   masking and dropped row-reference recovery through Playwright UI input.
   Run base RED and candidate GREEN with screenshots/console/log captures under
   this lane's evidence directory. A fixture HTML entry or direct IPC-only run
   cannot satisfy this gate. A provider is required for agent/takeover behavior.

The serialized shapes, credit/refusal rules and source/Vault/observation fences
remain the447/90 contract. No new protocol allocation is requested.

## MP-08/MP-10: real product build and MP-11 cleanup

Final runtime `c53693acf` also builds the real dev kernel, relay and CLI launcher
with the compile-slot lock, `CARGO_BUILD_JOBS=4`, `CARGO_PROFILE_DEV_DEBUG=0`.
All help probes pass; the supported kernel protocol print command returns447.
The initial unsupported `--protocol-version` probe exited2 after the build had
succeeded; its wrapper exit1 remains in `final-product-build.log`. Corrected
metadata and exact57 embedded-asset checks are in `final-product-binaries.json`.
These executables are retained in the lane's `phase17-bin/` for the coordinator's
real-app harness. The CLI launcher help is not a built-TUI or provider acceptance.

|MP-10 executable|SHA256|
|---|---|
|chariox-kernel|`f201c4b0bc3eeb2a67f41b03dc123a29edb9670a4d739a4ecac62f554fd12ca4`|
|chariox-cli|`7fd4056dc992e9e6bf604e4d0c7a598b3c90c4e2eccef59870ace6ac83489d99`|
|chariox-relay|`1e5f853f12a18b9f9789b6340a579bb8c0472d15ff4e70e31e3c219d310a0123`|

MP-11 cleanup inventories 74 phase17 receipts: all
128 exact disposable state/short-temp roots and
74 own namespaces are absent; exact-root process
inventory is empty. Public perf symbol mirrors removed. Final MemAvailable is
38.56 GiB and disk free is
115.49 GiB; per-run samples and
`final-cleanup.json` retain evidence. No durable owner keys, provider credentials,
Docker images/containers or deployments created. Public reproduction dependencies
and needed executables are retained; shared Cargo output, other lanes, reviewer
state, credential profiles and key stores are excluded from cleanup.

## MP-08/MP-10 Coordinator asks and MP-10 Owner questions

Supply the paired Cloud kernel-browser 447 display integration, real entry/flags,
shared `scripts/e2e-stack`, and a product-linked provider for agent/takeover
acceptance. The frozen Cloud source remains
`f6cfd0066d75844dbab795cdf715789ec5fa37d6`; both checkouts lack the shared harness.
This is the exact real-app blocker; fixture drills do not close it.

MP-10 owner chooses encoder default later and schedules laptop GPU measurement.
CPU/typing/wheel gaps are measured engineering work, not owner decision blockers.
This requested phase17 handoff stops with those gaps RED. No source/protocol
allocation question prevents continuing them in a later lane task.
