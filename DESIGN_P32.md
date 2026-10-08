# MP-08 / MP-10 / MP-11 — display phase 32

MP-08/MP-10: **RED, no acceptance or superiority claim.** This round separates source/capture CPU, codec CPU, control/transport CPU and viewer presentation. MP-11 covers the changed capture/protection seams and reviewer regression; it does not close the behavioral/provider parity matrix.

## MP-08 / MP-10 — source identities and method

The requested #900 head `d4230ce4c` was merged into `md/display-perf` by merge commit `9af2f29a2`. The profiler commit is `db48b8c3d5c432e1add69e84b93aacea8b225503`. Its clean production ELF and exact test executable were built through the shared cargo-slot service. The first instruction profile deliberately used the phase31 ELF plus explicit current controller assets; its receipt retains both identities. It is not a merged-ELF acceptance run. The clean base comparison uses the db48 ELF and its embedded assets.

MP-08/MP-10: Diagnostic `perf record -e cpu-clock -F 499 --no-inherit -p <owned PIDs>` samples source, pipeline and viewer separately. No system-wide sampling, stacks, memory or credentials are collected. Native `CLOCK_THREAD_CPUTIME_ID` timers distinguish comparison, copies, conversion, encoding and privacy guards. Controller/kernel/client wall timers cover packet preparation, encryption, relay delivery, decode and presentation. Nested wall spans are **not additive**, and CPU-clock instruction samples do not measure GPU work. Local rAF/canvas presentation is a software proxy, not photon timing. Synthetic wheel/canvas cases are supplementary regression measurements, never real-site acceptance.

Evidence root: `/root/.codex/evidence/browser-resume-20260930/display/phase32/`. `stage-profile.json` records the presentation-bounded motion interval; each profile receipt records PID/start identities. Perf samples include the final settle interval. The upstream legacy GStreamer source is inspected locally, but its run fails with `Namespace Gst not available`; no measured legacy result is substituted.

## MP-08 / MP-10 — where CPU and frames go

| MP-08/MP-10 wheel60, 1080p software | Source cores | Pipeline cores | Source + pipeline | Viewer cores | Changed-content fps |
|---|---:|---:|---:|---:|---:|
| Selkies 2.0.0 pixelflux/x264 diagnostic | 0.382 | 0.582 | 0.964 | 0.668 | 61.29 |
| Chariox diagnostic, explicit source assets | 0.469 | 0.846 | 1.316 | 0.727 | 57.27 |
| Chariox clean db48 base | 0.459 | 0.835 | 1.294 | 0.721 | 58.74 |
| Chariox dense-motion video-only candidate, same db48 ELF + candidate assets | 0.509 | 0.940 | 1.449 | 0.833 | 58.47 |

MP-08/MP-10: The phase31 report's 2.44–2.48 cores is **not reproduced** by this round's clean base. Current measured excess is about 0.33–0.35 cores, not 1–1.5. Host load/frequency and diagnostic differences remain confounders; the phase31-to-phase32 change must not be attributed to this patch. No CPU governor or another lane's processes were modified.

| MP-08/MP-10 Chariox motion stage | Samples | Mean ms | P95 ms | Interpretation |
|---|---:|---:|---:|---|
| Native damage comparison, thread CPU | 597 | 0.210 | 0.385 | Exact byte comparison, not a hash-only omission |
| Native capture copy, thread CPU | 597 | 0.631 | 1.049 | Leased shared raster population |
| BGRA→I420, thread CPU | 568 | 0.707 | 1.163 | Full-resolution motion conversion |
| x264, thread CPU | 568 | 4.136 | 5.659 | Existing persistent ultrafast/zerolatency codec |
| Controller packet serialization | 584 | 0.011 | 0.019 | Small metadata, native payload lives separately |
| Codec packetize wall span | 568 | 0.771 | 0.128 | Long tail up to71.84 ms; includes private reply scheduling |
| Kernel event serialization/encryption | 572 | 0.029 | 0.045 | Excludes later relay/client work |
| Client request encryption | 1331 | 0.201 | 0.400 | Input and pipelined credits |
| Client decode | 573 | 1.432 | 2.300 | Actual WebCodecs path |
| Client present | 573 | 3.228 | 4.400 | Canvas work, software presentation proxy |
| Post-input reconciliation | 617 | 1.129 | 1.715 | Tab/document/focus discovery retained |

