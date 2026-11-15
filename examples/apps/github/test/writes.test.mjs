import assert from 'node:assert/strict';
import test from 'node:test';
import { createApp } from '../src/app.mjs';
import { fakeGitHub, fixture, heads, memoryState, REPO } from './support.mjs';

const NOW = Date.parse('2026-11-15T12:00:00Z');
const json = (status, body, headers = {}) => ({ status, headers, body: JSON.stringify(body) });
const pull = { repo: REPO, number: 2 };
const head = heads[2];
const recorded = name => JSON.parse(fixture(name).body);

/**
 * The App with a loopback GitHub: reads answer from the recordings, a change answers as `answers` says (by
 * "METHOD /path-after-the-repository"), and every request is kept for the assertions.
 */
function scenario(answers = {}, { login } = {}) {
  const state = memoryState();
  const double = fakeGitHub({
    override: (url, init) => {
      const key = `${init.method} ${url.pathname.replace(`/repos/${REPO}`, '')}`;
      if (login && key === 'GET /user') return json(200, { login });
      const answer = answers[key];
      if (typeof answer === 'function') return answer(init.body === undefined ? undefined : JSON.parse(init.body), url);
      if (answer) return answer;
      if (init.method !== 'GET') assert.fail(`unexpected change ${key}`);
      return undefined;
    },
  });
  const definition = createApp({ state, events: { occurrenceId: () => 'x', emit: async () => ({}) }, now: () => NOW });
  const ctx = { fetch: double.fetch, signal: new AbortController().signal };
  const call = (name, input = {}) => definition.tools[name](input, ctx);
  const changes = () => double.calls.filter(c => c.method !== 'GET');
  return { call, calls: double.calls, changes, state };
}
const withPull = (number, edit) => ({ [`GET /pulls/${number}`]: () => { const body = recorded(`pull_${number}`); edit(body); return json(200, body); } });
const refused = async (promise, code, message) => assert.rejects(promise, error => { assert.equal((error?.payload ?? error).code, code); if (message) assert.match((error?.payload ?? error).message, message); return true; });

test('a comment is one POST with the text, as the account, and tells open windows the pull request changed', async () => {
  const { call, changes } = scenario({ 'POST /issues/2/comments': body => json(201, { id: 77, user: { login: 'mgutierrez09' }, body: body.body, created_at: '2026-11-15T12:00:00Z', updated_at: '2026-11-15T12:00:00Z', author_association: 'OWNER' }) });
  const { comment } = await call('comment', { ...pull, body: 'Thanks, merging after CI.' });
  assert.deepEqual([comment.id, comment.body], [77, 'Thanks, merging after CI.']);
  assert.deepEqual(changes().map(c => [c.method, c.path, c.connectionId, c.body]), [['POST', `/repos/${REPO}/issues/2/comments`, 'account:github', { body: 'Thanks, merging after CI.' }]]);
  assert.deepEqual((await call('view_changes', { since: 0 })).changes.map(c => c.delta), [{ kind: 'event', repo: REPO, number: 2 }]);
});

test('a reply goes to the comment it answers', async () => {
  const { call, changes } = scenario({ 'POST /pulls/2/comments/55/replies': body => json(201, { id: 56, user: { login: 'mgutierrez09' }, body: body.body, path: 'src/hello.js', line: 2, side: 'RIGHT', in_reply_to_id: 55, created_at: '2026-11-15T12:00:00Z', updated_at: '2026-11-15T12:00:00Z' }) });
  const { comment } = await call('reply_to_review_comment', { ...pull, comment_id: 55, body: 'Done.' });
  assert.deepEqual([comment.inReplyTo, comment.path], [55, 'src/hello.js']);
  assert.deepEqual(changes()[0].body, { body: 'Done.' });
});

