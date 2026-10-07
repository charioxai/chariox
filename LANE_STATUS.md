# MP-08 / MP-10 / MP-11 — credcopies round 3 FINAL (2026-10-07)

Start: `05aebd7d9` (PR #913). Evidence:
`/root/.codex/evidence/browser-resume-20260930/credcopies/round3/`
(`round3-summary.json`, `*-79.tsv`, logs, binary hashes, `env.sh`, cleanup).

## MP-10 / MP-11 — full kernel lib suite: GREEN

- Root cause of all 79 round-2 failures (introduced by this branch): the new
  copied-login check on the local dispatch and launch seams called
  `copied_login_needs_login`, which errored for unsupported providers
  (`dev-stub`) and unregistered profiles; dispatch propagated it with `?`, so
  no stub prompt ever became active. Fix: a non-account provider or an
  unregistered profile is never a copied login (`Ok(false)`).
- Fail-first: `mp08_mp10_mp11_unregistered_or_unsupported_profile_is_not_a_copied_login`
  fails on the pre-fix binary (exit 101, `red-unregistered_or_unsupported_profile.log`).
- 79 round-2 failures run one by one, same CI-like env: pre-fix 79/79 FAIL,
  base `e325afa58` 79/79 PASS, fixed 79/79 PASS. **Pre-existing on base: none.**
- Full fixed suite: **6,072 passed / 0 failed / 22 ignored, exit 0**
  (`green-full2-kernel-lib.log`, `--test-threads=2`, `RUST_MIN_STACK=16777216` as CI).
  A first attempt without CI's `RUST_MIN_STACK` aborted on a stack overflow and an
  exported `CLAUDE_CONFIG_DIR` broke two default-profile tests; both were
  environment-only and superseded (`green-full-kernel-lib.log`).
- Base full suite was not run; only the 79 were compared on base.

## MP-08 / MP-10 — live acceptance: BLOCKED

**BLOCKED(owner: one disposable Codex device login for the drill).** No shared
login was logged out or revoked; no live drill ran this round. The receiving-copy
`logout` step runs official `codex logout` on a copy that shares the source
refresh token, so it can revoke the shared login. Steps once the owner provides
a disposable account used by nothing else:
1. Owner signs in with official `codex login --device-auth` in a fresh, lane-owned
   `CODEX_HOME`; link it through Chariox (`ImportNativeProviderAccountProfile`).
2. Build `chariox-kernel`, `chariox-relay` and the CLI from this branch; run
   `apps/cli/scripts/live-managed-login-copies-drill.mjs` with
   `CREDCOPIES_{RUNTIME,BINARY,EVIDENCE}_ROOT` and that `CODEX_HOME`.
3. `prompt` (CREDCOPIES_LIVE_OK on the worker copy) → `logout` (disposable copy
   only) → `queued` (two turns held, receiving login request shown) → `login`
   (owner completes the new official device login) → expect
   CREDCOPIES_AFTER_LOGIN then CREDCOPIES_QUEUE_RESUMED → `stop`.
4. Real-path acceptance also requires the same flow through the built TUI and the
   hosted `wss` relay. The drill script drives IPC and a local relay only, so it is
   supplementary. Then revoke the disposable login.

Rust work held the shared compile lock, except the pre-fix 79 single-test runs
(test binary only); `CARGO_BUILD_JOBS=4`. Minimum during full run: 20.68 GiB
MemAvailable, 88.48 GiB free. Removed own base worktree and disposable scratch
(HOME/state, verified Node, binary copies). Shared target, provider accounts,
reviewer state and other lanes are untouched. Review inbox absent. No protocol change or
number requested. Local `[skip ci]` commit only.

---

# (previous) MP-08 / MP-10 / MP-11 — credcopies round 2 FINAL (2026-10-07)

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
