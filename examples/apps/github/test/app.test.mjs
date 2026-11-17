import assert from 'node:assert/strict';
import test from 'node:test';
import { createApp } from '../src/app.mjs';
import { tools as catalog } from '../src/catalog.mjs';
import { searchQuery } from '../src/pulls.mjs';
import { FAILING_CHECK, fakeGitHub, fixture, heads, memoryState, REPO } from './support.mjs';

const NOW = Date.parse('2026-11-15T12:00:00Z');
function app(options = {}, state = memoryState()) {
  const double = fakeGitHub(options);
  const events = { occurrenceId: () => 'x', emit: async () => ({}) };
  const definition = createApp({ state, events, now: () => NOW });
  const ctx = { fetch: double.fetch, signal: new AbortController().signal };
  const call = (name, input = {}) => definition.tools[name](input, ctx);
  return { ...double, state, call, definition };
}
const pull = { repo: REPO, number: 2 };

test('mine, review requested, repo, org and all each become one GitHub search', () => {
  assert.equal(searchQuery({ scope: 'mine' }), 'is:pr author:@me is:open sort:updated-desc');
  assert.equal(searchQuery({ scope: 'review_requested', state: 'all' }), 'is:pr review-requested:@me sort:updated-desc');
  assert.equal(searchQuery({ scope: 'repo', repos: ['o/a', 'o/b'], state: 'merged', draft: 'hide', labels: ['bug', 'good first issue'], author: 'octo', review: 'approved', query: 'hello world' }),
    'is:pr repo:o/a repo:o/b is:merged draft:false label:"bug" label:"good first issue" author:octo review:approved hello world sort:updated-desc');
  assert.equal(searchQuery({ scope: 'org', org: 'acme', draft: 'only' }), 'is:pr org:acme is:open draft:true sort:updated-desc');
  assert.equal(searchQuery({ scope: 'all' }), 'is:pr involves:@me is:open sort:updated-desc');
  assert.throws(() => searchQuery({ scope: 'repo' }), { code: 'INVALID_ARGUMENT' });
  assert.throws(() => searchQuery({ scope: 'org' }), { code: 'INVALID_ARGUMENT' });
});

test('the list shows GitHub\'s four states, labels and the author; the page says whether more follow', async () => {
  const { call, calls } = app();
  const open = await call('list_pull_requests', { scope: 'repo', repos: [REPO], state: 'all' });
  assert.deepEqual(open.pulls.map(p => [p.number, p.state]), [[3, 'draft'], [2, 'open'], [1, 'open']]);
  assert.deepEqual(open.pulls[1].labels.map(l => l.name), ['bug', 'documentation']);
  assert.equal(open.pulls[1].author.login, 'mgutierrez09');
  assert.equal(open.pulls[1].comments, 1);
  assert.equal(open.hasNext, false);
  assert.equal(calls[0].path, '/search/issues');
  assert.equal(calls[0].connectionId, 'account:github');
  const merged = await call('list_pull_requests', { scope: 'repo', repos: [REPO], state: 'merged' });
  assert.deepEqual(merged.pulls.map(p => [p.number, p.state]), [[5, 'merged']]);
  assert.deepEqual((await call('list_pull_requests', { scope: 'review_requested' })).pulls, []);
});

test('a pull request in detail: branches, size, reviewers and a merge state that GitHub is still computing', async () => {
  const { pull: detail } = await app().call('get_pull_request', pull);
  assert.equal(detail.title, 'Add hello, notes and a large data file');
  assert.deepEqual([detail.base.ref, detail.head.ref, detail.head.sha], ['main', 'drill/gha4-rich', heads[2]]);
  assert.deepEqual([detail.additions, detail.deletions, detail.changedFiles], [42015, 0, 4]);
  assert.equal(detail.mergeable, null);
  assert.equal(detail.state, 'open');
  assert.ok(detail.body.startsWith('Exercises the read tools'));
});

test('files carry their patches, and a file GitHub did not give a patch for is marked by having none', async () => {
  const { files, hasNext } = await app().call('get_pull_request_files', pull);
  assert.equal(hasNext, false);
  assert.deepEqual(files.map(f => [f.path, f.status, f.patch !== null]), [['README.md', 'modified', true], ['data/big.txt', 'added', false], ['docs/notes.md', 'added', true], ['src/hello.js', 'added', true]]);
  assert.equal(files[1].changes, 42000);
});

