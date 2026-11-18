import assert from 'node:assert/strict';
import test from 'node:test';
import { createApp } from '../src/app.mjs';
import { memoryState, REPO } from './support.mjs';
import { AGENT, checkRun, checksOverride, comment, harness, kernelRouting, NOW, payload, PERSON, pullRequest } from './subscription-support.mjs';

const HOUR = 3600_000;
const OTHER = { kind: 'agent', id: 'agent-2' };
const opened = (h, metadata = pullRequest()) => h.deliver('pull_request_opened', payload('pull_request_opened', metadata));
const repoScope = (...repos) => ({ kind: 'repos', repos: repos.length ? repos : [REPO] });
const mine = (events, extra = {}) => ({ scope: repoScope(), events, target: { kind: 'agent' }, ...extra });
const results = async h => (await h.call('list_deliveries')).deliveries.map(({ result, event, reason }) => [result, event, reason]);

test('a subscription asks the kernel for one route per repository and event type, and nothing else', async () => {
  const h = harness();
  const { subscription } = await h.call('create_subscription', mine(['pull_request.opened', 'pull_request.updated']), AGENT);
  assert.deepEqual([...h.routing.routes.keys()].sort(), [`pull_request.opened@${REPO}`, `pull_request.synchronize@${REPO}`]);
  assert.deepEqual([...h.routing.routes.values()].map(route => route.event).sort(), ['pull_request_opened', 'pull_request_updated']);
  assert.equal(subscription.target.agentId, 'agent-1');
  assert.deepEqual([...h.routing.targets.keys()], ['auto-agent-1']);
  await h.call('create_subscription', mine(['pull_request.opened', 'issue.opened']), AGENT);
  assert.equal(h.routing.routes.size, 3, 'a second subscription shares the route it has in common');
  assert.equal(h.routing.targets.size, 1, 'one agent, one automation');
});

test('an event reaches the agent as one directed occurrence, once, and the ring says so', async () => {
  const h = harness();
  await h.call('create_subscription', mine(['pull_request.opened'], { name: 'Mine' }), AGENT);
  await opened(h);
  assert.equal(h.emitted.length, 1);
  const [occurrence] = h.emitted;
  assert.equal(occurrence.automationId, 'auto-agent-1');
  assert.match(occurrence.invocation.prompt, /Add a feature/);
  assert.equal(JSON.parse(occurrence.payload).repo, REPO);
  assert.deepEqual(await results(h), [['delivered', 'pull_request.opened', undefined]]);
  await opened(h, pullRequest({ repo: 'other/repo' }));
  assert.equal(h.emitted.length, 1, 'a repository outside the scope reaches no one');
});

test('a person target is a notification; nothing is emitted to an automation', async () => {
  const h = harness();
  await h.call('create_subscription', { scope: repoScope(), events: ['pull_request.opened'], target: { kind: 'person' } });
  await opened(h);
  assert.equal(h.emitted.length, 0);
  assert.equal(h.notified.length, 1);
  assert.match(h.notified[0].title, new RegExp(REPO));
  assert.equal(h.routing.targets.size, 0);
});

test('room and workflow targets bind their own automations', async () => {
  const h = harness();
  const room = await h.call('create_subscription_for_others', { scope: repoScope(), events: ['pull_request.opened'], target: { kind: 'room', agentId: 'agent-9' } });
  assert.equal(room.subscription.target.roomId, 'room-1');
  const flow = await h.call('create_subscription_for_others', { scope: repoScope(), events: ['pull_request.opened'], target: { kind: 'workflow', sessionId: 's1', publicationRef: 'p1' } });
  assert.deepEqual([...h.routing.targets.keys()].sort(), ['auto-agent-9', 'auto-wf-p1']);
  await opened(h);
  assert.deepEqual(h.emitted.map(o => o.automationId).sort(), ['auto-agent-9', 'auto-wf-p1']);
  await h.call('delete_subscription', { id: flow.subscription.id });
  assert.deepEqual([...h.routing.targets.keys()], ['auto-agent-9'], 'deleting the last subscription of a target unbinds it');
});

