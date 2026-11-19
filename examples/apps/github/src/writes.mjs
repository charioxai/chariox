// The tools that change something on GitHub, and the previews the person reads before the riskiest of them.
// Every refusal that GitHub would also make, and the ones it would not (a moved head, the default branch), is made
// here first, with the reason in words, so no card is shown for something that can never run.
import * as n from './normalize.mjs';
import { MERGE_METHODS } from './catalog.mjs';

const fail = (code, message, extra) => { throw { code, message, retryable: false, ...extra }; };
const splitRepo = repo => { const [owner, name] = repo.split('/'); return { owner, repo: name }; };
const short = sha => String(sha).slice(0, 7);
const METHOD_WORDS = { merge: 'Create a merge commit', squash: 'Squash and merge', rebase: 'Rebase and merge' };
const GRAPHQL = {
  ready: 'mutation($id: ID!) { markPullRequestReadyForReview(input: { pullRequestId: $id }) { pullRequest { isDraft } } }',
  draft: 'mutation($id: ID!) { convertPullRequestToDraft(input: { pullRequestId: $id }) { pullRequest { isDraft } } }',
  enable: 'mutation($id: ID!, $method: PullRequestMergeMethod!) { enablePullRequestAutoMerge(input: { pullRequestId: $id, mergeMethod: $method }) { pullRequest { autoMergeRequest { mergeMethod } } } }',
  disable: 'mutation($id: ID!) { disablePullRequestAutoMerge(input: { pullRequestId: $id }) { pullRequest { number } } }',
};
/** A completed run that ended without success; running or successful ones are not run again. */
const FAILED_RUN = ['failure', 'timed_out', 'cancelled', 'startup_failure'];

