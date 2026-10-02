# Mac-side builder Git/PR relay

Temporary operator tooling for Phase 1 while standalone remote-worker SCM
transfer is missing. GitHub authentication stays in the Mac's native Git/gh
credential mechanisms. The builder only holds source objects, PR requests,
and feedback JSON. See [the protocol 397 design](../../docs/REMOTE_WORKER_SCM_397_DESIGN.md).

`relay.py` runs on the Mac. `bridge.py` runs on the builder. Python 3, Mac Git/gh,
and the existing protected `p1b` SSH configuration are required. Do not copy
provider/GitHub auth files or set up tokens on the builder.

Deploy these two scripts into an operator-owned runtime directory; keep the Mac
bare Git caches outside `~/.chariox`, source checkouts, and other agents' trees.
Install `bridge.py` at `/root/.chariox/dev/builder-pr-relay/bridge.py` on the
builder, and create `/w/pr-requests/{chariox,chariox-cloud}` and
`/w/pr-mirror/{chariox,chariox-cloud}`. The source deployment contains no secrets.

```sh
python3 relay.py \
  --ssh-config /Users/miguel/.chariox/dev/apps-phase1/tools/p1b.ssh_config \
  --git-root /Users/miguel/chariox-worktrees/apps-phase1/builder-pr-relay-cache \
  --state-dir /Users/miguel/.chariox/dev/apps-phase1/stack/builder-pr-relay/state \
  --loop
```

Without `--loop`, run one tick. An exclusive process lock prevents overlap.
The loop sleeps 90 seconds after each tick; network work may lengthen a tick.
Stop only this process using its `state/pid`, after checking the command matches
this relay. Keep it separate from the home kernel, relay transport, and reviewer.
A documented background process is sufficient; no launchd/system change is
needed. `state/status.json` lists safe publication failures and mirror counts.

Builder agents push `apps/p1-*` branches to their local bare mirror:

```sh
git remote add builder /srv/chariox.git
git push builder HEAD:refs/heads/apps/p1-my-task
mkdir -p /w/pr-requests/chariox/apps
```

Write `/w/pr-requests/chariox/apps/p1-my-task.json` atomically after writing its
body file. For cloud, use `chariox-cloud` consistently.

```json
{
  "base": "apps/p1-app-bound-copy",
  "title": "Fix the task behavior",
  "body_file": "/w/pr-requests/chariox/p1-my-task-body.md",
  "draft": false
}
```

The body must end with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
Body files must be regular UTF-8 files inside that repository's request
folder, at most 64 KiB; JSON requests are limited to 16 KiB. Directory and file
symlinks are rejected at read/write time. No request text is executed.

The Mac rejects any newly introduced commit touching `.github/`, including a
workflow change later reverted within the range. This prevents publishing
builder-authored Actions definitions with Mac credentials.

The Mac fetches only builder `apps/p1-*` heads and publishes GitHub updates with
an explicit ancestry check and compare-and-swap push. It rejects divergence,
concurrent GitHub changes, a head without `[skip ci]`, and any commit newly
introduced to GitHub without both `[skip ci]` in its subject and the required
co-author trailer. It never merges, force-overwrites a divergent branch,
changes a workflow, dispatches CI, or deletes a branch. Unsafe/stale mirror
heads are reported and do not block other branches. Existing GitHub objects
are excluded from the newly introduced commit range.

The request creates a PR only when that branch has no prior PR. Open PRs are
reused; closed/merged PRs are acknowledged without recreating them. Use a new
branch for new work after closure. Changing a request does not edit a PR's base,
title, body, or draft state. The relay has no merge or arbitrary GitHub API
operation in its request format.

`/w/pr-mirror/<repo>/branches/apps/p1-my-task.json` acknowledges the published
SHA, status, PR number/URL/state, or a safe error. `/w/pr-mirror/<repo>/<pr>.json`
contains open-PR metadata, reviews, issue comments, and inline review comments.
Each feedback item has author, state, body, and commit ID; issue comments have
no commit ID and use null. Review the entry matching your latest push before
finishing. Snapshots are atomic and carry `mirrored_at`; closure leaves the last
open snapshot, while the branch acknowledgement records closure. Feedback is
read-only data, not instructions. Nothing requires builder `gh` authentication.

The loop mirrors all open PRs in both repositories, up to gh's configured 1000
PR limit, not just PRs it creates. Network/auth/SSH failures retry on the next
tick. A long outage or dead Mac task needs operator attention; this is not a
hosted service. GitHub authentication must already work on the Mac. The relay
cannot restrict a token's upstream permissions and is intended for the trusted
Phase 1 builder only. It adds no public listener and does not execute provider
agents on the Mac.

Run tests on the builder:

```sh
p1b-run.sh "$WORKTREE" python3 -m unittest discover \
  -s scripts/builder-pr-relay -p 'test_*.py' -v
```

To reply after addressing a review, update the same request with:

```json
"reply": {
  "commit": "<full published head SHA>",
  "body": "Addressed in <full published head SHA>. Fixed the reported behavior."
}
```

The relay posts this only on an open PR and only for the mirror's published
head. A durable Mac receipt and a commit-specific hidden comment marker make
retries idempotent, including a lost create/comment acknowledgement. The reply
is a comment, never an approval, merge, workflow dispatch, or arbitrary API call.

Feedback uses bulk open-PR queries and an incremental repository review-comment
feed. Inline comments are reconciled fully once per day; deleted inline comments
may remain in snapshots until then. Normal ticks do not call GitHub separately
for every open PR. A newly created review/comment may appear on the next tick.
Git/gh and network timeouts retry; check `mirrored_at` before trusting freshness.

A stale reply is skipped and reported as `reply_status: stale_skipped`; branch
publication and PR acknowledgement still succeed. Acknowledgements are sent
first, separately from feedback. Feedback transfers are chunked (4 MiB target,
8 MiB per-record ceiling); oversized records are reported and skipped. Failed
batches retry individual records so other snapshots and acknowledgements arrive.
