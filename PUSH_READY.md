# MP-08/MP-10/MP-11 — phase17 local commits ready for coordinator; acceptance RED

Base f5d3b1da96e06a57a6f4b7b590abbbd2bb958885, md/display-perf.
Final measured runtime c53693acf8434970898d15199faa6f666977160a. Protocol447/90
unchanged; no allocation needed. All commits [skip ci] with required coauthor;
no push, PR, GitHub comment, CI, merge, deployment or Cloud staging operation.

MP-08/MP-10: capture-start cadence, concurrent independent decoder rows, optional
libyuv direct planes, preserved normal credit budget, admitted negative-only
native empty credits, sparse bounded exact-region tiles and actual encoding-mode
reference recovery. Backend remains pluggable; default not owner-selected here.
Require actual backend/converter metadata; no ignored setting can claim a result.

MP-10 result: real60Hz x26458.98fps/1.76owned cores vsbase53.07fps/1.99.
TypingP9536.90–55.50ms vsSelkies30.40–35.40. Actual OpenH264 scroll4.18fps
vsx26429.76; VP85.19. Exact settle/zero idle bytes/65s lease renewal pass.
Fresh15-row baseline comparison and15-row actual encoder table in
 docs/MULTIDOMAIN_DISPLAY_PHASE17_PERF.md; full25-row receipts external.
Software CPU/typing/wheel and every-row Selkies win remain RED; GPU unmeasured.

MP-11 review13:41 mapping (all fail-first, no disputes):
1. actual raw-less full-video stale/queued/inflight retirement b5df3da5c;
   overflow3796eb6aa. review-recovery-red/green and review-overflow-red/green.
2. complete independent cover recovers lost outer cursor b5df3da5c;
   producer-to-presenter regression in review-recovery logs.
3. pinned protocol report/LAN metadata b5df3da5c;
   review-protocol-red/green:447accepted,446rejected.
Additional real child env boundary RED/GREEN; metadata corrections12a16132a/
3c5f9226b/1fe1b1e19. Empty-credit136e64ba3 and sparse-region c53693acf have
fail-first coverage and preserve current observation/cursor/signal fences.
MP-11 security/parity scope remains the narrowed owner scope; no broad blob gate.

MP-08/MP-10 acceptance blocker: coordinator must deliver Cloud kernel-browser447
integration and scripts/e2e-stack, real entry/flags and linked provider. Frozen
Cloud f6cfd0066d75844dbab795cdf715789ec5fa37d6 has only Room display. Fixture entry
with optimized b5df libtest ELF + pinned final assets is component evidence.
Real final kernel/relay/CLI launchers now built/preserved with exact57 assets;
help/protocol smoke does not close real-app/TUI/provider red/green acceptance.

MP-08/MP-10/MP-11 validation:27 final comparison cases, final profile and65s idle
pass. Node596/597 (root mode000 fixture base failure; unprivileged baseGREEN);
Rust host27pass; local383/385 +affected TypeScript3pass under pinned official
Node22.20. Focused encoder/planes/metadata/security checks pass. Preserve original
RED logs; no broad-green claim. All source/build identities explicitly separate.

MP-11 cleanup:128 roots/74namespaces absent, exact-root process inventory empty.
Needed public binaries/libraries retained; no private owner keys/provider state,
Docker resources or shared services touched. Evidence and commands/exit/resource/
cleanup metadata under /root/.codex/evidence/browser-resume-20260930/display/phase17/.
FINAL; stop.
