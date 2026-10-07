# MP-08/MP-10/MP-11 display phase25 — FINAL RED, live BLOCKED

MP-08/MP-10 performance targets remain unmet. This lane is not accepted or ready for staging. Required base: `b340ae0f7af6209510e71b321521007903a56050`. Runtime, final builds and laptop kit: `67e5231fba2276478ff7deebd99abe55c9bf315f`. Later handoff changes are documentation only. Local protocol 447 and relay 90 are unchanged; native worker metadata and diagnostics are private. No push, PR, CI, deployment or hosted-machine contact occurred.

MP-08/MP-10 implementation reduces redundant dense capture comparison, distinguishes adjacent from cumulative exact damage, selects full-height encoding for broad unprotected motion, prepares exact tiles without an unnecessary full PNG, and uses a bounded exact RGB dictionary. Retina input patches allow 128 tiles at 2560×1600 physical pixels. Empty credits park until source wake; CPU accounting follows an actual Chromium exec. Protected dense motion keeps independently guarded rows. Safe PNG fallback has a bounded 4 MiB queue and uses credited exact tiles over an admitted compositor base within the unchanged 1 MiB wire bound. Document, policy, serial, codec-revision, independent-key and decoded masking checks remain authoritative. The motion-height hint grants no pixel reuse.

MP-08/MP-10 local conditions: real production kernel services, sandboxed Chromium and a local scoped encrypted relay; shared display-harness viewer, synthetic fixtures and a dev-stub focused-MCP bootstrap. The actual Cloud entry, TUI and official provider runs were not exercised. These are supplementary component regressions. Requested budget is 8 Mbit/s; no WAN shaping. DPR 1 uses 1920×1080 CSS; DPR 2 uses 1280×800 CSS (2560×1600 physical). Each case has 20 click and 20 type probes. CPU is pipeline cores over the 10-second motion interval, or the type-input interval for docs; source Chromium and viewer are separately classified. rAF measures software presentation, not physical photons.

MP-08/MP-10 strict gates: click and type P95 ≤50 ms, pipeline CPU ≤0.6 cores at DPR 1 / ≤1.2 at DPR 2, exact presentation <300 ms after motion. Scroll60/wheel60 require ≥59 measured fps at DPR 1 and ≥50 at DPR 2; the DPR 1 gate is a proxy for the requested 60 Hz target. Other motion requires ≥30 fps. Values below these limits fail.

| MP-08/MP-10 condition | case | CPU cores | fps | click P95 ms | type P95 ms | exact ms |
|---|---|---:|---:|---:|---:|---:|
| 1080p / DPR 1 | canvas | 0.730 | 59.48 | 24.0 | 37.5 | 219.9 |
| 1080p / DPR 1 | docs | 0.293 | — | 25.0 | 37.9 | — |
| 1080p / DPR 1 | scroll30 | 0.635 | 29.85 | 52.5 | 37.7 | 707.1 |
| 1080p / DPR 1 | scroll60 | 1.055 | 58.94 | 36.9 | 39.0 | 795.0 |
| 1080p / DPR 1 | video | 0.476 | 29.93 | 37.8 | 37.6 | 222.6 |
| 1080p / DPR 1 | wheel30 | 0.761 | 30.13 | 39.2 | 53.8 | 710.7 |
| 1080p / DPR 1 | wheel60 | 1.127 | 48.67 | 37.9 | 54.7 | 747.6 |
| Retina / DPR 2 | canvas | 1.257 | 57.48 | 40.7 | 54.9 | 478.8 |
| Retina / DPR 2 | docs | 0.370 | — | 54.8 | 36.4 | — |
| Retina / DPR 2 | scroll30 | 1.054 | 28.07 | 40.7 | 54.1 | 1452.8 |
| Retina / DPR 2 | scroll60 | 1.420 | 38.00 | 54.7 | 54.2 | 1480.7 |
| Retina / DPR 2 | video | 0.750 | 29.81 | 51.2 | 54.9 | 515.6 |
| Retina / DPR 2 | wheel30 | 1.271 | 29.54 | 54.5 | 87.3 | 1217.0 |
| Retina / DPR 2 | wheel60 | 1.634 | 36.98 | 54.4 | 104.2 | 1524.0 |

