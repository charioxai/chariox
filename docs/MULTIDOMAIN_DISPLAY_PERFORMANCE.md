# MP-08/MP-10/MP-11: multidomain display performance

MP-08/MP-10/MP-11 current phase18 results: [source-bound report](MULTIDOMAIN_DISPLAY_PHASE18_RESULTS.md). Performance remains RED; native privacy is INTERMITTENT_RED; real-app/GPU acceptance is blocked. Protocol447/relay90 are unchanged in phase18. The following measurements are historical and retain their original protocol/source identities.

**RED_PERFORMANCE.** At1920×1080 DPR1, software only, Chariox preserves exact
settled RGB but misses the≤1 pipeline-core, click/type P95<50ms and scroll≥30fps
owner targets. It does not beat Selkies on every metric. GPU acceptance is
unmeasured. The experiment remains default-off; local protocol441/relay84 and
serialized public shapes are unchanged. Default1280×800/DPR2 remains compatible.

Base: `a28ac99935baaacd90358753a34b08a85bc2efd0`.
Measured harness: `75877f60d` (clean); embedded kernel build: `f59184e57`
(clean). The receipt binds every executed embedded controller/client blob;
harness-only changes after the build do not change embedded assets. ELF SHA256:
`88829473ffb8c3ad598a1a1765c58aab15eb177b0025b017de686645b800545c`.

## MP-10 same-host software comparison

Selkies2.0.0 uses upstream pixelflux2.1.0/WebSocket x264. Legacy is upstream
`17a3d5a1213257dc3285be5c194b16580dc19fee`, GStreamer/WebRTC x264enc.
All rows use the same fixtures,1080p DPR1,8Mbps ceiling,20 click and20 typing
pixel acknowledgements,10-second motion windows and identical Linux CPU counters.
Both campaigns use owned namespaces with MTU1500, loopback offloads disabled and
an unrouted TEST-NET interface for normal WebRTC ICE gathering. Other lanes share
this host; these are owned-process counters, not an isolated-machine benchmark.

CPU(a)=source Chromium; CPU(b)=Xvfb/capture/encode/kernel-server/relay transport;
CPU(c)=viewer browser. The owner core target applies to(b). Active means motion,
or the docs click window. Idle uses continuously attached viewers. Source+pipeline
reports(a)+(b); harness CPU is separately retained in JSON. Exited tasks retain
sampled high-water ticks; tasks shorter than100ms can be missed.

|Fixture|Backend|Click P50/P95 ms|Type P50/P95 ms|FPS / content FPS|Pipeline idle/active cores|Source / viewer active cores|Source+pipeline cores|Video idle/moving Mbps|Live PSNR dB|Settled PSNR / exact|
|---|---|---|---|---|---|---|---|---|---|---|
|docs|Selkies 2.0.0|25.50/31.30|26.40/30.40|—/—|0.40/0.43|0.11/0.90|0.53|2.08/—|—|33.01/False|
|docs|Selkies legacy|32.00/47.70|46.30/47.20|—/—|0.70/0.74|0.10/0.87|0.83|4.98/—|—|31.79/False|
|docs|ours|54.00/55.40|92.70/95.70|—/—|0.40/1.17|0.42/0.29|1.59|0.00/—|—|exact/True|
|canvas|Selkies 2.0.0|31.40/37.10|29.90/32.80|61.91/59.92|0.34/0.41|0.26/0.87|0.67|0.21/0.67|26.47–30.67|41.67/False|
|canvas|Selkies legacy|31.30/33.10|29.90/32.10|58.93/58.73|0.61/0.70|0.27/0.85|0.97|0.58/1.07|39.04–39.04|39.04/False|
|canvas|ours|38.60/55.40|62.60/80.40|46.61/46.61|0.45/3.00|2.47/0.93|5.47|0.00/2.44|19.17–23.58|exact/True|
|video|Selkies 2.0.0|30.60/48.00|29.30/46.70|61.81/29.96|0.37/0.40|0.12/0.88|0.52|0.20/0.58|27.83–27.83|41.70/False|
|video|Selkies legacy|31.30/39.40|29.30/30.90|58.12/30.06|0.58/0.64|0.12/0.82|0.76|0.56/0.95|39.05–39.05|39.05/False|
|video|ours|38.60/62.70|62.50/77.60|30.02/30.02|0.42/1.97|1.76/0.69|3.74|0.00/0.92|32.54–32.57|exact/True|
|scroll30|Selkies 2.0.0|31.60/35.50|29.90/36.20|61.83/61.63|0.42/0.53|0.17/0.98|0.71|3.13/6.91|14.36–14.54|30.76/False|
|scroll30|Selkies legacy|31.40/37.90|29.30/46.00|39.59/39.29|0.72/0.79|0.16/0.65|0.95|5.88/7.81|14.35–15.18|24.55/False|
|scroll30|ours|38.60/55.10|124.70/132.00|29.35/29.35|0.41/1.74|0.29/0.55|2.03|0.00/3.34|15.24–15.41|exact/True|
|wheel30|Selkies 2.0.0|31.00/48.50|28.80/34.60|61.31/49.12|0.40/0.57|0.24/0.98|0.82|3.12/7.40|14.89–15.02|30.93/False|
|wheel30|Selkies legacy|47.10/49.30|46.00/48.40|59.08/52.08|0.70/0.93|0.25/0.91|1.18|5.80/7.02|14.75–15.14|29.76/False|
|wheel30|ours|39.00/56.00|124.40/143.00|20.25/20.25|0.44/1.79|0.50/0.46|2.30|0.00/2.31|14.67–19.06|exact/True|