test('filters: labels with globs and negation, branches, authors, draft, and the account\'s own actions', async () => {
  const h = harness();
  const make = filters => h.call('create_subscription', mine(['pull_request.opened'], { filters, target: { kind: 'person' } }));
  await make({ labels: ['bug', 'team-*', '!wip'] });
  await make({ branches: ['release/*'] });
  await make({ authors: ['octocat'] });
  await make({ draft: 'hide' });
  await opened(h, pullRequest({ labels: ['team-a'], branch: 'main' }));
  assert.equal(h.notified.length, 3, 'the label glob, the author and the non-draft match; the branch does not');
  h.notified.length = 0;
  await opened(h, pullRequest({ labels: ['team-a', 'wip'], draft: true, author: 'someone', branch: 'release/1' }));
  assert.equal(h.notified.length, 1, 'only the release branch is left: wip negates the labels and draft is hidden');
  const filtered = (await h.call('list_deliveries')).deliveries.filter(record => record.result === 'filtered');
  assert.ok(filtered.length >= 3 && filtered.every(record => record.reason));
});

test('a subscription that wants one conclusion ignores the other, and says why', async () => {
  const h = harness();
  await h.call('create_subscription', mine(['check.completed'], { filters: { conclusion: ['failure'] }, target: { kind: 'person' } }));
  await h.deliver('check_run_completed', payload('check_run_completed', checkRun({ conclusion: 'success' })));
  assert.equal(h.notified.length, 0);
  await h.deliver('check_run_completed', payload('check_run_completed', checkRun({ conclusion: 'failure' })));
  assert.equal(h.notified.length, 1);
  assert.deepEqual((await results(h)).map(([result]) => result), ['delivered', 'filtered']);
});

test('path filters read the changed files once and filter by glob', async () => {
  const h = harness({ override: url => url.pathname.endsWith('/pulls/2/files') ? { status: 200, headers: {}, body: JSON.stringify([{ filename: 'docs/readme.md' }]) } : undefined });
  await h.call('create_subscription', mine(['pull_request.opened'], { filters: { paths: ['src/**'] }, target: { kind: 'person' } }));
  await h.call('create_subscription', mine(['pull_request.opened'], { filters: { paths: ['docs/**', '!docs/private/**'] }, target: { kind: 'person' } }));
  await opened(h);
  assert.equal(h.notified.length, 1);
  assert.equal(h.calls.filter(call => call.path.endsWith('/pulls/2/files')).length, 1, 'both subscriptions share one read');
  assert.ok((await results(h)).some(([result, , reason]) => result === 'filtered' && /path/.test(reason)));
});

test('mention matches the account\'s own login and nothing else', async () => {
  const h = harness();
  await h.call('create_subscription', mine(['mention'], { target: { kind: 'person' } }));
  await h.deliver('issue_comment_created', payload('issue_comment_created', comment({ body: 'hello @someone-else', mentions: ['someone-else'] })));
  assert.equal(h.notified.length, 0);
  await h.deliver('issue_comment_created', payload('issue_comment_created', comment({ body: 'ping @mgutierrez09', mentions: ['mgutierrez09'] })));
  assert.equal(h.notified.length, 1);
});

test('an agent\'s own comment does not wake it when excludeSelf is on', async () => {
  const h = harness();
  await h.call('create_subscription', mine(['pull_request.comment'], { filters: { excludeSelf: true } }), AGENT);
  await h.deliver('issue_comment_created', payload('issue_comment_created', comment({ sender: 'mgutierrez09' })));
  assert.equal(h.emitted.length, 0);
  await h.deliver('issue_comment_created', payload('issue_comment_created', comment({ sender: 'reviewer' })));
  assert.equal(h.emitted.length, 1);
});

test('organization and all scopes follow the granted connection; more routes than the installation allows is refused', async () => {
  const h = harness({ routing: kernelRouting({ resources: ['charioxai/a', 'charioxai/b', 'other/c'], maxRoutes: 3 }) });
  const org = await h.call('create_subscription', { scope: { kind: 'org', org: 'charioxai' }, events: ['pull_request.opened'], target: { kind: 'person' } });
  assert.deepEqual(org.sources.live.sort(), ['charioxai/a', 'charioxai/b']);
  await assert.rejects(h.call('create_subscription', { scope: { kind: 'all' }, events: ['pull_request.opened', 'push'], target: { kind: 'person' } }), { code: 'ROUTE_LIMIT' });
  assert.equal((await h.call('list_subscriptions')).subscriptions.length, 1, 'the refused subscription left nothing behind');
  assert.equal(h.routing.routes.size, 2);
  await h.call('create_subscription', { scope: { kind: 'all' }, events: ['pull_request.opened'], target: { kind: 'person' } });
  assert.equal(h.routing.routes.size, 3);
});