test('a review with comments on lines is one review of the commit that was read, comments on the new side by default', async () => {
  const { call, changes } = scenario({ 'POST /pulls/2/reviews': body => json(200, { id: 9, user: { login: 'mgutierrez09' }, state: 'COMMENTED', body: body.body, submitted_at: '2026-11-15T12:00:00Z', commit_id: body.commit_id, author_association: 'OWNER' }) });
  const { review } = await call('submit_review', { ...pull, body: 'Two notes.', comments: [{ path: 'src/hello.js', line: 2, body: 'Use a default.' }, { path: 'src/hello.js', line: 6, start_line: 4, side: 'LEFT', body: 'Old block.' }] });
  assert.equal(review.state, 'COMMENTED');
  assert.deepEqual(changes()[0].body, {
    event: 'COMMENT', commit_id: head, body: 'Two notes.',
    comments: [{ path: 'src/hello.js', line: 2, side: 'RIGHT', body: 'Use a default.' }, { path: 'src/hello.js', line: 6, side: 'LEFT', start_line: 4, start_side: 'LEFT', body: 'Old block.' }],
  });
  await refused(call('submit_review', { ...pull }), 'INVALID_ARGUMENT', /message or a comment/);
  assert.equal(changes().length, 1, 'an empty review is never sent');
});

test('a review of a head that moved is refused before GitHub is asked', async () => {
  const { call, changes } = scenario();
  await refused(call('submit_review', { ...pull, body: 'x', sha: heads[1] }), 'HEAD_MOVED', /moved/);
  assert.equal(changes().length, 0);
});

test('approving or asking for changes on a pull request the account opened is refused with the reason; another account may', async () => {
  const own = scenario();
  await refused(own.call('approve_pull_request', pull), 'OWN_PULL_REQUEST', /Another account has to review/);
  await refused(own.call('request_changes', { ...pull, body: 'Please fix' }), 'OWN_PULL_REQUEST');
  assert.equal(own.changes().length, 0, 'nothing was sent to GitHub');
  const other = scenario({ 'POST /pulls/2/reviews': body => json(200, { id: 10, user: { login: 'reviewer' }, state: body.event === 'APPROVE' ? 'APPROVED' : 'CHANGES_REQUESTED', body: body.body ?? '', submitted_at: '2026-11-15T12:00:00Z', commit_id: body.commit_id }) }, { login: 'reviewer' });
  assert.equal((await other.call('approve_pull_request', pull)).review.state, 'APPROVED');
  assert.equal((await other.call('request_changes', { ...pull, body: 'Rename it', comments: [{ path: 'README.md', line: 1, body: 'here' }] })).review.state, 'CHANGES_REQUESTED');
  assert.deepEqual(other.changes().map(c => c.body.event), ['APPROVE', 'REQUEST_CHANGES']);
  assert.equal(other.changes()[1].body.comments[0].side, 'RIGHT');
  await refused(other.call('request_changes', { ...pull, body: '  ' }), 'INVALID_ARGUMENT', /what to change/);
});

test('labels are added together and removed one by one; a label that is not on the pull request is not an error', async () => {
  const labels = [{ name: 'bug', color: 'd73a4a', description: null }];
  const { call, changes } = scenario({
    'POST /issues/2/labels': json(200, labels),
    'DELETE /issues/2/labels/bug': json(200, []),
    'DELETE /issues/2/labels/gone': json(404, { message: 'Label does not exist' }),
  });
  assert.deepEqual((await call('add_labels', { ...pull, labels: ['bug'] })).labels.map(l => l.name), ['bug']);
  assert.deepEqual(changes()[0].body, { labels: ['bug'] });
  assert.deepEqual((await call('remove_labels', { ...pull, labels: ['bug', 'gone'] })).labels, []);
  assert.deepEqual(changes().slice(1).map(c => [c.method, c.path]), [['DELETE', `/repos/${REPO}/issues/2/labels/bug`], ['DELETE', `/repos/${REPO}/issues/2/labels/gone`]]);
});

