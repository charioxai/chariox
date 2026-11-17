# GitHub

`com.chariox.github`: the GitHub App. Its window looks and reads like GitHub (Primer CSS and Octicons, GitHub's own
words): the pull request list with Open and Closed counts, labels, check and review state; a pull request with
Conversation, Commits, Checks and Files changed (unified or split diff, comments under their lines). Every function is
also a tool any agent granted the App can call; the window and the tools are the same backend.

It also writes: comment, review with comments on lines, approve or ask for changes, merge, close and reopen, update or
delete a branch, ready for review and back to draft, auto-merge, reviewers, labels, re-run failed checks and open a pull
request.

## Subscriptions

A subscription says which GitHub events reach whom: a scope (repositories, an organization or every repository the
account's installation can reach), the events (`pull_request.opened`, `check.completed`, `ci.finished`, `mention`, ...),
filters (branches, authors, labels, paths, draft, check conclusion; globs, `!` negates) and a target: the person
(a notification), an agent, a workflow or a Room's agent. A one-shot ends after its first delivery or on its expiry
(at most 7 days, 4 hours by default); `watch_pull_request` is the agent's shortcut for one pull request until it merges.
`ci.finished` fires when a commit has checks and none is pending, or after 15 minutes with none. An agent subscribes
itself without a card; any other target is asked for. Five failed deliveries in a row disable a subscription.

The App keeps the subscriptions and the last 200 deliveries (delivered, filtered with the reason, failed) in its state
and asks the kernel for one inbox route per (event type, repository) they need: `src/subscriptions/routes.mjs`
reconciles them after every change and daily. `src/subscriptions/matcher.mjs` is pure, so the polling sources of a later
step feed the same function. The window's Subscriptions tab and the tools (`create_subscription`, `update_subscription`,
`pause_subscription`, `delete_subscription`, `list_deliveries`, ...) are the same backend.

## Accounts

The App never holds a GitHub credential. The owner binds an account in the kernel: a name (the account label) for a Vault
entry made for `https://api.github.com` (`chariox.vault.copy_for_use` makes one from `github-push`), with a level
(`/app account grant INSTALLATION github github ENTRY --allow read`). The App calls GitHub as `account:<label>`; the kernel
checks the call against the signed route table in `app.json`, adds the token and removes it from the answer.
The App keeps only labels: which accounts it uses, the default, and which account a repository uses
(`add_account`, `set_default_account`, `set_repo_account`). `github` is the label it tries first.

REST calls send `If-None-Match` with the ETag of the last answer, so an unchanged answer is a 304 that costs no rate
limit, and the App stops short of the limit (100 core, 2 search requests left) with `RATE_LIMITED` and the time it resets.
`get_status` reads the limits GitHub counts now.

## Tools

Reads: `list_accounts`, `get_status`, `list_repositories`, `list_pull_requests` (scope `mine`, `review_requested`,
`repo`, `org`, `all`; state, draft, labels, author, review, text), `get_pull_request`, `get_pull_request_summary`,
`get_pull_request_files`, `get_pull_request_diff`, `list_commits`, `get_checks`, `list_reviews`, `list_comments`,
`get_mergeability`, `list_branches`, `get_compare`. Account settings (App state only): `add_account`, `remove_account`,
`set_default_account`, `set_repo_account`. `src/catalog.mjs` is the one list; `node scripts/generate.mjs` writes the
signed tool and UI action declarations and the manifest's routes from it (`--check` fails when they differ).

## Writing: tiers, cards and refusals

Each changing tool has a tier, declared in the signed UI actions (`effect`), so what asks the person is not up to the App:

| Tier | Tools | What happens |
|---|---|---|
| `write` | `comment`, `reply_to_review_comment`, `submit_review` (comments and a summary, no verdict), `add_labels`, `remove_labels`, `request_reviewers`, `remove_reviewers`, `rerun_failed_checks` | runs |
| `approval` | `approve_pull_request`, `request_changes`, `open_pull_request`, `mark_ready_for_review`, `convert_to_draft`, `update_branch`, `reopen_pull_request`, `enable_auto_merge`, `disable_auto_merge` | an agent asks through the kernel: allow once, for the session, or deny |
| `person_confirmed` | `merge_pull_request`, `close_pull_request`, `delete_branch` | an agent's call raises a card that shows the rows of `preview_merge_pull_request`, `preview_close_pull_request` or `preview_delete_branch` (the signed `confirmationView`) |

The person at the window acts for themselves, as on GitHub: merging has its own confirmation step, the other buttons run
at once. An agent calling the backend tool of a gated action directly is refused by the kernel (`USE_APP_VIEW_INVOKE`);
it asks through `chariox.app_view_invoke`. `approve_pull_request` and `request_changes` carry the standing class `review`,
the merge, close and branch actions their own (`merge`, `close`, `delete_branch`): the owner may let one repository skip
its card (`/app standing`).

What GitHub would also refuse, and what it would not, is refused here first, with the reason and no card: a merge for a head
that is no longer the commit that was shown (`HEAD_MOVED`), of a draft, a closed or a merged pull request, or of the
default branch; deleting the default branch or a protected branch; approving or asking for changes on a pull request the
account opened. A force push is not a tool at all. The account must be bound at the level of the most that a route does
(`write` for the writing tools, `destructive` for merge, close and delete; the install card binds the highest the App
declares). `merge_pull_request` always carries the head `sha` the person saw. The text of a job log lives on GitHub's
storage behind a redirect. The signed route for it names the one host suffix the kernel may follow it to
(`*.blob.core.windows.net`), so the check details show the output, the annotations, the steps and the last 200 lines of the job's log;
a log over 1 MiB is reported as unavailable.

## The window

The window is a backend-served view: `view_actions`, `view_state`, `view_account` (the account a call that names none runs as, which the kernel pins before it judges a standing permission) and `view_changes` (a revisioned change feed written with
the change itself) are served by the backend. The page is one self-contained script and stylesheet
(`view/`, built by `scripts/build-package.mjs`) that calls the tools through `window.chariox.call`. The page has no
network, so pictures of people are drawn as a disc with an initial, and links do not navigate.

## Build and check

```sh
pnpm --dir examples/apps/github --ignore-workspace install --frozen-lockfile
node --test examples/apps/github/test/*.test.mjs
node examples/apps/github/scripts/build-package.mjs "$OUT"     # $OUT outside the checkout; push it with /app dev $OUT
```

The tests run the real App code against recordings of the sandbox repository
(`charioxai/chariox-github-app-drill`, `test/fixtures`, URLs trimmed) behind a fake that refuses an `Authorization`
header from the App and checks every call against the declared routes.

Third-party notices: `THIRD-PARTY-NOTICES.md` (Primer CSS, Primer Primitives and Octicons, MIT).