MP-08/MP-10 both aggregates are `RED_PERFORMANCE` (report command exit 1). Functional checks pass 7/7 at each density; combined click/type gates pass 4/7 at DPR 1 and 0/7 at DPR 2. Retina docs type P95 is 36.4 ms, but click P95 is 54.8 ms. The large docs type outlier was absent in these local probes; this does not establish real-site stability. Dense scroll/wheel still misses CPU/cadence and exact-repair targets. Encoding dominates native CPU; lossless preparation and paced PNG volume dominate settling. Thread/ME/VBV/filter and input-wake experiments failed to meet targets and were not merged. Masking and ownership checks were retained.

MP-08/MP-10 final dense stage profile, mean thread CPU milliseconds per frame. Elapsed fence, packet and pacing timings are separately recorded and must not be summed as CPU cores.

| MP-08/MP-10 condition | case | capture compare | capture copy | convert | encode | decoded guard | reference copy |
|---|---|---:|---:|---:|---:|---:|---:|
| DPR 1 | scroll60 | 0.551 | 0.395 | 0.729 | 5.830 | 1.180 | 0.480 |
| DPR 1 | wheel60 | 0.564 | 0.385 | 0.698 | 6.176 | 1.219 | 0.481 |
| DPR 2 | scroll60 | 1.305 | 1.004 | 1.662 | 14.400 | 2.377 | 1.174 |
| DPR 2 | wheel60 | 1.381 | 1.035 | 1.740 | 14.520 | 2.395 | 1.201 |

MP-08/MP-10 fresh same-host upstream Selkies2 comparison uses unchanged common 1080p fixtures, the same input probes and corrected CPU accounting, requested 8 Mbit/s, sequential cases. Its actual fixture/harness source is `89e4bf3ed1525aea9ca7a28aa7de6f65f2ba961c`; it is not relabeled as the final source. This baseline excludes Chariox kernel/relay/privacy costs and does not establish acceptance. Wheel30 observed 8.85 Mbit/s despite the requested budget. Shared-builder runs are not isolated-machine or real-network acceptance.

| MP-08/MP-10 Selkies2 case | CPU cores | fps | click P95 ms | type P95 ms |
|---|---:|---:|---:|---:|
| canvas | 0.402 | 61.94 | 47.9 | 33.5 |
| docs | 0.391 | — | 31.8 | 30.4 |
| scroll30 | 0.530 | 61.81 | 48.3 | 33.8 |
| scroll60 | 0.547 | 61.94 | 48.0 | 30.7 |
| video | 0.393 | 61.80 | 47.9 | 33.5 |
| wheel30 | 0.607 | 61.91 | 36.4 | 31.2 |
| wheel60 | 0.589 | 61.91 | 48.3 | 35.4 |

MP-08/MP-10 red evidence is source-bound: frozen-base and preliminary display runs, Retina input outliers, fail-first tile/adjacent-exact/idle-credit/CPU-exec/vertical-span/report/hardware-error checks, and protected-scroll queue/wire failures are retained separately. Source `25f61a71d` failed full protected scroll at codec rejection/fallback queue exhaustion; `89e4bf3ed` then exposed the wire PNG limit. `9d5f730bc` bounds fallback to credited exact tiles over an admitted base. Final full-screen protected DPR 2 scroll passes five additional cycles with zero violations. These do not replace the unchanged canvas masking gate.

MP-11 final focused Rust checks: 45/45 at the exact final source (native 7, packets 7, cancellation 2, actors 7+19, signals 3). Targeted Node checks: 256 at `9d5f730bcbcb5eb5ee27bf69b05f4f8ba618ef32`, including the 78-check protected-fallback motion/display group; report checks 2 and runtime-pin check 1 pass at that source. Source `67e5231fb` changes only an ignored drill's grant lifetime. Preliminary dependency/probe failures and the incorrect runtime-test filename invocation are retained and excluded from passing counts. No full Rust/Cloud suite or CI acceptance is claimed.

MP-11 unchanged three-70 DPR 2 gate passes 210 complete cycles, 10,548 protected presentations and zero violations. The third leg's abrupt supervisor crash reclaims 49,152,000 raster bytes, packet roots and Xauthority before parent cleanup; the browser profile remains durable. Source `9d5f730bc` long legs previously failed navigation/stale-input checks because the fixture grant expired at 300 seconds while its watchdog allowed 600. The real relay correctly enforced expiry. Test-only commit `67e5231fb` uses a bounded 900-second fixture grant; production issuance/expiry/admission is unchanged. All three final legs were rerun fully. Failed attempts remain RED and contribute no cycles to the final count.

