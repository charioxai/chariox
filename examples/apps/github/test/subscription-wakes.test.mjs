// A one-shot's scheduled wakes follow the subscription: its settings, its pause, its end.
import assert from 'node:assert/strict';
import test from 'node:test';
import { REPO } from './support.mjs';
import { AGENT, checksOverride, harness, NOW, payload, pullRequest } from './subscription-support.mjs';

const HOUR = 3600_000;
const pr = { repo: REPO, number: 2 };
const wakesOf = h => [...h.wakes.keys()].filter(id => id !== 'reconcile').sort();
const results = async h => (await h.call('list_deliveries')).deliveries.map(({ result, event, reason }) => [result, event, reason]);

test('a watch with an expiry ends there: the person is told once, and it holds no route, target or wake afterwards', async () => {
  const h = harness();
  const { subscription } = await h.call('watch_pull_request', { repo: REPO, number: 2, expiresAt: new Date(NOW + HOUR).toISOString() }, AGENT);
  await h.wake(`end:${subscription.id}`);
  assert.equal(JSON.parse(h.emitted.at(-1).payload).outcome, 'expired');
  assert.deepEqual((await h.call('list_subscriptions')).subscriptions, []);
  assert.equal(h.routing.routes.size, 0);
  assert.equal(h.routing.targets.size, 0);
  assert.deepEqual(wakesOf(h), []);
  const delivered = h.emitted.length;
  await h.deliver('pull_request_closed', payload('pull_request_closed', pullRequest({ merged: true })));
  assert.equal(h.emitted.length, delivered, 'nothing arrives after the end');
});

test('an expiry whose delivery fails still ends the subscription', async () => {
  const h = harness();
  const { subscription } = await h.call('watch_pull_request', { repo: REPO, number: 2, expiresAt: new Date(NOW + HOUR).toISOString() }, AGENT);
  h.rejecting.add('auto-agent-1');
  await h.wake(`end:${subscription.id}`);
  assert.deepEqual((await h.call('list_subscriptions')).subscriptions, []);
});

test('changing a one-shot\'s expiry moves its end wake, and a firing of the old time does nothing', async () => {
  const h = harness();
  const { subscription } = await h.call('create_subscription', { events: ['ci.finished'], once: { pr, expiresAt: NOW + 2 * HOUR } }, AGENT);
  const old = { id: `end:${subscription.id}`, ...h.wakes.get(`end:${subscription.id}`) };
  assert.equal(old.dueAtMs, BigInt(NOW + 2 * HOUR));
  await h.call('update_subscription', { id: subscription.id, once: { expiresAt: NOW + 5 * HOUR } }, AGENT);
  assert.equal(h.wakes.get(old.id).dueAtMs, BigInt(NOW + 5 * HOUR), 'extended');
  await h.fire(old);
  assert.equal(h.emitted.length, 0, 'the old firing is stale');
  assert.equal((await h.call('list_subscriptions')).subscriptions.length, 1);
  const { subscription: current } = await h.call('get_subscription', { id: subscription.id });
  assert.deepEqual(current.once.pr, pr, 'the pull request it waits for is kept');
  await h.call('update_subscription', { id: subscription.id, once: { expiresAt: NOW + HOUR } }, AGENT);
  assert.equal(h.wakes.get(old.id).dueAtMs, BigInt(NOW + HOUR), 'shortened');
  await h.wake(old.id);
  assert.equal(h.emitted.length, 1);
  assert.deepEqual((await h.call('list_subscriptions')).subscriptions, []);
});

test('making a subscription a one-shot arms its end and its no-checks wake', async () => {
  const h = harness();
  const { subscription } = await h.call('create_subscription', { scope: { kind: 'repos', repos: [REPO] }, events: ['ci.finished'], target: { kind: 'agent' } }, AGENT);
  assert.deepEqual(wakesOf(h), []);
  await h.call('update_subscription', { id: subscription.id, once: { pr, expiresAt: NOW + HOUR } }, AGENT);
  assert.deepEqual(wakesOf(h), [`end:${subscription.id}`, `none:${subscription.id}`]);
  assert.equal(h.wakes.get(`none:${subscription.id}`).dueAtMs, BigInt(NOW + 15 * 60_000));
});

