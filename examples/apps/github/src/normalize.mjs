// GitHub's answers reduced to what the window and agents use. Pure functions: no calls, no state.
const user = u => u ? { login: u.login, avatarUrl: u.avatar_url, type: u.type } : null;
const label = l => ({ name: l.name, color: l.color, description: l.description ?? null });

export const repoOf = item => text(item.repository_url).replace(/^.*\/repos\//, '');
const text = value => String(value ?? '');

/** What a pull request is, in GitHub's four states. */
export const pullState = ({ state, draft, merged, merged_at }) =>
  merged || merged_at ? 'merged' : state === 'closed' ? 'closed' : draft ? 'draft' : 'open';

export function searchRow(item) {
  return {
    repo: repoOf(item), number: item.number, title: item.title,
    state: pullState({ state: item.state, draft: item.draft, merged_at: item.pull_request?.merged_at }),
    author: user(item.user), labels: (item.labels ?? []).map(label), comments: item.comments,
    createdAt: item.created_at, updatedAt: item.updated_at, closedAt: item.closed_at,
    url: item.html_url,
  };
}

export function pullRow(pull) {
  return {
    repo: pull.base.repo.full_name, number: pull.number, title: pull.title, state: pullState(pull),
    author: user(pull.user), labels: (pull.labels ?? []).map(label), comments: (pull.comments ?? 0) + (pull.review_comments ?? 0),
    createdAt: pull.created_at, updatedAt: pull.updated_at, closedAt: pull.closed_at, url: pull.html_url,
  };
}

export function pullDetail(pull) {
  return {
    ...pullRow(pull),
    body: pull.body ?? '', merged: Boolean(pull.merged), mergedAt: pull.merged_at, mergedBy: user(pull.merged_by),
    base: { ref: pull.base.ref, sha: pull.base.sha, repo: pull.base.repo.full_name },
    head: { ref: pull.head.ref, sha: pull.head.sha, label: pull.head.label, repo: pull.head.repo?.full_name ?? null },
    assignees: (pull.assignees ?? []).map(user),
    requestedReviewers: (pull.requested_reviewers ?? []).map(user),
    requestedTeams: (pull.requested_teams ?? []).map(team => ({ slug: team.slug, name: team.name })),
    milestone: pull.milestone ? { title: pull.milestone.title } : null,
    additions: pull.additions, deletions: pull.deletions, changedFiles: pull.changed_files, commits: pull.commits,
    reviewComments: pull.review_comments, conversationComments: pull.comments,
    mergeable: pull.mergeable ?? null, mergeableState: pull.mergeable_state ?? 'unknown', rebaseable: pull.rebaseable ?? null,
    autoMerge: pull.auto_merge ? { method: pull.auto_merge.merge_method, enabledBy: user(pull.auto_merge.enabled_by) } : null,
    maintainerCanModify: Boolean(pull.maintainer_can_modify), authorAssociation: pull.author_association,
  };
}

export const file = f => ({
  path: f.filename, previousPath: f.previous_filename ?? null, status: f.status,
  additions: f.additions, deletions: f.deletions, changes: f.changes, patch: f.patch ?? null, blobUrl: f.blob_url,
});

export const commit = c => ({
  sha: c.sha, message: c.commit.message, author: user(c.author) ?? { login: c.commit.author?.name ?? null, avatarUrl: null },
  authoredAt: c.commit.author?.date ?? null, url: c.html_url,
});

export const review = r => ({
  id: r.id, user: user(r.user), state: r.state, body: r.body ?? '', submittedAt: r.submitted_at ?? null,
  commitId: r.commit_id, authorAssociation: r.author_association,
});

export const issueComment = c => ({
  id: c.id, user: user(c.user), body: c.body ?? '', createdAt: c.created_at, updatedAt: c.updated_at, authorAssociation: c.author_association,
});

export const reviewComment = c => ({
  id: c.id, user: user(c.user), body: c.body ?? '', path: c.path, line: c.line ?? c.original_line ?? null, side: c.side ?? 'RIGHT',
  diffHunk: c.diff_hunk, inReplyTo: c.in_reply_to_id ?? null, reviewId: c.pull_request_review_id ?? null,
  createdAt: c.created_at, updatedAt: c.updated_at, outdated: c.line === null || c.line === undefined,
});

/** The state of every check on a commit, read as GitHub's merge box reads it. */
export function checksSummary(runs, statuses) {
  const failed = runs.filter(r => r.status === 'completed' && ['failure', 'timed_out', 'startup_failure', 'action_required', 'stale'].includes(r.conclusion)).length
    + statuses.filter(s => s.state === 'failure' || s.state === 'error').length;
  const pending = runs.filter(r => r.status !== 'completed').length + statuses.filter(s => s.state === 'pending').length;
  const total = runs.length + statuses.length;
  return { state: total === 0 ? 'none' : failed ? 'failure' : pending ? 'pending' : 'success', total, failed, pending };
}

export const checkRun = r => ({
  id: r.id, name: r.name, status: r.status, conclusion: r.conclusion ?? null,
  startedAt: r.started_at ?? null, completedAt: r.completed_at ?? null,
  app: r.app?.name ?? null, title: r.output?.title ?? null, summary: r.output?.summary?.slice(0, 500) ?? null, url: r.details_url ?? r.html_url,
});

export const commitStatus = s => ({ context: s.context, state: s.state, description: s.description ?? null, url: s.target_url ?? null });

/** What the reviews add up to: each reviewer's latest review that says something, comments and dismissed ones aside. */
export function reviewDecision(reviews, requested) {
  const latest = new Map();
  for (const r of reviews) {
    if (r.state === 'COMMENTED' || r.state === 'PENDING' || !r.user) continue;
    latest.set(r.user.login, r.state);
  }
  const states = [...latest.values()];
  if (states.includes('CHANGES_REQUESTED')) return 'changes_requested';
  if (states.includes('APPROVED')) return 'approved';
  return requested > 0 ? 'review_required' : 'none';
}