MP-08/MP-10: The instruction profile attributes about29.6% of Chariox pipeline samples to x264,17.7% to Node's libnode,11.4% to Xvfb's libc work,8.7% to kernel Tokio code,7.1% to native static code/conversion and6.6% to native libc/copy work. Kernel namespace and libc/serialization work are additional sampled buckets. These percentages include settle; they must not be multiplied into an exact per-stage motion CPU bill. Selkies2 spends roughly60% of its smaller pipeline sample total in x264. Chrome is substantially stripped, so its internal rendering cost cannot be assigned to named stages with confidence.

MP-08/MP-10: The motion interval contains597 native readbacks but568 encode operations and573 decode/present operations. Coalescing, bounded queues, source fencing and scheduling tails lose frames before presentation; decode means alone do not explain the loss. Capture/encode remain asynchronous and credit bounded. The legacy GStreamer implementation directly links ximagesrc, conversion, x264enc and RTP/WebRTC elements; Selkies2 uses pixelflux/WebSocket x264 instead. Both avoid Chariox's kernel-owned input/authority round trips and exact-canvas repair machinery. This is a source-supported architectural explanation, not a measured legacy CPU comparison.

## MP-08 / MP-10 / MP-11 — quality costs and rejected experiments

MP-08/MP-10: Chariox already uses shared-memory capture, a persistent zero-latency x264 encoder, whole-frame coding for dense native motion and quiet-time exact refinement. Dense-motion-only lossless-scroll deferral was tested in a disposable asset copy. It did not improve matched CPU or fps and was **not landed**. The fixture still settled to independently verified exact RGB; that does not make the candidate a performance win.

MP-08/MP-10/MP-11: Disabling x264 full reconstruction in an isolated codec probe had no consistent win over three repetitions: baseline4.434/3.750/3.740 ms/frame versus4.209/4.004/4.012. Packet bytes were identical. Full reconstruction is retained because exact repair certificates use the normative decoded raster. Existing protected rows independently decode and check output blackness; those guards are not removed. Empty native masks cost about0.005 ms controller work and0.0005 ms native guard CPU in this unprotected diagnostic; protected/Retina costs require their own measurements.

MP-08/MP-10/MP-11: No faster post-wheel state shortcut is landed. Its measured1.13 ms wall span is small and includes authoritative popup/navigation/focus reconciliation. Removing it without preserving those semantics would trade correctness for a misleading timing win.

## MP-08 / MP-10 / MP-11 — selected source-capture correction

MP-08/MP-10: The largest observed hosted gain comes from restoring the native path. The old default host was physical DPR2 while a DPR1 client requested1280×800. Its owned Chromium window was2560×1774. Native admission correctly rejected the width and repeatedly fell back to CDP PNG/readback. Dividing bounds by the host DPR reached native readback but still failed exact RGB attestation. Keeping the host at physical DPR1 and sizing its window to **negotiated physical raster +87 pixels of browser chrome** restored exact-attested native capture on the real Wikipedia portal. For DPR2 the compositor view also needs CDP `scale: 2`; `deviceScaleFactor` alone sets page density but leaves a different physical view image. A disposable source-copy real portal run restored native exact admission at DPR2 with both values selected. Shared `displayDeviceMetrics` now applies that setting to mirror and video negotiation in `d8df6a4ad`. Its embedded final ELF was not built before the stop instruction. The CDP [primary protocol definition](https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/json/browser_protocol.json) describes `scale` as the resulting view-image scale.

MP-11: Bounds never authorize pixels by themselves. Unique owned-window/PID checks, exact RGB attestation, document and visibility fences, trusted region masking, masked encoder inputs, independent protected decode guards and kernel delivery admission remain in place. No serialized shape or protocol number changes. The fail-first tests cover both negotiated scales and physical host scale. Commit `d972ed890` corrects physical host/window geometry; `d8df6a4ad` completes view-image scale negotiation. Display, mirror and geometry regressions failed before their changes;174 focused regressions pass after. The first mirror regression attempt had a test-edit syntax error; the corrected fail-first receipt is `mirror-view-scale-fail-first.log`, and the passing suite is `view-scale-regressions-fixed.log`.

MP-08/MP-10/MP-11: The17:06 review P2 is fixed by `fcce80f6f`: rejected native lossy output now retains `native_exact`, prepared repair tiles and the refinement serial. An actual indexed PNG passes from MotionEncoder through DisplayStream without the RGBA-only Node decoder. The regression failed before and passes after;90 focused controller/display tests pass. This supplementary regression does not establish a live protected-site acceptance cell.

