# MP-11 — mp11narrow FINAL (2026-10-05)

## MP-11 — Local handoff complete; acceptance OPEN / RED

- Owner narrowing implemented: BOTH external MP-01..MP-10 + b211 parity dispositions and exact-blob security-critical reviews. Checked-in eight-class directory/content definition fails closed for unknown trust-root classes; whole-file/class anchors catch zero-match helpers. Full inventory is diagnostic/non-gating. Amendments are in both canonical plan and scanner inventory doc.
- Local commits: `dee49c6c6` (scope/gate/tests/docs), `2da5b417c` (39 exact current reviews, findings and strict verdict/malformed-scope checks). All [skip ci], required co-author; no publication by this lane.
- Exact OSS `74e50b787a5919ee5c3d580c5b088989fd4a1adf`, tree `3cb4ef5c883e8d358d0beab455797ded180a2406`: **1,046 narrowed anchors / 906 files**. **39 records / 30 complete files reviewed**, 35 OK and four FINDING records across three seams. **1,007 remaining**, including 26 unknown-class files. Over ~600, so owner-authorized prioritization/remainder used; no blanket approval or claim of exhaustive completion.
- MP-11 findings (product unchanged): P2 Vault extension can revive an expired cached lease after awaiting management reply (`secret/vault.rs:486`); P2 raw process-group guard gaps in host observer (`managed-browser-computer-parity-host-observer.mjs:42`) and macOS storage CI (`test-app-storage-macos-ci.mjs:50`). Concrete fixes/regressions are in external REPORT.md FINAL; static review, no live exploit claim. B211-KEY's old default is corrected at the current source, without inventing live parity disposition.
- Focused Node checks **213/213 PASS**, zero skips. Exact OSS narrowed scan exit **1**: current reviews/findings/unknowns plus external matrix still open. Full inventory diagnostic exit **0**. Independent Python/Git verifies **1,046 anchors + 39 records**, zero binding errors; all eleven tool module hashes match `2da5b417c`.
- Cloud requested `f1ad79056` **PENDING**: no checkout at that head found. Existing Cloud checkout was neither scanned nor changed. External parity matrix remains owned by other lanes; no MP-01..MP-11 acceptance item closes.
- Final receipt minima **16.52 GiB MemAvailable / 61.26 GiB free disk**. Node/Python only; no Rust/provider/kernel/container/credential/key/port/build output, protocol allocation, push/PR/comment/CI/merge/deploy or protected-host contact. Own source-only worktree removed after exact identity/clean check; implementation and small public-source evidence retained. Shared infrastructure untouched.
- REVIEW_INBOX.md absent after both milestone batches and final handoff; prior PR #848 fixes remain mapped. Evidence: `/root/.codex/evidence/browser-resume-20260930/mp11narrow/REPORT.md`, review records, remainder index, scans, commands/exits/resources and cleanup receipt. Coordinator handoff: PUSH_READY.md.

## MP-11 — Coordinator asks

- Publish/review local scanner commits; no protocol allocation needed.
- Assign F1/F2 source fixes, remaining class/review work and Cloud exact-head scan.
- Supply current MP-01..MP-10 + B211-KEY/APP/CAPTURE evidence/dispositions; this lane does not own their acceptance matrix.

## MP-11 — Owner questions

- No new owner-only decision blocks this authorized handoff. Overall semantic/parity/Cloud gates remain open; owner-authorized >~600 priority/remainder exception was used.

# MP-11 — b211scan

## MP-11 — PR #848 reviewer fixes ready for coordinator publication

