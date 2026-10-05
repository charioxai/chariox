# MP-11 — mp11narrow local commits ready for coordinator source review

- `dee49c6c6e304f0866ea730fd9967965a6cba962`: owner narrowing, checked-in scope definition, whole-blob security anchors, external parity gate, classifier/gate tests and CI test-command wiring. Full inventory stays non-gating.
- `2da5b417cade4dff2d7a15159453cfcc05f33a4f`: 39 exact current OSS semantic records, explicit verdict/severity validation, malformed-scope refusal regression and findings. Product source unchanged.
- MP-11 final focused checks: **213/213 PASS**, independent 1,046-anchor/39-record Git bindings pass, eleven tool module hashes verified against committed tool. No Rust or GitHub CI run.
- MP-11 gate remains RED: **1,007 reviews pending** (including 26 unknown classes), four finding records across three seams, external behavioural matrix and Cloud `f1ad79056` pending. Owner's >~600 priority/remainder provision applies. No per-anchor approval required for ordinary noncritical source; no live acceptance claimed.
- Evidence/report/findings/fix proposals/cleanup: `/root/.codex/evidence/browser-resume-20260930/mp11narrow/REPORT.md` FINAL.
- MP-11 review-inbox mapping: REVIEW_INBOX.md absent after scope commit, after semantic commit and at final handoff; no unhandled entries. Prior PR #848 reviewer P1/P2 mapping below remains unchanged. Independent scanner-tool review and publication belong to coordinator; no push/PR/comment/merge/deploy by this lane.

# MP-11 — reviewable scanner repair, not acceptance-ready

Branch: `agent/b211scan`, based on OSS `9334141d420f8a32393f206102c5b8b4a1b0b609`.
Coordinator publishes; this lane has not pushed or opened a PR.

## MP-11 — PR #848 reviewer findings addressed

| Finding | Fix commit | Fail-first and fixed evidence |
| --- | --- | --- |
| P1: shallow Node CI lacks frozen source | `fd0a173315a0cc923105b16fb0138f8685c658f8` | Both Git-bound tests fail without G2 in a depth-one checkout. The Node job explicitly fetches `9334141d420f8a32393f206102c5b8b4a1b0b609` before testing; the same tests pass with HEAD unchanged. Public GitHub origin verified. |
| P2: UTF-8 decoding removes the BOM | `721756ebd815124f9841e360e1efebfdae2c60fc` | Four new tests fail with columns one character early, then pass with `ignoreBOM: true` at both decoder sites. Covers fixtures, immutable blobs, both fragment formats and malformed UTF-8 rejection. |

MP-11 final code head `721756ebd815124f9841e360e1efebfdae2c60fc` passes
**197/197 scanner tests, zero skips**, in a fresh depth-one checkout after the
exact CI fetch from the public origin. The full failure/fetch/success sequence,
commands, resource samples and cleanup are retained in
`reviewer-final-shallow-suite.{log,receipt.json}` under the lane evidence root.
`REVIEWER_FIXES.md` indexes every RED/GREEN receipt and exact source limitation.
The complete Node/web monorepo suite and GitHub CI were not run by this lane.

MP-11: the **9,949-anchor semantic audit remains paused pending owner decision**.
No review records, predicates or runtime source identities were repinned or
re-audited. These reviewer fixes are ready for coordinator publication and
independent exact-head tooling review; they do not close an MP acceptance gate.

MP-11: fixes narrow shipped source formats, exact blob/mode fixture exclusions,
immutable object reads and standard unified/Git patches. It preserves physical
patch lines, old/new roles and historical rule/review records. Current declaration
expectations and 81 newly authored bounded runtime-source dispositions live in
separate modules; source drift receives no inherited approval. Same-source stale
reviews still block admission; historical/other-source records remain visible.
No runtime protocol or product behavior changed.

MP-11 validation: 193 focused Node tests, zero skips; 10,030 independently
Git-bound candidates and 81 current review anchors verified. Both frozen-source
scans have zero source-audit gaps and still exit 1. Evidence and exact final tool
identity/commands/resources/cleanup are indexed under
`/root/.codex/evidence/browser-resume-20260930/b211scan/`.

MP-11 limitation: the requested complete current semantic audit is **unfinished**.
9,949 candidates remain unreviewed, with a complete responsibility-specific,
anchor-bound pending request index. These counts are not defect counts. The
three unavailable historical predicates retain OWNER status. Independent tooling
review and fresh-machine MP-10 comparison remain required; no MP acceptance item
is closed. This handoff is ready for source review, not a source-gate or merge
approval.

MP-11 integration: do not copy current semantic records onto a new runtime commit
or enlarge declaration ranges to make a scan green. Re-review exact aggregate
source and preserve the frozen G2 and historical evidence identities. No protocol
allocation is needed for this tooling-only delta.
