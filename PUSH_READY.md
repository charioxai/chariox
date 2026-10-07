# MP-08 / MP-10 / MP-11 — PUSH_READY: NOT READY FOR ACCEPTANCE

Round 2 addresses all three PR #913 findings on the exact `1f43bfc7a` base.
Coordinator publishes local commits; this lane never pushes or stages anything.

| MP items / finding | Fix | Fail-first and final evidence |
| --- | --- | --- |
| MP-10 / MP-11, P1 protocol guards | Ten local guards expect 455; four stale peer fixtures exercise 74 and reject 73 | `red-guard-*.log`; `red-additional-peer-gate.log`; earlier broad/full RED logs; `clean-focused-*.log` |
| MP-08 / MP-10 / MP-11, P2 account-copy receipts | Forward selected receipts and remaps on fresh completion and completed replay | `red-mp08_mp10_mp11_*receipt*.log`; final receipt checks |
| MP-10 / MP-11, round 3: 79 full-suite failures | `copied_login_needs_login` returns false for unsupported providers/unregistered profiles | `round3/red-unregistered_or_unsupported_profile.log`; `round3/{red,base,green}-79.tsv`; `round3/green-full2-kernel-lib.log` (6,072 passed, 0 failed) |
| MP-08 / MP-10 / MP-11, P2 missing receiving login | Fresh observation; authenticated receiver retained, missing login returns actionable failure | `red-mp08_mp10_mp11_slice_reimport_missing_login_is_actionable_failure.log`; final missing/valid checks |

Evidence root:
`/root/.codex/evidence/browser-resume-20260930/credcopies/round2/`.
Round 3: full kernel lib suite GREEN, 6,072 passed / 0 failed / 22 ignored;
the 79 round-2 failures were branch-introduced (all pass on base `e325afa58`). Full command, exact source patches,
binary hashes, failure comparisons, toolchain workaround and cleanup are recorded
there and explained in `LANE_STATUS.md`. A passing source check is not acceptance.

**BLOCKED(owner: one disposable Codex device login for the drill):** receiving Codex login and real queued-work resumption
remain required. Do not run an official logout/revocation on the shared login.
Historical live results are retained separately and are not builder1 evidence.
No new real client/provider/hosted-relay acceptance success is claimed.

Protocol constants remain local 455 / relay peer 74. No serialized shape changed.
The absolute credcopies review inbox was absent at milestone checks; no additional
inbox findings were available. The three supplied findings map to the rows above.
MP-11 narrowed scope applies; full parity and current independent security review
are not closed by these supplementary checks.

No GitHub CI, push, PR, merge, deployment or Cloud staging activity.
