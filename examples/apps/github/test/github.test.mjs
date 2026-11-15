import assert from 'node:assert/strict';
import test from 'node:test';
import { createGitHub, expand, parseLink, queryString } from '../src/github.mjs';
import { fakeGitHub, memoryState } from './support.mjs';

const client = (options = {}, state = memoryState(), now = () => Date.parse('2026-11-15T12:00:00Z')) => {
  const double = fakeGitHub(options);
  return { ...double, state, github: createGitHub({ state, fetch: double.fetch, now }) };
};

test('a path template is filled with encoded segments and a query is built from set values only', () => {
  assert.equal(expand('/repos/{owner}/{repo}/branches/{branch}/protection', { owner: 'o', repo: 'r', branch: 'chariox/feature one' }), '/repos/o/r/branches/chariox%2Ffeature%20one/protection');
  assert.throws(() => expand('/repos/{owner}', {}), { code: 'INVALID_ARGUMENT' });
  assert.equal(queryString({ a: 1, b: undefined, c: '', d: 'x y' }), '?a=1&d=x%20y');
  assert.deepEqual(parseLink('<https://api.github.com/x?page=2>; rel="next", <https://api.github.com/x?page=9>; rel="last"'), { next: 'https://api.github.com/x?page=2', last: 'https://api.github.com/x?page=9' });
});

test('an answer is kept with its ETag and the next call is conditional: GitHub answers 304 and the App serves its copy', async () => {
  const { github, calls } = client();
  const first = await github.get('github', '/user', {});
  const second = await github.get('github', '/user', {});
  assert.deepEqual(second.data, first.data);
  assert.equal(calls.length, 2);
  assert.equal(calls[0].headers['if-none-match'], undefined);
  assert.ok(calls[1].headers['if-none-match'], 'the second call revalidates');
  assert.equal(second.cached, true);
});

test('the cache is per account: another account never sees an answer kept for the first', async () => {
  const { github, calls } = client();
  await github.get('github', '/user', {});
  await github.get('work', '/user', {});
  assert.equal(calls[1].headers['if-none-match'], undefined);
  assert.equal(calls[1].connectionId, 'account:work');
});

test('only the headers the kernel allows are sent', async () => {
  const { github, calls } = client();
  await github.get('github', '/user', {}, { accept: 'application/vnd.github.diff' });
  assert.deepEqual(Object.keys(calls[0].headers).sort(), ['accept', 'x-github-api-version']);
});

test('the rate limit of every answer is kept per account and resource, and the App stops short of the end of it', async () => {
  let remaining = 150;
  const { github, calls } = client({
    override: url => ({ status: 200, headers: { 'x-ratelimit-limit': '5000', 'x-ratelimit-remaining': String(remaining), 'x-ratelimit-reset': '1794750000', 'x-ratelimit-resource': 'core' }, body: '{"login":"me"}' }),
  });
  await github.get('github', '/user', {}, { fresh: true });
  assert.equal((await github.rate('github', 'core')).remaining, 150);
  remaining = 99;
  await github.get('github', '/user', {}, { fresh: true });
  await assert.rejects(github.get('github', '/user', {}, { fresh: true }), error => error.code === 'RATE_LIMITED' && error.resetAt === 1794750000 * 1000 && error.resource === 'core');
  assert.equal(calls.length, 2, 'no request is made once the reserve is reached');
  assert.equal((await github.rate('work', 'core')), null, 'another account is not limited');
});

test('while limited, a kept answer is still served and marked stale; /rate_limit is always allowed', async () => {
  let remaining = 4000;
  const { github } = client({
    override: url => url.pathname === '/user' || url.pathname === '/rate_limit'
      ? { status: 200, headers: { etag: 'W/"a"', 'x-ratelimit-limit': '5000', 'x-ratelimit-remaining': String(remaining), 'x-ratelimit-reset': '1794750000', 'x-ratelimit-resource': 'core' }, body: '{"login":"me"}' } : undefined,
  });
  await github.get('github', '/user', {});
  remaining = 10;
  await github.get('github', '/rate_limit', {}, { fresh: true });
  await github.get('github', '/user', {}, { fresh: true }).catch(() => {});
  const stale = await github.get('github', '/user', {});
  assert.equal(stale.stale, true);
  assert.equal(stale.data.login, 'me');
});

test('the search resource has its own limit', async () => {
  const { github } = client({
    override: url => url.pathname === '/search/issues' ? { status: 200, headers: { 'x-ratelimit-limit': '30', 'x-ratelimit-remaining': '1', 'x-ratelimit-reset': '1794750000', 'x-ratelimit-resource': 'search' }, body: '{"total_count":0,"items":[]}' } : undefined,
  });
  await github.get('github', '/search/issues', {}, { query: { q: 'is:pr' }, fresh: true });
  await assert.rejects(github.get('github', '/search/issues', {}, { query: { q: 'is:pr' }, fresh: true }), { code: 'RATE_LIMITED', resource: 'search' });
  assert.equal((await github.get('github', '/user', {})).data.login.length > 0, true, 'core is unaffected');
});

test('answers become the App\'s own codes: not bound, unauthorized, not found, forbidden, rate limited', async () => {
  const answer = (status, body, headers = {}) => ({ override: () => ({ status, headers, body: JSON.stringify(body) }) });
  await assert.rejects(client({ unbound: ['github'] }).github.get('github', '/user', {}), { code: 'ACCOUNT_NOT_BOUND' });
  await assert.rejects(client(answer(401, { message: 'Bad credentials' })).github.get('github', '/user', {}), { code: 'ACCOUNT_UNAUTHORIZED' });
  await assert.rejects(client(answer(404, { message: 'Not Found' })).github.get('github', '/user', {}), { code: 'NOT_FOUND' });
  await assert.rejects(client(answer(403, { message: 'Resource not accessible' })).github.get('github', '/user', {}), { code: 'FORBIDDEN' });
  await assert.rejects(client(answer(403, { message: 'API rate limit exceeded' }, { 'x-ratelimit-remaining': '0', 'x-ratelimit-reset': '1794750000' })).github.get('github', '/user', {}), { code: 'RATE_LIMITED' });
  await assert.rejects(client(answer(429, { message: 'slow down' }, { 'retry-after': '30' })).github.get('github', '/user', {}), error => error.code === 'RATE_LIMITED' && error.resetAt === (Date.parse('2026-11-15T12:00:00Z') / 1000 + 30) * 1000);
  await assert.rejects(client(answer(502, {})).github.get('github', '/user', {}), { code: 'GITHUB_ERROR', status: 502 });
});

test('a list says whether another page follows', async () => {
  const { github } = client({ override: url => url.pathname === '/user/repos' ? { status: 200, headers: { link: '<https://api.github.com/user/repos?page=2>; rel="next"' }, body: '[]' } : undefined });
  const page = await github.list('github', '/user/repos', {}, { page: 1, perPage: 5 });
  assert.deepEqual([page.page, page.hasNext], [1, true]);
});

test('an answer too large to keep is not stored, so a slot never holds more than 120 KiB', async () => {
  const body = JSON.stringify(Array.from({ length: 4000 }, (_, i) => ({ i, text: 'x'.repeat(40) })));
  const { github, state } = client({ override: () => ({ status: 200, headers: { etag: 'W/"big"' }, body }) });
  await github.get('github', '/user/repos', {});
  assert.equal([...state.saved.keys()].filter(k => k.startsWith('cache:')).length, 0);
});
