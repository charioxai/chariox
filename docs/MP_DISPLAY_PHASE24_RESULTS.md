# MP-08/MP-10/MP-11 — display phase24 FINAL: performance RED, real live BLOCKED

MP-08/MP-10/MP-11: local commits only; no push, PR, GitHub CI, merge, deployment or hosted-service contact. The native implementation and component regressions are delivered. Phase24 is NOT accepted or ready for staging: every final workload misses at least one performance target, and mandatory real live conditions are blocked.

## MP-08/MP-10 source and behavior

Required base `c4a39c647ac192451a99f886d4c7268ea48b09d5`; branch `md/display-perf`. The performance campaign kernel test executable, production native worker and embedded assets bind to clean `e9f6c5c5a651c2cbc1d8f4e5ba8890fae6cd746d`. The masking drill executable is rebuilt at `5b33e9d48` for a test-only watchdog change; its production assets/runtime match e9. Final drill/collector source `c7ea4565b` adds separate stdout/stderr parsing and receipt-safe diagnostics. Each artifact keeps its own source/hash. Later report-only commits do not change build identities. Local daemon 447 / relay 90: public serialized shapes unchanged; private worker commands only.

MP-08/MP-10: base wheel60 reproduced 7.24 fps / 1.02 pipeline cores. Capture itself ran near 56 Hz, but queued wheel mutations held the shared controller mutex, delaying display admission to roughly 200 ms P95 and expiring delta/reference chains. The fail-first held-input assertion reproduces this starvation. Input mutations now have their own bounded serial lane; waiting on capture/input RPCs releases the shared controller mutex. Actor, cancellation, focus, document and protection barriers remain. Final wheel60 reaches 48.29 fps at DPR1 and 37.31 fps at DPR2; the original starvation is fixed, parity is still RED. The1080p60Hz harness sent493 wheel inputs and dropped129 generated ticks at its four-in-flight bound: residual CDP/input throughput is explicitly unresolved, not hidden by the FPS improvement.

MP-08/MP-10: a kernel-owned native child performs XShm/XComposite window readback, XDamage wakeups, exact byte comparisons, immutable leases, masks, libyuv conversion, x264/VAAPI encoding, independent decoder guards and private packet files. Native lossless preparation runs on one bounded background thread, leaving the capture/control loop responsive. Node retains CDP document/visibility/protection control and client sequencing. No Python process performs native-path frame processing. Completed exact bases anchor sparse changed tiles across dropped captures; mask changes or uncertain references still force conservative repair. Timing logs batch bounded records rather than opening files per stage.

## MP-08/MP-10 final component measurements

Real Chromium/decoder/canvas, built kernel test executable with the real kernel/router implementation, production native worker and scoped encrypted local relay; synthetic pages and a component presenter, not the actual Cloud app/TUI or real providers. 8 Mbit/s ceiling, four credits, software x264, 10 s motion windows; DPR1 CSS1920×1080 and DPR2 CSS1280×800 (2560×1600 raster). Embedded asset hashes match with no source override. Host resource/load samples are retained. Exact presentation is canvas/rAF time later independently proved exact, not photon timing. Static docs CPU is the typing window.

| Condition | FPS | Pipeline cores | Click P95 ms | Type P95 ms | Exact after motion ms | Phase24 |
|---|---:|---:|---:|---:|---:|---|
| 1080p canvas | 59.67 | 0.69 | 28.40 | 40.60 | 211.70 | RED |
| 1080p docs | — | 0.47 | 40.20 | 38.80 | — | RED |
| 1080p scroll30 | 29.86 | 0.67 | 43.00 | 53.50 | 810.98 | RED |
| 1080p scroll60 | 54.62 | 1.34 | 40.00 | 54.60 | 958.29 | RED |
| 1080p video | 30.01 | 0.43 | 40.60 | 41.60 | 317.20 | RED |
| 1080p wheel30 | 30.13 | 1.08 | 36.40 | 51.70 | 753.25 | RED |
| 1080p wheel60 | 48.29 | 1.59 | 42.60 | 53.40 | 1015.51 | RED |
| DPR2 canvas | 58.44 | 1.24 | 49.50 | 55.80 | 473.91 | RED |
| DPR2 docs | — | 0.62 | 50.30 | 200.40 | — | RED |
| DPR2 scroll30 | 29.42 | 1.12 | 40.90 | 54.90 | 1507.93 | RED |
| DPR2 scroll60 | 38.85 | 1.38 | 41.20 | 54.90 | 1447.33 | RED |
| DPR2 video | 29.91 | 0.70 | 49.90 | 67.50 | 581.12 | RED |
| DPR2 wheel30 | 28.90 | 1.26 | 41.70 | 56.00 | 1363.53 | RED |
| DPR2 wheel60 | 37.31 | 1.55 | 54.90 | 318.50 | 1446.23 | RED |