FPS counts presentations, including repeated Selkies frames. Content FPS is a
common64×36 thumbnail heuristic (>0.1% pixels differ by>8 RGB); codec noise can
inflate it and small changes can be missed. Neither count proves source-frame
alignment. Video Mbps counts encrypted Chariox frame envelopes, Selkies2 stripe
messages or legacy inbound RTP payloads; it excludes requests, TCP/TLS/DTLS
framing and diagnostic screenshots. These scopes differ at the transport layer
and must not be called physical-link bandwidth. The same8Mbps ceiling is used.

Live PSNR compares latest source PNG with a later viewer PNG and includes temporal
drift. Thus the former7–20dB-vs46dB claim compared different measurements: under
this common sampler, both Selkies scroll paths also measure14–15dB. Held-motion
PSNR is reported separately in the settled column. This does not prove superior
moving codec quality; Chariox's downscaled motion still loses detail.

## MP-10 laptop GPU comparison

|Fixture|Selkies2.0.0 hardware|Legacy VAAPI|Chariox VAAPI|
|---|---|---|---|
|docs|unmeasured|unmeasured|unmeasured|
|canvas|unmeasured|unmeasured|unmeasured|
|video|unmeasured|unmeasured|unmeasured|
|scroll30|unmeasured|unmeasured|unmeasured|
|wheel30|unmeasured|unmeasured|unmeasured|

Builder DRM is virtio, not a proven hardware encoder. The kit's successful VAAPI
probe/fallback establishes startup only. The Intel UHD620/iHD laptop must prove
actual backend use and every metric before either mode can reach acceptance.

## MP-08/MP-10 work removed and remaining gaps

Software motion at≤16Mbps now encodes1080p at1280×720 using fast bilinear scaling;
exact PNG/tile restoration stays native. Hardware retains native dimensions.
The final canvas pipeline uses3.00 cores versus3.60 in the before pilot; scroll
uses1.74 versus2.09. These are bounded observations with different harness
revisions, not a causal isolated pipeline experiment. The encode-only alternating
captured-frame benchmark measured4-thread720p CPU0.679→0.419 seconds/120 frames
for bicubic→fast bilinear. One/two/four-thread receipts are retained; wall-time
and end-to-end CPU are separate. Existing ultrafast/zerolatency, no B-frames,
bounded VBV and intra-refresh remain. Independent-IDR recovery is tested.

Damage-driven capture and unchanged-frame suppression remain active; idle emits
no Chariox video frames. A one-credit pilot reduced CPU but cut scroll to18fps
and wheel to4.7fps, so the three-credit window is retained. Sparse native rasters
already share immutable64-row bands and exact tiles read regions directly;
dense motion still materializes/copies a full raster through the encoder pipe.

Largest open costs: full-raster copies, empty-credit CDP/control work and periodic
exact CDP verification. Typing overlaps slow exact readbacks; a bounded type-window
summary is retained in `critical-path.json`, without causal per-keystroke claims.
Protection, document/visibility fences, replay admission and process identity checks
remain authoritative. No verification seam was weakened to improve a number.

