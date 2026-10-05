# MD-DISPLAY-02/04 — Phase 6 milestone ready for review, performance RED

Branch agent/display-impl. Execution/kernel build86b3015bd55d43fd811f505081fe13ef8625066a;
this handoff changes documentation/status only. Shared protocol419, relay
unchanged, default-off. Future runtime changes await coordinator md/stack-on-main
and retain its427/74 allocation. No publishing action; no acceptance item closes.

MD-DISPLAY-02 changes since phase5: four credited slots (selectable1–8), bounded
receive/reorder, persistent CBR VP9/deltas, corrected image-to-video PTS, decode
stale deltas without drawing them, input-woken idle backoff, protected complete
CSS-resolution JPEG motion only with an empty Vault registry, native exact idle
repair, small native tiles and per-user queued-capture input priority.
The source/input/Vault/actor barriers and caller-scoped admission remain intact.

MD-DISPLAY-02 results: clean13-case final campaign exits0 and passes260/260
counter checks, exact settled/final/navigation RGB, same-stream navigation,
stale input, shared takeover/focused MCP fencing/release and owned cleanup.
These are functional passes. Separate immutable-receipt aggregator exits1 with
RED_PERFORMANCE: local docs161.7/259.3ms; WAN40/80/150 P95 288/326/463ms exceeds
RTT+100ms. Canvas/video15–17fps at2–4Mbps; discrete wheel2–3fps fails24fps.
Exact motion settling4–13s local,7–20s WANcanvas. Initial WANdocs repair13/27/89s
at negotiated2/1/0.5Mbps; loss/jitter/caps preserved. No universal Selkies claim.

MD-DISPLAY-02 remaining seam: moving protected host IPC50–56ms/frame; full PNG
verification can block input100–200ms. Encode and pacing share the host mutex.
Four credits hide round trips, not serial source work. Continuous scrolling is
not yet a valid24fps fixture. Next fixes/design are in the transport doc.

MD-DISPLAY-04 source and evidence binding:

- Kernel binary /root/work/cargo-target-display-impl/debug/deps/chariox_kernel-f815052ed359e608,
  SHA25647320741feb53d8ea98a36e8b5f738a3c44340133130c86358d28eb8a6d67287.
- External /root/.codex/evidence/browser-resume-20260930/display/phase6/:
  handoff-window4/campaign.json and13 results.json receipts/screenshots/diffs;
  handoff-binary.json verifies30 embedded assets; handoff-report/report.json
  checks measured execution bytes against86b3015bd and hashes all measured PNGs.
- Replay recipe is in docs/MULTIDOMAIN_KERNEL_BROWSER_DISPLAY.md. Case receipts
  retain exact drill argv and source/dirty flag. Performance aggregation script
  is lane-owned agents/display/phase6-report.py; its command/exit/hash are external.
- Minimum sampled resources:29.1GiB MemAvailable and197.1GiB free. All final cases
  remove only their own namespace and disposable state; exact-root inventory empty.

MD-DISPLAY-04 review mapping:

- Inbox04:53 PNG-only navigation:0c1ce3843 negotiates independent kind from codec;
  focused navigation unit pass. Final campaign negotiates VP9, not PNG-only live.
- Inbox03:42 navigation/dependency:7197ac202; prior fail-first/fixed and relocated
  receipts retained. Every final419 navigation/exactness check passes. The06:28
  union navigation finding belongs to mdval and is not duplicated by this lane.
- Inbox02:17 registration/cursor/no-replay:c8518a3e2/69e1782ef; phase4 fail-first,
  focused protocol/registration/queue/actor and static100s evidence retained.
- New priority/cancellation:983110b9e RAII input-admission/capture cancellation
  regressions pass. This bounds queued input starvation, not in-progress capture.
- Small-change/motion/timebase:60627b58a/63eb8ef2f/6c029a5d9/7aefaf2d6;
  tests retain encoder/draw fail-first receipts and bounded rate/delta regression.
- Historical RED coarse-damage/source, oversized repair and scrollbar/compositor
  refinements remain in their original evidence dirs. Final13 cases verify late
  native paints before claiming pixel exactness;86b3015bd is the measured source.

MD-DISPLAY-04 checks:58 Node,22 focused Rust (two ignored live drills), real
encoder budget/delta regression, slot2 build and419 snapshot/hash guards pass.
Logs final-node-tests.log,final-rust-kbrowser-tests.log,final-encoder-test.log,
final-build-tests.log remain external. No new test execution claimed for union427.
Signal helpers reject0/1/-1/NaN/missing PIDs and recheck owned group/start identity.
No new runtime signaled on resume; shared/default interface/resources untouched.
Removed5.94GiB compiler-only incremental output under slot2; current replay
binary/public dependencies retained. Cleanup inventory:phase6/resume-cleanup.json.

MD-DISPLAY-04 publication boundary: inbox06:28 requires further implementation
on md/stack-on-main. Its public ref is still absent. Coordinator must supply the
exact canonical source; this milestone finishes419 evidence only. Native,
Cloud/live Vault/multi-viewer/reconnect/Room gates and performance remain open.
Research agent/display remains unchanged. No GitHub CI/push/PR/merge/deploy.
