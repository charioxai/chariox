# MP-08/MP-10/MP-11 — phase18 CPU, typing and protected display evidence

MP-08/MP-10 performance and real-app acceptance remain RED. MP-11 P1 has fail-first source-mask evidence, but final native privacy is INTERMITTENT_RED and the unchanged Cloud gate has not run. Current security-anchor review is coordinator work. This report does not close ordinary/managed parity or hardware acceptance.

MP-08/MP-10 source: basec42d74363; final implementationc0e8a1fdfc352dcb6548d12222e811386db004bd; local447/relay90. [skip ci] local commits only. Protocol and client serialized shapes are unchanged. The native descriptor is private controller IPC, consumed/removed by the kernel before projecting the original stripe event.

MP-08/MP-10 changes:
- Admitted input wakes actual XDamage readback and may borrow bounded32KiB from pacing; debt is repaid and normal traffic retains the negotiated ceiling. This does not prove causal key-to-paint matching: capture timestamps and overlapping trace spans remain a limitation.
- Encoded stripe bytes move from external codecs through a kernel-created private spool into one kernel-assembled event. Node handles control/row headers and capture admission, rather than encoded payload bytes. Node still performs source policy/document/visibility work; it has not disappeared from all per-frame work.
- Pluggable x264/OpenH264/VP8 remain external. One codec thread per row; ultrafast/zerolatency/no-scenecut; unchanged row bytes skip conversion/codec work. One shared1/2/4-worker pool is selectable. Live scroll60 previews1.718/1.839/2.065 owned cores had similar54fps cadence; serial is now default. Isolated pool wall-time benefits were not a live CPU win.
- Trusted empty DOM protection is event-bound. The first full trusted DOM snapshot also enables CDP DOM events. Native source retirement covers relevant tree/marker changes during initial metadata loading, attestation and streaming; a late metadata reply cannot revive admission. Only a stable empty snapshot skips repeated DOM transfers. Protected pages retain per-frame metadata checks. Source/window ownership, document/visibility and Vault admission fences remain in force. CDP contract: https://chromium.googlesource.com/devtools/devtools-frontend/+/main/third_party/blink/public/devtools_protocol/browser_protocol.json (DOM.getDocument).

MP-11 P1 fixes:
- Native rasters are copied/masked before encoding; unmasked shared-file/readRegion accessors are removed from masked samples. Private-only pixel changes have identical displayed fingerprints. Shared leases are never mutated.
- CDP screencast bytes are wake signals for fresh protected captures before fingerprinting/full-video encoding. Display screenshot, exact PNG/tile repair, ordinary screenshots and legacy streams share trusted masking; crops/thumbnails operate on masked viewports. Unknown/racing metadata becomes opaque; stale documents/cancellation still refuse.
- Attribute-only protection changes retire cached pixels/references. CDP region revisions retain the hidden renderer lease, so retirement cannot interrupt paired pointer input. Full protected screenshot capture uses the existing input-priority sample lane.
- Lossy video can reconstruct a black mask with dark RGB ringing. The component harness separately requires source screenshot black0, checks lossy presented masks RGB<=64 with alpha255, and exact PNG/settled black0. Boundary regressions require masked native bytes before any codec/shared-file route. The pre-mask red/text leak still fails the final harness. Earlier dark RGB9/9/15 and max34 diagnostics are retained. Final native run fails this unchanged check in two frames; source and settle assertions pass. Cause remains unresolved, so these findings are not dismissed as codec noise.

MP-11 review mapping:15:28 disjoint row overflow ->bafbc9352 (independent full-row recovery, ordinary/native-descriptor fail-first regressions);16:04/16:08 protected display leak ->d89b733e2+a10ecd6b6+c0e8a1fdf. c0 also closes a pre-attestation marker-admission window and prevents late guard revival. No reviewer approval is inferred. Non-security exact-blob review is outside narrowed MP-11 scope.