MP-08/MP-10 targets: ≤0.6 pipeline cores at 1080p, ≤1.2 at DPR 2, click/type P95≤40 ms and exact presentation<300 ms. None of the seven workloads meets all targets at either density. Canvas 1080p exact repair passes at212 ms; scroll/wheel repair remains753–1016 ms atDPR1 and1364–1508 ms at DPR 2. DPR2 wheel typing includes556/318 ms first repairs; those samples are retained. There is no claim of whole-page exactness based on a lossy patch.

## MP-08/MP-10 per-stage wheel60 table

| Native stage | Mean ms/frame | P95 ms | Estimated cores |
|---|---:|---:|---:|
| capture_copy | 0.39 | 0.52 | 0.02 |
| compare | 0.01 | 0.02 | 0.00 |
| convert | 0.78 | 0.92 | 0.04 |
| damage_compare | 0.57 | 0.74 | 0.03 |
| encode | 8.46 | 10.73 | 0.41 |
| mask_guard | 0.00 | 0.00 | 0.00 |
| output_guard | 1.65 | 2.15 | 0.08 |
| reference_copy | 0.38 | 0.53 | 0.02 |
| xshm_fence_read | 0.08 | 0.11 | 0.00 |

MP-08/MP-10: native counters measure thread CPU inside stages over the presentation window, not whole-host CPU. Their sum is about 0.60 cores, dominated by encoding 0.41 and decoded output guard 0.08. Nearest process samples give native worker 0.66, kernel 0.29, Node control 0.26, Chromium 0.27 and Xvfb 0.11 cores. Those approximate samples differ slightly from the exact 1.587 motion accounting window. Node/Python pixel work has moved native; control/routing/source costs still matter.

| Control/repair stage | Mean ms | P95 ms |
|---|---:|---:|
| cdp_input | 15.96 | 29.84 |
| codec_packetize | 0.09 | 0.12 |
| input_post_reconcile | 2.44 | 3.41 |
| native_document_fence | 0.49 | 0.69 |
| native_exact_prepare | 125.63 | 108.85 |
| native_region_fence | 0.01 | 0.01 |
| native_visibility_fence | 0.39 | 0.59 |

MP-08/MP-10: these are nested wall durations, not additive CPU. Residual engineering work is explicit: reduce native codec/control/routing CPU, avoid conservative large repairs during scroll/focus transitions without weakening mask/reference guards, and reduce input-to-capture plus canvas/rAF scheduling latency. Native background compression removes blocking but does not remove large repair encode/egress cost.

## MP-08/MP-10 fresh same-host Selkies comparison

| Workload | Presented FPS | Pixel-changing FPS | Pipeline cores | Click/type P95 ms |
|---|---:|---:|---:|---:|
| canvas | 61.98 | 59.88 | 0.42 | 35.00/30.30 |
| docs | — | — | 0.45 | 34.50/29.90 |
| scroll30 | 61.81 | 61.51 | 0.53 | 35.30/39.60 |
| scroll60 | 61.94 | 61.34 | 0.57 | 37.50/35.20 |
| video | 61.78 | 29.99 | 0.40 | 48.10/33.10 |
| wheel30 | 61.97 | 61.47 | 0.63 | 38.00/34.10 |
| wheel60 | 61.91 | 61.61 | 0.61 | 31.90/34.70 |