## MP-08 / MP-10 — hosted evidence and limits

MP-08/MP-10: Fresh normal device approval enrolled the disposable kernel on `pr893-perf`; no credentials were copied or printed. Real built Cloud `59eaea100f8bef62af1552a969b35eb34e669167` and its Caddy-fronted WSS relay are used. A bootstrap receipt alone did not ensure the Browser panel's selected client was ready: the standalone waiting-room panel could still report protocol443 while bootstrap reported466. Real machine/kernel selection and opening the real session restored the initial measurement flow. A later read-only guard probe after actual machine/kernel selection, **without opening a session**, observed a client at protocol466 and opened a tab with no alert (`selected-client-guard/guard.json`). The earlier transient readiness failure is not reproduced in that cell, so no speculative Cloud change was made. This does not establish readiness under target switching or reconnection.

MP-08/MP-10: The unshaped old-geometry article run connected but presented about0.30 fps. The corrected source-asset preview reached exact-attested native capture and about18.05 presented fps on the shaped article cell (84.5–86.1 ms HTTP request/response proxy,8 Mbit/s qdisc). This is RED against30 fps. Its339 decode callbacks collapsed into181 distinct rAF presentation timestamps. A bounded trusted CDP wheel driver replaced the old serial14Hz Playwright loop; dropped dispatch slots and actual driver rate are recorded.

MP-08/MP-10: Later cells in that first preview did not wait for navigation to finish; screenshots exposed the previous page while controls were still disabled. Their apparent per-site fps/coverage numbers are invalid and must be repeated. DOM-mode labels and viewer screenshots alone do not prove full DOM coverage or product screenshot success. Real WSS8Mbit/80ms source-echo probes on DPR1 measured100 click samples atP95374.8ms and100 type/backspace samples atP95562ms. DPR2 source-value calibration failed before view-image scale correction. An idle-exact credit-wait bypass worsened DPR1 click/type to422.6/861.9ms and was rejected in an external asset copy. The later corrected-scale DPR2 portal preview admitted native pixels but presented only7.4fps unshaped; this is still RED. None of these asset-copy previews is a final embedded-kernel acceptance run.

MP-08/MP-10: The native1080p ≤1-core goal, full public-site mirroring, hostedP95≤150ms, ≥30fps scroll and superiority on every CPU/GPU metric remain unaccepted. A working legacy baseline runtime, Intel/iHD machine, physical Retina/desktop/geographic client path and provider scopes for relevant provider cells are coordinator requests. A public service challenge is an external-resource block, never replaced with a fixture. No CI, push, deployment to another stack or shared-service restart occurs in this lane.

## MP-08 / MP-10 / MP-11 — coordinator handoff, 2026-10-08 18:02 UTC

MP-08/MP-10/MP-11: The17:57 review-inbox instruction stops this implementation lane and transfers the branch to the independent Opus audit lane. No new experiment or final build is started after reading it. Source correction `d8df6a4ad` is committed;174 focused tests pass. The clean built kernel/test pair is from `27d2a829d561fc79cccd4cf1c808862a97397f0f`, which includes the indexed-PNG fix and DPR1/window correction but **does not include the final view-image-scale change**. Native13 tests and host28 tests pass (5 host integration tests ignored); receipts preserve exact ELF hashes.

MP-08/MP-10: No phase32 LAN kit exists. The previous phase31 kit remains at `phase31/final-kit-52c971a08/display-lan-kit.tar.gz`, SHA256 `533ab368b5e8c470c4f497d2974c50e91b96813bf1b2bc07cd4451d47cea540a`; it is **not** a phase32 source artifact. The successor must build the current clean head, repeat matched before/after wheel60/wheel30/canvas runs, complete the per-site DPR1/2 real hosted cells, product screenshots/coverage, final70-cycle masking and multi-hour stability. The phase31 masking result is not relabelled as phase32. GPU/physical desktop and geographic-network cells need coordinator machines.

MP-11: The owned disposable kernel unit and its19 inventoried processes are stopped. Public experimental assets and launchers are retained outside git for the successor; disposable state, generated identities and temporary captures are removed. The exact build artifacts remain for base comparison. Shared fleet/reviewer/other lane services are untouched; `pr893-perf` is left running. Cleanup receipt is `phase32/cleanup-final.json` under the evidence root. No feature acceptance is claimed.