MP-08/MP-10 tuning:240 identical public historical wrap-scroll1080p BGRx frames,8Mbps negotiation/60fps normalization. Frozen base stripe SHA2569a10bc4b7494eff36e079f1f90ac2545d766f84a6e539bb349a43a315d7ae42a. Isolated tuning excludes capture, Node, kernel, relay/viewer and real-app flows. QP variants reduce CPU but their nested-base64 event bytes exceed8Mbps, including QP40; they were rejected. Pool1/2/4 emits identical bytes. Exact commands/settings/hashes are in external tuning JSON/scripts.

MP-08/MP-10 comparison limits: identical1080p DPR1 public fixtures,8Mbps ceiling, sequential owned namespaces with MTU1500/offloads disabled. Owned CPU = source+pipeline, with viewer/harness separate. Linux task ticks can miss very short-lived children. Bandwidth excludes request/TCP/TLS/DTLS/diagnostic overhead. Live PSNR has temporal drift; it is not frame-aligned codec PSNR. Exact settled RGB is independently read back. Canvas/rAF is a software presentation proxy, not photon latency. Trace spans overlap and are not additive or guaranteed causal.

MP-08/MP-10 source identities: pre-P1 exact9ba27cases are historical/unsafe, not current performance evidence. Checkpoint runtimea10/harness2ab27cases are distinct: all component checks pass but repeated full-DOM transfer raised scroll60 sourceCPU~0.91, ownedCPU2.40, typeP9570.6ms. c0 final tables supersede them. Dirty/overridden previews stay diagnostic. The final pre-mask RED is exact9ba ELF/controller assets with a borrowed c0 harness (the only dirty source file); its separate identity receipt binds both. The attempted old ELF/current-source run was rejected by embedded-asset verification before privacy assertions and is not leak evidence.

MP-08/MP-10 WebCodecs: measured Chromium admits avc1.420033 for x264/OpenH264 and vp8 for libvpx. Edge/Firefox/Safari are UNMEASURED here; actual platform capability admission must use VideoDecoder.isConfigSupported. x264 is outside the kernel process. No encoder-license/default owner decision is invented.

MP-10 GPU/VAAPI: Chariox and Selkies hardware rows are UNMEASURED for docs/canvas/video/scroll30/wheel30. Builder2 has no GPU and no owner laptop endpoint was supplied. Existing LAN instructions list owner-installable hardware packages; software results do not establish hardware acceptance.

MP-08/MP-10/MP-11 real-app blocker: coordinator18:10 additionally requires real desktop DPR1/2, hosted relay shaped~8Mbps/real RTT, Wikipedia article/portal, GitHub repo, Google results, news, MDN, a real video site and canvas app. The lane is explicitly forbidden to contact the hosted relay; coordinator execution/access is required.  supplied Cloudf6cfd006 lacks local md/display-webaa9c44f8/e2e-stack object/worktree and real entry/flags. Coordinator16:08 requires rerunning its unchanged /masked gate on the lane head. The fixture/libtest launcher exercises real scoped admission, relay transport, Chromium capture, codecs and client canvas, with source/presentation screenshots/logs; its entry/launcher do not satisfy the binding real built Cloud/TUI/provider acceptance path. Product binary builds/help are not substitutes.

MP-11 resource/cleanup proof and exact command exits are in the external final manifest. Evidence root:<lane evidence>/phase18/. Own temporary base/red worktrees and generated client files are removed; final binaries remain in the lane-owned final18-bin for coordinator drills; eight obsolete lane ELF copies totaling794,327,856 bytes were removed. No GitHub push/PR/comment/CI, deployment/Cloud staging, provider credential operation, shared service restart or foreign cache/container/image deletion occurred.

## MP-08/MP-10 — final same-host software comparison

