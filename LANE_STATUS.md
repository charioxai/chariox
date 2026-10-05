# MD-DISPLAY-02/04 — Phase 4 latency work in progress

2026-10-05: execution source 501c53dfc; harness follow-up c89d6a73b.
Protocol 419 and default-off flag unchanged. Instrumented baseline at cb382fb22:
p50/p95 744/778 ms. Full-readback optimization at 8473bf69a: 437/456 ms,
exact settled/patch RGB, stale input and takeover/release pass. Both remain RED
against p50 <=80 / p95 <=150 ms. Detailed raw stages remain in phase4 receipts.

Largest causes: per-RPC process membership scans (14/click), full DPR2 PNG
readback/decoding, event batching. Small admitted display events bypass batching;
large frames retain bounded event credit. 16 KiB maximum accrued bitrate credit.
Now testing incremental identity reads with fresh full verification at signal
and protected thumbnail-guided native DPR crops with full idle verification.
Focused Node checks pass; build in reserved slot 2; >16 GiB memory floor retained.
No acceptance closure. Historical receipts keep original commits.

MD-DISPLAY-02 cleanup: every completed drill removed exact owned groups and
scratch; no containers, provider accounts, other lanes or shared caches touched.
New ownership fixture failed because of a missing synthetic /proc stat field;
corrected in 501c53dfc, final Rust checks pending. Signal guard logic preserved.

## Coordinator asks

MD-DISPLAY-04: 01:35 REVIEW_INBOX mapping remains handled in Phase 3. No newer
inbox present. No protocol request, push/PR/CI/merge/deploy/Cloud action.
Phase4 evidence: /root/.codex/evidence/browser-resume-20260930/display/phase4/.

## Owner questions

MD-DISPLAY-04: transport design, moving-frame quality and native OS/Room gates
remain open. Crops are an experimental first paint; full idle readback verifies
fine details missed by the private thumbnail. Feature remains off.