test('reviewers are requested and withdrawn with a body, and naming nobody is refused', async () => {
  const answer = json(201, { requested_reviewers: [{ login: 'octo' }], requested_teams: [{ slug: 'core', name: 'Core' }] });
  const { call, changes } = scenario({ 'POST /pulls/2/requested_reviewers': answer, 'DELETE /pulls/2/requested_reviewers': answer });
  const result = await call('request_reviewers', { ...pull, reviewers: ['octo'], team_reviewers: ['core'] });
  assert.deepEqual([result.requestedReviewers, result.requestedTeams], [[{ login: 'octo' }], [{ slug: 'core', name: 'Core' }]]);
  await call('remove_reviewers', { ...pull, reviewers: ['octo'] });
  assert.deepEqual(changes().map(c => [c.method, c.body]), [['POST', { reviewers: ['octo'], team_reviewers: ['core'] }], ['DELETE', { reviewers: ['octo'], team_reviewers: [] }]]);
  await refused(call('request_reviewers', { ...pull }), 'INVALID_ARGUMENT', /at least one/);
});

test('failed checks run again: each failed workflow run of the latest commit, none when nothing failed', async () => {
  const { call, changes, calls } = scenario({
    'POST /actions/runs/43680658254/rerun-failed-jobs': json(201, {}), 'POST /actions/runs/43680655184/rerun-failed-jobs': json(201, {}),
  });
  const { rerun } = await call('rerun_failed_checks', pull);
  assert.deepEqual(rerun.map(r => r.id), [43680658254, 43680655184]);
  assert.equal(calls.find(c => c.path.endsWith('/actions/runs')).query.get('head_sha'), head);
  assert.equal(changes().length, 2);
  const healthy = scenario({ 'GET /actions/runs': json(200, { workflow_runs: [{ id: 1, name: 'ci', status: 'completed', conclusion: 'success' }, { id: 2, name: 'ci', status: 'in_progress', conclusion: null }] }) });
  await refused(healthy.call('rerun_failed_checks', pull), 'NOTHING_TO_RERUN');
  assert.equal(healthy.changes().length, 0);
});

test('a pull request opens against the default branch unless a base is named, never from the base itself', async () => {
  const created = body => json(201, { ...recorded('pull_2'), title: body.title, draft: body.draft, number: 12 });
  const { call, changes } = scenario({ 'POST /pulls': created });
  const { pull: opened } = await call('open_pull_request', { repo: REPO, title: 'New thing', head: 'drill/gha4-rich', body: 'Why', draft: true });
  assert.deepEqual([opened.number, opened.state], [12, 'draft']);
  assert.deepEqual(changes()[0].body, { title: 'New thing', head: 'drill/gha4-rich', base: 'main', body: 'Why', draft: true });
  await call('open_pull_request', { repo: REPO, title: 'Again', head: 'feature/x', base: 'develop' });
  assert.deepEqual([changes()[1].body.base, changes()[1].body.draft], ['develop', false]);
  await refused(call('open_pull_request', { repo: REPO, title: 'Same', head: 'main' }), 'SAME_BRANCH', /itself/);
  assert.equal(changes().length, 2);
});

test('updating the branch names the head that was read; a pull request that is not open is refused', async () => {
  const { call, changes } = scenario({ 'PUT /pulls/2/update-branch': json(202, { message: 'Updating pull request branch.' }) });
  assert.deepEqual(await call('update_branch', { ...pull, sha: head }), { updating: true, head });
  assert.deepEqual(changes()[0].body, { expected_head_sha: head });
  await refused(call('update_branch', { ...pull, sha: heads[1] }), 'HEAD_MOVED');
  await refused(call('update_branch', { ...pull, number: 5 }), 'NOT_OPEN', /merged/);
  await refused(call('update_branch', { ...pull, number: 4 }), 'NOT_OPEN', /closed/);
  assert.equal(changes().length, 1);
});