MP-08/MP-10/MP-11 — software1080p DPR1, 8 Mbps; RED_PERFORMANCE.
|Fixture|Backend|Click P50/P95 ms|Type P50/P95 ms|FPS / content FPS|Pipeline idle/active cores|Source / viewer active cores|Source+pipeline cores|Video idle/moving Mbps|Live PSNR dB|Settled PSNR / exact|
|---|---|---|---|---|---|---|---|---|---|---|
|docs|Selkies 2.0.0|30.30/31.40|26.70/29.80|—/—|0.42/0.44|0.10/0.92|0.54|2.10/—|—|33.01/False|
|docs|Selkies legacy|31.10/58.70|30.00/60.50|—/—|0.80/0.84|0.11/0.92|0.95|4.98/—|—|31.83/False|
|docs|ours x264|36.90/53.00|33.90/43.70|—/—|0.17/0.99|0.11/0.50|1.11|0.00/—|—|exact/True|
|docs|ours OpenH264|36.00/54.00|34.00/55.80|—/—|0.20/0.99|0.10/0.50|1.09|0.00/—|—|exact/True|
|docs|ours VP8|34.00/53.40|32.80/48.70|—/—|0.20/0.95|0.24/0.55|1.19|0.00/—|—|exact/True|
|canvas|Selkies 2.0.0|31.20/47.90|29.00/35.90|61.92/59.62|0.38/0.43|0.28/0.90|0.71|0.16/0.63|30.58–30.67|41.67/False|
|canvas|Selkies legacy|31.00/31.60|28.40/29.90|59.80/55.50|0.70/0.70|0.26/0.85|0.96|0.60/1.05|39.04–39.04|39.04/False|
|canvas|ours x264|36.70/39.10|37.30/55.30|59.65/59.25|0.17/1.34|0.38/1.03|1.72|0.00/1.36|24.92–26.15|exact/True|
|canvas|ours OpenH264|35.90/53.00|49.30/53.70|59.58/59.58|0.20/1.53|0.45/1.11|1.98|0.00/1.34|26.12–27.87|exact/True|
|canvas|ours VP8|35.40/52.10|50.30/57.90|59.37/59.37|0.20/1.75|0.53/1.16|2.29|0.00/2.46|26.17–26.17|exact/True|
|video|Selkies 2.0.0|31.20/48.10|28.70/33.80|61.73/30.02|0.36/0.40|0.13/0.87|0.53|0.20/0.57|30.58–41.70|41.70/False|
|video|Selkies legacy|31.00/47.50|30.20/46.20|59.15/29.97|0.66/0.69|0.13/0.86|0.82|0.57/0.92|27.70–39.05|39.05/False|
|video|ours x264|36.40/55.20|37.00/55.00|29.95/29.95|0.16/0.99|0.24/0.73|1.23|0.00/0.74|27.81–41.64|exact/True|
|video|ours OpenH264|35.80/55.70|50.00/53.70|29.95/29.95|0.19/1.13|0.29/0.79|1.42|0.00/0.70|27.81–27.81|exact/True|
|video|ours VP8|36.60/55.30|36.70/54.80|29.96/29.96|0.18/1.23|0.33/0.81|1.57|0.00/1.29|27.84–27.86|exact/True|
|scroll30|Selkies 2.0.0|31.00/34.40|30.10/47.00|61.83/61.63|0.43/0.54|0.17/0.98|0.71|3.13/6.89|14.49–14.57|30.85/False|
|scroll30|Selkies legacy|47.70/49.00|46.10/47.00|52.46/51.66|0.75/0.84|0.18/0.84|1.02|5.83/7.81|14.15–14.28|24.65/False|
|scroll30|ours x264|48.40/54.70|48.00/59.60|29.80/29.80|0.17/0.85|0.28/0.73|1.13|0.00/3.70|15.32–16.68|exact/True|
|scroll30|ours OpenH264|37.10/55.30|50.60/72.10|4.18/4.18|0.18/0.71|0.23/0.29|0.95|0.00/6.33|13.84–14.37|exact/True|
|scroll30|ours VP8|37.10/54.50|48.50/56.20|5.08/5.08|0.16/0.95|0.23/0.34|1.17|0.00/5.87|13.72–14.56|exact/True|
|wheel30|Selkies 2.0.0|31.10/35.50|28.60/46.30|61.37/49.27|0.43/0.59|0.25/1.00|0.84|3.09/7.58|14.92–15.01|30.93/False|
|wheel30|Selkies legacy|47.40/48.80|45.80/47.40|59.48/53.09|0.75/0.97|0.26/0.93|1.22|5.82/7.01|14.62–15.21|23.52/False|
|wheel30|ours x264|46.20/53.30|48.60/59.50|21.99/21.99|0.18/0.94|0.47/0.67|1.41|0.00/2.73|16.65–16.95|exact/True|
|wheel30|ours OpenH264|36.40/54.20|49.50/56.90|2.99/2.99|0.21/0.85|0.47/0.34|1.32|0.00/4.61|13.75–13.82|exact/True|
|wheel30|ours VP8|35.40/54.70|49.80/52.20|3.69/3.69|0.21/1.05|0.46/0.39|1.51|0.00/4.22|13.50–14.06|exact/True|

