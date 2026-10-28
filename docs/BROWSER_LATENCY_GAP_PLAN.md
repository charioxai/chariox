# PLAN — close the latency gap to Selkies in the Chariox browser (for an Opus 5.5 high implementer)

latfable r1, 2026-10-28. MP-08 / MP-09 / MP-10 / MP-11. Companion: `AUDIT.md` (causes with evidence), `history/*.md` (file:line digests), `OPUS_BRIEF.md` (first-round brief). Baseline: OSS main `362ef830c9`, Cloud main `8bbdf60b3c`.

> **Terms (Miguel, 2026-10-28).** kernel browser = the Chariox browser running in the kernel (DOM mirror, video regions, tabs); a Room is a session whose agents share ONE kernel browser; slice desktop projection = the video projection of a slice's desktop (its own Chromium, Computer-use panel, today also shown by the Room's Browser view). The slice desktop projection is a separate item, not part of browser use and latency: see the appendix "Parked: slice desktop projection".

Rules the plan respects (Miguel): strict per-cell parity for keystroke-to-paint and omnibox first paint; other gates P95 <= RTT + 60 with RTT printed; software target <= 1 core at 1920x1080 moving, click/type -> present P95 < 50 ms on the local path, exact settled pixels; beat Selkies in both software (builder) and GPU (owner laptop) modes on latency, scroll fps, CPU, bandwidth, moving quality and settled exactness; DOM mirror primary, video regions inside it, whole-page protected video fallback stays, browser chrome as a protected video region; pinned kernel Chromium; OpenH264 stays; never downgrade to stills. Multi-domain redesign and its display redesign are on hold: each item below is marked INDEPENDENT or BLOCKED-BY-REDESIGN. Nothing below invests in the Selkies-in-slice bridge (`slice-selkies-stream.py`, `room_display.rs`), which current clients no longer use.

Ordering: items that close the most cells per unit of work first. P0 is the rig and must come first because every later item is proven with it. P9 (mirror credit cadence) runs right after P0: the quiet-host run on main (`host/MEASURED.md`) shows 6-10 credit round trips (600-700 ms) between the kernel's first mirror packet and the paint of a typed-URL page on the product path, and P9 needs no protocol change. P1-P3 are one connected piece of work on the kernel-browser navigation path and should be done in that order by one implementer. P4 and P10 (slice desktop projection) are PARKED: a separate item, not browser use or latency.

## P0 — One paired rig for every cell (prerequisite) — INDEPENDENT

- Goal: one harness that measures Chariox and Selkies in the same run with the same stop criterion, geometry and browser build, and records per-stage marks for both arms, for the kernel-browser path and the slice desktop projection path.
- Owning code: harness only (no product behaviour). Start from `/root/.chariox/dev/navlat/r3/harness` (navigation-pair.mjs, scroll-pair.mjs, pixel-probe.py, aggregate-display-pipeline.py) and `/root/.chariox/dev/vidlat/r45` (keyboard glyph detection, band.mjs, video-sink.mjs) and slicemem r5 `measure.mjs` (desktop path). Runbook: `history/rig_runbook.md`.
- Changes: (a) Selkies arm at the cell geometry (`--manual-width/height` = W*DPR x H*DPR, not the fixed 1280x800) so ROIs are physically equal; (b) both arms on CfT 155.0.8059.79, verified by `--version` in evidence; (c) stop criterion = physical headline region (<2.5 % px differ > 24 levels) for navigation and glyph detection for keystrokes, identical for both arms; (d) ABAB interleaving per block with the order alternated across passes; (e) add the omnibox-suggestion cell (typed character -> suggestion list pixels in the browser-chrome region) which does not exist today; (f) add the slice desktop projection cells (key, click, wheel, fps) on the same host with the same probe; (g) per-stage marks (diagnostic cells only, `CHARIOX_BROWSER_DISPLAY_TIMING=1 BUX_TIMING=1 BUX_RECEIVE_SIZE=1`): input | Chrome commit | Chrome first paint | display retired/admitted | frame encoded (bytes, QP, kind) | wire | received | decoded | presented | pixel error vs target per presented frame | exact repair sent/arrived/presented | mirror snapshot/sheets/frames/apply/paint | stop; Selkies arm: input | each frame sent (bytes) | presented | pixel error | stop; (h) byte counters per class (kinds 1-5, RPC, relay TLS) and per-process CPU per stage; (i) io probe (fdatasync p99) logged next to every run; (j) a table generator that prints the exit table (section "Exit table") with RTT next to every P95.
- Target: the rig runs the whole exit table on a fresh ccx33 in one command and on Miguel's Mac for the GPU arm; acceptance cells with diagnostics OFF.
- Command: `R/host/ssh "OSS_HEAD=... CLOUD_HEAD=... /opt/browserperf/run.sh <label> kernel-final '<WxH>@<DPR>' '14-open-band,browserperf-navigation-pair' BP_NAV_TRIALS=10 BP_TYPE_DELAY=40 BP_SOURCE_PROBE=1 BP_ARM_ORDER=native-first"` then `report.py`; extend with `BP_OMNIBOX=1`, `BP_DESKTOP=1`.
- Expected gain: none in product; it removes the measurement artefacts that made 43 rounds incomparable (AUDIT section 4.5).
- Risk: harness time (1-2 rounds). Dependencies: a quiet host (coordinator), the navlat SSH identity and peer allowance.

