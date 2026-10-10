# MP-08/MP-10/MP-11: phase 16 bounded stripe motion

**MP-10 correction, phase17 (2026-10-06): the requested OpenH264 live rows
below actually used x264.** Audit of their retained `host_timings` reports
`motion_backend_x264`; the Rust host environment allowlist discarded the
software selector and native adapter path. Those rows establish x264 behavior
with an ignored OpenH264 request, not OpenH264 screen-mode performance. Standalone
native codec tests remain separate evidence. Phase17 fixes and checks the real
spawn boundary and requires actual backend/converter receipts before labeling
a comparison. The historical source identities and raw receipts are unchanged.


Assigned base: multidomain round 2 `6dde21a8c10c9ef2b9f7f0a271cace00591f0bd1`.
The phase-15 source is retained locally at `md/display-phase15-retained`.
The replay preserves round-2 private CDP pipes, admission/grant revocation,
typed refusals and DOM mirroring. Coordinator allocation: local 447, relay peer 90.

## MP-08/MP-10: motion and exact scheduling

An owned XDamage readback compares complete raster bands with exact bytes;
XXH3 is a scheduling prefilter. Equal hashes never authorize observation or
suppress different bytes. The native helper publishes changed readbacks using
three private mapped buffers, with serial-bound leases. The encoder holds a
lease until it has consumed the mapping. Node transfers bounded metadata rather
than multi-megabyte raster bytes through its main thread. Startup attestation
reads one immutable snapshot and compares native RGB against protected CDP RGB.
The pool lives inside the disposable owned-display directory, files are 0600,
and the directory is 0700. Encoders verify owner, mode, type and length before
mapping. All files are removed after the owned helper exits.

Motion does not run Node PNG, zlib or whole-raster cryptographic hashing.
Native readback compares exact bands independently of XDamage hints. SHA-256
remains for artifact/source identities and exact PNG verification, outside the
motion scheduler. The protected CDP fallback still decodes its JPEG fingerprint
in the helper; it is outside the admitted native fast path and remains a
performance limitation. Hashes are never credential, source-scope, document,
protection, grant or relay-admission authority.

Exact refinement begins after 300ms quiet. Encoding PNG tiles and decoding the
exact capture run in the existing pixel worker. Pending exact work does not
hold the kernel backend mutex while waiting. Document, policy, input epoch and
source serial are rechecked before delivery. New input retires old exact work;
quiet scheduling cannot weaken the observation barrier.

Microbenchmark: 100 1080p readbacks, same host. SHA helper to XXH3 helper CPU:
dense 466.0ms to 75.3ms; sparse 46.3ms to 23.6ms; idle 8.87ms to 10.87ms.
This measures the helper only, not end-to-end latency or acceptance.

## MP-08/MP-10/MP-11: protocol 447/90

A viewer explicitly offers `chariox-stripes-v1` plus an ordinary supported
codec and PNG. Without that offer the existing whole-frame path remains.
The frame kind is `stripes`, with the existing subscription, generation, tab,
document, canvas geometry and monotonically increasing outer sequence. It also
contains `base_sequence` and one to eight full-width rows. Each row has `row`,
`y`, `height`, `codec`, `key`, `sequence`, `reference_sequence`, `data_base64`.
H.264 uses `avc1.420033`; the requested BSD software comparison additionally
uses `vp8`. The relay transports encrypted opaque events as before.

Eight even-height rows partition the complete canvas. Only changed rows encode.
Exact byte comparisons guard fast-hash collisions. Each row owns its own codec
reference chain. Dropped queued rows force only those rows to independent keys;
full navigation/protection/source retirement resets all rows. Fresh document
bootstrap requires a complete independent cover. Client validation rejects
lost row references, overlap, dimensions, bounds and wrong canvas base before
any draw. Every decoded row is validated before one synchronous draw commit.
Exact PNG/tile settle remains on the same protected capture path. An unchanged
readback creates no motion packet; idle measurement distinguishes media bytes
from existing control credits.

No cursor or copy-rectangle protocol was added. The implementation is original;
no pixelflux dependency or upstream source was copied.

## MP-11: encoder choice and licensing

`CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER` selects `libx264`, `libopenh264`,
or `libvpx`; codec negotiation must match H.264 or VP8. Encoding runs in the
separate Python helper; GPL libraries are not linked into the kernel process.
The choice is pluggable, not a distribution approval. x264 is GPL; OpenH264
is the BSD H.264 software alternative; libvpx is BSD. Separating a process does
not remove the distributed dependency's obligations.