- Starting tool head: `5667ae4d8784e21607f616a3f551a42c6f8a6b68`; existing `agent/b211scan` branch retained.
- P1 commit: `fd0a17331`. Both frozen-Git tests fail in a depth-one checkout without G2 (2/2 RED). The Node CI job now fetches the exact pinned source; the same clone passes both tests after that step, with HEAD unchanged. Public GitHub origin verification also passed. Evidence: `reviewer-p1-{red,green,public-origin}` receipts/logs.
- P2 commit: `721756ebd815124f9841e360e1efebfdae2c60fc`. Both strict UTF-8 decoders retain U+FEFF with `ignoreBOM: true`. Four fail-first regressions cover fixture/committed-blob first-line columns (19 rather than 18) and both fragment boundary columns (2 rather than 1). Invalid UTF-8 remains rejected in both decoder paths; candidate disposition remains unreviewed. Evidence: `reviewer-p2-{red,green}` receipts/logs.
- Complete scanner suite: **197/197 PASS, zero skips**. Final code head `721756ebd` also passes the complete suite in a fresh depth-one checkout after the exact CI fetch from public GitHub, with HEAD unchanged (`reviewer-final-shallow-suite`). Historical rule/review modules remain unchanged. `PUSH_READY.md` maps both findings to commits; `REVIEWER_FIXES.md` indexes evidence.
- The 9,949-anchor semantic audit remains paused pending the owner decision. No runtime/protocol changes or approval repinning are in scope.
- Initial resources: 66 GiB available disk and 19.95 GiB MemAvailable, above MP-11 lane floors. Node/Python only; coordinator retains publication.
- Reviewer-fix receipt minima: 11.52 GiB MemAvailable / 69.84 GB disk; no resource interruption. All own shallow clones and test fixtures were removed. No build output, provider/runtime/process/container state, credentials or durable keys were created; foreign state was untouched. No Rust build, protocol allocation, push, GitHub CI, PR/comment, merge, deployment or protected-host contact.
- Both findings are addressed; independent review and coordinator publication remain external gates. The semantic audit is paused, not completed, and no MP item closes. This lane stops at the reviewable handoff.

## MP-11 — 2026-10-04, reviewable scanner repair; semantic gate OPEN

- Base: OSS `9334141d420f8a32393f206102c5b8b4a1b0b609` / tree `f78d25308bd4cc012f129a86df8ff537699712df`; branch `agent/b211scan`. Cloud read-only target: `50eb909aa70298e16daca533a6b3a4f5b63ffeae` / tree `9f9c7c813af08d94da4f2f079cc60ef93b1b89d5`.
- Repaired shipped formats, exact blob/mode fixture exclusions, immutable Git reads and standard/Git patch sections with physical offsets and old/new paths. Unsupported formats, binary/symlink source, unsafe paths, malformed hunks and changed fragment consumers fail closed.
- Resolved 45 b211 declaration expectations plus two additional shipped prompt-assembly gaps using exact current point ranges/hashes. Declarations grant no approval. Historical predicates/rule records and all 747 shipped historical semantic reviews are unchanged; no external F records imported or repinned.
- Authored 81 independent runtime-source dispositions in 26 bounded scopes (80 OSS, one Cloud). Scanner implementation remains subject to independent tooling review. This is partial semantic coverage: **9,949 remaining current candidates have no approval**. Every remaining anchor has a responsibility-specific review request in external evidence. The requested exhaustive current semantic review is not complete.
- Focused Node suite: 193/193, no skips. Independent Python/Git verification: 10,030 candidate bindings and all 81 current reviews, zero binding failures. Frozen-source scans: OSS 7,094 / Cloud 2,936 candidates, zero declaration/fragment gaps, both exit 1 because semantic coverage remains incomplete. Final committed-tool rerun receipts are in the evidence index; no MP item closes.
- Initial RED: old shipped scanner exits 2 on the synthetic Claude PTY fixture. Intermediate integration RED was a module-count assertion and later an invalid test selection for a range-bearing patch rule (the shipped patch rules had no ranges); both test corrections are retained in logs. Later resource samples stayed above the 9 GiB / 60 GB floors.
- Node/Python only; no Rust build, protocol allocation, provider run, kernel/runtime identity, container/image/volume/port, push, PR, CI, merge, deployment or protected-machine contact. Temporary test fixtures clean themselves; public tool snapshots and verification code are retained as evidence. Foreign checkouts, reviewer state, credentials, caches and processes remain untouched.
- Evidence: `/root/.codex/evidence/browser-resume-20260930/b211scan/`; handoff: `PUSH_READY.md`. Source-only worktree retained for coordinator publication.

## MP-11 — Coordinator asks

- Publish/review the coherent scanner-only correction when appropriate. No protocol allocation is needed.
- Arrange independent exact-head tooling review. Keep the **9,949 outstanding current semantic dispositions paused pending owner decision**; retain `REMAINING_SEMANTIC_REVIEW_REQUESTS.json` as exact commit/tree/blob/line/hash requests, not approvals or defect claims. Frozen G2 reviews cannot approve a later aggregate automatically.

## MP-11 — Owner questions

- Retain the three unavailable historical predicates as OWNER at their original identities. This lane neither reconstructed nor repinned them. No new owner-level policy decision was needed for scanner repair.