test('the diff of one file is cut out of the whole; a larger diff is truncated at a line; an oversized one says so', async () => {
  const { call } = app();
  const whole = await call('get_pull_request_diff', { ...pull });
  assert.match(whole.diff, /^diff --git /);
  assert.equal(whole.truncated, false);
  const one = await call('get_pull_request_diff', { ...pull, number: 3, path: 'src/draft.js' });
  assert.match(one.diff, /^diff --git a\/src\/draft.js b\/src\/draft.js/);
  // a file's patch comes from the file list, so another file's size cannot make it too large to show
  const patch = await call('get_pull_request_diff', { ...pull, path: 'src/hello.js' });
  assert.match(patch.diff, /^diff --git a\/src\/hello.js b\/src\/hello.js\n@@ -0,0 \+1,6 @@/);
  assert.match(patch.diff, /\+export const answer = 42;/);
  assert.equal((await call('get_pull_request_diff', { ...pull, number: 3, path: 'nothing.js' })).diff, '');
  const small = await call('get_pull_request_diff', { ...pull, number: 3, maxBytes: 1024 });
  assert.ok(small.bytes > 0);
  const big = app({ override: (url, init) => init.headers.accept.includes('diff') ? { status: 200, headers: {}, body: 'diff --git a/a b/a\n' + '+line\n'.repeat(500) } : undefined });
  const cut = await big.call('get_pull_request_diff', { ...pull, maxBytes: 1024 });
  assert.equal(cut.truncated, true);
  assert.ok(cut.diff.length <= 1024 && cut.diff.endsWith('\n'));
  // the kernel answers at most 1 MiB; GitHub refuses a diff of more than 20000 lines
  const kernelCap = app({ override: () => { throw { payload: { code: 'APP_HTTP_LIMIT', message: 'response exceeds' } }; } });
  assert.deepEqual(await kernelCap.call('get_pull_request_diff', pull), { diff: '', tooLarge: true, truncated: false, bytes: 0 });
  const refused = app({ override: () => ({ status: 406, headers: {}, body: '{"message":"Sorry, this diff is taking too long to generate."}' }) });
  assert.equal((await refused.call('get_pull_request_diff', pull)).tooLarge, true);
});

test('checks: every run of the head commit, failing ones make the state failure', async () => {
  const checks = await app().call('get_checks', pull);
  assert.equal(checks.sha, heads[2]);
  assert.deepEqual([checks.state, checks.total, checks.failed, checks.pending], ['failure', 4, 2, 0]);
  assert.deepEqual(checks.checks.map(c => [c.name, c.conclusion]), [['failing', 'failure'], ['passing', 'success'], ['failing', 'failure'], ['passing', 'success']]);
  assert.equal(checks.checks[0].app, 'GitHub Actions');
});