test('a repository the connection cannot reach is shown as without a live source, not refused', async () => {
  const h = harness({ routing: kernelRouting({ outside: ['elsewhere/repo'] }) });
  const { sources } = await h.call('create_subscription', { scope: repoScope(REPO, 'elsewhere/repo'), events: ['push'], target: { kind: 'person' } });
  assert.deepEqual(sources.live, [REPO]);
  assert.deepEqual(Object.keys(sources.unrouted), ['elsewhere/repo']);
});

test('pausing stops delivery and drops the routes; resuming brings them back; the daily wake repairs a missing route', async () => {
  const h = harness();
  const { subscription } = await h.call('create_subscription', mine(['pull_request.opened'], { target: { kind: 'person' } }));
  await h.call('pause_subscription', { id: subscription.id });
  assert.equal(h.routing.routes.size, 0);
  await opened(h);
  assert.equal(h.notified.length, 0);
  await h.call('resume_subscription', { id: subscription.id });
  assert.equal(h.routing.routes.size, 1);
  h.routing.routes.clear();
  await h.wake('reconcile');
  assert.equal(h.routing.routes.size, 1);
  assert.ok(h.wakes.has('reconcile'), 'the next reconcile is scheduled');
});

test('an agent subscribes itself; another target needs the approval-tier tool, and an agent changes only its own', async () => {
  const h = harness();
  await assert.rejects(h.call('create_subscription', { scope: repoScope(), events: ['push'], target: { kind: 'person' } }, AGENT), { code: 'TARGET_NEEDS_APPROVAL' });
  await assert.rejects(h.call('create_subscription', mine(['push'], { target: { kind: 'agent', agentId: 'agent-2' } }), AGENT), { code: 'TARGET_NEEDS_APPROVAL' });
  const own = await h.call('create_subscription', mine(['push']), AGENT);
  const theirs = await h.call('create_subscription', mine(['push']), PERSON).catch(() => null);
  assert.equal(theirs, null, 'a person with no agent in the call names the agent');
  const person = await h.call('create_subscription', { scope: repoScope(), events: ['push'], target: { kind: 'person' } });
  await assert.rejects(h.call('delete_subscription', { id: person.subscription.id }, AGENT), { code: 'NOT_YOURS' });
  await assert.rejects(h.call('pause_subscription', { id: own.subscription.id }, OTHER), { code: 'NOT_YOURS' });
  await h.call('delete_subscription', { id: own.subscription.id }, AGENT);
  assert.deepEqual((await h.call('list_subscriptions')).subscriptions.map(item => item.id), [person.subscription.id]);
});

test('update changes what a subscription says and moves its automation with the target', async () => {
  const h = harness();
  const { subscription } = await h.call('create_subscription', mine(['pull_request.opened']), AGENT);
  await h.call('update_subscription', { id: subscription.id, events: ['pull_request.merged'] }, AGENT);
  assert.deepEqual([...h.routing.routes.keys()], [`pull_request.closed@${REPO}`]);
  await h.call('update_subscription_for_others', { id: subscription.id, target: { kind: 'person' } });
  assert.equal(h.routing.targets.size, 0, 'the old automation is released');
  await assert.rejects(h.call('update_subscription', { id: subscription.id, target: { kind: 'agent', agentId: 'agent-2' } }, AGENT), { code: 'TARGET_NEEDS_APPROVAL' });
});

