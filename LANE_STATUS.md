# MD-DISPLAY-02/04 — Phase 4 local latency, in progress

2026-10-05 03:24 UTC: source 05387e1e4, reserved slot 2 build passes.
Protocol 419, feature defaults off. Two end-to-end DPR2/2Mbps runs passed
p50/p95 78.10/106.60 and 72.80/88.60 ms, exact settled RGB, 20/20 visual
acks each, stale-document/takeover/release and owned cleanup. A third clean
run was RED on p50 88.50 ms (p95 100.20); do not hide that variability.
Native crop bounds now limit tile scanning; measuring the correction.

MD-DISPLAY-04 review 02:17: all three fail-first fixes implemented. Real
100s/254 polls followed by live input passes registration renewal. Its dirty
flag records a temporary test dependency symlink, now removed; execution
files were unchanged. IPC cursor 500 survives transient display/reconnect;
uncached display_next has no automatic stall/loss replay. Node39 / Rust13 /
IPC11 / TS and formatting pass. Current binary includes native tile bounds.

MD-DISPLAY-02 evidence stays external under display/phase4, original source
identities preserved. Every finished live run settles owned groups and removes
exact scratch. No shared resources, accounts, Docker, CI, push or deployment.

## Coordinator asks

MD-DISPLAY-04: REVIEW_INBOX 01:35 and 02:17 handled; no newer entries at 03:24.
No allocation change: local419, relay unchanged. No PID1 signal from this lane;
phase3 startup audit and guarded ownership/cleanup remain in evidence.

## Owner questions

MD-DISPLAY-04: moving fidelity, WAN/Cloud, native OS, live Vault, Room migration
and final transport design remain owner acceptance gates. Small crop paint is
a damage heuristic; full protected settled verification establishes exactness.
