# MP-08 / MP-10 / MP-11 — credcopies round 2 FINAL (2026-10-07)

Local branch `cred/managed-login-copies`; exact starting source
`1f43bfc7a2a7fd0df42d23798d05abcc4332d19a` (PR #913). Read its public PR body,
AGENTS.md and the three frozen Browser/Computer/Path-1 plans. Builder1 evidence:
`/root/.codex/evidence/browser-resume-20260930/credcopies/round2/`.
Historical p1b reports are preserved there as `historical-LANE_STATUS.md` and
`historical-PUSH_READY.md`; their runs are not relabelled as builder1 results.

## MP-08 / MP-10 / MP-11 — implementation and review mapping

- P1: all ten remaining local-protocol assertion guards now expect 455,
  including the four included `lib_tests.rs` guards. Full-suite discovery also
  found four stale peer fixtures: they now exercise 74 admission and explicit
  rejection of 73. Shared local 455 / peer 74 constants remain unchanged.
- P2 receipt: forward selected imported provider-account receipts, preserving
  receiving-default remaps and complete copy metadata. Fresh completion and
  stored completed-import replay use the same projection.
- P2 slice reimport: observe an existing receiving profile freshly. Preserve
  authenticated receiving credentials and their tracked generation; otherwise
  fail with the receiving `/slice auth login` command instead of claiming an
  import. This chooses the review's actionable-failure option.

No serialized shape change or new protocol allocation. Existing home-owner,
bootstrap slice and pinned-home-key admission checks remain intact. Synthetic
fixtures contain no real credentials; no shared official logout/revocation ran.

## MP-10 / MP-11 — checks, exact identities and limits

- Fail-first build: base plus new regression tests and a neutral extraction of
  the synchronous import body (`red-source.patch/json`). Ten stale local guards,
  fresh/replayed copy receipts and absent receiving login each failed (exit 101).
  Authenticated-login preservation and empty receipt controls passed.
- Final source (`clean-green-source.patch/json`): all **19 focused checks PASS**
  (exit 0), covering all ten local guards, five receipt/import checks and four
  peer fixtures. Earlier broader selected suite: 66 passed, 1 X11-dependent
  Computer test ignored (`final-green-focused.log`, earlier recorded source).
- Full final kernel library suite: **RED**, **5,992 passed / 79 failed / 22
  ignored**, exit 101 (`clean-full-kernel-lib.log`). First failing seam:
  `direct_prompt_cancel_uses_explicit_target_agent_when_multiple_agents_are_active`
  expects `Some(Running)` but sees `None`; the next cancellation fixture reports
  `NoActivePrompt`. Remaining prompt/runtime/workflow fixture failures are
  unresolved; no full-suite success or original-base regression claim is made.
- Reran all 79 failures individually on the saved pre-binding-fixture build:
  78 failed again; `remote_completion_dispatches_prompts_queued_by_detached_clients`
  passed. That build includes the main P1/P2 fixes and differs only by the last
  peer test update (`full-baseline-hold.json`, `final-green-source.patch`). This
  comparison is not original-base evidence.
- Installed Node 22.22.1 lacks TypeScript support. Final checks used an isolated
  official same-version Node download verified against its official SHA manifest
  (`node-toolchain.json`); shared Node was unchanged. Short disposable HOME and
  no inherited provider credentials prevented long Unix socket paths and default
  Claude-profile environment interference. Command/exit/build identities are in
  `clean-batch-results.json` and `clean-green-test-binary.json`.

Rust work held the shared compile lock, used four build jobs and resource
watchdogs. Across recorded runs, minima: 12.08 GiB
MemAvailable, 91.53 GiB disk free. Disposable HOME/state,
saved test-binary holds, temporary Node and exactly inventoried lane-generated
test state were removed. No own live child remained. Shared cache, old
PID-collision paths, provider accounts, reviewer state and other lanes were
preserved. See cleanup manifests and resource samples outside Git.

## MP-08 / MP-10 — Owner questions

**BLOCKED(owner re-login):** owner must perform a new official receiving Codex
login. Then run the real built-client/kernel/hosted-relay/official-provider drill
and confirm the admitted turn and queued work resume. No replacement login,
logout or revocation is attempted here. These source checks do not establish
real live acceptance. Existing OpenCode/Claude account and broader real-path
acceptance gaps from the historical report are not closed by this round.

## MP-11 — Coordinator asks

Investigate the remaining full-suite prompt/runtime fixture failures and the
shared Node build's missing TypeScript support. No protocol number requested.
The narrowed MP-11 scope applies: this report is focused behavior/security-anchor
inspection, not closure of the full parity matrix or independent semantic review.

Review inbox absolute path:
`/root/.chariox/dev/browser-resume-20260930/agents/credcopies/REVIEW_INBOX.md`.
Absent at checks through this milestone; review mapping is in `PUSH_READY.md`.
Local `[skip ci]` commit only; no GitHub CI, push, PR, merge, staging or deployment.
