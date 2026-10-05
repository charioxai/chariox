# MD-DISPLAY-02/04 — Phase 6 completed milestone, performance RED

2026-10-05: execution/embedded kernel 86b3015bd55d43fd811f505081fe13ef8625066a.
Clean 13-case campaign finished 07:20 UTC during the interrupted turn, exit 0.
260/260 click checks; exact settled/final/navigation RGB, stale input, takeover,
focused MCP fencing/release, and owned cleanup pass. Protocol 419/default-off;
no union427 acceptance claim. All MD/MP acceptance items remain open.

MD-DISPLAY-02: WAN40/80/150 docs P95 288/326/463ms (targets140/180/250).
Local docs161.7/259.3ms regresses from phase5. Local canvas/video15–17fps at
2–4Mbps; discrete wheel2–3fps, below24fps. Settled motion RGB remains exact.
The independent receipt aggregator exits1 RED_PERFORMANCE; functional exit0
must not be called performance acceptance. Timings identify host capture/
verification mutex stalls; four credits do not parallelize the serial source.

MD-DISPLAY-04: 58 Node,22 Rust (two ignored live drills), real encoder budget/
delta check and slot2 build pass. Inbox04:53 PNG navigation fixed0c1ce3843;
06:28 union navigation belongs to mdval, no duplicate fix. No new inbox entries.
Detailed review mapping and replay: PUSH_READY.md and display transport doc.

MD-DISPLAY-02/04 evidence: external display/phase6/handoff-window4,
handoff-binary.json and handoff-report/report.json. Bound source/binary/assets,
receipts, screenshots/diffs, resource minima and stage histograms. Minimum
MemAvailable29.1GiB/free197.1GiB. Historical RED runs remain untouched.

MD-DISPLAY-04 cleanup: completed namespaces/process inventories empty; no lane
drill leftovers on resume. Signal audit retains explicit unsafe-PID and fresh
ownership guards; no runtime signaled on resume. Removed5.94GiB lane compiler
incremental output after compiler-only inventory/slot2 lock; replay binary retained.
No CI/push/PR/merge/deploy/Cloud/accounts/shared service/cache changes.

## Coordinator asks

MD-DISPLAY-04: publish exact md/stack-on-main source before runtime follow-ups,
as required by inbox06:28. Public ref absent on resume; no lane-specific
COORDINATOR_NOTES.md exists, so the current inbox is the coordinator source.
This handoff finishes
our existing419 milestone; future work must retain canonical427/relay74.
Next fixes: move encode/pacing outside source/input mutex while fencing policy/
document changes; bounded latest protected source; continuous-scroll and
slow-viewer/base recovery drill. No protocol number requested or allocated here.
Historical host-incident attribution remains unknown; no unsafe resume signal.

## Owner questions

MD-DISPLAY-04: final transport/rollout and acceptable moving/WAN/egress budgets.
Cloud/hosted transport, native OS, live Vault, multi-viewer, reconnect, cursor/
IME/file chooser and Room migration remain separate gates. Performance is RED;
this milestone is reviewable and is not ready for rollout.