## P1 — One native page display per Tab for the life of the Tab (no display per document) — INDEPENDENT

- Goal: a navigation produces frames on the existing stream; nothing is retired, admitted or re-attested per document on the critical path.
- Owning code: D/kernel-browser-page-display.mjs (`subscribeVisiblePage`: admission, protection, document fencing), D/kernel-browser-navigation-display.mjs (pre-start/adopt; delete when no longer needed), D/kernel-browser-native.mjs:116-123 (per-display attestation), K/runtime/state/kernel_browser_runtime.rs:124-150 (`document_id` required by display_subscribe and input admission), K/transport/relay_client/display_pump.rs (reset/IDR on document change), W/kernel-browser/page-navigation.ts:85-200 and page-runtime.ts (client keeps the stream; `document_id` becomes a frame-header field the client reads, not a subscription key), BD/presenter.mjs (document change = keyframe boundary, not a re-subscribe).
- Selkies mechanism copied: one stream of one surface for the session; navigation is more frames (P/x11/mod.rs run_capture; S selkies.py `_VideoRelay`).
- Design notes: protection (masks, Vault targets, protected regions) and document fencing are per frame already in the worker/Node path; keep the fence by tagging frames with `document_id` and dropping stale input at the kernel, not by tearing the stream down. The IDR on document change stays (geometry may change) but it is produced by the same encoder session (`ForceIntraFrame`, DN/openh264.c:35-38) with the P2 budget. The viewer's window layer (`windowLayerDemand`) is then unnecessary for ordinary navigations: see P3.
- Target and cells: STAGE-BROWSER-LATENCY-LINK 4/4 and -URL 4/4 within Selkies P95 together with P2; BROWSER-UX-05 new tab <= 1 s 9/9; DISPLAY-P1-GATE no strict-stop misses; DISPLAY-P1 CPU/nav within 1.2x Selkies together with P3. Stage target: "display ready" <= 20 ms after Chrome commit (today stream init 178 / ready 241 ms after click).
- Rig/command: P0 navigation cells (1440/1920 x DPR1/2, N=10, both orders) with the diagnostic pass showing the removed stages.
- Expected gain: 250-300 ms per navigation (init->ready 65 + admission 70-100 + IDR reset 170-190 ms, navlat r1 PUSH_READY.md:44, r3 PIPELINE_FINDINGS.md:39-61); plus the 310-830 ms attestation wait on the new-tab path (browserux LANE_STATUS.md.r36bak:29,49).
- Risk: protocol shape changes (frame header `document_id`, display_subscribe without `document_id`): follow the AGENTS.md protocol rule (bump LOCAL_DAEMON_PROTOCOL_VERSION, snapshot tests, drill; coordinator allocates the number); the mirror handover (`readyForHandover`) must keep working on top of the persistent stream; stale-input fencing must stay exact (MP-11 anchors: a click meant for the old document must not land on the new one).
- Dependencies: none. Does not depend on the multi-domain redesign (the kernel browser stays; the redesign adds the browser on slices too, which reuses this).

## P2 — First frame converges to the settled criterion without waiting for the lossless repair — INDEPENDENT

