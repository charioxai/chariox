# MP-11 — reviewable scanner repair, not acceptance-ready

Branch: `agent/b211scan`, based on OSS `9334141d420f8a32393f206102c5b8b4a1b0b609`.
Coordinator publishes; this lane has not pushed or opened a PR.

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