test('what a check reports: its summary and the annotations that carry the failure message', async () => {
  const output = await app().call('get_check_output', { repo: REPO, check_run_id: FAILING_CHECK });
  assert.deepEqual([output.check.name, output.check.conclusion], ['failing', 'failure']);
  assert.equal(output.annotations[0].message, 'Process completed with exit code 1.');
  assert.equal(output.annotations[0].level, 'failure');
  assert.deepEqual(output.steps.map(step => [step.number, step.name, step.conclusion]), [[1, 'Set up job', 'success'], [2, 'Run echo "e2e: expected a green banner"', 'failure'], [3, 'Complete job', 'success']]);
  const plain = app({ override: url => /\/actions\/jobs\//.test(url.pathname) ? { status: 404, headers: {}, body: '{"message":"Not Found"}' } : undefined });
  assert.deepEqual((await plain.call('get_check_output', { repo: REPO, check_run_id: FAILING_CHECK })).steps, [], 'a check that is not a GitHub Actions job has no steps');
});

test('check state: none, pending, success, failure from runs and statuses', async () => {
  const { checksSummary } = await import('../src/normalize.mjs');
  const run = (status, conclusion) => ({ status, conclusion });
  assert.equal(checksSummary([], []).state, 'none');
  assert.equal(checksSummary([run('completed', 'success'), run('completed', 'skipped')], []).state, 'success');
  assert.equal(checksSummary([run('completed', 'success'), run('in_progress', null)], []).state, 'pending');
  assert.equal(checksSummary([run('in_progress', null), run('completed', 'timed_out')], []).state, 'failure');
  assert.equal(checksSummary([], [{ state: 'error' }]).state, 'failure');
  assert.equal(checksSummary([run('completed', 'success')], [{ state: 'pending' }]).state, 'pending');
});

test('reviews: the decision is each reviewer\'s latest verdict; comments, pending and dismissed ones do not decide', async () => {
  const { reviewDecision } = await import('../src/normalize.mjs');
  const r = (login, state) => ({ user: { login }, state });
  assert.equal(reviewDecision([r('a', 'APPROVED'), r('b', 'CHANGES_REQUESTED')], 0), 'changes_requested');
  assert.equal(reviewDecision([r('b', 'CHANGES_REQUESTED'), r('b', 'APPROVED')], 0), 'approved');
  assert.equal(reviewDecision([r('a', 'APPROVED'), r('a', 'DISMISSED')], 1), 'review_required');
  assert.equal(reviewDecision([r('a', 'COMMENTED'), r('a', 'PENDING')], 0), 'none');
  const reviews = await app().call('list_reviews', pull);
  assert.deepEqual([reviews.decision, reviews.reviews.length, reviews.reviews[0].state, reviews.requestedReviewers], ['none', 2, 'COMMENTED', []]);
});

test('reviews: every page is read, so a later request for changes outweighs an earlier approval', async () => {
  const review = (id, state) => ({ id, user: { login: 'a' }, state, body: '', submitted_at: '2026-11-15T12:00:00Z', commit_id: heads[2] });
  const pages = { 1: [review(1, 'APPROVED')], 2: [review(2, 'CHANGES_REQUESTED')] };
  const seen = [];
  const paged = app({ override: url => {
    if (!url.pathname.endsWith('/pulls/2/reviews')) return undefined;
    const page = Number(url.searchParams.get('page') ?? 1);
    seen.push(page);
    const link = page === 1 ? { link: `<https://api.github.com${url.pathname}?per_page=100&page=2>; rel="next"` } : {};
    return { status: 200, headers: link, body: JSON.stringify(pages[page]) };
  } });
  const reviews = await paged.call('list_reviews', pull);
  assert.deepEqual([reviews.decision, reviews.reviews.length, seen], ['changes_requested', 2, [1, 2]]);
  const summary = await paged.call('get_pull_request_summary', pull);
  assert.equal(summary.review, 'changes_requested');
});

test('comments: the conversation and the comments on lines of the diff', async () => {
  const { comments, lineComments, hasNext } = await app().call('list_comments', pull);
  assert.equal(comments[0].body, 'Looks good so far. Can you add a test?');
  assert.deepEqual([lineComments[0].path, lineComments[0].line, lineComments[0].side, lineComments[0].body], ['src/hello.js', 2, 'RIGHT', 'Prefer a default parameter here.']);
  assert.equal(hasNext, false);
});

test('commits of a pull request', async () => {
  const { commits } = await app().call('list_commits', pull);
  assert.equal(commits[0].message, 'Describe the drill');
  assert.equal(commits[0].author.login, 'mgutierrez09');
});

test('the summary of a row: size, check state and review decision from three calls', async () => {
  const { call, calls } = app();
  const summary = await call('get_pull_request_summary', pull);
  assert.deepEqual(summary, { repo: REPO, number: 2, additions: 42015, deletions: 0, changedFiles: 4, checks: { state: 'failure', total: 4, failed: 2, pending: 0 }, review: 'none' });
  assert.deepEqual(calls.map(c => c.path.replace(REPO, 'R')).sort(), [`/repos/R/commits/${heads[2]}/check-runs`, `/repos/R/commits/${heads[2]}/status`, '/repos/R/pulls/2', '/repos/R/pulls/2/reviews'].sort());
});

test('mergeability: behind the base branch, and protection that the account may not read is unknown, not an error', async () => {
  const { call, calls } = app();
  const result = await call('get_mergeability', pull);
  assert.deepEqual([result.mergeable, result.mergeableState, result.behindBy, result.aheadBy, result.protection, result.draft], [null, 'unknown', 1, 4, { state: 'unknown' }, false]);
  assert.ok(calls.some(c => c.path.endsWith('/compare/main...' + heads[2])), 'compared with the base branch and the head sha');
  const unprotected = app({ override: url => url.pathname.endsWith('/protection') ? { status: 404, headers: {}, body: '{"message":"Branch not protected"}' } : undefined });
  assert.deepEqual((await unprotected.call('get_mergeability', pull)).protection, { state: 'unprotected' });
  const protectedBranch = app({ override: url => url.pathname.endsWith('/protection') ? { status: 200, headers: {}, body: JSON.stringify({ required_pull_request_reviews: { required_approving_review_count: 2 }, required_status_checks: { strict: true, contexts: ['ci'] } }) } : undefined });
  assert.deepEqual((await protectedBranch.call('get_mergeability', pull)).protection, { state: 'protected', requiredApprovals: 2, requiredChecks: ['ci'], requireUpToDate: true });
});

test('mergeability carries what the repository allows: the merge methods, auto-merge, deleting the branch after a merge', async () => {
  const open = await app().call('get_mergeability', pull);
  assert.deepEqual([open.methods, open.autoMergeAllowed, open.deleteBranchOnMerge], [['merge', 'squash', 'rebase'], false, false]);
  const squashOnly = app({ override: url => /^\/repos\/[^/]+\/[^/]+$/.test(url.pathname) ? { status: 200, headers: {}, body: JSON.stringify({ ...JSON.parse(fixture('repo').body), allow_merge_commit: false, allow_rebase_merge: false, allow_auto_merge: true, delete_branch_on_merge: true }) } : undefined });
  const only = await squashOnly.call('get_mergeability', pull);
  assert.deepEqual([only.methods, only.autoMergeAllowed, only.deleteBranchOnMerge], [['squash'], true, true]);
  const unreadable = app({ override: url => /^\/repos\/[^/]+\/[^/]+$/.test(url.pathname) ? { status: 403, headers: {}, body: '{"message":"Forbidden"}' } : undefined });
  assert.deepEqual((await unreadable.call('get_mergeability', pull)).methods, ['merge', 'squash', 'rebase'], 'settings that cannot be read leave every method to GitHub to refuse');
});

test('a branch with a slash in its name stays one path segment', async () => {
  const { call, calls } = app({ override: url => url.pathname.includes('/pulls/2') && !url.pathname.includes('/pulls/2/') ? undefined : undefined });
  await call('get_compare', { repo: REPO, base: 'main', head: 'chariox/feature' });
  assert.equal(new URL(calls[0].url).pathname, `/repos/${REPO}/compare/main...chariox%2Ffeature`);
});

test('repositories, branches and a comparison', async () => {
  const { call } = app();
  const repos = await call('list_repositories', { query: 'drill' });
  assert.ok(repos.repos.every(r => r.fullName.includes('drill')));
  const org = app();
  await org.call('list_repositories', { org: 'charioxai' });
  assert.equal(org.calls[0].path, '/orgs/charioxai/repos');
  const branches = await call('list_branches', { repo: REPO });
  assert.ok(branches.branches.some(b => b.name === 'main'));
  const compare = await call('get_compare', { repo: REPO, base: 'main', head: heads[2] });
  assert.deepEqual([compare.status, compare.aheadBy, compare.behindBy], ['diverged', 4, 1]);
});

test('a call as an account the owner never bound fails with a code that says what to do', async () => {
  const { call } = app({ unbound: ['github'] });
  await assert.rejects(call('get_pull_request', pull), { code: 'ACCOUNT_NOT_BOUND', message: /bound/ });
});

test('the repository decides the account, then the default, then the first known', async () => {
  const state = memoryState();
  const one = app({}, state);
  assert.deepEqual((await one.call('list_accounts')).accounts.map(a => [a.label, a.login, a.status, a.isDefault]), [['github', 'mgutierrez09', 'ok', true]]);
  await one.call('add_account', { account: 'work' });
  await one.call('set_repo_account', { repo: REPO, account: 'work' });
  await one.call('get_pull_request', pull);
  assert.equal(one.calls.at(-1).connectionId, 'account:work');
  await one.call('get_pull_request', { ...pull, account: 'github' });
  assert.equal(one.calls.at(-1).connectionId, 'account:github');
  await one.call('set_repo_account', { repo: REPO });
  await one.call('get_pull_request', pull);
  assert.equal(one.calls.at(-1).connectionId, 'account:github');
  await one.call('set_default_account', { account: 'work' });
  await one.call('get_pull_request', pull);
  assert.equal(one.calls.at(-1).connectionId, 'account:work');
  await one.call('remove_account', { account: 'work' });
  const left = await one.call('list_accounts');
  assert.deepEqual([left.default, left.accounts.map(a => a.label)], ['github', ['github']]);
});

test('view_account answers the account a call runs as, the one accounts.resolve picks', async () => {
  const one = app({}, memoryState());
  await one.call('add_account', { account: 'work' });
  assert.deepEqual(await one.call('view_account', { repo: REPO }), { account: 'github' });
  await one.call('set_repo_account', { repo: REPO, account: 'work' });
  assert.deepEqual(await one.call('view_account', { repo: REPO }), { account: 'work' });
  assert.deepEqual(await one.call('view_account', { repo: REPO.toUpperCase() }), { account: 'work' });
  assert.deepEqual(await one.call('view_account', { repo: 'o/other' }), { account: 'github' });
  assert.deepEqual(await one.call('view_account', { repo: REPO, account: 'github' }), { account: 'github' });
  await one.call('set_repo_account', { repo: REPO });
  await one.call('set_default_account', { account: 'work' });
  assert.deepEqual(await one.call('view_account', { repo: REPO }), { account: 'work' });
});

test('an account is added only once GitHub accepts it, and one that fails does not hide the others', async () => {
  const state = memoryState();
  const { call } = app({ unbound: ['nope'], override: (url, init) => init.connectionId === 'account:old' && url.pathname === '/user' ? { status: 401, headers: {}, body: '{"message":"Bad credentials"}' } : undefined }, state);
  await assert.rejects(call('add_account', { account: 'nope' }), { code: 'ACCOUNT_NOT_BOUND' });
  assert.deepEqual((await call('list_accounts')).accounts.map(a => a.label), ['github']);
  await assert.rejects(call('add_account', { account: 'old' }), { code: 'ACCOUNT_UNAUTHORIZED' });
  // a binding that stopped working later: listed with its reason, the rest keep working
  await state.transact({ schemaVersion: 0, checks: [{ key: 'accounts', version: null }], writes: [{ tag: 'put', val: ['accounts', JSON.stringify({ default: 'github', labels: ['github', 'old', 'nope'], repos: {} })] }], occurrences: [], wakes: [] }).catch(() => {});
  const listed = await call('list_accounts');
  assert.deepEqual(listed.accounts.map(a => [a.label, a.status]), [['github', 'ok'], ['old', 'unauthorized'], ['nope', 'unbound']]);
  await assert.rejects(call('set_default_account', { account: 'missing' }), { code: 'NOT_FOUND' });
});

test('status reads the rate limits GitHub counts now and flags a paused resource', async () => {
  const status = await app().call('get_status');
  assert.equal(status.accounts[0].status, 'ok');
  assert.deepEqual(Object.keys(status.accounts[0].core).sort(), ['limit', 'paused', 'remaining', 'resetAt']);
  assert.equal(status.accounts[0].core.paused, false);
});

test('the view: signed actions, state and a change feed written with the change itself', async () => {
  const { call, state } = app();
  const actions = await call('view_actions');
  assert.deepEqual(actions.map(a => a.name), catalog.map(tool => tool.name));
  assert.ok(actions.length > 40, 'the read tools and the write tools of the pull request');
  assert.deepEqual((await call('view_state')).accounts.labels, ['github']);
  assert.deepEqual(await call('view_changes', { since: 0 }), { latest: 0, dropped: false, changes: [] });
  await call('add_account', { account: 'work' });
  await call('remove_account', { account: 'work' });
  assert.deepEqual(await call('view_changes', { since: 0 }), { latest: 2, dropped: false, changes: [{ seq: 1, delta: { kind: 'accounts' } }, { seq: 2, delta: { kind: 'accounts' } }] });
  assert.deepEqual((await call('view_changes', { since: 1 })).changes.map(c => c.seq), [2]);
  assert.ok(state.saved.has('chariox.view.latest'));
});

test('the feed keeps 128 changes and tells an older reader to read the state again', async () => {
  const { call, state } = app();
  for (let i = 0; i < 130; i++) await call('set_default_account', { account: 'github' }).catch(() => call('add_account', { account: 'github' }));
  const feed = await call('view_changes', { since: 0 });
  assert.equal(feed.latest, 130);
  assert.equal(feed.dropped, true);
  assert.deepEqual([feed.changes.length, feed.changes[0].seq], [128, 3]);
  assert.equal(state.saved.has('chariox.view.change.2'), false);
});

test('an event delivery is a change of the feed, so an open window reloads what it is about, even before any automation is set', async () => {
  const state = memoryState();
  const { definition, call } = app({}, state);
  const delivery = {
    source: { generator_id: 'dev.chariox.github', connection_id: 'c', event_type: 'pull_request.synchronize', event_type_version: 1 },
    occurred_at: '2026-11-15T12:00:00.000Z', text: 'x', metadata: { repository: { full_name: REPO }, pull_request: { number: 2 } }, artifacts: [],
  };
  await assert.rejects(definition.events.pull_request_updated(delivery, { occurrenceId: 'o1' }), { code: 'NOT_CONFIGURED' });
  const feed = await call('view_changes', { since: 0 });
  assert.deepEqual(feed.changes.map(c => c.delta), [{ kind: 'event', repo: REPO, number: 2 }]);
  await call('configure_automations', { automations: { pull_request_updated_notification: [] } });
  await definition.events.pull_request_updated({ ...delivery, metadata: { repository: { full_name: REPO } } }, { occurrenceId: 'o2' });
  assert.deepEqual((await call('view_changes', { since: 1 })).changes.map(c => c.delta), [{ kind: 'event', repo: REPO, number: null }]);
});