Docs click stages (nested spans are not additive; rAF is a software proxy):

|Stage|P50ms|P95ms|
|---|---|---|
|native_damage_scan|0.15|0.19|
|native_pipe|0.18|0.35|
|native_xshm_capture|3.76|6.74|
|host_input|6.73|10.33|
|input_round_trip|17.30|24.50|
|input_to_draw|41.60|46.70|
|draw_to_raf|10.40|16.00|
|queued_event_to_viewer|1.00|1.70|

## MP-08/MP-11 capture/encode interface for Computer Use

Capture implements `start()`, `subscribe(callback)`, `sample(afterSerial)`,
`valid()` and `close()`. Samples carry source/document/policy binding, monotonic
serial/signature and immutable raw width/height/BGR0 pixels, with optional damage
bounds and `readRegion()`. LinuxCapture currently admits one kernel-owned browser
window on its owned Xvfb after exact CDP attestation; it never captures a root
window. Browser-specific document/visibility checks stay in that adapter. A
whole-desktop/region adapter must prove the kernel-owned display and scope rather
than bypass those checks or pretend to be a browser window.

MotionEncoder consumes this interface, coalesces one pending sample and bounds
encoded dependencies to two packets. Encoders accept admitted pixels, bitrate,
reset/codec and close; they never open a capture source. NativeRefiner receives
an exact-capture callback and binds repairs to immutable source/document/policy/
input-epoch/serial. These source/encode/settle boundaries can serve desktop regions;
root-display capture, WM integration, copy-rect detection and a cursor channel
are future work, not implemented acceptance claims. Encoding exists only while
an attached display owns the producer. Actor pointer projection now follows the
bounded host viewport, including the1080p right/bottom edge.

## MP-10/MP-11 validation, kit and evidence

All15 final fixture rows complete; Chariox5/5 settles exactly. Owner target rows:
0. Configured Node suite165 passes plus the standalone upstream-wheel test1 pass,
zero skips; Python encoder6 passes; Rust32
passes/4 deliberately ignored live drills. Real campaigns invoke the ignored display protocol drill. Geometry/masks, software decoded dimensions, presenter bounds, pointer
projection and upstream wheel direction have fail-first receipts. Initial stale
shared Cargo/type diagnostics and loopback-only ICE failures remain separate RED
receipts; they are not relabelled as passing comparisons.

The official Node22.20.0 archive is checksum-pinned, with no distro libnode.
Initial KIT_READY was delivered within45 minutes and verified on clean Arch and
Bookworm, including native1080p software docs and VAAPI-probe/fallback. The refreshed
kit includes both upstream baseline viewers and exact software/VAAPI invocations,
plus the owner-authorized Arch system-plugin package list in `LAN_KIT.md`.

Evidence root:
`<lane evidence>/phase14/`:
`comparison-final/`, `ours-final3/`, `baselines-final2/`, `binary.json`,
`critical-path.json`, `encoder-tuning.json`, failure/green logs and resource/cleanup
inventories. Portable kit/manifest/SHA/revision/README/KIT_READY remain under the
sibling `phase10b/`. Local commits only; no CI/push/deploy/shared-service changes.
No hosted/managed/provider, physical-photon or laptop GPU acceptance is claimed.
These source tests and fixture runs do not close MP-10 or MP-11 security review.


## MP-08/MP-10/MP-11: phase16 mapped capture interface for Computer Use

The source contract remains `start()`, `subscribe(callback)`, `sample(afterSerial)`
and `close()`. An admitted native sample's raw raster now has `width`, `height`,
`format: bgr0`, `length`, `shared: {path,length}` and non-serialized `retain()` /
`release()` leases. Three private mapped slots are serial-bound and cannot be
reused until all source/encoder references release. `pixels` is a cached immutable
startup/exact snapshot getter; motion encoders consume the shared mapping instead.
The source owns window/display fences and observation admission; the encoder has
no capture authority. Desktop sources must preserve those fences and lease
lifetimes before reusing this interface. Stripe encoding, quiet exact scheduling
and the presenter remain independent of browser DOM. Cursor/copy-rectangle work
is deferred. See [phase16 contract and results](MULTIDOMAIN_DISPLAY_PHASE16_STRIPES.md).