MP-08/MP-10: sequential fresh upstream Selkies2 baseline, same fixture/host/DPR1/8 Mbit/s. All seven baseline component cases pass. Selkies uses its own local transport without Chariox Vault admission or relay routing; its cost is a comparison, not an identical security path. This does not authorize Selkies/noVNC product investment.

## MP-11 masking, crash and review mapping

MP-11: final protected DPR2 three 70-cycle runs = 210 cycles, 12,444 protected presentations, 0 violations. Every cycle toggles motion, verifies settled pixels and forces independent recovery after a lost reference. Third run also exercises abrupt supervisor death. See exact per-run receipts/screenshots; component gates do not close real live masking acceptance.

MP-11 crash: 49152000 captured bytes; remaining packet roots=0, remaining transient files=0, durable profiles retained=True. Assertions run before harness parent removal.

MP-11 review mapping: coordinator 00:55 xxhash finding → `edc8d16c8` explicit dlopen roots/dependency bundle + isolated import check, `7c9d54bff` executable resolution and `113701e88` native ELF loader staging. Missing-xxhash fail-first import exits 1; restored imports exit0; final kit isolated imports pass. Coordinator 01:00 VAAPI/floor findings → `edc8d16c8` native VAAPI selection/proportional laptop memory floor, `2fca22cde` visible hardware dimensions and runtime fallback, `113701e88` staged executable. Native requests attempt accessible render nodes; initialization/encoding failures produce explicit fallback receipts. Actual Intel/iHD success remains blocked on coordinator hardware. Builder floor remains authoritative 12 GiB; default laptop reserve is 15% RAM clamped 0.5–2 GiB. No credentials or device permissions are changed.

MP-11 fail-first masking run: the old test fixture shut down the browser host at its 240-second deadline after 57 completed recoveries, with 3,548 masked presentations and zero violations. The original RED receipt is retained. Test-only commit 5b33e9d48 extends the bounded watchdog to 600 seconds; STOP cleanup remains immediate. The final unchanged three 70-cycle DPR2 gate uses the rebuilt executable, not a reduced cycle count.

MP-10/MP-11 diagnostic fail-first: the second fresh 70-cycle run completed its screenshots but lost its receipt when stdout test-status text entered a split stderr timing record. Commits 0908156b8/c7ea4565b separate the pipes, preserve raw stream captures, mark malformed diagnostics RED without bypassing receipt/cleanup, and save non-secret root bindings at startup. The real RED output and unit red/green evidence remain; that run does not count toward 210. Its uniquely bound disposable roots were inventoried and removed after confirming no owned processes remained. The second leg was rerun in full.

MP-11 current security review is requested for native capture ownership, masks/immutable leases, independent output guard, exact/base revision fences, private packet hydration, background exact jobs and descriptor cleanup. No non-security exact-blob review requirement is asserted under narrowed MP-11. Existing capture-pool/Xauthority crash cleanup is preserved.

## MP-08/MP-10/MP-11 checks and identities

Frozen Rust 38/38: native contracts 5, host/actor 19, current-input actor cache 7, packet hydration 7. Focused Node 115 unique checks pass (65 latest pipeline/control/timing, 48 lease/refinement and 2 kernel-log diagnostics). Fail-first evidence includes held-input starvation, completed exact-base reuse and timing batching; prior wrong-dependency compile failure is not counted as the red assertion. Clean full builds used the builder2 compile flock and four jobs. No GitHub CI ran.

Performance kernel test executable SHA256 `d50b4d0bdae8fbbd20ea39629198f8e3c2dca3c4453b1ed3e06cd5d8b6994bd9`; production kernel/native-worker SHA256 `dc8e82811dcb12ae55c319b1aff9e76c1e70f1ea32dc848ae0e5b7bf4b8760d6`. The rebuilt masking test executable SHA256 is `eea994878e1668e2832c5a79dd6e7d4d44359707205690d9dfc0403649890df4`, build source 5b33e9d48. Final receipts record clean source, exact binaries and embedded allowlisted asset SHA matches. Previews explicitly retain separate source overrides/artifacts and are excluded from final metrics.

