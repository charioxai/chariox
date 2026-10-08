# Lane status

2026-10-08 — `cred/managed-login-copies`, PR #913 baseline `791c01ebde`.

F2–F7 are fixed in local [skip ci] commits; exact finding/commit mapping and F1's evidence-backed withdrawn disposition are in PUSH_READY.md. The F6 existing-profile return is fixed in `240c78da7`.

Validation: 13 focused suites pass; kernel clippy --all-targets passes with warnings; workspace fmt and diff checks pass. The real linked Codex import/local-removal/encrypted-replay drill passes, including preserved-login import replay after receiving-kernel restart. No credential was restored. No official logout, revocation or re-login was invoked.

Protocol: local 455 / relay peer 74 unchanged; no serialized contract changes and no number request needed.

HARD RULE: never official logout, revoke or re-login a shared login or its copy. Official logout/new-login/queued-resume acceptance is `BLOCKED(Miguel: disposable login)`. Claude live acceptance remains `BLOCKED(owner)` without a linked account. F1 was withdrawn by the security review; documentation and Cloud G9 follow-ups are recorded in PUSH_READY.md.

No push or GitHub CI. Ready for coordinator review; this lane stops after the final local validation commit.
