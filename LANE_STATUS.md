# MD-DISPLAY-02/04 — Phase 4 instrumentation in progress

2026-10-05: fixed-label local stage timers added without changing protocol 419
wire shapes. Measuring viewer-local input to rAF after draw, excluding Playwright
round trips. 33 focused Node checks pass; instrumented Rust build in slot 2.
No owned leftover drill resources found (inventory shell false positive retained
in external receipt). Feature stays off. No MD acceptance closure.

# MD-DISPLAY-02/04 — Phase 3 reviewable, default off

2026-10-05 final: implementation source `836e64630bc53d42e488dc97142416fdb0c92271`,
rebased onto kbrowser `d6d03751ffea37198fb33530829f4cd76ae30fbf` per 01:35 inbox.
Protocol 419; protected DPR2 VP9 + exact PNG/tiles through the typed capture/input
API and existing encrypted kernel/relay queues. Public takeover/release/actors
share the kernel ledger. Cloud integration module/harness and note are complete.

MD-DISPLAY-02/04 final clean relay receipt: exact settled/patched RGB; 20/20 visual
click acknowledgements; p50/p95 771.71/822.16 ms; 7,451 received encrypted bytes/s
in the click interval; 70.5% observed owned CPU. Stale input is rejected; human
takeover fences focused MCP input, owner input works, release resumes the agent.
Latency remains RED against the comparable-latency goal. Bootstrap VP9 34.04 dB
also does not beat historical Selkies docs fidelity. No acceptance closure.

33 Node checks, 14 focused Rust checks, TypeScript, rustfmt and local build pass.
Evidence/commands/source and binary hashes:
`/root/.codex/evidence/browser-resume-20260930/display/phase3/provenance.json`.
Final live receipt `final-typed-relay-2mbps/results.json`. Prior receipts remain
historical; pre-rebase source retained at `agent/display-impl-pre-typed-api`.

MD-DISPLAY-02 cleanup: all owned drill state/processes gone; no containers created.
Removed 6.16 GiB own incremental output, own temporary dependency symlinks and
empty helper. Final binary/dependency objects retained for coordinator replay.
No shared state, reviewer services, Docker caches, credentials or other lanes touched.

2026-10-05 preceding milestones: pre-rebase clean component p95 704–772 ms;
presenter event-before-receipt credit race fixed with fail-first test; typed API
integration at `0949b015d` passed p95 803 ms; failed encoder retry spawn fixed
with fail-first test at final source. Direct browser/local socket check correctly
failed the existing Origin guard; unsupported drill mode removed. RED receipts
are retained with their original sources. Research branch unchanged.

## Owner questions

MD-DISPLAY-04: acceptable moving/settled fidelity, p95 latency and WAN/frame/total
egress budgets, platform codecs, initial page scope, IME/cursor/clipboard/file
chooser coverage and Room migration criteria remain decisions. Flag stays off.

## Coordinator asks

MD-DISPLAY-04: 01:35 REVIEW_INBOX handled: rebase + typed API + shared actors,
protocol 419 snapshots and end-to-end takeover/release proof. See PUSH_READY.md.
Cloud wiring, hosted WAN and native Mac/Windows validation remain external.
No protocol allocation requested; no push/PR/CI/deploy actions performed.
MD-DISPLAY-02: no owned incident leftovers found on initial inventory. This new
lane was not running at 23:28:37; research incident attribution remains there.
