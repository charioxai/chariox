# MP-08/MP-10/MP-11 — phase22 RUNNING

2026-10-06: base f7dff63a24c4319a845206fc95e2ca40d205a5ad, branch md/display-perf. Applicable scope is MP-08 runtime/client parity, MP-10 measurements/live validation, MP-11 observation protection and owned cleanup. Required frozen plans and actual lane inbox read; third #893 finding is first priority. Fail-first abrupt supervisor regression is queued behind builder2 compile flock. Base 1080p/DPR2 profiles completed against phase21 release ELF with unchanged embedded assets; results are supplementary component evidence, not acceptance. Evidence: /root/.codex/evidence/browser-resume-20260930/display/phase22/.

## MP-08/MP-10 Coordinator asks

Real live acceptance requires coordinator-authorized access to the actual Cloud app/flags and hosted relay (lane is expressly forbidden from contacting the relay machine or Cloud staging), a real desktop/DPR2 client and owner LAN GPU host, plus multi-hour real-site measurements. Keep this item BLOCKED until those resources/actions are supplied; continue all local cleanup/profile/fix/regression work. No protocol shape planned; request allocation here if scope requires one.

## MP-11 Review mapping

Actual inbox: /root/.chariox/dev/browser-resume-20260930/agents/display/REVIEW_INBOX.md. Phase21 checked the wrong display-perf path; phase22 corrects the mapping. Raster-reclamation finding unhandled on base; fail-first must establish it before correction.

---

# MP-08/MP-10/MP-11 — phase21 FINAL, RED_PERFORMANCE

2026-10-06: required base fadfcd59bc629ed7aeb2cf46ce01ab3ee9b4b48d; runtime e4440628e6b3d4c54086d8ab6af0cadf70ce3049 (18be64d0b, eca8b2492, e4440628e), branch md/display-perf. Report commit changes documentation only. Local447/relay90 unchanged. No push, PR, GitHub CI, deployment or hosted contact.

MP-11: both phase21 P2 findings fixed fail-first: clamp/omit empty offscreen raster intersections; restore complete full-raster mask metadata after protected crop merging. Actual capture→encoder over-budget regression and real input/decoded guard coverage pass. Canonical210cycles/7,046protected presentations/zero violations; extra DPR2ten cycles zero violations.252/253 focused Node (one skip separately covered),25 client,26 Python and11 focused release Rust checks pass. Broad Docker glob retains one unrelated root-permission provider wrapper failure; no blanket pass claim.

MP-08/MP-10: admitted native protection refresh retains window attestation while dropping stale rasters; actual source input wakes restored; exact idle native conversion/PNG runs off the input loop. Final protected DPR2:42.96fps,2.24cores,click/typeP95 55.6/71.6ms,exact settle1541ms.1080p canvas:59.60fps,1.40cores,55.2/54.9ms,settle1124ms.50ms/one-core targets remain RED; wheel30 is21.96fps. Fresh same-host Selkies canvas:62.00fps/.44cores/48.2/34.1ms. Historical debug vs final release profile differences are disclosed, not source-only speedups.

MP-08/MP-10/MP-11: exact product/component kits built and manifest-verified outside Git; hashes and commands in docs/MULTIDOMAIN_DISPLAY_PHASE21_RESULTS.md and phase21/FINAL21.json. All38 recorded drill scratch roots and final Rust state removed; baseline namespaces/process inventories empty. Final/baseline minimum32.82GiB memory/75.37GiB disk; earlier preview stopped at11.79GiB and cleaned state. Shared/foreign/key/credential/reviewer/Docker paths excluded.

## MP-08/MP-10 Coordinator asks — real live BLOCKED here

Phase21 delegates real live execution to coordinator and forbids lane hosted access. Execute the exact kit contract at /root/.codex/evidence/browser-resume-20260930/display-perf/phase21/COORDINATOR_LIVE.md: real Cloud app/entry/flags, actual hostedwss/~8Mbps/RTT>=60ms, fixed public-site list DPR1/2, real linked providers/TUI and multi-hour session. Performance remains RED independently; do not stage as accepted. No protocol allocation requested.

## MP-11 Review mapping

P2 geometry→18be64d0b, bounds-red.log→configured Python/full-frame and stripe guards. P2 crop metadata→18be64d0b, crop-red.log→actual capture-to-encoder over-budget GREEN. Refresh/wakes/native refinement→eca8b2492/e4440628e, refresh/input/refinement RED→GREEN regressions, exact release masking gate unchanged. Absolute inbox absent at every milestone; final report check recorded externally. Absence is not semantic reviewer approval. Security-critical review remains coordinator work; non-security per-blob review is outside narrowed MP-11. FINAL; lane stopped after documentation commit.