MP-11 current security notes cover protected observation, bounded geometry/header/raster leases, exact-base and codec admission, private diagnostics and owned cleanup. Coordinator semantic review is still required for changed security anchors. Narrowed MP-11 does not require an exact-blob audit of unrelated non-security code.

MP-11 inbox mapping: 00:55 xxhash → phase24 commits `edc8d16c8`/`7c9d54bff`/`113701e88` (explicit dependencies, isolated imports and native loader). 01:00 floor/hardware/pool → `edc8d16c8`/`2fca22cde`/`113701e88` (proportionate laptop reserve, visible dimensions and crash cleanup). 04:55 diagnostics/host driver stack → `75dc87332`/`25f61a71d` (bounded private mode-0600 driver/init/encode errors, host-first loader/library preflight and documented receipts). The actual renderD128 probe captures `/usr/lib/x86_64-linux-gnu/dri/virtio_gpu_drv_video.so init failed` and FFmpeg I/O error (-5), then reports x264 fallback. Successful Intel UHD620/iHD packets remain BLOCKED; loader compatibility alone is insufficient. The 04:55 hardware-success request is partially addressed, not closed.

MP-08/MP-10 real live acceptance remains BLOCKED for every listed site and DPR: Wikipedia article and portal, GitHub repository, Google results, BBC news and MDN. DOM coverage, screenshot success, hosted input latency, smoothness and multi-hour stability are unmeasured. Exact coordinator action: provide/run the authorized actual Cloud entry and flags, real CLI/TUI and exact kernel, a scoped hosted wss path with approximately 8 Mbit/s client uplink and measured RTT ≥60 ms, official product-linked providers/accounts where relevant, and a real desktop browser at DPR 1 and 2 for a multi-hour session. This lane is prohibited from contacting the hosted relay/Apps machines. Separately rerun the updated kit on stock Arch/Omarchy Intel UHD620/iHD and require receipts with `motion_backend_vaapi` and per-case diagnostics. These blockers do not excuse the independently RED software results.

MP-08/MP-10 updated laptop kit: `/root/.codex/evidence/browser-resume-20260930/display/phase25/owner-kit-phase25-final/display-lan-kit.tar.gz`. SHA256 `cedd4737257d939f053eccae71325e32656a0ed214dd431560ee1692255f5428`; 359,850,915 bytes. All 1,720 public manifest hashes, exact source/build identities, isolated dependency imports and the actual extracted-kit component pass. The kit is a diagnostic/component artifact; it does not run the real Cloud/TUI/provider acceptance workflow.

MP-11 cleanup/resource evidence: all 177 recorded lane-created temporary roots, including baseline roots, are absent after owned-process settlement. Superseded public kits/build copies and completed codec/raster probes were removed by manifest or exact filename after retaining hashes/receipts. The final production kernel and verified kit remain; the unused debug-heavy test copy was removed. Resource samples stay above 12 GiB MemAvailable and 10 GiB disk; owned monitors are stopped. Credentials, owner keys, other lanes, shared reviewer state, Docker resources and shared caches were not touched.

MP-08/MP-10/MP-11 evidence root: `/root/.codex/evidence/browser-resume-20260930/display/phase25`. Main receipts: `final-v4-build-receipt.json`, `final-v4-rust-focused-summary.json`, `final-v4-check-commands.json`, `final-v4-report-commands.json`, both `final-v4-*-report/report.json` aggregates, `final-v4-summary.json`, `final-v4-stage-profile.json`, `selkies-final-v2-summary.json`, `final-v4-mask-summary.json`, `v3-masking-expiry-root-cause.json`, `final-v4-kit-manifest-verification.json`, `final-v4-owned-root-audit.json`, `phase25-final-resource-summary.json`, `phase25-security-anchor-notes.md`, `real-live-acceptance-blockers.json`, and `review-inbox-checks.jsonl`. Per-case directories retain screenshots, console/log captures, asset hashes, settings and cleanup. Each receipt retains its actual source identity.

MP-08/MP-10/MP-11 pre-strip binary SHA256 at the runtime source above: `kernel-tests` hash `ee371257810c57448dd64821dc71d95abb1d45bac12f5ed34034bcd9d36ed1b6`; `chariox-kernel` hash `d6235ea3e8fd864d48e55879284107944a131bfc3b2f86996f1f25676236ab7d`. The kit manifest records its separate strip transformation and resulting hashes.