## MP-08/MP-10 — final extra cases and profile

|Case|Click P50/P95 ms|Type P50/P95 ms|FPS|Pipeline idle/active cores|Source+pipeline active cores|Video idle Mbps|Settle|
|---|---|---|---|---|---|---|---|
|x264 scroll60|36.9/53.4|50.0/73.6|54.13|0.20/1.56|1.90|0|exact|
|x264 docs,65s idle|36.4/53.9|32.4/52.1|—|0.178/—|idle0.183|0|exact|

MP-08/MP-10:27 cases =10 fresh upstream baselines +16 three-encoder motion/docs cases +65s idle. All pass component checks. No performance or real-app acceptance follows. First typing samples reach641.4ms after long idle and748.2ms in scroll60; reporting P95 alone would hide these maxima.

MP-10 motion CPU components in the bounded scroll60 profile: encoder0.680, Node0.219, kernel0.256, capture0.142, Xvfb0.121, source Chromium0.340, controller Chromium0.138 cores. Viewer1.273 and harness0.558 are separate. Endpoint process intersection excludes very short-lived processes; the whole-window CPU table remains authoritative.

|MP-08/MP-10 stage|Motion P50/P95 ms|Typing P50/P95 ms|
|---|---|---|
|Native capture|4.86/6.65|3.98/4.63|
|Encode|12.38/15.35|34.80/35.20 (2 spans)|
|Pacing|12.00/15.64|1.16/1.23|
|Input injection|—|3.86/5.16|
|Host input|—|6.58/8.57|
|Input round trip|—|12.10/15.30|
|Decode|1.90/3.50|4.00/7.10|
|Input→draw|—|36.50/63.00|
|Draw→rAF|—|11.70/15.70|
|Input→rAF|—|50.00/73.60|

MP-08: this traces correlated key/request, injection, actual damage/capture, codec and presentation stages. It does not establish a per-key causal paint graph. Bounded input urgency substantially reduces sampled pacing waits but does not meet type P95≤35ms.

## MP-08/MP-10 — encoder setting measurements

MP-10 isolated240-frame tune; codec CPU normalized to60fps, media bandwidth excludes envelope expansion.

|Setting|Threads/stripe|Encode P95 ms|Codec cores@60|Media Mbps|
|---|---|---|---|---|
|base ultrafast/zerolatency|1|11.37|0.621|3.576|
|no scenecut|1|10.19|0.692|3.576|
|sliced threads2|2|9.05|0.867|3.610|
|sliced threads4|4|9.98|0.800|3.613|
|CRF28|1|10.50|0.679|3.573|
|QP23|1|8.01|0.439|20.387|
|QP30|1|7.73|0.437|13.494|
|QP35|1|8.08|0.437|10.071|
|QP40|1|7.64|0.425|7.085|
|shared pool1 (separate run)|1|10.38|0.592|3.576|
|shared pool2|1|6.00|0.632|3.576|
|shared pool4|1|4.23|0.689|3.576|