test('a one-shot ci.finished waits for every check, delivers once and ends itself with its routes, automation and wakes', async () => {
  let runs = [['build', 'completed', 'success'], ['lint', 'in_progress', null]];
  const h = harness({ override: (url, init) => checksOverride(runs)(url, init) });
  const { subscription } = await h.call('create_subscription', { events: ['ci.finished'], once: { pr: { repo: REPO, number: 2 } } }, AGENT);
  assert.deepEqual([...h.wakes.keys()].sort(), [`end:${subscription.id}`, `none:${subscription.id}`, 'reconcile']);
  assert.ok(h.routing.routes.has(`check_run.completed@${REPO}`), 'the checks of the pull request are what it needs');
  await h.deliver('check_run_completed', payload('check_run_completed', checkRun({ conclusion: 'success', name: 'build' })));
  assert.equal(h.emitted.length, 0, 'a check is still running');
  runs = [['build', 'completed', 'success'], ['lint', 'completed', 'failure']];
  await h.deliver('check_run_completed', payload('check_run_completed', checkRun({ conclusion: 'failure', name: 'lint' })));
  assert.equal(h.emitted.length, 1);
  assert.equal(JSON.parse(h.emitted[0].payload).event, 'ci.finished');
  assert.match(h.emitted[0].invocation.prompt, /failed|1 of 2/);
  assert.deepEqual((await h.call('list_subscriptions')).subscriptions, []);
  assert.equal(h.routing.routes.size, 0);
  assert.equal(h.routing.targets.size, 0);
  assert.deepEqual([...h.wakes.keys()], ['reconcile']);
  await h.deliver('check_run_completed', payload('check_run_completed', checkRun({ conclusion: 'failure', name: 'lint' })));
  assert.equal(h.emitted.length, 1, 'it ends after its first delivery');
});

test('a one-shot reaches an agent that has ended its turn: the delivery is a wake of its automation, not a poll', async () => {
  const h = harness({ override: checksOverride([['build', 'completed', 'success']]) });
  await h.call('create_subscription', { events: ['ci.finished'], once: { pr: { repo: REPO, number: 2 } } }, AGENT);
  const calls = h.calls.length;
  await h.deliver('check_suite_completed', payload('check_suite_completed', { ...checkRun(), check_suite: { head_sha: checkRun().check_run.head_sha, conclusion: 'success', pull_requests: [{ number: 2 }] } }));
  assert.equal(h.emitted.length, 1);
  assert.equal(h.emitted[0].automationId, 'auto-agent-1');
  assert.equal(h.calls.length - calls, 2, 'the account\'s login and the checks of the head, once each');
});

test('a one-shot ends at its expiry with one honest delivery, and not later', async () => {
  const h = harness();
  const { subscription } = await h.call('create_subscription', { events: ['ci.finished'], once: { pr: { repo: REPO, number: 2 } } }, AGENT);
  assert.equal(subscription.once.expiresAt, NOW + 4 * HOUR, 'four hours by default');
  await h.wake(`end:${subscription.id}`);
  assert.equal(h.emitted.length, 1);
  assert.match(h.emitted[0].invocation.prompt, /ended without a result/);
  assert.deepEqual((await h.call('list_subscriptions')).subscriptions, []);
  await h.wake(`end:${subscription.id}`);
  assert.equal(h.emitted.length, 1);
  await assert.rejects(h.call('create_subscription', { events: ['ci.finished'], once: { pr: { repo: REPO, number: 2 }, expiresAt: NOW + 8 * 24 * HOUR } }, AGENT), { code: 'INVALID_ARGUMENT' });
});

test('with no checks 15 minutes after the request, ci.finished says so; with checks it stays quiet', async () => {
  let total = 0;
  const h = harness({ override: url => url.pathname.endsWith('/check-runs') ? { status: 200, headers: {}, body: JSON.stringify({ total_count: total, check_runs: total ? [{ name: 'a', status: 'in_progress', conclusion: null }] : [] }) } : undefined });
  const { subscription } = await h.call('create_subscription', { events: ['ci.finished'], once: { pr: { repo: REPO, number: 2 } } }, AGENT);
  total = 1;
  await h.wake(`none:${subscription.id}`);
  assert.equal(h.emitted.length, 0);
  total = 0;
  await h.wake(`none:${subscription.id}`);
  assert.equal(h.emitted.length, 1);
  assert.match(h.emitted[0].invocation.prompt, /no checks reported after 15 minutes/);
});