test('a paused one-shot says nothing when its no-checks wake fires, and hears again after it resumes', async () => {
  const h = harness({ override: checksOverride([]) });
  const { subscription } = await h.call('create_subscription', { events: ['ci.finished'], once: { pr } }, AGENT);
  await h.call('pause_subscription', { id: subscription.id }, AGENT);
  await h.wake(`none:${subscription.id}`);
  assert.equal(h.emitted.length, 0, 'paused');
  assert.equal((await h.call('list_subscriptions')).subscriptions.length, 1, 'the pause did not spend it');
  await h.call('resume_subscription', { id: subscription.id }, AGENT);
  assert.ok(h.wakes.has(`none:${subscription.id}`), 'the wait for checks starts again');
  await h.wake(`none:${subscription.id}`);
  assert.equal(h.emitted.length, 1);
  assert.match(h.emitted[0].invocation.prompt, /no checks reported/);
});

test('a paused one-shot that reaches its expiry ends without notifying anyone, and the ring says so', async () => {
  const h = harness();
  const { subscription } = await h.call('create_subscription', { events: ['ci.finished'], once: { pr } }, AGENT);
  await h.call('pause_subscription', { id: subscription.id }, AGENT);
  await h.wake(`end:${subscription.id}`);
  assert.equal(h.emitted.length, 0);
  assert.deepEqual((await h.call('list_subscriptions')).subscriptions, []);
  assert.deepEqual(await results(h), [['filtered', 'expired', 'paused when it ended']]);
  assert.deepEqual(wakesOf(h), []);
});

test('the last one-shot of a target releases it from the background, and a release that fails is kept until it works', async () => {
  const h = harness({ override: checksOverride([['a', 'completed', 'success']]) });
  await h.call('create_subscription', { events: ['ci.finished'], once: { pr } }, AGENT);
  await h.call('create_subscription', { scope: { kind: 'repos', repos: [REPO] }, events: ['pull_request.opened'], target: { kind: 'person' } });
  h.routing.failUnbind = 1;
  await h.deliver('check_run_completed', payload('check_run_completed', { ...pullRequest(), check_run: { id: 1, name: 'a', conclusion: 'success', status: 'completed', head_sha: pullRequest().chariox.head_sha, pull_requests: [{ number: 2 }] }, sender: { login: 'bot' } }));
  assert.equal(h.emitted.length, 1, 'the delivery was made');
  assert.equal((await h.call('list_subscriptions')).subscriptions.length, 1, 'the one-shot is gone, the other subscription stays');
  assert.deepEqual([...h.routing.routes.keys()], [`pull_request.opened@${REPO}`], 'the routes followed the removal although the release failed');
  assert.equal(h.routing.targets.size, 1, 'the automation waits for its release');
  await h.wake('reconcile');
  assert.equal(h.routing.targets.size, 0, 'the daily reconcile finishes the release');
});

test('a pending release keeps its daily retry although no subscription is left, until it works', async () => {
  const h = harness();
  const { subscription } = await h.call('create_subscription', { events: ['ci.finished'], once: { pr } }, AGENT);
  h.routing.failUnbind = 2;
  await h.call('delete_subscription', { id: subscription.id }, AGENT);
  assert.equal(h.routing.targets.size, 1, 'the release failed on removal');
  h.clock.now += 24 * HOUR;
  h.wakes.delete('reconcile');
  await h.fire({ id: 'reconcile' });
  assert.equal(h.routing.targets.size, 1, 'and failed again at the first daily retry');
  assert.equal(h.wakes.get('reconcile')?.dueAtMs, BigInt(h.clock.now + 24 * HOUR), 'the next retry is armed');
  h.clock.now += 24 * HOUR;
  h.wakes.delete('reconcile');
  await h.fire({ id: 'reconcile' });
  assert.equal(h.routing.targets.size, 0, 'the next retry releases it');
  assert.equal(h.wakes.has('reconcile'), false, 'nothing is left to retry');
});