- Goal: after a document change or a large change, the native stream reaches the 2.5 %/24-level criterion within 2-3 frames; the lossless WebP repair finishes the exact pixels afterwards and is off the stop's critical path.
- Owning code: DN/openh264.c:33-66 (QP model: `CX_QP_MIN`, `fill` = 1.5 x frame bits; intra when >= 80 % MBs changed; inter QP -1 per frame), DN/codec.c:254-311 (x264 path, same VBV), DN/worker.rs:400-432 (readback only when dirty: add refinement frames after a key/large change while the picture is above tolerance), D/kernel-browser-motion.mjs:28-38 (latest-only, <=2 buffered), DN/direct.rs:17-21 and D/kernel-browser-display.mjs:214 (QUIET 120 / quietNativeMs 150 before the exact repair), DN/raster.rs:157-184 (patch budget 24-192 KB).
- Selkies mechanism copied: streaming mode keeps encoding after a change so the picture converges within a few frames (P/pipeline.rs ~676), plus the paint-over burst at CRF 18 after the screen is static (P/pipeline.rs:39,75; settings `use_paint_over_quality`, `video_paintover_crf=18`, `paint_over_trigger_frames=15`). Ours: (a) an IDR for a document change gets an intra budget of N frames of bitrate (start with 6 frames = 100 KB at 8 Mbit/s; Selkies CRF mode has no cap at all) instead of 1.5; (b) after any frame encoded above a quality floor (QP > 30 or predicted error > 1 %), the worker schedules refinement P-frames at falling QP at the frame interval even without new damage until the encoder's own reconstruction error is under tolerance (the worker already keeps `previous` planes to compute activity; reuse them for the error estimate); (c) the lossless repair starts after the refinement, not after a 120-150 ms quiet wait, and it no longer gates the "settled" state the viewer shows.
- Target and cells: with P1, STAGE-BROWSER-LATENCY-URL/LINK 8/8 within Selkies; first presented frame within tolerance at P95 in the diagnostic cell; bytes of the first second after a navigation <= Selkies x1.2; the DPR2 click/type tails (195-273 / 128-265 ms) back under 187/116.
- Rig/command: P0 navigation cells + the per-frame pixel-error trace; P0 keyboard/click cells at DPR2.
- Expected gain: 150-200 ms per navigation (first->exact send 91 + exact->arrival 68 + part of arrival->paint 36 ms, navlat r3) and the "12.9 % off" first frame (navlat r1); removes the strict-stop misses in browserperf r14. Bench evidence (AUDIT 7a, `measure/ENCODER_BENCH.md`): today's IDR is QP 51 at 25/46 KB and the `fill` clamp forbids anything larger; a 6-frame IDR budget (79 KB) converges on the headline ROI at frame 5 (84 ms at 60 fps) and a 15-frame budget (150 KB) on the IDR itself; the current -1 QP per frame descent needs 16-18 frames (~300 ms) even if refinement frames were emitted. Start with budget 4-6 frames, descent -3..-4 QP per frame, measure bytes on the 8 Mbit/s wire.
- Risk: more bytes in the first 100 ms (bounded by the intra budget; the ACK gate and CreditBudget still back off on congestion); CPU for refinement frames on a static page (bounded: stop when under tolerance, at most ~10 frames). Keep OpenH264 (owner).
- Dependencies: P0 for the pixel-error trace; independent of P1 but measured together.

## P3 — Encode each pixel once; send the bytes Selkies sends — INDEPENDENT

