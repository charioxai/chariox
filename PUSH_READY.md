# MD-DISPLAY-02/04 — Phase 5 implementation milestone (validation running)

Local commits 740a7ffaa and 2cbfe5618; no publication. Protocol 419, flag off.
First adds owned namespace WAN/moving harness; second fixes moving/oversized
repair with fail-first tests and adds unavailable native design stubs.
41 existing+repair Node tests and 2 native/netem tests pass. Slot-2 rebuild and
final live receipts pending; do not attribute Phase-4 coverage to these files.

MD-DISPLAY-02 exploratory campaign 740a7ffaa with binary built at 05387e1e4:
local 78/92 ms, WAN40 172/193, WAN80 285/307, WAN150 457/490; exact settled
static pixels. MTU/offloads were default in that campaign. Canvas/video under
1 fps; dense-scroll RED on an oversized full repair. Parent exits 1; all seven
namespace and process inventories empty after cleanup. Final packet-realism
rerun uses only owned namespace MTU1500/offloads disabled.

MD-DISPLAY-04 review inbox: no new entries beyond fixed 02:17 findings. Previous
mapping retained below. Native design and stubs are unmeasured on Mac/Windows.
Evidence: /root/.codex/evidence/browser-resume-20260930/display/phase5/.

# MD-DISPLAY-02/04 — Phase 4 ready for coordinator review

Local branch agent/display-impl, execution + kernel build 05387e1e4; repeat
69a4897ef differs only in lane status. Default-off flag and protocol 419 remain.
No publishing action performed. Documentation/status-only final commits follow.

MD-DISPLAY-02 result: real sandboxed headed kernel Chromium outside slices,
DPR2 at 1280x800 CSS, 2 Mbps, existing encrypted production local-relay path.
Before p50/p95 744.20/778.20 ms. Final two runs 73.80/92.40 and 72.80/83.90 ms,
20/20 clicks each; exact settled/final RGB (MSE 0). Repeat includes 100.05 seconds /
256 static polls then live input. Bootstrap VP9 remains 34.04 dB; large video/PNG
refinement and arbitrary moving pages do not meet this small-change latency proof.
Software rAF + checked canvas readback is a presentation proxy, not hardware photons.
Observed CPU 162–172% of one core, received application 27.7/26.7 KB/s; rate excludes
bootstrap/requests/TLS and includes resource-sampling time. 2 Mbps budget charges
bootstrap/repair, with a bounded 16 KiB idle allowance. Stale input, shared actor
takeover/fencing/owner input/release/resumption and cleanup pass in both runs.

MD-DISPLAY-04 implementation: optimized Linux owned-process identity reads
(fresh full signal verification preserved); protected fast native PNG + private
thumbnail-guided DPR crops; full idle verification; crop-bounded PNG tiles;
small encrypted events use bounded priority delivery; TCP_NODELAY; event decode
starts before receipt but single credit waits for both. No new protocol shape,
relay inspection/authority, hosted service, Cloud media route or provider path.

MD-DISPLAY-04 review mapping (REVIEW_INBOX 02:17):

- Static active polls expired registration: c8518a3e2 renews only successful
  admitted display_next. Fail-first static-100s-fail-first-v2/results.json;
  deterministic paused-time unit + final-damage-static-100s live receipt pass.
  Foreign key cannot renew; genuine idle still expires.
- Transient frame overwrote session resume cursor: 69e1782ef uses local event_id 0
  and IPC ignores kernel_browser_frame for durable cursors. Fail-first real
  fake-WebSocket reconnect cursor 1 vs 500; fixed regression preserves 500.
- Uncached display_next auto replayed after stall/loss: 69e1782ef recognizes the
  nested command, keeps outcome-unknown/no automatic replay and no pixel cache.
  Fail-first delayed/lost-response cases in client-review-fail-first.log;
  final-client-tests.log passes 11. Cloud integration note requires same behavior.

MD-DISPLAY-02 provenance: external phase4/provenance.json binds binary SHA256,
134 unchanged execution hashes, 30 exact embedded assets, commands/exits/checks
and historical/final receipts. Final clean receipts: final-damage-2mbps and
final-damage-static-100s. Source 05387e1e4 and 69a4897ef respectively; no dirty flags.
Earlier successes, failures and mixed-source/dirty receipts retain their identities.
Intermediate final-clean-2mbps at 6cfcf6410 is RED (p50 88.50, p95 100.20); bounded
tile scan addresses its remaining cost. Nothing is relabelled as final-file coverage.

MD-DISPLAY-04 checks: 39 Node, 13 Rust ownership/registration/queue/protocol/event/
actor/takeover/origin, 11 IPC; TypeScript, rustfmt and slot 2 build pass (40 existing
warnings). All measurements/screenshots remain external under
/root/.codex/evidence/browser-resume-20260930/display/phase4/.
Cleanup.json records 6.41 GB removed own output, no scratch or owned processes;
current binary/dependencies retained for coordinator replay. No shared resources
or protected stores changed. Research branch agent/display remains untouched.

MD-DISPLAY-04 next gates: independent review and coordinator Cloud wiring;
matched moving-media/bitrate/WAN, slow viewer/reconnect/live Vault, native OS,
multi-viewer and Room acceptance. This is local component readiness, no MD/MP
acceptance closure. Owner transport/design decisions remain open; feature stays off.
