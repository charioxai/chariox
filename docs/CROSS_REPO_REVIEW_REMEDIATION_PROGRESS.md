# Cross-Repo Review Remediation — Progress Log

Companion to [`CROSS_REPO_REVIEW_REMEDIATION_PLAN.html`](CROSS_REPO_REVIEW_REMEDIATION_PLAN.html).
One line per landed task; keep this file short. Format: `date task — outcome (commit repo)`.

## Landed

- 2026-07-06 Plan + progress log committed (arroba).
- 2026-07-06 A3/A4/A5 relay auth hardening — constant-time shared-token compare, clock fail-closed, open-access opt-in (`f721ce1bd` arroba).
- 2026-07-06 A1/A2 cloud fail-closed startup — no silent dev secrets, prod guard on testAuth0IdentityHeader, `ARROBA_CLOUD_COOKIE_SECRET`/`ARROBA_CLOUD_CSRF_HMAC_KEY` env path (`118e6289` arroba-cloud).
- 2026-07-06 A3-TS timingSafeEqual in relay-tokens (`9751e8f3` arroba-cloud).
- 2026-07-06 A6 CI workflow for arroba-cloud (`834438b3` arroba-cloud). **Phase 0 complete.**

## In progress / notes for the next agent

- Next: Phase 1 (B1 lock instrumentation → B2 sleeps → B3 relay off-lock → B4 SQLite/fs → B5 ureq → B6 poisoning).
- arroba-cloud `apps/web` is red on main: `ui/routes.test.js` asserts on terminal-app source that is mid-refactor by a parallel agent (uncommitted work in `apps/web/src/terminal`). Not touched on purpose; CI will go green when that lands. Everything else (api, worker, packages) is green — verified in a clean worktree.
- arroba-cloud has parallel-agent activity under `apps/web/src/terminal` — stage only your own files, never `git add -A` there.
- Both repos commit directly to `main`; rebase before push if the remote advanced (use `--autostash` in arroba-cloud).