test('ready for review, back to draft and auto-merge are GraphQL mutations of the pull request id', async () => {
  const seen = [];
  const double = fakeGitHub({ override: (url, init) => {
    if (url.pathname !== '/graphql') return undefined;
    seen.push(JSON.parse(init.body));
    return json(200, { data: { ok: true } });
  } });
  const definition = createApp({ state: memoryState(), events: {}, now: () => NOW });
  const run = (name, input) => definition.tools[name](input, { fetch: double.fetch });
  const draft = { repo: REPO, number: 3 };
  assert.deepEqual(await run('mark_ready_for_review', draft), { draft: false });
  assert.match(seen[0].query, /markPullRequestReadyForReview/);
  assert.equal(seen[0].variables.id, recorded('pull_3').node_id);
  assert.deepEqual(await run('convert_to_draft', pull), { draft: true });
  assert.match(seen[1].query, /convertPullRequestToDraft/);
  assert.deepEqual(await run('enable_auto_merge', { ...pull, method: 'squash' }), { autoMerge: { method: 'squash' } });
  assert.equal(seen[2].variables.method, 'SQUASH');
  assert.deepEqual(await run('disable_auto_merge', pull), { autoMerge: null });
  assert.match(seen[3].query, /disablePullRequestAutoMerge/);
  await assert.rejects(run('enable_auto_merge', { ...pull, number: 5, method: 'merge' }), { code: 'NOT_OPEN' });
  assert.equal(seen.length, 4);
});

test('GraphQL errors come back as a refusal with GitHub\'s words', async () => {
  const double = fakeGitHub({ override: url => url.pathname === '/graphql' ? json(200, { errors: [{ message: 'Pull request Auto merge is not allowed for this repository' }] }) : undefined });
  const definition = createApp({ state: memoryState(), events: {}, now: () => NOW });
  await assert.rejects(definition.tools.enable_auto_merge({ ...pull, method: 'merge' }, { fetch: double.fetch }), { code: 'UNPROCESSABLE', message: /not allowed for this repository/ });
});

test('closing and reopening: only the right state, never a merged pull request', async () => {
  const { call, changes } = scenario({ ...withPull(4, () => {}), 'PATCH /pulls/2': body => json(200, { ...recorded('pull_2'), state: body.state }), 'PATCH /pulls/4': body => json(200, { ...recorded('pull_4'), state: body.state }) });
  const closed = await call('close_pull_request', pull);
  assert.equal(closed.pull.state, 'closed');
  assert.deepEqual(changes()[0].body, { state: 'closed' });
  const reopened = await call('reopen_pull_request', { ...pull, number: 4 });
  assert.equal(reopened.pull.state, 'open');
  assert.deepEqual(changes()[1].body, { state: 'open' });
  await refused(call('reopen_pull_request', pull), 'NOT_CLOSED', /open already/);
  await refused(call('reopen_pull_request', { ...pull, number: 5 }), 'NOT_CLOSED', /merged/);
  await refused(call('close_pull_request', { ...pull, number: 4 }), 'NOT_OPEN', /closed/);
  await refused(call('close_pull_request', { ...pull, number: 5 }), 'NOT_OPEN', /merged/);
  assert.equal(changes().length, 2);
});

