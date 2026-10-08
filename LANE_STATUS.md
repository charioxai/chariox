# Lane status

2026-10-08 — `cred/managed-login-copies`, PR #913 reviewed published head `65a4c8c35`.

Both P2 findings are fixed in local [skip ci] commits: home Claude cold dispatch `5eb73ab98` (fail-first `07b0be92d`); lost-ack reconciliation `1b5fc6f31` (fail-first `b20e15a41`). Both fail-first failures reproduce the review errors, and all four new regressions now pass. PUSH_READY.md records the mapping and earlier review history.

Validation: 20 focused suites plus four explicit regressions pass (438 test executions across overlapping filters). Kernel test build, clippy --all-targets (existing warnings), workspace fmt and diff checks pass. Real linked Codex import/retry/local-removal/encrypted-replay cells pass, including receiving-kernel restart. Home reconciles an unconfirmed G1 receipt on a G2 retry and observes local removal; replay does not restore the credential. The production receiver regression separately drops the first successful response and returns a timeout. Evidence: `/Users/miguel/.codex/evidence/credcopies-review-ack/`.

Protocol: local 455 / relay peer 74 unchanged; no serialized client/relay contract change or number request.

HARD RULE: no official logout, revocation or re-login ran on any shared login or its receiving copy. Those steps and live new-login/queued-resume acceptance remain `BLOCKED(Miguel: disposable login)`. Claude live cold-launch acceptance is `BLOCKED(owner)`; read-only official status reports no linked login. The synthetic Claude home regression reaches transport; it does not certify a real provider run. Cloud G9 remains a follow-up.

Cleanup completed: receiving copy locally removed by the kernel, owned test/relay processes ended, dedicated compiler output removed; protected runtime identities and source linked profiles retained.

No push and NO GitHub CI. Ready for coordinator review; stop after the final local handoff commit.