- Goal: one stream carries a navigation; auxiliary and window streams exist only while their pixels are visible; the relay forwards binary frames as binary.
- Owning code: W/kernel-browser/band-navigation.ts:90 (`windowLayerDemand`) and W/kernel-browser/display.ts:10 (`browser_window` target): with P1 the window layer is only needed when the page display is genuinely absent (no Tab yet, protected fallback); D/kernel-browser-display-regions.mjs and the `browser_auxiliary` (kind 3) stream (omnibox/popup region) encoded only while the popup is drawn (navlat's hidden-omnibox fix `c3999b67` on branch navlat/oss is the right shape and is not in main: port it); R/server/connection/writer.rs:165-177 `receiver_message` (binary > 16 KiB -> JSON envelope): send large binary through the existing chunks-v2 binary transfer (R/frame_transport.rs, client W/kernel/browser-relay-frame-transport.ts:7-62) and remove the legacy envelope path for clients that negotiated binary; K/transport/relay_client.rs:156-190 lane split stays.
- Selkies mechanism copied: one stream per surface; no second encode; no base64.
- Target and cells: bytes per typed-URL navigation <= Selkies x1.2 (today 702-1251 KB vs 266-338 KB); DISPLAY-P1 CPU-s per navigation within 1.2x Selkies (today 1.5-1.8x: the window stream is a second 2560x1600 encode); MIRROR-P2 idle bytes <= 2 KB/s (probe from P9).
- Rig/command: P0 byte counters per class and CPU per process; relay binary check = `receive-wire-metadata.json` shows no Text frames > 16 KiB for display routes.
- Expected gain: -0.3..-0.6 MB and -0.5..-1.0 CPU-s per navigation (navlat r3 byte classes; browserperf r14 CPU_NAV.json), -148 KB relay envelope per navigation (PIPELINE_FINDINGS.md:28); on a real 8 Mbit/s kernel uplink 1 MB is a full second, so this also moves the hosted tails.
- Risk: relay change touches every client (protocol rule; the relay must keep the JSON envelope for clients that did not negotiate binary); window layer removal must not reintroduce white frames on cold open (COLD-FRAME rule C: check with the stageboot cold-open harness).
- Dependencies: P1 for the window-layer part; the relay part is independent.

## P4 — PARKED (slice desktop projection; see the appendix)

## P5 — DPR2: motion at DPR1 geometry, settled pixels exact at DPR2 — INDEPENDENT (APPROVED by Miguel 2026-10-28: Selkies does the same)

- Goal: a DPR2 viewer's moving frames cost what a DPR1 viewer's cost; settled frames are exact at device scale 2.
- Owning code: DN/codec.c:232-247 (`motion_geometry`: reduced 2560x1600 -> 1280x800 only under contention today; make it the default for unprotected whole-frame motion at dsf 2 with the exact repair restoring the native raster), D/kernel-browser-process.mjs:34-36 (`--force-device-scale-factor=2` stays, so settled pixels are exact), BD/presenter.mjs:76 and BD/video-sink.mjs:150-153 (present a 1280x800 motion frame scaled into the 2560x1600 backing until the repair lands), DN/openh264.c:102-104 (2 threads above 2.5 MP becomes moot for motion).
- Selkies mechanism copied: Retina viewers get a DPR1 source and the client scales (S selkies-ws-core.js:407,2245-2270); we keep DPR2 for settled frames, which Selkies never has.
- Target and cells: DPR2 click/type P95 <= Selkies (today 195-273 vs 187; 128-265 vs 116); STAGE-BROWSER-LATENCY-SCROLL 1920 DPR2 >= 50 fps (today 47-47.5); DPR2 video region fps >= Selkies; DISPLAY-P1-GATE DPR2/click-p95 and DPR2/news-scroll PASS; cores at DPR2 moving <= 1.0.
- Rig/command: P0 click/type/scroll/video cells at DPR2; moving-quality check = per-frame PSNR of the presented motion frames vs the DPR2 source, compared with Selkies' DPR1-upscaled frames in the same run (must be >= Selkies).
- Expected gain: -50..-80 ms on DPR2 interactions (displayopus FINAL-r2.md:58), 4x fewer pixels through capture/convert/encode during motion.
- Risk: the Chromium repaint at DSF2 itself (part of the +50-80 ms) is not removed by this item; if the measurement shows the repaint dominates, the alternative is rendering at DSF1 during motion, which is a bigger change to the owned-window geometry. Moving quality at DPR2 equals Selkies' (both DPR1 sources).
- Dependencies: P2 (the repair must be fast enough that the DPR1 motion frame is replaced within 150 ms of motion stop).

## P6 — OpenH264 threaded like x264; activity scan only over changed rows — INDEPENDENT

- Goal: the encoder costs what Selkies' x264 costs at the same picture.
- Owning code: DN/openh264.c:102-125 (iMultipleThreadIdc = slices = clamp(cores-1,1,4) for all sizes, SM_FIXEDSLCNUM already in use; try `LOW_COMPLEXITY` for motion frames, `HIGH_COMPLEXITY` only for the final refinement frame), DN/openh264.c:48 `activity()` (restrict to the damage rows the worker already knows: DN/capture.c:548-589 row diff), DN/codec.c:268-269 (thread clamp shared with x264), DN/worker.rs:620-623 (codec thread queue depth 4 stays).
- Selkies mechanism copied: x264 sliced threads = cores-1 clamped [1,4] (P/encoders/software.rs:841-852).
- Target and cells: kernel encode <= Selkies encoder CPU at 1920x1080 and 2560x1600 moving (today 0.41 vs 0.26 cores; 13.6 vs 7.5 ms/frame at 2560x1440); DISPLAY-P2 cores <= Selkies; DISPLAY-P1-GATE keyboard on animated content p50 within +8 ms (today +12.3); total <= 1 core at 1920x1080 moving in software.
- Rig/command: P0 CPU per process on the animated-balls and Commons-video workloads; per-frame encode ms in the diagnostic cell; the vidlat `bench` comparison (13.6 vs 7.5) rerun with threads.
- Expected gain: bench on b3 (AUDIT 7a): 4 slices cut the motion encode wall time from 8.4 to 2.5 ms at 1920x1080 and from 14.8 to 6.0 ms at 2560x1600 at -19 % / +6 % CPU; the +12 ms p50 under animation came from encode time on the same thread as capture. Bytes per slice frame were within +9 %.
- Risk: the product's own comment in openh264.c says sliced threads spun (3x CPU for the same wall) on a quiet 16-vCPU host at 1264x708 and 1920x1080; the b3 bench contradicts it. FIRST STEP of P6: rerun `measure/bench` on the quiet host (1, 2, 4 threads, both geometries, wall and CPU) and only then change the thread rule; if the spin reappears, pin the encoder threads to the worker's own cores or use 2 slices only at >= 1920x1080. More slices = more bytes (~3-9 %); Chromium decodes up to 4 slices without glitches (pixelflux comment). Keep OpenH264 (owner).
- Dependencies: none.

## P7 — Per-frame path without Node and without files — INDEPENDENT (larger, after P1-P3)

- Goal: a frame goes worker -> kernel -> relay with one copy and no file; Node keeps the control plane (admission, protection policy, mirror) and leaves the per-frame loop.
- Owning code: DN/sessions.rs:132-150 and D/kernel-browser-native-pipe.mjs (packet files), DN/direct.rs (direct lease: make it the only path for all displays, regions and stripes, not only whole frames during motion; IN_FLIGHT and QUIET become ACK-gate terms), K/runtime/browser_controller_process/display_frames.rs (UDS frames), D/kernel-browser-motion.mjs (latest-only logic moves to the worker), D/kernel-browser-native.mjs:206 `maskNativeRaster` (masking in the worker, one copy), display_pump.rs:375-378 (lease end on congestion -> keep the lease, let the gate throttle).
- Selkies mechanism copied: capture thread -> encode thread -> socket (no third process on the frame path).
- Target and cells: Node CPU ~0 during motion (today 0.04-0.10 cores); kernel per-frame CPU -30 % (today capture 2.7 + encode 2.3 + packetize 1.7 + codec queue 1.3 ms on b3); P95 tails of click/type within 10 ms of p50 in the local cell (click/type -> present P95 < 50 ms on the local path per the owner's software target).
- Rig/command: P0 local-path cell (viewer on the same host, no shaper) plus the hosted cells; per-process CPU.
- Expected gain: the "encoded -> frame out" 2.3/22.6 ms hop and the Node share of cores (vidlat SELKIES_AUDIT.md:30-41, r37 split); removes the lease churn vidlat r29-r32 saw as fps regressions.
- Risk: the biggest structural change in the plan; the attestation/protection oracle must still see every frame (it can run in the worker as a mask policy pushed from Node). Do it after P1-P3 have landed and been measured, so its gain is attributable.
- Dependencies: P1 (persistent display) simplifies the lease model.

## P8 — Viewer sink: one presentation path for page, regions, stripes and direct frames — INDEPENDENT

- Goal: every decoded frame is presented by the compositor from a worker-fed track; no canvas draw overwrites another within one compositor frame.
- Owning code: BD/video-sink.mjs:5-50 (MediaStreamTrackGenerator -> `<video>` for the whole page; regions/direct/stripes still canvas 2D), BD/video-sink.mjs:95-160 (paced commit), BD/stripe-presenter.mjs:14-44, W/kernel/browser-mirror2-region-video.ts:6 (region writer newest-1), BD/presentation-queue.mjs (decode-ahead 3).
- Selkies mechanism copied: VideoTrackGenerator in the video worker -> `<video>.srcObject`, with the same fallback ladder (S selkies-ws-core.js:1084-1100); requestVideoFrameCallback for presented-time only.
- Target and cells: Commons video fps DPR1 >= Selkies (today 44.9-45.9 vs 53.2-54.2); DISPLAY-P2 fps >= Selkies; echo -> pixel stage unimodal (vidlat r35/r37 bimodality gone); 1920 DPR2 scroll >= 50 fps with P5.
- Rig/command: P0 video and scroll cells; client-display-timing `client_present` histogram.
- Expected gain: +5..+8 fps on video (vidlat r44 depth-3 sink gave +5 alone), -10..-20 ms on the upper mode of echo -> pixel.
- Risk: region compositing order inside the mirror (video regions are DOM-positioned boxes; a `<video>` per region replaces a canvas per region); Safari/WebKit support for VideoTrackGenerator in a worker (fallback ladder stays).
- Dependencies: none.

## P9 — Mirror interaction cadence and the MIRROR-P2 probe — INDEPENDENT

- Goal: a mirror interaction whose answer is N packets costs one round trip, not N; the kernel's delta cost and idle bytes are measured.
- Owning code: D/kernel-browser-mirror2.mjs:305-345 (queued credits: answer every outstanding credit with the deltas already pending instead of one change per credit; let a credit carry more than one packet when more are ready), W/kernel/browser-mirror2.ts:1352 (idle throttle 1 credit after 3 s: refill before the input is sent, which :1255 does, and confirm the kernel has them queued when the first packet is produced), D/kernel-browser-mirror2-observer.mjs (delta cost timing mark per packet), harness probe for idle bytes.
- Selkies mechanism copied: none (Selkies has no mirror); the model is our own display_pump: credits are a window, not a turn-taking protocol.
- Target and cells: V-UX-02 add-click P95 <= RTT + 60 at all four densities (today 247-300 vs 146) and key 1920 DPR2 <= gate; MIRROR-P2 kernel delta cost P95 <= 20 ms and idle bytes <= 2 KB/s measured; TR-VIEW type P95 under budget on main.
- Rig/command: appsfix r14 split method (kernel emit time vs viewer receive time per packet) inside P0; `mirror_next` request stages in `client-request-stages.json`.
- Expected gain: -1 RTT per extra packet (+62 ms steps observed in RESULTS_R14.md:6-16), so -60..-130 ms on multi-packet interactions; on the product navigation path measured on main (host/MEASURED.md B): the typed-URL mirror arrives as 6-10 credit-gated packets one RTT apart after the first (503-542 -> 886-935 ms), so delivering the pending resource batches on the outstanding credits at once is worth 300-600 ms per typed URL and 150-250 ms per link click at RTT 88.
- Risk: more packets in flight means ordering/apply discipline in the renderer (already in-sequence apply); verify first with the trace that the batching is kernel-side (hypothesis) before changing it.
- Dependencies: none.

## P10 — PARKED (Rooms use the kernel browser instead of the slice desktop projection; see the appendix)

## P11 — Web-client cold raster and exactness rows — INDEPENDENT (small)

- P11a AT-17: Cloud `apps/web/src/styles/terminal-first-screen.ts` and the first-screen render path; residual: commit -> own paint 63 vs 51 ms and own paint -> first presentation 72.5 vs 19.8 ms in the failing cell (trig PUSH_READY.r16.md:37). Trace the failing cell with the Chrome tracing categories for raster/compositor and fix the rasterised area or the font load; target rule B 8/8 at 40 ms.
- P11b BROWSER-UX-COLD-OPEN: re-run 5 x 3 on the final head with P3's window-layer change; target every rep (criterion: blank <= control + 1 sample, first frame dark).
- P11c STAGE-REGION-CAPTURE: lay the composed scene out at device scale (Cloud `apps/web/src/screenshot/dom-raster.ts:234,258`); target mean RGB error <= 8 at 400 DPR2. Not a transport item; listed because the owner's exact-pixels rule covers screenshots.

## P12 — Native worker stability over 2 h (CU-ROOM-SOAK-2H) — INDEPENDENT

- Goal: no worker exit, no stall, no unexplained socket close over 2 h with two owner viewers.
- Owning code: DN/worker.rs (exit paths, slot exhaustion: "no free slot: 1000 ms" poll), D/kernel-desktop-source.mjs (UNOBSERVED 200 ms / HEARTBEAT 100 ms restarts), K/runtime/state/room_shared_display/worker.rs (60 s lease renewals), home log (empty at the exit: add the reason).
- Target: CU-ROOM-SOAK-2H PASS; the worker's exit reason always logged.
- Rig: culinux r60 soak harness on the quiet host after P4.
- Dependencies: P4 changes the loop; soak after it.

## What depends on what

- Start now: P0 (rig), then P9 (measured 300-600 ms per typed URL on main, no protocol change), then P1 -> P2 -> P3 (one implementer, kernel-browser navigation); P6, P8, P11 any time (small).
- After P1-P3 land: P5 (needs P2's fast repair), P7 (attributable only after P1-P3). P12 (worker stability) stays, measured on the kernel-browser native display.
- BLOCKED-BY-REDESIGN: none in the active list (P10, parked, was the only one). Items that would be useless once the new Browser/Apps setup arrives: none of P1-P9 (the kernel browser, its native display, the relay and the viewer stay; the redesign adds the browser on slices and a higher-quality transmission for which P1/P2 are the prerequisites). Explicitly not planned: any work on the Selkies-in-slice bridge (`slice-selkies-stream.py`, `room_display.rs`, `display_tunnel.rs`), already unused by current clients.

## Pre-requisites from Miguel / the coordinator

1. A quiet rented host of the ccx33 class for every measuring round (one per implementer round; 8 h), the navlat SSH identity and peer allowance; the pr-stack with the implementer's OSS head and `--base-cloud` (or a Cloud branch when the client changes).
2. A Mac session (owner laptop) for the GPU arm of the exit table (VideoToolbox/VAAPI vs Selkies): one paired run per completed phase, driven by the same harness; Miguel's eyes on the moving-quality A/B of P5 (DPR1 motion source at DPR2) and on the final side-by-side.
3. Protocol numbers for P1 (frame-header `document_id`, display_subscribe shape) and P3 (relay binary passthrough): allocated by the coordinator 2026-10-28 (local 720-739, relay peer 280-289).
4. P5 is approved (Miguel 2026-10-28). The Selkies parity arm is confirmed: `--framerate=60 --rate-control-mode=cbr --video-bitrate=8000` at the cell geometry (the slice runs Selkies at 30 fps CRF 25: not a parity arm).

## Exit criterion (gap closed)

All rows of AUDIT.md section 6 at their thresholds in one paired run per host class (software on the builder-class host; GPU on the owner laptop), N >= 10, diagnostics OFF, CfT 155.0.8059.79 on both arms, RTT next to every P95, with the io probe showing fsync p99 < 20 ms during the run. Cells: STAGE-BROWSER-LATENCY-URL x4, -LINK x4, omnibox-suggestion x4, keystroke static x4 and animated x4, click x4, SCROLL x4 (+ >= 50 fps), video/canvas fps DPR1+2, cores <= 1.0 and <= Selkies, bytes/nav <= Selkies x1.2 and idle <= 2 KB/s, settled exactness (raster gate 0.5 %/1.5 %, drift <= 4 px, repair <= 150 ms), BROWSER-UX-05, V-UX-02, AT-17, BROWSER-UX-COLD-OPEN. The slice desktop projection cells (WT-22, ND-06, CU-*) are not part of this criterion (parked). DISPLAY-P1, DISPLAY-P1-GATE and DISPLAY-P2 are PASS when the above hold.

### Exit table template (one per host class; fill both)

| Cell | Geometry | RTT ms | Chariox P95 | Selkies P95 | Chariox p50 | Selkies p50 | fps C / S | cores C / S | KB C / S | N | Verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|
| URL typed | 1440x900 @1 | | | | | | - | | | 10 | |
| URL typed | 1440x900 @2 | | | | | | - | | | 10 | |
| URL typed | 1920x1080 @1 | | | | | | - | | | 10 | |
| URL typed | 1920x1080 @2 | | | | | | - | | | 10 | |
| Link click | (4 rows) | | | | | | - | | | 10 | |
| Omnibox suggestion | (4 rows) | | | | | | - | | | 20 | |
| Keystroke static | (4 rows) | | | | | | - | | | 40 | |
| Keystroke animated | (4 rows) | | | | | | - | | | 40 | |
| Click | (4 rows) | | | | | | - | | | 20 | |
| Scroll (wheel -> paint; motion fps) | (4 rows) | | | | | | | | | 10 | |
| Video region fps | @1, @2 | | - | - | - | - | | | | 3x60 s | |
| Canvas (balls) fps | @1, @2 | | - | - | - | - | | | | 3x60 s | |
| Cores 1920x1080 moving (software) | @1 | | - | - | - | - | | | | 3x60 s | <= 1.0 |
| Bytes per URL nav / idle | (4 rows) | | - | - | - | - | - | - | | 10 | |
| Settled exactness (raster gate, drift, repair ms) | (4 rows) | | | | | | - | - | - | 20 | |


## Appendix — Parked: slice desktop projection (separate item, not browser use or latency)

Miguel 2026-10-28: not part of browser use and latency. Two parked items, kept with their full text for the separate item:

### (a) Speed up the projection path (old P4)

## P4 — slice desktop projection path: push frames from the worker, inject input without a process per event — INDEPENDENT (Computer); the Room-browser mirror dimension is BLOCKED-BY-REDESIGN

- Goal: the slice desktop projection stream behaves like the kernel-browser native stream: frames pushed with the ACK gate from where the worker is, input injected inline.
- Owning code: K/runtime/state/room_shared_display.rs:482-532 and room_shared_display/worker.rs (replace per-credit `Capture` peer RPCs with a worker-side pump: the worker kernel runs `display_pump`-style capture/seal and streams sealed frames to the home kernel as events; the home kernel relays and keeps the ACK gate), K/transport/relay_client/display_pump.rs:373-410,512-531 (the direct-frame path with `lease` must also serve Room captures), K/runtime/state/room_browser_controller.rs:67-95 (local slice: skip the relay), K/runtime/state/human_environment_action_runtime_state.rs:24,319 (10 ms admission polling -> event), K/runtime/state/tool_dispatch/slice.rs:842-870,1044-1300 (one long-lived input helper per slice with a pipe, instead of a `slice-screen.sh` process per event; XTest for humans too, keeping the xdotool path for what XTest cannot do), D/slice-screen.sh:601-699, D/slice-keyboard.py:157,200-257 (40 ms per key pace -> only between repeated keys), cloud packages/terminal-domain/src/room/screen-input-queue.ts:4-14 (do not wait a full round trip per event; pipeline input like the kernel-browser `display_input`), K/config.rs:69 `DEFAULT_KERNEL_WEBSOCKET_WRITE_DELAY_MS=33` (exempt display and input lanes from the local write delay, as culinux r56 found).
- Selkies mechanism copied: capture thread -> socket with ack backpressure (we already have display_pump); XTEST input inline without sync (S input_handler.py:4525,5132).
- Target and cells: WT-22 / ND-06 input P95 <= RTT + 60 (138 ms at RTT 78) and scroll >= 30 fps on the 6 sites at DPR1 and DPR2 with 0 timeouts; CU-ROOM-TAKEOVER-RELEASE P95 <= RTT + 60 at both DPRs; CU-PARITY click/type/fps/cores <= Selkies in the same paired run (Selkies arm at `--framerate=60 cbr 8000` on the same slice display, same geometry); CU-ROOM-SOAK-2H 2 h without a worker restart (see P12); the stall at scroll start (0.5-3 s) root-caused and gone.
- Rig/command: P0 desktop cells (`BP_DESKTOP=1`) on the quiet host with the slice on the same host; `slicemem/r5/harness/measure.mjs` as the starting point.
- Expected gain: frame cadence from 1/(2 x relay RTT + wait) to the ACK-gated push (fps 0-17 -> >= 30; culinux r58 already saw ~20 fps capped by the serial RPC chain at p50 48.6 ms); key latency from (round trip + fork/exec + 40 ms pace) x serial to one way (382-432 -> ~RTT + 60); click 271-1682 -> under the gate.
- Risk: peer protocol change (RELAY_PEER_PROTOCOL_VERSION) for the pushed-frame event and the input pipe; the worker kernel must keep sealing frames for the viewer key (it already does); the slice Chromium is Debian chromium, not CfT (parity rule): record versions, and consider shipping CfT into the slice image.
- Dependencies: none for the video/input part. The mirror-coverage dimension of WT-22 (Room's Browser view as a DOM mirror) depends on the pending redesign that puts the kernel browser on slices too (P10): do not build a mirror bridge into the headed slice before that design lands.
- Redesign note: if the "higher-quality transmission replacing Selkies" replaces the desktop video transport itself, the worker-side pump is still the mechanism that transport needs (frames must be pushed from where they are captured); the input pipe is independent of transport. Neither is wasted.


### (b) Rooms use the kernel browser (shared tabs) instead of the slice desktop projection (old P10)

## P10 — Rooms using the kernel browser (shared tabs) (DOM mirror in Rooms) — BLOCKED-BY-REDESIGN

- Goal: the Room's Browser view (slice desktop projection) shows the user-domain kernel browser (mirror primary, video regions, protected video fallback) instead of a slice desktop projection; mirror coverage per site in Rooms.
- Owning code: everything behind `shared_display`/`RoomSharedDisplay` for the browser case; the slice would host the kernel browser (pinned CfT) with its mirror ops routed through the Room peer protocol.
- Why blocked: this is the "browser in the kernel as well as on slices" item of the pending multi-domain redesign. Building a mirror bridge into the current headed slice (Debian chromium, desktop-only controller) would be thrown away.
- What is useful now: P4's worker-side pump and input pipe, which that design needs too; and recording WT-22's mirror dimension as BLOCKED-BY-REDESIGN with this reference.