test('watch_pull_request keeps waking on checks, comments and reviews until the pull request is merged or closed', async () => {
  const h = harness({ override: checksOverride([['build', 'completed', 'failure']]) });
  const { subscription } = await h.call('watch_pull_request', { repo: REPO, number: 2, on: ['checks_failed', 'comment'] }, AGENT);
  assert.equal(subscription.target.agentId, 'agent-1');
  await h.deliver('issue_comment_created', payload('issue_comment_created', comment({ number: 2, sender: 'reviewer' })));
  await h.deliver('issue_comment_created', payload('issue_comment_created', comment({ number: 3, sender: 'reviewer' })));
  await h.deliver('check_run_completed', payload('check_run_completed', checkRun()));
  assert.deepEqual(h.emitted.map(o => JSON.parse(o.payload).event), ['pull_request.comment', 'ci.finished']);
  assert.equal((await h.call('list_subscriptions')).subscriptions.length, 1, 'a watch outlives its deliveries');
  await h.deliver('pull_request_closed', payload('pull_request_closed', pullRequest({ merged: true })));
  assert.equal(JSON.parse(h.emitted.at(-1).payload).event, 'pull_request.merged');
  assert.deepEqual((await h.call('list_subscriptions')).subscriptions, []);
  assert.equal(h.routing.routes.size, 0);
});

test('the ring keeps the last 200 deliveries newest first, and five failures in a row disable a subscription', async () => {
  const h = harness();
  const { subscription } = await h.call('create_subscription', mine(['pull_request.opened'], { filters: { labels: ['x'] }, target: { kind: 'person' } }));
  for (let n = 0; n < 205; n++) await opened(h, pullRequest({ number: 100 + n }));
  const all = (await h.call('list_deliveries', { limit: 500 })).deliveries;
  assert.equal(all.length, 200);
  assert.equal(all[0].subject, '#304', 'newest first');
  assert.equal((await h.call('list_deliveries', { subscription: subscription.id, limit: 50 })).deliveries.length, 50);

  const failing = harness();
  const wired = await failing.call('create_subscription', mine(['pull_request.opened']), AGENT);
  failing.rejecting.add('auto-agent-1');
  for (let n = 0; n < 5; n++) await opened(failing, pullRequest({ number: 10 + n }));
  const [after] = (await failing.call('list_subscriptions')).subscriptions;
  assert.equal(after.id, wired.subscription.id);
  assert.equal(after.enabled, false);
  assert.equal(failing.routing.routes.size, 0, 'a disabled subscription holds no route');
  assert.deepEqual((await results(failing)).map(([result]) => result), Array(5).fill('failed'));
  await opened(failing, pullRequest({ number: 99 }));
  assert.equal((await results(failing)).length, 5, 'nothing more is attempted');
});

test('a restarted App keeps its subscriptions, ring and routes, and delivers the next event', async () => {
  const state = memoryState();
  const routing = kernelRouting();
  const first = harness({ state, routing });
  await first.call('create_subscription', mine(['pull_request.opened']), AGENT);
  await opened(first);
  const second = harness({ state, routing });
  assert.equal((await second.call('list_subscriptions')).subscriptions.length, 1);
  assert.equal((await second.call('list_deliveries')).deliveries.length, 1);
  await opened(second, pullRequest({ number: 7 }));
  assert.equal(second.emitted.length, 1);
  assert.equal(routing.routes.size, 1, 'reconciling after the restart adds nothing');
  await second.wake('reconcile');
  assert.equal(routing.routes.size, 1);
});

test('the window follows subscription and delivery changes through the change feed', async () => {
  const h = harness();
  await h.call('create_subscription', mine(['pull_request.opened'], { target: { kind: 'person' } }));
  await opened(h);
  const { changes } = await h.call('view_changes', { since: 0 });
  assert.deepEqual([...new Set(changes.map(change => change.delta.kind))].sort(), ['deliveries', 'event', 'subscriptions']);
});

test('there is one path: no automation-name settings, and a delivery must be the event its route names', async () => {
  const h = harness();
  for (const name of ['get_automations', 'configure_automations']) assert.equal(h.definition.tools[name], undefined, name);
  const wrong = payload('pull_request_opened', pullRequest());
  await assert.rejects(h.definition.events.pull_request_updated(wrong, { fetch: h.fetch }), { code: 'INVALID_ARGUMENT' });
  await assert.rejects(h.definition.events.pull_request_opened({ ...wrong, occurred_at: 'soon' }, { fetch: h.fetch }), { code: 'INVALID_ARGUMENT' });
});

test('at most 100 subscriptions', async () => {
  const h = harness();
  for (let n = 0; n < 100; n++) await h.call('create_subscription', { scope: repoScope(), events: ['push'], target: { kind: 'person' } });
  await assert.rejects(h.call('create_subscription', { scope: repoScope(), events: ['push'], target: { kind: 'person' } }), { code: 'LIMIT' });
});
