# MP-08/MP-10/MP-11 — phase16 IN PROGRESS

2026-10-06: round2 replay committed at 7af025627 on md/display-perf;
assigned base 6dde21a8c10c9ef2b9f7f0a271cace00591f0bd1. Phase15 history
retained as md/display-phase15-retained. Local447/relay90 allocation introduced.
Original bounded rows, immutable pooled raster leases, fast exact-band change
checks, worker PNG settle after300ms quiet and #893 DPR admission/default fix.
230 Node tests, real H.264/VP8 row codec tests and26 Rust host tests pass.
Protocol guards caught old peer expectations and the new stripe hash; updated,
rebuild/green guard run in progress. Product binaries are building.
No MP acceptance claim. Source/component evidence is labelled separately.

## MP-08/MP-10 Coordinator asks

Supply the paired real-app integration ref or shared e2e-stack harness: neither
supplied checkout contains scripts/e2e-stack; supplied Cloud source contains no
attachBrowserDisplay/browser-display integration, and private Cloud refs cannot
be fetched with current Git access. Product binaries plus presenter/libtest
fixtures do not satisfy real-app red/green acceptance. Continue independent
comparison and implementation while this path is blocked.

## MP-11 Review inbox mapping

Absolute agents/display/REVIEW_INBOX.md checked through the10:14 encoder request.
09:33 #893 viewport/DPR: default negotiation RED before fix, GREEN after; explicit
1080p/DPR2 rejects before emulation/subscription mutation. Real-app drill blocked
as above. Earlier protection/reference/process fixes survive round2 replay.
10:14 encoder comparison: x264/OpenH264/libvpx choices implemented in helper;
OpenH264 supplied FFmpeg adapter lacks screen-content usage option and bundled
library is not the requested Cisco runtime-download distribution. Its normal
adapter measurements will be labelled diagnostic, never claimed screen mode.

## MP-10 Owner questions

Distribution remains owner-selected: x264 GPL, OpenH264 BSD, libvpx BSD. No
GPL codec library is linked into kernel. Screen-content OpenH264 needs a native
API adapter and separately proven Cisco runtime-download provenance before
that requested comparison can pass; ordinary adapter diagnostics continue.

Evidence: /root/.codex/evidence/browser-resume-20260930/display/phase16/.
