# Cross-Repo Review Remediation — Progress Log

Companion to [`CROSS_REPO_REVIEW_REMEDIATION_PLAN.html`](CROSS_REPO_REVIEW_REMEDIATION_PLAN.html).
One line per landed task; keep this file short. Format: `date task — outcome (commit repo)`.

## Landed

- 2026-07-06 Plan + progress log committed (arroba).

## In progress / notes for the next agent

- Phase 0 order: A3/A4/A5 (arroba relay auth) → A1/A2 (cloud startup) → A3-TS (relay-tokens) → A6 (cloud CI) → Phase 1 B1.
- arroba-cloud has parallel-agent activity under `apps/web/src/terminal` — stage only your own files, never `git add -A` there.
- Both repos commit directly to `main`; rebase before push if the remote advanced.
