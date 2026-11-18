// What "CI finished" and a path filter read from GitHub: every page of it, and the whole commit's result.
import assert from 'node:assert/strict';
import test from 'node:test';
import { REPO } from './support.mjs';
import { AGENT, checkRun, checksOverride, harness, payload, pullRequest } from './subscription-support.mjs';

const results = async h => (await h.call('list_deliveries')).deliveries.map(({ result, event, reason }) => [result, event, reason]);
const completed = (conclusion, name) => payload('check_run_completed', checkRun({ conclusion, name }));
const next = (url, page) => `<${url.origin}${url.pathname}?per_page=100&page=${page}>; rel="next"`;

/** A list answered 100 at a time from `pages`, with the Link header GitHub sends while more follow. */
const paged = (suffix, pages, wrap = items => items) => url => {
  if (!url.pathname.endsWith(suffix)) return undefined;
  const page = Number(url.searchParams.get('page') ?? 1);
  return { status: 200, headers: page < pages.length ? { link: next(url, page + 1) } : {}, body: JSON.stringify(wrap(pages[page - 1])) };
};
const runs = (count, status = 'completed', conclusion = 'success') => Array.from({ length: count }, (_, n) => ({ name: `job-${n}`, status, conclusion: status === 'completed' ? conclusion : null }));
const files = (count, prefix) => Array.from({ length: count }, (_, n) => ({ filename: `${prefix}/file-${n}.txt` }));

test('a failed workflow is reported when the last one finishes with success, and the conclusion filter reads the whole commit', async () => {
  let state = [['early', 'completed', 'failure'], ['late', 'in_progress', null]];
  const h = harness({ override: (url, init) => checksOverride(state)(url, init) });
  await h.call('watch_pull_request', { repo: REPO, number: 2, on: ['checks_failed'] }, AGENT);
  await h.deliver('check_run_completed', completed('failure', 'early'));
  assert.equal(h.emitted.length, 0, 'a check is still running');
  state = [['early', 'completed', 'failure'], ['late', 'completed', 'success']];
  await h.deliver('check_run_completed', completed('success', 'late'));
  assert.equal(h.emitted.length, 1, 'the commit failed even though the last run to finish passed');
  assert.equal(JSON.parse(h.emitted[0].payload).conclusion, 'failure');
  assert.match(h.emitted[0].invocation.prompt, /1 of 2/);
});

test('checks_failed stays quiet for a commit whose every check passed, and the ring says why', async () => {
  const h = harness({ override: checksOverride([['a', 'completed', 'success'], ['b', 'completed', 'success']]) });
  await h.call('watch_pull_request', { repo: REPO, number: 2, on: ['checks_failed'] }, AGENT);
  await h.deliver('check_run_completed', completed('success', 'b'));
  assert.equal(h.emitted.length, 0);
  assert.deepEqual(await results(h), [['filtered', 'ci.finished', 'conclusion success is not selected']]);
});

test('a conclusion filter still applies to the individual check events a subscription asks for', async () => {
  const h = harness({ override: checksOverride([['a', 'completed', 'success']]) });
  await h.call('create_subscription', { scope: { kind: 'repos', repos: [REPO] }, events: ['check.completed'], filters: { conclusion: ['failure'] }, target: { kind: 'agent' } }, AGENT);
  await h.deliver('check_run_completed', completed('success', 'a'));
  assert.equal(h.emitted.length, 0);
  await h.deliver('check_run_completed', completed('failure', 'a'));
  assert.equal(h.emitted.length, 1);
});

test('a check beyond the first page decides whether CI finished', async () => {
  const pending = [runs(100), [...runs(49), { name: 'matrix-last', status: 'in_progress', conclusion: null }]];
  const h = harness({ override: paged('/check-runs', pending, items => ({ total_count: 150, check_runs: items })) });
  await h.call('create_subscription', { events: ['ci.finished'], once: { pr: { repo: REPO, number: 2 } } }, AGENT);
  await h.deliver('check_run_completed', completed('success', 'job-0'));
  assert.equal(h.emitted.length, 0, 'the 150th check is still running');
  assert.equal((await h.call('list_subscriptions')).subscriptions.length, 1, 'a one-shot is not spent before its result');
  const done = harness({ override: paged('/check-runs', [runs(100), [...runs(49), { name: 'matrix-last', status: 'completed', conclusion: 'failure' }]], items => ({ total_count: 150, check_runs: items })) });
  await done.call('create_subscription', { events: ['ci.finished'], once: { pr: { repo: REPO, number: 2 } } }, AGENT);
  await done.deliver('check_run_completed', completed('success', 'job-0'));
  assert.equal(done.emitted.length, 1);
  assert.match(done.emitted[0].invocation.prompt, /CI failed \(1 of 150 checks\): matrix-last/);
});

test('a path on the second page of changed files counts, for the paths wanted and the paths refused', async () => {
  const pages = [files(100, 'docs'), [...files(1, 'src'), ...files(99, 'docs')]];
  const h = harness({ override: paged('/pulls/2/files', pages) });
  const mine = filters => ({ scope: { kind: 'repos', repos: [REPO] }, events: ['pull_request.opened'], filters, target: { kind: 'agent' } });
  await h.call('create_subscription', mine({ paths: ['src/**'] }), AGENT);
  await h.deliver('pull_request_opened', payload('pull_request_opened', pullRequest()));
  assert.equal(h.emitted.length, 1, 'the wanted path is the 101st file');
  const refusing = harness({ override: paged('/pulls/2/files', pages) });
  await refusing.call('create_subscription', mine({ paths: ['!src/**'] }), AGENT);
  await refusing.deliver('pull_request_opened', payload('pull_request_opened', pullRequest()));
  assert.equal(refusing.emitted.length, 0, 'the refused path is the 101st file');
  assert.deepEqual(await results(refusing), [['filtered', 'pull_request.opened', 'no selected path changed']]);
});
