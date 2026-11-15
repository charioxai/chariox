# GitHub

`com.chariox.github`: the GitHub App. Its window looks and reads like GitHub (Primer CSS and Octicons, GitHub's own
words): the pull request list with Open and Closed counts, labels, check and review state; a pull request with
Conversation, Commits, Checks and Files changed (unified or split diff, comments under their lines). Every function is
also a tool any agent granted the App can call; the window and the tools are the same backend.

This is the read side. The write side (comment, review, merge ...) is the next lane; subscriptions and the live event
settings follow. The events-only App this one replaces lives on as `src/events.mjs`: the event fan-out, unchanged, which
still sets which automations a delivered event reaches (`get_automations`, `configure_automations`).

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

## The window

The window is a backend-served view: `view_actions`, `view_state` and `view_changes` (a revisioned change feed written with
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
header from the App and checks every call against the declared routes. The events fan-out component has its own
Wasmtime drill (`test/component-drill.rs`, entry `test/events-entry.ts`):

```sh
node packages/app-sdk-tools/build-js.mjs examples/apps/github/test/events-entry.ts "$TMPDIR/github-fanout.wasm"
CHARIOX_GITHUB_FANOUT_COMPONENT="$TMPDIR/github-fanout.wasm" \
  cargo test -p chariox-app-sdk-tools --features component-drills --test github-fanout-drill
```

Third-party notices: `THIRD-PARTY-NOTICES.md` (Primer CSS, Primer Primitives and Octicons, MIT).
