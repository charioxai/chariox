import assert from 'node:assert/strict';
import test from 'node:test';
import { createApp } from '../src/app.mjs';
import { fakeGitHub, fixture, memoryState, REPO } from './support.mjs';

const json = (status, body) => ({ status, headers: {}, body: JSON.stringify(body) });
const recorded = name => JSON.parse(fixture(name).body);
const WORKTREE = '/work/room-1';

/** The App with a kernel workspace that keeps its calls and answers like the kernel does. */
function scenario({ remote = REPO, branch = 'main', failing } = {}) {
  const asked = [];
  const workspace = {
    changes: async worktree => { asked.push(['changes', worktree]); return JSON.stringify({ files: [], branch, remote }); },
    commit: async (worktree, files, message, named, account) => {
      asked.push(['commit', worktree, files, message, named, account]);
      if (failing === 'commit') throw { code: 'WORKSPACE_ACTION_FAILED', message: '`a.txt` is not a changed file of this worktree' };
      return JSON.stringify({ commit: 'a'.repeat(40), branch: named ?? 'chariox/add-hello', createdBranch: true });
    },
    push: async (worktree, account) => {
      asked.push(['push', worktree, account]);
      if (failing === 'push') throw { code: 'WORKSPACE_ACTION_FAILED', message: 'remote: Permission denied' };
      return JSON.stringify({ commit: 'a'.repeat(40), branch: 'chariox/add-hello' });
    },
  };
  const double = fakeGitHub({
    override: (url, init) => {
      if (init.method === 'POST' && url.pathname === `/repos/${REPO}/pulls`) return json(201, { ...recorded('pull_2'), title: JSON.parse(init.body).title, number: 14 });
      if (init.method !== 'GET') assert.fail(`unexpected change ${init.method} ${url.pathname}`);
      return undefined;
    },
  });
  const app = createApp({ state: memoryState(), events: {}, workspace, now: () => Date.parse('2026-11-17T12:00:00Z') });
  const ctx = { fetch: double.fetch, signal: new AbortController().signal };
  return { call: input => app.tools.ship_changes(input, ctx), asked, posts: () => double.calls.filter(c => c.method === 'POST') };
}

test('shipping commits the named files, pushes as the account and opens the pull request on that branch', async () => {
  const { call, asked, posts } = scenario();
  const shipped = await call({ worktree: WORKTREE, files: ['src/hello.txt'], message: 'Add hello\n\nWhy', draft: true });
  assert.deepEqual(shipped.steps.map(step => [step.step, step.state]), [['commit', 'done'], ['push', 'done'], ['pr', 'done']]);
  assert.deepEqual([shipped.ok, shipped.branch, shipped.steps[2].number], [true, 'chariox/add-hello', 14]);
  assert.deepEqual(asked.map(call => call[0]), ['changes', 'commit', 'push']);
  assert.deepEqual(asked[1].slice(1), [WORKTREE, ['src/hello.txt'], 'Add hello\n\nWhy', null, 'github']);
  assert.equal(asked[2][2], 'github');
  assert.deepEqual(posts().map(c => c.body), [{ title: 'Add hello', head: 'chariox/add-hello', base: 'main', body: '', draft: true }]);
});

test('a failed step stops the rest with its reason, and the call runs again with only the steps left', async () => {
  const stopped = scenario({ failing: 'push' });
  const result = await stopped.call({ worktree: WORKTREE, files: ['a.txt'], message: 'Add a' });
  assert.deepEqual(result.steps.map(step => [step.step, step.state]), [['commit', 'done'], ['push', 'failed'], ['pr', 'not_run']]);
  assert.match(result.steps[1].message, /Permission denied/);
  assert.equal(result.ok, false);
  assert.equal(stopped.posts().length, 0);

  const retry = scenario({ branch: 'chariox/add-hello' });
  const again = await retry.call({ worktree: WORKTREE, steps: ['pr'], title: 'Add a' });
  assert.deepEqual(retry.asked.map(call => call[0]), ['changes']);
  assert.deepEqual([again.ok, again.steps[0].state, retry.posts()[0].body.head], [true, 'done', 'chariox/add-hello']);

  const commit = await scenario({ failing: 'commit' }).call({ worktree: WORKTREE, files: ['a.txt'], message: 'Add a' });
  assert.deepEqual(commit.steps.map(step => step.state), ['failed', 'not_run', 'not_run']);
});

test('shipping needs files and a message to commit, a title to open, and a GitHub origin to ship to', async () => {
  const refused = (input, code) => assert.rejects(scenario(input.options).call(input.call), error => (error?.payload ?? error).code === code);
  await refused({ call: { worktree: WORKTREE, files: ['a.txt'] } }, 'INVALID_ARGUMENT');
  await refused({ call: { worktree: WORKTREE, message: 'Add a' } }, 'INVALID_ARGUMENT');
  await refused({ call: { worktree: WORKTREE, steps: ['pr'] } }, 'INVALID_ARGUMENT');
  await refused({ options: { remote: null }, call: { worktree: WORKTREE, files: ['a.txt'], message: 'Add a' } }, 'NO_GITHUB_REMOTE');
});

test('a pull request shipped from a Room remembers the Room and agent that made it, and a call outside a Room links nothing', async () => {
  const double = fakeGitHub({ override: (url, init) => (init.method === 'POST' || url.pathname.endsWith('/pulls/14') ? json(init.method === 'POST' ? 201 : 200, { ...recorded('pull_2'), number: 14 }) : undefined) });
  const workspace = {
    changes: async () => JSON.stringify({ files: [], branch: 'chariox/x', remote: REPO }),
    commit: async () => JSON.stringify({ commit: 'a'.repeat(40), branch: 'chariox/x', createdBranch: true }),
    push: async () => JSON.stringify({ commit: 'a'.repeat(40), branch: 'chariox/x' }),
  };
  const app = createApp({ state: memoryState(), events: {}, workspace, now: () => 1000 });
  const ctx = { fetch: double.fetch, signal: new AbortController().signal, roomId: 'room-1', agentId: 'agent-7' };
  await app.tools.ship_changes({ worktree: WORKTREE, files: ['a.txt'], message: 'Add a' }, ctx);
  const made = await app.tools.get_pull_request({ repo: REPO, number: 14 }, ctx);
  assert.deepEqual(made.madeBy, { roomId: 'room-1', agentId: 'agent-7', branch: 'chariox/x', at: 1000 });
  assert.equal((await app.tools.get_pull_request({ repo: REPO, number: 2 }, ctx)).madeBy, null);
  await app.tools.link_pull_request({ repo: REPO, number: 2 }, ctx);
  assert.equal((await app.tools.get_pull_request({ repo: REPO, number: 2 }, ctx)).madeBy.agentId, 'agent-7');
  await assert.rejects(app.tools.link_pull_request({ repo: REPO, number: 3 }, { ...ctx, roomId: null }), error => (error?.payload ?? error).code === 'NO_ROOM');
  const outside = await app.tools.ship_changes({ worktree: WORKTREE, files: ['a.txt'], message: 'Add a' }, { ...ctx, roomId: null });
  assert.equal(outside.ok, true);
});