test('a merge goes through only for the head that was read, a pull request that is open and not a draft, with the method', async () => {
  const { call, changes } = scenario({ 'PUT /pulls/2/merge': body => json(200, { merged: true, sha: 'cafe0000cafe0000cafe0000cafe0000cafe0000', message: `Pull Request successfully merged (${body.merge_method})` }) });
  const merged = await call('merge_pull_request', { ...pull, method: 'squash', sha: head, commit_title: 'Add hello (#2)' });
  assert.deepEqual([merged.merged, merged.sha.slice(0, 4)], [true, 'cafe']);
  assert.deepEqual(changes()[0].body, { merge_method: 'squash', sha: head, commit_title: 'Add hello (#2)' });
  assert.equal(changes()[0].path, `/repos/${REPO}/pulls/2/merge`);
  // a stale sha, a draft, a merged and a closed pull request never reach GitHub
  await refused(call('merge_pull_request', { ...pull, method: 'merge', sha: heads[1] }), 'HEAD_MOVED', /Read the pull request again/);
  await refused(call('merge_pull_request', { repo: REPO, number: 3, method: 'merge', sha: heads[3] }), 'DRAFT', /ready for review/);
  await refused(call('merge_pull_request', { repo: REPO, number: 5, method: 'merge', sha: heads[5] }), 'NOT_OPEN', /already merged/);
  await refused(call('merge_pull_request', { repo: REPO, number: 4, method: 'merge', sha: heads[4] }), 'NOT_OPEN', /closed/);
  assert.equal(changes().length, 1);
  for (const method of ['merge', 'rebase']) await call('merge_pull_request', { ...pull, method, sha: head });
  assert.deepEqual(changes().map(c => c.body.merge_method), ['squash', 'merge', 'rebase']);
});

test('GitHub\'s own refusal of a merge is shown in its words with a code the window and agents can act on', async () => {
  const refuse = (status, message) => scenario({ 'PUT /pulls/2/merge': json(status, { message }) });
  await refused(refuse(405, 'Required status check "ci" is expected.').call('merge_pull_request', { ...pull, method: 'merge', sha: head }), 'NOT_ALLOWED', /Required status check/);
  await refused(refuse(409, 'Head branch was modified. Review and try the merge again.').call('merge_pull_request', { ...pull, method: 'merge', sha: head }), 'CONFLICT', /Head branch was modified/);
  await refused(refuse(404, 'Not Found').call('merge_pull_request', { ...pull, method: 'merge', sha: head }), 'NOT_FOUND');
  const validation = scenario({ 'POST /pulls': json(422, { message: 'Validation Failed', errors: [{ message: 'A pull request already exists for charioxai:drill/gha4-rich.' }] }) });
  await refused(validation.call('open_pull_request', { repo: REPO, title: 't', head: 'drill/gha4-rich' }), 'UNPROCESSABLE', /Validation Failed: A pull request already exists/);
  await refused(scenario({ 'POST /issues/2/comments': json(401, { message: 'Bad credentials' }) }).call('comment', { ...pull, body: 'x' }), 'ACCOUNT_UNAUTHORIZED');
  const limited = scenario({ 'POST /issues/2/comments': json(403, { message: 'You have exceeded a secondary rate limit.' }, { 'retry-after': '60' }) });
  await refused(limited.call('comment', { ...pull, body: 'x' }), 'RATE_LIMITED');
});

test('a merge of the default branch into itself is never run, and neither is closing it', async () => {
  const onMain = withPull(2, body => { body.head.ref = 'main'; body.head.repo.full_name = body.base.repo.full_name; });
  const { call, changes } = scenario(onMain);
  await refused(call('merge_pull_request', { ...pull, method: 'merge', sha: head }), 'DEFAULT_BRANCH', /default branch/);
  await refused(call('close_pull_request', pull), 'DEFAULT_BRANCH');
  assert.equal(changes().length, 0);
});

test('a branch is deleted unless it is the default branch or protected; a branch with a slash is one path segment', async () => {
  const { call, changes } = scenario({ 'DELETE /git/refs/heads/drill%2Fgha4-rich': json(204, null) });
  assert.deepEqual(await call('delete_branch', { repo: REPO, branch: 'drill/gha4-rich' }), { deleted: 'drill/gha4-rich' });
  assert.equal(changes()[0].path, `/repos/${REPO}/git/refs/heads/drill%2Fgha4-rich`);
  await refused(call('delete_branch', { repo: REPO, branch: 'main' }), 'DEFAULT_BRANCH', /never deleted/);
  const protectedBranch = scenario({ 'GET /branches/release': () => json(200, { ...recorded('branch_main'), name: 'release', protected: true }) });
  await refused(protectedBranch.call('delete_branch', { repo: REPO, branch: 'release' }), 'PROTECTED_BRANCH', /protected branch/);
  assert.equal(protectedBranch.changes().length, 0);
  assert.equal(changes().length, 1);
});