The supplied FFmpeg/PyAV adapter does not expose screen-content usage. The
original `kernel-browser-openh264.cpp` adapter selects `SCREEN_CONTENT_REAL_TIME`
through Cisco's native API and verifies the selected usage through `GetOption`.
Its Python wrapper pins Cisco 2.6.0, consumes existing admitted I420 frame planes,
and bounds output to 1MiB. No capture or file/network access occurs in the adapter.
`CHARIOX_BROWSER_DISPLAY_OPENH264_ADAPTER` selects the external native adapter;
without it OpenH264 uses the older FFmpeg adapter and must not be described as
screen-content mode. Protected PNG fallback preserves the selected encoder.

The drill downloads Cisco's prebuilt 2.6.0 library over HTTPS outside the source
checkout, builds the original adapter against pinned BSD API headers, then copies
only those two public libraries into its disposable runtime. Runtime-download
URLs, source/header/library hashes and build commands are retained in evidence.
No upstream source or binary is committed. Distribution and patent coverage
remain owner decisions; a runtime download receipt is not legal clearance.
See the [Cisco release](https://github.com/cisco/openh264/releases/tag/v2.6.0),
[Cisco API](https://github.com/cisco/openh264/blob/v2.6.0/codec/api/wels/codec_api.h),
[FFmpeg adapter](https://github.com/FFmpeg/FFmpeg/blob/n8.0/libavcodec/libopenh264enc.c)
and [libvpx license](https://www.webmproject.org/license/software/).

## MP-08/MP-10/MP-11: acceptance boundary

The supplied `/root/work/cloud` checkout has no real-app display integration,
and neither checkout contains `scripts/e2e-stack`. The private paired Cloud ref
cannot be fetched with the current Git access. A presenter harness and kernel
libtest relay drill are component evidence only. Building product binaries does
not turn those fixtures into the real-app path. Real-app red/green acceptance
requires the coordinator to supply the paired app integration ref or harness.
The software comparison and focused guards are recorded separately below;
none closes an MP item on its own.

## MP-08/MP-10/MP-11: final measurements and evidence

Final controller/client source: `de80aba523d67c06e8c2708591b70ac0c89fd489`,
clean during every final encoder case. Clean baseline source: `94b4fe903`.
Five identical fixtures per backend, 1920x1080 DPR1, software only, 8 Mbps
ceiling, sequential owned namespaces with MTU1500 and offloads disabled.
Baselines use their normal whole-frame software pipelines; no Selkies changes.
Chariox includes the production encrypted local-relay request/event path.
Each receipt binds the optimized kernel libtest ELF SHA-256 separately from
its source-overridden controller assets and presenter assets. The ELF SHA-256
is `c5d779d672a58f2c238376c4b7b7a0140861f3e3adc43493c797dac95e45e470`.
These runs use a fixture entry and embedded kernel/relay test services, so
`PASS_LOCAL_COMPONENT` does not mean real-app acceptance.

All fifteen Chariox encoder cases pass exact settle, default DPR negotiation,
explicit unsupported-DPR refusal, navigation, takeover and cleanup, with zero
idle media bytes. Control credits remain. The owner target and Selkies win are
**RED**: none of the five final x264 rows meets the combined CPU/latency target.
Owned CPU means source Chromium plus capture/encode/kernel/relay, excluding
the viewer. The report now uses that combined metric rather than pipeline alone.
Source/pipeline partition can vary if the sampler observes a wrapper before
exec; the combined total is the useful conservative result.

Restoring ordinary Chromium animation cadence reduces owned canvas CPU from
4.48 to 1.90 cores and video from 2.96 to 1.49 (same software x264 fixtures),
retaining about 55fps/30fps. That before source is `4a338bbe1`; the after source
also includes quiet native verification and VP8 configuration, so this is a
combined live comparison, not an isolated attribution. Native serials now
capture exact PNG once after quiet; lossy CDP fingerprints retain their 250ms
hidden-change verification deadline. Both scheduling changes have fail-first
source tests. The mapped pool moves raw raster copies off Node's motion pipe;
final requested-OpenH264 (actual x264) docs native metadata/copy P95 is 0.09ms and motion encode 4.44ms.
A separate full-path pool-only ablation was not completed; do not claim a
measured CPU reduction attributable solely to the pool.

The x264 row VBV bound repairs the earlier scrolling burst/pacing failure:
1.40fps at `ce5c6feb2` to 29.65fps at `96497c6c7`, with matching fixture/geometry.
VP8 remains RED after its burst options: scrolling 4.00fps at `4a338bbe1` to
4.99fps at `de80aba52`. Its final scrolling motion encode P95 is 6.02ms but
transported event bytes average roughly 144kB per presented frame, versus
about 16kB for x264. Packet size, pacing and reference retirement need further
isolation; a definitive VP8 root cause is not claimed. Final requested-OpenH264 (actual x264) docs
input round trip P95 is 11.40ms, native capture 6.54ms, encode 4.44ms,
queued-event-to-viewer 1.30ms, decode 0.80ms, input-to-draw 39.30ms and draw-to-rAF
14.20ms. Spans overlap; summing their P95 values would be invalid.

Evidence root: `<lane evidence>/phase16/`.
`final-{x264,openh264,vp8}/` contain per-step screenshots, kernel/client logs,
resource samples, identities, command/exit receipts and cleanup.
`baselines-clean/` is the final ten-row baseline. Earlier failed/dirty campaigns
are retained under their own identities and are excluded from the final table.
`comparison-final/` contains machine-readable rows and the following tables.

MP-08/MP-10 software comparison (full fifteen rows):

|Fixture|Backend|Click P50/P95 ms|Type P50/P95 ms|FPS / content FPS|Pipeline idle/active cores|Source / viewer active cores|Source+pipeline cores|Video idle/moving Mbps|Live PSNR dB|Settled PSNR / exact|
|---|---|---|---|---|---|---|---|---|---|---|
|docs|Selkies 2.0.0|30.40/31.60|28.90/30.90|—/—|0.39/0.38|0.09/0.86|0.47|2.17/—|—|33.01/False|
|docs|Selkies legacy|32.00/47.70|46.10/47.50|—/—|0.70/0.74|0.09/0.90|0.83|4.98/—|—|31.77/False|
|docs|ours|37.70/54.40|35.50/36.80|—/—|0.19/0.98|0.41/0.47|1.39|0.00/—|—|exact/True|
|canvas|Selkies 2.0.0|30.70/35.60|29.30/41.00|61.89/59.69|0.34/0.40|0.26/0.86|0.66|0.21/0.65|26.47–30.67|41.67/False|
|canvas|Selkies legacy|31.20/32.00|29.40/30.20|59.83/59.44|0.70/0.67|0.26/0.84|0.93|0.60/1.06|39.04–39.04|39.04/False|
|canvas|ours|23.90/53.20|38.20/55.30|55.10/54.70|0.20/1.66|0.24/0.96|1.90|0.00/1.29|26.15–26.15|exact/True|
|video|Selkies 2.0.0|31.60/32.00|29.60/31.20|61.81/30.01|0.34/0.38|0.12/0.85|0.50|0.18/0.52|41.70–41.70|41.70/False|
|video|Selkies legacy|31.20/31.90|29.50/30.20|59.85/29.97|0.65/0.67|0.12/0.86|0.79|0.57/0.95|27.68–39.05|39.05/False|
|video|ours|23.50/40.00|24.90/55.00|30.04/30.04|0.16/1.05|0.44/0.69|1.49|0.00/0.75|27.84–41.15|exact/True|
|scroll30|Selkies 2.0.0|31.00/48.50|30.10/34.80|61.82/61.52|0.43/0.51|0.17/0.94|0.68|3.14/6.92|14.63–14.91|30.88/False|
|scroll30|Selkies legacy|38.20/55.30|46.80/62.10|31.33/31.03|0.69/0.81|0.17/0.58|0.98|5.80/7.75|14.30–14.52|29.71/False|
|scroll30|ours|37.60/54.10|38.00/55.30|29.72/29.72|0.19/0.91|0.28/0.67|1.19|0.00/3.69|16.12–16.82|exact/True|
|wheel30|Selkies 2.0.0|31.70/48.00|30.00/35.50|61.35/49.28|0.41/0.55|0.24/0.95|0.79|3.09/7.52|14.66–14.70|30.93/False|
|wheel30|Selkies legacy|31.50/55.30|31.60/46.90|49.64/44.55|0.67/0.92|0.25/0.78|1.17|5.76/7.12|14.82–15.01|29.76/False|
|wheel30|ours|24.30/54.00|37.60/55.20|22.05/22.05|0.16/0.94|0.46/0.60|1.39|0.00/2.74|14.89–17.11|exact/True|

MP-10 GPU/VAAPI: Selkies2, legacy and ours are unmeasured for all five fixtures. No MP item closes from this matrix.

MP-08/MP-10/MP-11 encoder, browser and GPU comparison:


|Fixture|Encoder|Click P95 ms|Type P95 ms|FPS / content FPS|Owned active / idle cores|Media moving / idle Mbps|Live PSNR dB|Settled|
|---|---|---|---|---|---|---|---|---|
|docs|x264|54.40|36.80|—/—|1.39/0.24|—/0.00|—|exact|
|docs|x264 (OpenH264 requested)|47.70|37.70|—/—|1.42/0.21|—/0.00|—|exact|
|docs|VP8|53.30|39.40|—/—|1.38/0.27|—/0.00|—|exact|
|canvas|x264|53.20|55.30|55.10/54.70|1.90/0.25|1.29/0.00|26.15–26.15|exact|
|canvas|x264 (OpenH264 requested)|55.20|56.00|54.79/54.59|1.93/0.27|1.28/0.00|24.91–26.16|exact|
|canvas|VP8|54.90|55.30|55.25/55.25|2.10/0.26|2.47/0.00|24.92–26.15|exact|
|video|x264|40.00|55.00|30.04/30.04|1.49/0.25|0.75/0.00|27.84–41.15|exact|
|video|x264 (OpenH264 requested)|55.00|54.60|29.97/29.97|1.53/0.27|0.75/0.00|27.85–41.67|exact|
|video|VP8|55.10|54.40|29.98/29.98|1.58/0.25|1.28/0.00|27.85–42.56|exact|
|scroll30|x264|54.10|55.30|29.72/29.72|1.19/0.28|3.69/0.00|16.12–16.82|exact|
|scroll30|x264 (OpenH264 requested)|55.50|53.80|29.14/29.14|1.26/0.25|3.63/0.00|14.76–15.20|exact|
|scroll30|VP8|54.60|54.90|4.99/4.99|1.14/0.25|5.74/0.00|14.38–14.55|exact|
|wheel30|x264|54.00|55.20|22.05/22.05|1.39/0.23|2.74/0.00|14.89–17.11|exact|
|wheel30|x264 (OpenH264 requested)|53.70|55.00|22.02/22.02|1.50/0.26|2.74/0.00|16.60–16.92|exact|
|wheel30|VP8|54.50|55.10|3.88/3.88|1.41/0.24|4.36/0.00|14.00–14.24|exact|

MP-10 decode support:

|Browser|x264 H.264|OpenH264 H.264|VP8|
|---|---|---|---|
|Chromium on builder2|Measured yes|Measured yes|Measured yes|
|Edge|Unmeasured|Unmeasured|Unmeasured|
|Firefox|Unmeasured|Unmeasured|Unmeasured|
|Safari|Unmeasured|Unmeasured|Unmeasured|

MP-10 GPU/VAAPI: owner laptop required; builder2 has no GPU.

|Fixture|Selkies2 VAAPI|Legacy VAAPI|Chariox VAAPI|
|---|---|---|---|
|docs|Unmeasured|Unmeasured|Unmeasured|
|canvas|Unmeasured|Unmeasured|Unmeasured|
|video|Unmeasured|Unmeasured|Unmeasured|
|scroll30|Unmeasured|Unmeasured|Unmeasured|
|wheel30|Unmeasured|Unmeasured|Unmeasured|

## MP-08/MP-10/MP-11: validation and remaining acceptance

Node focused suites:236 passed. Rust:26 host tests (five ignored),194 local
protocol guards,16 relay-peer guards,20 kernel-event guards,95 library tests;
optimized local protocol guards 194 passed. Client TypeScript contract tests:
191 passed initially with one missing compiled local dependency; after compiling
that public module, its two tests pass (193 total across the two runs).
Real codec tests:four x264/VP8 and four native OpenH264/VP8 passed; raster tests:
seven passed. These counts describe focused checks, not the full repository CI.

Red-capable receipts include `geometry-default-red.log` / `geometry-green.log`,
`stripe-contract-red.log` / `stripe-node-tests-green.log`,
`source-cadence-red.log` and `native-settle-red.log` / the 236-test green log.
Focused presenter tests cover lost row references without partial draw,
per-row recovery and invalid geometry; source tests cover protection/reset,
pooled leases, import closure and lossy fingerprint hidden changes. The real
app loss/masking/settle/idle/geometry drill remains blocked by the paired app
source/harness gap above. No fixture result closes MP-08, MP-10 or MP-11.

The feature stays opt-in. MP-11 scope follows the owner's narrowed behavioural
parity and security-anchor review rule; no non-security exact-blob backlog is
asserted. The10:14 request is addressed by original Cisco screen-content API,
runtime binary provenance, pluggable software choices and the measured table.
The09:33 #893 P2 is fixed fail-first and passes the default/probe component path;
real-app confirmation requires the missing paired source. Final status:
**NOT DONE / RED_PERFORMANCE / BLOCKED_REAL_APP_ACCEPTANCE**.

MP-08/MP-10/MP-11 final product kernel/CLI/relay builds pass. Kernel reports 447;
CLI/relay help smoke pass. Cleanup 118 roots/70 namespaces absent; exact-root
process inventory empty. Validation manifest binds binary hashes and commands.
