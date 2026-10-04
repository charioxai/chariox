# MD-DISPLAY-01/02/03/04 — Phase 2 research handoff

READY FOR REVIEW on local `agent/display`, frozen G2 base
`9334141d420f8a32393f206102c5b8b4a1b0b609`. Coordinator publishes; owner chooses
the production design. This research does not authorize a production merge.

MD-DISPLAY-01/04: [comparison and seam proposal](docs/MULTIDOMAIN_DISPLAY_TRANSPORT.md)
contains 0.5/1/2/4/8 Mbps configured/observed curves, default Selkies parser/bytes,
colour/DPR2/interaction/security/OS costs, prior art, API, migration and decisions.
Best static path: paced RGB PNG with dirty tiles at 2 Mbps, exact pixels on frozen
fixtures. Native RGB H.264 improves frozen fixture fidelity. Dense scrolling and
media latency still fail the broader higher-quality/comparable-latency goal.
Recommend a transport-neutral tab source, budgeted exact idle refinement and
selective DOM experiments; preserve Room desktop coverage. No protocol change.

MD-DISPLAY-02/03: [reproduction instructions](experiments/multidomain-display/README.md).
Historical 105-cell matrix and supplements stay bound to original source/module
hashes. They do not cover the new lifecycle code. The 187-row interactive report
preserves RED points, actual/target traffic, CPU, latency, PNG pairs and diffs.
Native ultrafast/veryfast and rate controls plot as separate series.

## MD-DISPLAY-02/03 reviewer mapping

| Finding | Fix and focused proof |
| --- | --- |
| 1: launch errors/readiness leak resources | `runtime.mjs` + `owned-process.mjs`; missing Chrome/Xvfb, display timeout and owned descendant tests. Real missing-Chrome run writes RED/exit 1 and cleans both displays/profile/listeners. |
| 2: Selkies callback exceptions escape finally | `selkies-packets.mjs` latches stripe/JSON/send errors into awaited ready/stop/global checks. All three callback tests pass. Real baseline injected send failure writes RED/exit 1, closes stream/browser and deletes its labelled container. |
| 3: campaign ignores child failures | Nonzero/null/signal child statuses fail parent; interruption is explicit 130. Tests cover child 0/1/124/130/signal, launch error and parent interruption; fast completion registered before launch returns. |
| 4: stale final-file provenance | Phase 1/2 identities remain historical. New clean `a0c88cce7` confirmations bind 14 core module hashes that match final execution files; the full historical ladder is not claimed as rerun. Report generator has separate exact file/hash validation. |

MD-DISPLAY-02/03 incident audit: reject invalid/unregistered/reused PIDs, verify
all group members before negative signals. No unsafe signal issued in tests.
No retained run proves cleanup at 23:28:37; sender remains unknown. Initial and
final owned inventories empty. No REVIEW_INBOX.md found after milestone checks.

## MD-DISPLAY-02/03 validation and provenance

Evidence root `/root/.codex/evidence/browser-resume-20260930/display/`:

- `PHASE2_MANIFEST.json`: 44 historical run receipts, exact sources/hashes,
  commands/exits/settings/resources/cleanup. No source identity rewritten.
- `review-recovery/clean-confirm/PROVENANCE.json`: clean `a0c88cce7`, 39 cases,
  780 clicks; H.264/VP9/AV1/RGB/native 2 Mbps, DOM/hybrid switches, default/CBR2
  baseline all exit 0. Expected missing-Chrome RED exits 1 with cleanup.
- `review-recovery/clean-tests.json` + log: 17/17 focused tests at clean source.
  `baseline-callback-fault/results.json`: real callback failure and owned teardown.
  `fail-first.log`: original unsafe PID/launch/campaign regressions.
- `phase2-review-v2/report/index.html`, `summary.json`, `report-validation.json`
  and PNGs: 187 receipt-bound rows, source/viewer pairs, chart/axis/point and
  opacity checks; no browser errors. Historical report receipts remain intact.
- `review-recovery/final-inventory.json`: no owned processes, containers or
  disposable profiles. Historical sample floors 22.03 GiB memory / 211.15 GiB
  disk; confirmation floors 40.20 GiB / 215.09 GiB. Pinned public tools/evidence
  retained outside Git; no build outputs created.

MD-DISPLAY-04 decisions: moving vs settled fidelity, p95/WAN/network budget,
initial page-only scope, client codecs/capture permissions, masking/geometry,
App transient state and action policy. Native OS/GPU, WAN, Vault, hostile pages,
IME/clipboard/filechooser/cursors, multi-viewer/reconnect and production encrypted
relay remain unvalidated. No acceptance item is closed. No providers/credentials,
shared services, protected keys, Cloud, GitHub writes, deployments or cache pruning.