test('the preview of a merge: what the person sees fits the card, and what can never run gets no card', async () => {
  const { call, changes } = scenario();
  const { rows } = await call('preview_merge_pull_request', { ...pull, method: 'squash', sha: head });
  assert.ok(rows.length >= 1 && rows.length <= 12);
  for (const { label, value } of rows) { assert.ok(label.length <= 48 && label.length > 0); assert.ok(value.length <= 400 && value.length > 0); assert.deepEqual(Object.keys({ label, value }), ['label', 'value']); }
  const text = Object.fromEntries(rows.map(r => [r.label, r.value]));
  assert.equal(text.Repository, REPO);
  assert.match(text['Pull request'], /^#2 Add hello, notes and a large data file/);
  assert.equal(text.Method, 'Squash and merge');
  assert.equal(text['Head commit'], head.slice(0, 7));
  assert.match(text.Checks, /failure: 2 failing/);
  assert.equal(text.Branches, 'drill/gha4-rich into main');
  assert.equal(changes().length, 0, 'a preview changes nothing');
  await refused(call('preview_merge_pull_request', { ...pull, method: 'merge', sha: heads[1] }), 'HEAD_MOVED');
  await refused(call('preview_merge_pull_request', { repo: REPO, number: 3, method: 'merge', sha: heads[3] }), 'DRAFT');
  await refused(call('preview_merge_pull_request', { repo: REPO, number: 5, method: 'merge', sha: heads[5] }), 'NOT_OPEN');
});

test('the previews of closing and deleting say what they close and what is lost, and refuse what is never done', async () => {
  const { call, changes } = scenario();
  const close = await call('preview_close_pull_request', pull);
  assert.deepEqual(close.rows.map(r => r.label), ['Repository', 'Pull request', 'Branches', 'Opened by', 'State']);
  const del = await call('preview_delete_branch', { repo: REPO, branch: 'drill/gha4-rich' });
  assert.deepEqual(del.rows.map(r => r.label), ['Repository', 'Branch', 'Last commit', 'Open pull requests']);
  assert.match(del.rows.at(-1).value, /^#2 /);
  await refused(call('preview_delete_branch', { repo: REPO, branch: 'main' }), 'DEFAULT_BRANCH');
  await refused(call('preview_close_pull_request', { ...pull, number: 5 }), 'NOT_OPEN');
  assert.equal(changes().length, 0);
});

test('the pickers: the labels of a repository and the people who can review', async () => {
  const { call } = scenario();
  assert.ok((await call('list_labels', { repo: REPO })).labels.some(l => l.name === 'bug'));
  assert.ok((await call('list_assignees', { repo: REPO })).people.some(p => p.login === 'mgutierrez09'));
});

test('a changing call is refused while the rate limit is used up, and the account is the one named', async () => {
  const { call, state } = scenario({ 'POST /issues/2/comments': json(201, { id: 1, user: { login: 'o' }, body: 'x', created_at: '2026-11-15T12:00:00Z' }) });
  await call('comment', { ...pull, account: 'github', body: 'x' });
  await state.transact({ schemaVersion: 0, checks: [{ key: 'rate:github:core', version: null }], writes: [{ tag: 'put', val: ['rate:github:core', JSON.stringify({ limit: 5000, remaining: 3, reset: NOW / 1000 + 600, at: NOW })] }], occurrences: [], wakes: [] });
  await refused(call('comment', { ...pull, body: 'y' }), 'RATE_LIMITED');
});