MP-10: QP settings exceed8Mbps once serialized envelopes are included. Pool2/4 lowers isolated wall latency but increases codec/live pipeline CPU. Default remains one worker, configurable1/2/4. No setting measured here establishes a win on all owner targets.

## MP-11 — P1 source masking and remaining RED

|Exact c0 embedded runtime check|Result|Evidence|
|---|---|---|
|Native dynamic marker,1920×1080 DPR1|RED:2/180 presented frames|protected-final18-native|
|Native diagnostic repeat|PASS:0/177 violations|protected-final18-native-diagnostic|
|Second native diagnostic repeat|PASS:0/188 violations|protected-final18-native-repeat|
|Full-video fallback,1280×800 DPR1|PASS:0/70 violations|protected-final18-fallback|
|PNG/tiles,1280×800 DPR1|PASS:0/59 violations|protected-final18-png|

MP-11 first failing seam is the native per-presented-frame lossy mask check at sequences51/52: first sampled RGBA18/25/67/255. Every source capture, exact settle and reference-loss recovery check passed. No image of those first failures was captured; diagnostic commit17f1d91bd now saves bounded failing-frame PNGs and maximum/above-threshold counts. Both repeats pass with the same RGB64 threshold. The intermittent RED remains unresolved; no claim of privacy acceptance or verified codec-noise root cause is made.

MP-11 source leak RED is bound to9ba runtime +borrowed final harness: protected source pixels are red before the fix. Unit regressions prove native mask-before-codec, removal of unmasked shared/readRegion routes, private-only fingerprint stability, CDP protection-before-hash/codec, exact crops/repairs, source-event retirement and late-attestation refusal. Physical marker click, motion→settle, navigation and independent reference recovery run on component relay/client paths. The reported private region is reproduced; this is not the unchanged640×400 public-canvas Cloud fixture or real app entry.

## MP-10 — GPU comparison

|Fixture|Selkies2 hardware|Legacy VAAPI|Chariox VAAPI|
|---|---|---|---|
|docs|UNMEASURED|UNMEASURED|UNMEASURED|
|canvas|UNMEASURED|UNMEASURED|UNMEASURED|
|video|UNMEASURED|UNMEASURED|UNMEASURED|
|scroll30|UNMEASURED|UNMEASURED|UNMEASURED|
|wheel30|UNMEASURED|UNMEASURED|UNMEASURED|

## MP-08/MP-10/MP-11 — final verification and handoff

MP-11:288 Node tests pass with zero skips; configured codec6/6 pass (an earlier unconfigured run skipped the libyuv test); compiled public-client11 pass on unchanged client source. Final libtest:59 host,3 protocol,1 transient-display-event and2 native packet checks pass. Release kernel/relay/CLI build and help checks pass;57 assets match in final libtest and real kernel. These are focused/component checks, not real-app acceptance. A mistyped Python executable exit127 remains in its log; corrected configured command exits0.

MP-11 resource floor:883 periodic samples, MemAvailable minimum21.37GiB, disk minimum110.08GiB; every case also samples resources. Own run roots/processes/namespaces are cleaned, old ELF copies removed, needed final binaries retained. Shared Cargo targets, other lane state, reviewer service, provider credentials, private owner keys and Docker resources are untouched.

MP-08/MP-10/MP-11 FINAL; acceptance NOT DONE. Coordinator must run the unchanged protected Cloud gate plus the18:10 real-site/hosted-relay/DPR1/2 matrix with real built app/kernel/client and provider where touched. Supplied Cloudf6cfd006 lacksaa9c44f8/e2e-stack/real flags; lane relay-contact prohibition blocks hosted runs. Hardware requires an owner laptop endpoint. Native intermittent RED and CPU/typing misses remain explicit lane results, not owner questions that silently close gates.
