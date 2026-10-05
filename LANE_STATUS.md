# MP-11 latest review handoff — 2026-10-05

MP-11 #879 round 2: `2358f266b` fixes the protocol-435 MCP consumer regression using explicit private provider execution through the existing kernel terminal path. Fail-first 10/10 consumer failures; focused 45/45 and adjacent 94/94 pass; CLI build passes; 26/26 Node entrypoints parse. Public MCP grant admission and observed process provenance replace private metadata reads; admission is not labelled tool execution. Evidence: `/root/.codex/evidence/browser-resume-20260930/mp11fix-879-consumers/REPORT.md`.

MP-11 #868 review mapping on `mp11/fixes`: import/default captured registration/generation retirement fixes in `0b122df01`; Linux regression scope in `63059cc67`; receipts in `6cfb8fdde`. Focused 82/82; lint 36 allowed/0 violations. Full root Node entry retained RED (628 pass, 6 prerequisite failures, 24 skip); all six first failing seams pass focused prerequisite reruns, including owned disposable Unix-role container. Evidence: `/root/.codex/evidence/browser-resume-20260930/mp11fix-review/REPORT.md`.

MP-11 publication isolation on `mp11/publication-run-isolation`, exact base `74e50b787`: fail-first `a08e6c687`, fix `6d1ddae65`. Cross-caller status/result/trace/SSE reads reproduced, then all lookups and refreshed reads bound to publication/caller identity. Full server tests 99/99. Evidence: `/root/.codex/evidence/browser-resume-20260930/mp11fix-publication/REPORT.md`.

MP-11 Rust evidence: focused adapter launch tests passed 3/3 before the input-digest CJS addition; final CJS tested as a real owned subprocess. Redundant final Rust rerun cancelled while childless and still waiting for the shared compile slot (exit 143); no final-source Rust rerun claimed. Enumerated own compiler/dist outputs removed, protected infrastructure untouched.

MP-11 inbox: #879 OAuth/shebang round 1 already addressed at published `1f691871d`; 13:30 three-P2 #868, 14:03 isolation and 14:30 #879 round 2 addressed above. No new inbox entry at last milestone check. The older 08:13 broader Rust/shell signal coverage item is a separate coordinator queue and is not claimed complete by this explicit review round. No protocol allocation or owner decision required for this batch. Commits local only, [skip ci]; no publishing/deployment/live provider-account acceptance.

---

# MP-11 F7 / PR #879 — FINAL

2026-10-05 14:10 UTC — MP-11 implementation head `5a568f3bc`, branch `mp11/credential-access-fixes`, assigned base `74e50b787`; previous published review head `ef8fc7621`. Local [skip ci] commits only; no push, PR, merge, deploy or real provider-account work.

MP-11 F7 complete at source/fixture scope: explicit public provider-run DTO across response/event contracts, private authoritative execution state retained, allocated local protocol **435**, peer **70** unchanged. Snapshot/hash, owner/private projection checks, source boundary guard and focused protocol drill pass.

MP-11 #879 round 1 complete: `d04734543` preserves pending OAuth completion during unrelated webhook delivery (both fail-first regressions reproduce before fix); `d148572b4` restores the first-line hashbang and parses all 57 touched script entries.

MP-11 checks: **175** protocol snapshots + **4** client conformance + **4** F7 regressions pass at `5a568f3bc`; focused drill **3/3**. Rebuilt SDK **55/55** at `d04734543` (SDK source unchanged afterwards). Full Node CI entry exits **0**: **5,469 passed / 25 skipped / zero failed**, plus **49 Bun passed**. Node entry started at `d04734543`; only the Rust fork fixture changed during it. Exact identities and invalid/cache/resource-interrupted attempts are retained without relabeling.

MP-11 cleanup: 13 owned generated roots removed, recorded own live validation roots 0. Observed memory minimum 10.00 GiB and disk minimum 107.55 GiB, above required floors. No shared/other-lane processes, credential profiles, keys/backups, reviewer state, caches or Docker resources touched.

MP-11 commits after previous review head: `ce63f9b98`, `5f5789058`, `7eac524a3`, `d148572b4`, `d04734543`, `5a568f3bc`; documentation handoff follows. FINAL evidence: `/root/.codex/evidence/browser-resume-20260930/mp11fix-ra/REPORT.md`; mappings in `PUSH_READY.md`.

## MP-11 next separate rounds

- PR #868: 08:13 signal coverage plus 13:30 import/viewer/PID-generation findings, queued for `mp11/fixes`; not marked fixed here.
- 14:03 publication run/caller isolation finding: queued for requested new branch `mp11/publication-run-isolation` from `74e50b787` after current items; no finding disposition claimed here.
- MP-11 RA F8: OWNED-BY-mp11fix3.

## MP-11 Coordinator asks

None for the completed F7/#879 stopping scope. Separate requested rounds are recorded above.

## MP-11 Owner questions

None blocking F7/#879. The named checks establish source/synthetic protocol/SDK seams, not the complete ordinary/managed behavioral matrix.