test('a no-checks firing captured before an update does nothing after the update moved or dropped it', async () => {
  const h = harness({ override: checksOverride([]) });
  const { subscription } = await h.call('create_subscription', { events: ['ci.finished'], once: { pr } }, AGENT);
  const old = { id: `none:${subscription.id}`, ...h.wakes.get(`none:${subscription.id}`) };
  await h.call('update_subscription', { id: subscription.id, once: { pr: { repo: REPO, number: 3 } } }, AGENT);
  await h.fire(old);
  assert.equal(h.emitted.length, 0, 'the old pull request\'s firing does not speak for the new one');
  assert.equal((await h.call('list_subscriptions')).subscriptions.length, 1, 'and does not spend the one-shot');
  const moved = { id: old.id, ...h.wakes.get(old.id) };
  await h.call('update_subscription', { id: subscription.id, events: ['pull_request.comment'] }, AGENT);
  await h.fire(moved);
  assert.equal(h.emitted.length, 0, 'a firing for an event the subscription no longer asks for does nothing');
  assert.equal((await h.call('list_subscriptions')).subscriptions.length, 1);
});

test('a no-checks firing of the same pull request armed before an update does nothing, and the re-armed wake speaks', async () => {
  const h = harness({ override: checksOverride([]) });
  const { subscription } = await h.call('create_subscription', { events: ['ci.finished'], once: { pr } }, AGENT);
  const old = { id: `none:${subscription.id}`, ...h.wakes.get(`none:${subscription.id}`) };
  h.clock.now += 10 * 60_000;
  await h.call('update_subscription', { id: subscription.id, once: { expiresAt: NOW + 5 * HOUR } }, AGENT);
  const current = { id: old.id, ...h.wakes.get(old.id) };
  assert.notEqual(current.revision, old.revision, 'the update armed a new wait');
  await h.fire(old);
  assert.equal(h.emitted.length, 0, 'the firing of the superseded wait does not speak');
  assert.equal((await h.call('list_subscriptions')).subscriptions.length, 1, 'and does not spend the one-shot');
  await h.fire(current);
  assert.equal(h.emitted.length, 1, 'the current wait speaks');
});

test('a repository name too long for a wake or a route is refused for its length, and the longest routable one keeps a short revision of its own', async () => {
  const h = harness({ override: checksOverride([]) });
  const longest = { repo: `${'o'.repeat(100)}/${'r'.repeat(100)}`, number: 123456 };
  await assert.rejects(h.call('create_subscription', { events: ['ci.finished'], once: { pr: longest } }, AGENT), { message: /too long to route/ });
  assert.deepEqual(wakesOf(h), [], 'nothing stays armed for the refused one');
  const first = { repo: `${'o'.repeat(40)}/${'r'.repeat(49)}a`, number: 123456 };
  const { subscription } = await h.call('create_subscription', { events: ['ci.finished'], once: { pr: first, expiresAt: NOW + HOUR } }, AGENT);
  const armed = { id: `none:${subscription.id}`, ...h.wakes.get(`none:${subscription.id}`) };
  assert.ok(new TextEncoder().encode(armed.revision).length <= 128, `${armed.revision.length} characters`);
  await h.call('update_subscription', { id: subscription.id, once: { pr: { ...first, repo: `${first.repo.slice(0, -1)}b` } } }, AGENT);
  assert.notEqual(h.wakes.get(armed.id).revision, armed.revision, 'two names that differ only at the end do not share a revision');
  await h.fire(armed);
  assert.equal(h.emitted.length, 0);
});
