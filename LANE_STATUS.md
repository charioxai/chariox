# MP-08/MP-10/MP-11 — phase19 RUNNING

MP-11: base b77569f2e. Reproduced mask threshold RED (3 presentations) with actual embedded phase18 runtime and new failure-retaining harness. Diagnostic adapter proves a masked native keyframe reconstructs RGB76 at the private codec boundary; replay of the same public raster deterministically reproduces it (source RGB0). This is a lossy reconstruction failure, not evidence of unmasked input reaching the codec. New guard validates black source input, decodes protected outputs before packetization, drops uncertain batches, resets references and sends an exact protected fallback. Revised 200-cycle motion/settle/reference-loss preview is running; earlier stall/queue failures remain retained diagnostics. No acceptance claimed.

MP-08/MP-10: performance follows the security regression. Local protocol447/relay90 shapes unchanged; private Symbol mask metadata never serializes into client frames. Rust compile uses the shared slot/jobs4; memory/disk floors sampled. Evidence: /root/.codex/evidence/browser-resume-20260930/display-perf/phase19/.

## MP-08/MP-10/MP-11 Coordinator asks

Real live acceptance is coordinator-owned per phase19 prompt. Supply/run paired Cloud md/display-web aa9c44f8 app entry+real flags, exact kernel/relay/CLI artifacts, hosted wss8Mbps/RTT>=60ms, real sites, DPR1/2 desktop, real provider where relevant and multi-hour stability. This lane remains prohibited from contacting the hosted relay or Apps machine. No owner laptop/GPU endpoint supplied. Prepare an exact build/kit and commands here; component runs do not close these gates.

## MP-11 Review inbox

Display-perf inbox absent at start; historical mappings below remain bound to their original commits. Check the absolute lane inbox after each commit batch.

# MP-08/MP-10/MP-11 — phase18 FINAL; acceptance NOT DONE

2026-10-06 18:21 UTC: runtime c0e8a1fdfc352dcb6548d12222e811386db004bd; diagnostic harness17f1d91bd. Base c42d74363; branch md/display-perf; protocol447/relay90 unchanged. Local commits only, [skip ci], required coauthor.

MP-08/MP-10: native stripe payloads bypass Node through kernel-owned private bounded packets; Node still handles control and source checks. Input-triggered XDamage capture/32KiB repayable pacing debt; pluggable x264/OpenH264/VP8; worker1 default after1/2/4 measurements. Stable empty trusted DOM snapshots use event-bound admission. Targets remain RED: x264 typeP9543.7–59.6ms on five fixtures, scroll60P9573.6ms; canvas59.65fps/1.34pipeline cores. Complete27-case comparison, exact settles, zero idle bytes and65s renewal pass component checks only.

MP-11 P1 source leak reproduced RED before masking; native/CDP/screenshots/refinement mask before codec/hash/shared routes. Exact final native run is INTERMITTENT_RED:2/180 presented frames exceed dark RGB64; source/settle/recovery checks pass. Two unchanged diagnostic repeats pass177/188frames; root cause unresolved, no privacy acceptance claimed. Diagnostic harness now retains failed-frame PNGs. Fallback70/70 and PNG/tiles59/59 presentations pass.

## MP-08/MP-10/MP-11 Coordinator asks / acceptance blockers

Supplied Cloudf6cfd006 lacksaa9c44f8/e2e-stack/real entry flags. Rerun unchanged /masked gate on these commits. Coordinator18:10 requires real web app/desktop DPR1/2, hosted relay, shaped8Mbps/real RTT and real-site/video/canvas list. Lane is forbidden to contact hosted relay, so coordinator execution/access is required. No owner laptop GPU endpoint supplied. Fixtures, libtest IPC, release builds and help do not satisfy live real-app/TUI/provider acceptance.

## MP-11 Review inbox mapping

15:28 disjoint overflow ->bafbc9352 fail-first ordinary/native independent recovery.16:04/16:08 P1 ->d89b733e2+a10ecd6b6+c0e8a1fdf, source-mask regressions and component checks; final intermittent native RED retained.18:10 real-site gate ->explicit coordinator blocker. Historical display inbox latest18:10 checked; display-perf inbox absent. No security review approval inferred; non-security exact-blob review is outside narrowed MP-11.

MP-11 checks:288Node,6configured codec,11unchanged compiled-client,59Rust host+3protocol+1transient event+2native packet pass. Release CLI/kernel/relay built;57asset identities match.32final receipts(27comparison+5privacy), all32state roots absent;31componentPASS,1RED.883resource samples: MemAvailable≥21.37GiB,disk≥110.08GiB. Removed8old own ELF copies; retained final18-bin. No push/CI/deploy/Cloud, foreign resources/services, credentials or private owner keys touched.

MP-08/MP-10/MP-11 report: docs/MULTIDOMAIN_DISPLAY_PHASE18_RESULTS.md. Evidence: /root/.codex/evidence/browser-resume-20260930/display-perf/phase18/FINAL18.json. Full commands/hashes/screenshots/logs and cleanup retained externally. FINAL; stop.