Unsigned updated public LAN kit: `/root/.codex/evidence/browser-resume-20260930/display/phase24/owner-kit-final/display-lan-kit.tar.gz`, 359641775 bytes, SHA256 `725105b524733a09c374c235a4bbb28cfc142523079c7f48b74a1a660da8592a`. Kit scripts source `c7ea4565b`, test executable build `5b33e9d48`; production worker source `e9f6c5c5a` (test-only source equivalence recorded); runtime/library/asset manifest 1,718 files all match. Bundled Node 22.20.0, Python 3.14.4, kernel executable enumeration and isolated xxhash/capture/stripe imports pass. The final kit component run passes with 575 native packet batches, embedded asset SHA matches, zero diagnostic errors and clean teardown. Root-private EACCES and cross-filesystem staging failures were driver issues; the traversable copied public-kit rerun is retained separately. No runtime state, credentials or keys are packaged. Coordinator should run `LIBVA_DRIVER_NAME=iHD MD_SOFTWARE=0 MD_CASES=local:canvas:8000000,local:scroll60:8000000,local:wheel60:8000000 sudo -E ./run-lan.sh` on the laptop, then repeat DPR 2/protected; require successful actual `vaapi` traces, not requested flags/device presence.

## MP-08/MP-10 real live BLOCKED — exact owner action

The lane no-contact rule forbids hosted relay/Apps/Cloud staging and owner-laptop execution. Coordinator must run the authorized real Cloud app entry/flags and real CLI/TUI against this exact kernel, with scoped hosted wss bootstrap and product-linked official providers where relevant, real desktop DPR1/2, shaped ~8 Mbit/s client uplink, measured ≥60 ms RTT and a multi-hour stability session. Laptop VAAPI needs the owner Intel/iHD machine. Local regression passing is insufficient, and local performance failures remain RED independently of these resource blocks.

| Mandatory public site | DPR1 | DPR2 | Coverage/screenshot/latency/scroll/hosted network |
|---|---|---|---|
| en.wikipedia.org article | BLOCKED | BLOCKED | unmeasured |
| www.wikipedia.org portal | BLOCKED | BLOCKED | unmeasured |
| github.com repository | BLOCKED | BLOCKED | unmeasured |
| Google search results | BLOCKED | BLOCKED | unmeasured |
| major news site | BLOCKED | BLOCKED | unmeasured |
| developer.mozilla.org docs | BLOCKED | BLOCKED | unmeasured |

MP-08/MP-10: no owner exception assumed; real-site full DOM mirroring, screenshots on every site/density, relay click/typeP95≤150 ms, smooth streamed scroll≥30fps, no primary PNG slideshow and multi-hour continuity remain unestablished. Nothing is staged for acceptance.

## MP-08/MP-10/MP-11 evidence and cleanup

Evidence root `/root/.codex/evidence/browser-resume-20260930/display/phase24/`: `final-1080p/`, `final-dpr2/`, `selkies-final/`, `final-mask70-fixed-1/`, `final-mask70-fixed-2-rerun/`, `final-mask70-fixed-3/` (old watchdog/collector RED receipts retained), `final-hardware-fallback/`, `final-kit-accessible-component/` (original root-path EACCES driver receipt retained); per-step screenshots, console/kernel captures, exact commands/exit codes, artifact receipts, native-stage/process tables, resource samples and owned-cleanup receipts. Base wheel evidence remains under phase23/phase24/base-wheel60 and is not relabeled. Large obsolete preview ELF/debug/import-kit artifacts were inventoried/verified and removed; retained final binaries/kit and evidence are coordinator diagnostic handoff assets. Final resource samples minimum MemAvailable 28.26 GiB/free disk 53.64 GiB, above 12/10 GiB floors. Final 53 recorded disposable roots are absent; the separately bound collector-failure roots and temporary public-kit trees are removed. No shared Cargo/BuildKit/Docker/provider/key/reviewer/other-lane cleanup.