export function createWrites({ accounts, github, pulls, notify }) {
  const where = input => splitRepo(input.repo);
  const pullPath = '/repos/{owner}/{repo}/pulls/{number}';
  const params = input => ({ ...where(input), number: input.number });
  const fetchPull = (account, input) => github.get(account, pullPath, params(input)).then(({ data }) => data);

  /** Runs a change as the account, then tells open windows that this pull request (or repository) moved. */
  async function change(input, run, subject = { repo: input.repo, number: input.number ?? null }) {
    const account = await accounts.resolve(input);
    const result = await run(account);
    await notify({ kind: 'event', ...subject }).catch(() => {});
    return result;
  }

  const graphql = async (account, query, variables) => {
    const { data } = await github.send(account, 'POST', '/graphql', {}, { body: { query, variables } });
    if (data?.errors?.length) fail('UNPROCESSABLE', data.errors.map(error => error.message).join('; '));
    return data?.data;
  };

  function mustBeAt(pull, sha) {
    if (sha && sha !== pull.head.sha) {
      fail('HEAD_MOVED', `The branch moved after it was read: it is now at ${short(pull.head.sha)}, not ${short(sha)}. Read the pull request again before acting on it.`, { head: pull.head.sha });
    }
  }

  const openOnly = (pull, doing) => {
    if (pull.merged || pull.merged_at) fail('NOT_OPEN', `This pull request is already merged, so it cannot be ${doing}.`);
    if (pull.state === 'closed') fail('NOT_OPEN', `This pull request is closed, so it cannot be ${doing}.`);
  };

  /** A head branch that is the repository's default branch is never merged away or closed over. */
  function notTheDefaultBranch(pull) {
    if (pull.head.repo?.full_name === pull.base.repo.full_name && pull.head.ref === pull.base.repo.default_branch) {
      fail('DEFAULT_BRANCH', `The branch ${pull.head.ref} is the default branch of ${pull.base.repo.full_name}, so this pull request is not merged or closed from here.`);
    }
  }

  async function review(input, event, { needsBody = false } = {}) {
    const text = input.body?.trim() ?? '';
    const comments = input.comments ?? [];
    if (needsBody && !text) fail('INVALID_ARGUMENT', 'Asking for changes needs a message that says what to change.');
    if (event === 'COMMENT' && !text && !comments.length) fail('INVALID_ARGUMENT', 'A review needs a message or a comment on a line.');
    return change(input, async account => {
      const pull = await fetchPull(account, input);
      mustBeAt(pull, input.sha);
      if (event !== 'COMMENT') {
        const login = (await github.get(account, '/user', {})).data.login;
        if (pull.user?.login?.toLowerCase() === login.toLowerCase()) {
          fail('OWN_PULL_REQUEST', `GitHub does not let ${login} ${event === 'APPROVE' ? 'approve' : 'ask for changes on'} a pull request ${login} opened. Another account has to review it.`);
        }
      }
      const body = { event, commit_id: pull.head.sha };
      if (text) body.body = text;
      if (comments.length) body.comments = comments.map(({ path, line, side, start_line, start_side, body: comment }) => ({ path, line, side: side ?? 'RIGHT', ...(start_line ? { start_line, start_side: start_side ?? side ?? 'RIGHT' } : {}), body: comment }));
      const { data } = await github.send(account, 'POST', `${pullPath}/reviews`, params(input), { body });
      return { review: n.review(data) };
    });
  }

  async function changeLabels(input, remove) {
    return change(input, async account => {
      if (!remove) {
        const { data } = await github.send(account, 'POST', '/repos/{owner}/{repo}/issues/{number}/labels', params(input), { body: { labels: input.labels } });
        return { labels: data.map(l => ({ name: l.name, color: l.color, description: l.description ?? null })) };
      }
      let remaining = null;
      for (const name of input.labels) {
        try {
          ({ data: remaining } = await github.send(account, 'DELETE', '/repos/{owner}/{repo}/issues/{number}/labels/{name}', { ...params(input), name }));
        } catch (error) {
          if ((error?.payload ?? error)?.code !== 'NOT_FOUND') throw error;
        }
      }
      return { labels: (remaining ?? (await fetchPull(account, input)).labels).map(l => ({ name: l.name, color: l.color, description: l.description ?? null })) };
    });
  }

  async function changeReviewers(input, method) {
    if (!(input.reviewers?.length || input.team_reviewers?.length)) fail('INVALID_ARGUMENT', 'Name at least one person or team.');
    return change(input, async account => {
      const { data } = await github.send(account, method, '/repos/{owner}/{repo}/pulls/{number}/requested_reviewers', params(input),
        { body: { reviewers: input.reviewers ?? [], team_reviewers: input.team_reviewers ?? [] } });
      return { requestedReviewers: (data.requested_reviewers ?? []).map(u => ({ login: u.login })), requestedTeams: (data.requested_teams ?? []).map(t => ({ slug: t.slug, name: t.name })) };
    });
  }

  async function setState(input, state) {
    return change(input, async account => {
      const pull = await fetchPull(account, input);
      if (state === 'closed') { openOnly(pull, 'closed'); notTheDefaultBranch(pull); }
      else if (pull.merged || pull.merged_at) fail('NOT_CLOSED', 'This pull request was merged; a merged pull request cannot be reopened.');
      else if (pull.state === 'open') fail('NOT_CLOSED', 'This pull request is open already.');
      const { data } = await github.send(account, 'PATCH', pullPath, params(input), { body: { state } });
      return { pull: n.pullRow(data) };
    });
  }

  async function viaGraphql(input, query, extra, done) {
    return change(input, async account => {
      const pull = await fetchPull(account, input);
      openOnly(pull, 'changed');
      const result = await graphql(account, query, { id: pull.node_id, ...extra });
      return done(result, pull);
    });
  }

  /** A pull request the way a card reads it: one line each, within the kernel's limits. */
  const clip = (text, limit = 300) => (text.length > limit ? `${text.slice(0, limit - 1)}…` : text);
  const row = (label, value) => ({ label, value: clip(String(value)) });
  const pullRows = (pull, input) => [row('Repository', input.repo), row('Pull request', `#${pull.number} ${pull.title}`), row('Branches', `${pull.head.ref} into ${pull.base.ref}`)];

  async function mergeable(account, input) {
    const pull = await fetchPull(account, input);
    openOnly(pull, 'merged');
    if (pull.draft) fail('DRAFT', 'This pull request is a draft. Mark it ready for review first.');
    mustBeAt(pull, input.sha);
    notTheDefaultBranch(pull);
    return pull;
  }

  async function branchFacts(account, input) {
    const [{ data: repository }, { data: found }] = await Promise.all([
      github.get(account, '/repos/{owner}/{repo}', where(input)),
      github.get(account, '/repos/{owner}/{repo}/branches/{branch}', { ...where(input), branch: input.branch }),
    ]);
    if (repository.default_branch === input.branch) fail('DEFAULT_BRANCH', `${input.branch} is the default branch of ${input.repo}. It is never deleted from here.`);
    if (found.protected) fail('PROTECTED_BRANCH', `${input.branch} is a protected branch of ${input.repo}. It is never deleted from here.`);
    return { sha: found.commit.sha };
  }

  return {
    comment: input => change(input, async account => {
      const { data } = await github.send(account, 'POST', '/repos/{owner}/{repo}/issues/{number}/comments', params(input), { body: { body: input.body } });
      return { comment: n.issueComment(data) };
    }),

    reply_to_review_comment: input => change(input, async account => {
      const { data } = await github.send(account, 'POST', `${pullPath}/comments/{comment_id}/replies`, { ...params(input), comment_id: input.comment_id }, { body: { body: input.body } });
      return { comment: n.reviewComment(data) };
    }),

    submit_review: input => review(input, 'COMMENT'),
    approve_pull_request: input => review(input, 'APPROVE'),
    request_changes: input => review(input, 'REQUEST_CHANGES', { needsBody: true }),
    add_labels: input => changeLabels(input, false),
    remove_labels: input => changeLabels(input, true),
    request_reviewers: input => changeReviewers(input, 'POST'),
    remove_reviewers: input => changeReviewers(input, 'DELETE'),

    rerun_failed_checks: input => change(input, async account => {
      const pull = await fetchPull(account, input);
      const { data } = await github.get(account, '/repos/{owner}/{repo}/actions/runs', where(input), { query: { head_sha: pull.head.sha, per_page: 50 } });
      const failed = (data.workflow_runs ?? []).filter(run => run.status === 'completed' && FAILED_RUN.includes(run.conclusion));
      if (!failed.length) fail('NOTHING_TO_RERUN', 'No workflow run of the latest commit has failed, so there is nothing to run again.');
      for (const run of failed) await github.send(account, 'POST', '/repos/{owner}/{repo}/actions/runs/{run_id}/rerun-failed-jobs', { ...where(input), run_id: run.id });
      return { rerun: failed.map(run => ({ id: run.id, name: run.name })) };
    }),

    open_pull_request: input => change(input, async account => {
      const base = input.base ?? (await github.get(account, '/repos/{owner}/{repo}', where(input))).data.default_branch;
      if (input.head === base) fail('SAME_BRANCH', `The branch ${base} cannot be compared with itself. Choose another branch to open a pull request from.`);
      const body = { title: input.title, head: input.head, base, body: input.body ?? '', draft: Boolean(input.draft) };
      const { data } = await github.send(account, 'POST', '/repos/{owner}/{repo}/pulls', where(input), { body });
      return { pull: n.pullRow(data) };
    }, { repo: input.repo, number: null }),

    update_branch: input => change(input, async account => {
      const pull = await fetchPull(account, input);
      openOnly(pull, 'updated');
      mustBeAt(pull, input.sha);
      await github.send(account, 'PUT', `${pullPath}/update-branch`, params(input), { body: { expected_head_sha: pull.head.sha } });
      return { updating: true, head: pull.head.sha };
    }),

    mark_ready_for_review: input => viaGraphql(input, GRAPHQL.ready, {}, () => ({ draft: false })),
    convert_to_draft: input => viaGraphql(input, GRAPHQL.draft, {}, () => ({ draft: true })),
    enable_auto_merge: input => viaGraphql(input, GRAPHQL.enable, { method: input.method.toUpperCase() }, () => ({ autoMerge: { method: input.method } })),
    disable_auto_merge: input => viaGraphql(input, GRAPHQL.disable, {}, () => ({ autoMerge: null })),
    reopen_pull_request: input => setState(input, 'open'),
    close_pull_request: input => setState(input, 'closed'),

    merge_pull_request: input => change(input, async account => {
      if (!MERGE_METHODS.includes(input.method)) fail('INVALID_ARGUMENT', `Unknown merge method ${input.method}`);
      await mergeable(account, input);
      const body = { merge_method: input.method, sha: input.sha };
      if (input.commit_title) body.commit_title = input.commit_title;
      if (input.commit_message) body.commit_message = input.commit_message;
      const { data } = await github.send(account, 'PUT', `${pullPath}/merge`, params(input), { body });
      return { merged: data.merged, sha: data.sha, message: data.message };
    }),

    delete_branch: input => change(input, async account => {
      await branchFacts(account, input);
      await github.send(account, 'DELETE', '/repos/{owner}/{repo}/git/refs/heads/{branch}', { ...where(input), branch: input.branch });
      return { deleted: input.branch };
    }, { repo: input.repo, number: null }),

    async preview_merge_pull_request(input) {
      const account = await accounts.resolve(input);
      const pull = await mergeable(account, input);
      const summary = await pulls.get_pull_request_summary(input);
      const checks = summary.checks;
      const reviews = { approved: 'Approved', changes_requested: 'Changes requested', review_required: 'Review required', none: 'No reviews' }[summary.review];
      const state = pull.mergeable === false ? 'Has conflicts' : pull.mergeable === null ? 'Being checked by GitHub' : pull.mergeable_state === 'behind' ? 'Behind the base branch' : 'Can be merged';
      return {
        rows: [...pullRows(pull, input), row('Method', METHOD_WORDS[input.method]), row('Head commit', short(input.sha)),
          row('Checks', checks.state === 'none' ? 'None' : `${checks.state}: ${checks.failed} failing, ${checks.pending} pending of ${checks.total}`), row('Reviews', reviews), row('Merge state', state)],
      };
    },

    async preview_close_pull_request(input) {
      const account = await accounts.resolve(input);
      const pull = await fetchPull(account, input);
      openOnly(pull, 'closed');
      notTheDefaultBranch(pull);
      return { rows: [...pullRows(pull, input), row('Opened by', pull.user?.login ?? 'ghost'), row('State', pull.draft ? 'Draft' : 'Open')] };
    },

    async preview_delete_branch(input) {
      const account = await accounts.resolve(input);
      const { sha } = await branchFacts(account, input);
      const { data } = await github.get(account, '/repos/{owner}/{repo}/pulls', where(input), { query: { head: `${input.repo.split('/')[0]}:${input.branch}`, state: 'open', per_page: 5 } });
      const rows = [row('Repository', input.repo), row('Branch', input.branch), row('Last commit', short(sha))];
      if (data.length) rows.push(row('Open pull requests', data.map(pull => `#${pull.number} ${pull.title}`).join(', ')));
      return { rows };
    },
  };
}
