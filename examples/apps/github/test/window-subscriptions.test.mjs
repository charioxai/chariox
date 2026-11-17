// The Subscriptions tab in headless Chrome: the built page, the App's real code and the kernel's imports as doubles.
// Skipped where no Chrome is installed (set CHARIOX_TEST_CHROME to one).
import assert from 'node:assert/strict';
import { mkdirSync } from 'node:fs';
import test, { after, before } from 'node:test';
import { REPO } from './support.mjs';
import { kernelImports, kernelRouting, NOW, payload, pullRequest } from './subscription-support.mjs';
import { buildWindow, CHROME, launch, open } from './window-support.mjs';

const skip = CHROME ? false : 'no Chrome to run the window in';
const shots = process.env.CHARIOX_SHOTS;
let built, browser;
before(async () => { if (skip) return; built = buildWindow(); browser = await launch(); if (shots) mkdirSync(shots, { recursive: true }); });
after(async () => { await browser?.close(); built?.remove(); });

async function window(options = {}) {
  const kernel = kernelImports({ routing: options.routing ?? kernelRouting() });
  const session = await open(browser, built, { ...options, now: NOW, imports: { routing: kernel.routing, events: kernel.events, host: kernel.host, schedule: kernel.schedule } });
  await session.page.waitForSelector('[data-pull]');
  await session.page.click('[data-tab=subscriptions]');
  await session.page.waitForSelector('[data-new-subscription]');
  return { ...session, kernel };
}
const shot = (session, name, dpr) => shots && session.page.screenshot({ path: `${shots}/${name}-dpr${dpr}.png` });
const created = session => session.tools.filter(call => call.name === 'create_subscription').map(call => call.input);

async function fillForm(page, { name, repos, events, target = 'person' }) {
  await page.click('[data-new-subscription]');
  await page.fill('[name=name]', name);
  await page.fill('[name=repos]', repos);
  for (const kind of events) await page.check(`[data-event="${kind}"]`);
  await page.selectOption('[name=target]', target);
}

test('a person creates a subscription for themselves and it is listed with its state', { skip }, async () => {
  const session = await window();
  const { page } = session;
  assert.equal(await page.locator('[data-subscription]').count(), 0);
  await page.waitForSelector('text=No subscriptions');
  await fillForm(page, { name: 'Pull requests of the drill', repos: REPO, events: ['pull_request.merged'] });
  await page.click('[data-save]');
  await page.waitForSelector('[data-subscription]');
  assert.deepEqual(created(session), [{
    name: 'Pull requests of the drill', scope: { kind: 'repos', repos: [REPO] }, events: ['pull_request.opened', 'pull_request.merged'],
    filters: { branches: [], authors: [], labels: [], paths: [], conclusion: [], draft: 'any', excludeSelf: false }, target: { kind: 'person' },
  }]);
  assert.match(await page.textContent('[data-subscription-name]'), /Pull requests of the drill/);
  assert.equal(await page.textContent('[data-state]'), 'Active');
  assert.equal(await page.locator('form').count(), 0, 'the form closes once it was saved');
  assert.deepEqual(session.errors, []);
  await shot(session, 'subscriptions-list', 1);
});

test('a refusal is shown above the form in the App\'s own words and nothing is made', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await fillForm(page, { name: 'No repository', repos: '', events: [] });
  await page.click('[data-save]');
  await page.waitForSelector('.flash-error');
  assert.match(await page.textContent('.flash-error'), /repository|events/i);
  assert.equal(await page.locator('[data-subscription]').count(), 0);
  assert.equal(await page.locator('form[data-form=create]').count(), 1, 'what was typed stays');
});

test('a form that is open is not replaced by a reload', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await page.click('[data-new-subscription]');
  await page.fill('[name=name]', 'half written');
  await session.app.tools.create_subscription({ name: 'By an agent', scope: { kind: 'repos', repos: [REPO] }, events: ['push'], target: { kind: 'agent', agentId: 'a1' } }, { fetch: session.double.fetch, actor: { kind: 'agent', id: 'a1' }, agentId: 'a1' });
  await page.waitForTimeout(6000);
  assert.equal(await page.inputValue('[name=name]'), 'half written');
});

test('a subscription made elsewhere appears by itself, with its target and who made it', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await session.app.tools.create_subscription({ name: 'Watch the build', scope: { kind: 'repos', repos: [REPO] }, events: ['ci.finished'], target: { kind: 'agent' } }, { fetch: session.double.fetch, actor: { kind: 'agent', id: 'agent-7' }, agentId: 'agent-7', roomId: 'room-1' });
  await page.waitForSelector('[data-subscription]', { timeout: 12000 });
  const text = await page.textContent('[data-subscription]');
  assert.match(text, /Watch the build/);
  assert.match(text, /Agent agent-7/);
  assert.match(text, /Made by an agent/);
  assert.match(text, /Checks finished/);
});

test('pause, resume and the two-step delete go through the App and the list follows', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await fillForm(page, { name: 'Keep', repos: REPO, events: [] });
  await page.click('[data-save]');
  await page.waitForSelector('[data-subscription]');
  const id = await page.getAttribute('[data-subscription]', 'data-subscription');
  await page.click(`[data-pause="${id}"]`);
  await page.waitForFunction(() => document.querySelector('[data-state]')?.textContent === 'Paused');
  assert.equal(session.kernel.routing.routes.size, 0, 'a paused subscription holds no route in the kernel');
  assert.equal(await page.textContent(`[data-pause="${id}"]`), 'Resume');
  await page.click(`[data-pause="${id}"]`);
  await page.waitForFunction(() => document.querySelector('[data-state]')?.textContent === 'Active');
  assert.equal(session.kernel.routing.routes.size, 1, 'resumed, its route is back');
  await page.click(`[data-delete="${id}"]`);
  assert.equal(await page.locator('[data-subscription]').count(), 1, 'the first click only asks');
  assert.match(await page.textContent(`[data-delete="${id}"]`), /Delete this subscription\?/);
  await page.click(`[data-delete="${id}"]`);
  await page.waitForSelector('text=No subscriptions');
  assert.equal(session.kernel.routing.routes.size, 0);
});

test('editing keeps the id, changes what was typed and offers no one-shot', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await fillForm(page, { name: 'Before', repos: REPO, events: [] });
  await page.click('[data-save]');
  await page.waitForSelector('[data-subscription]');
  const id = await page.getAttribute('[data-subscription]', 'data-subscription');
  await page.click(`[data-edit="${id}"]`);
  assert.equal(await page.locator('[name=oncePr]').count(), 0);
  assert.equal(await page.inputValue('[name=name]'), 'Before');
  await page.fill('[name=name]', 'After');
  await page.click('summary:has-text("Only when")');
  await page.fill('[name=labels]', 'bug, !wip');
  await page.click('[data-save]');
  await page.waitForFunction(() => document.querySelector('[data-subscription-name]')?.textContent === 'After');
  const update = session.tools.find(call => call.name === 'update_subscription').input;
  assert.equal(update.id, id);
  assert.deepEqual(update.filters.labels, ['bug', '!wip']);
});

test('the last deliveries of a subscription open in its row', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await fillForm(page, { name: 'Seen', repos: REPO, events: ['pull_request.opened'] });
  await page.click('[data-save]');
  await page.waitForSelector('[data-subscription]');
  await page.click('[data-subscription] summary');
  await page.waitForSelector('text=Nothing delivered or filtered yet.');
});

test('a one-shot for a pull request is created with its end time', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await page.click('[data-new-subscription]');
  await page.fill('[name=name]', 'Wait for CI');
  await page.check('[data-event="ci.finished"]');
  await page.click('summary:has-text("Only once")');
  await page.fill('[name=oncePr]', `${REPO}#2`);
  await page.fill('[name=onceHours]', '2');
  await page.click('[data-save]');
  await page.waitForSelector('[data-subscription]');
  const once = created(session)[0].once;
  assert.deepEqual(once.pr, { repo: REPO, number: 2 });
  assert.equal(typeof once.expiresAt, 'string', 'the tool declares expiresAt as a date-time text');
  assert.ok(Math.abs(Date.parse(once.expiresAt) - (Date.now() + 2 * 3600_000)) < 60_000);
  assert.match(await page.textContent('[data-subscription]'), /One-shot · .*#2/);
});

test('without a connection to GitHub events the tab says nothing is delivered and offers to connect', { skip }, async () => {
  const routing = kernelRouting();
  routing.connectionStatus = async () => ({ status: 'not_connected' });
  const session = await window({ routing });
  const { page } = session;
  await page.waitForSelector('[data-connect]');
  await page.click('[data-connect]');
  await page.waitForFunction(() => document.querySelector('[data-connect]').textContent.startsWith('https://github.com/'));
  await shot(session, 'subscriptions-not-connected', 1);
});

// Pictures for review, only when CHARIOX_SHOTS names a directory outside the repository.
for (const dpr of [1, 2]) {
  test(`pictures of the tab with subscriptions, deliveries and the form at device pixel ratio ${dpr}`, { skip: skip || !shots }, async () => {
    const session = await window({ dpr });
    const { page } = session;
    const ctx = actor => ({ fetch: session.double.fetch, signal: new AbortController().signal, actor, agentId: actor?.id, roomId: 'room-1' });
    const person = { kind: 'human', id: 'owner' };
    await session.app.tools.create_subscription({ name: 'Pull requests of the drill', scope: { kind: 'repos', repos: [REPO] }, events: ['pull_request.opened', 'pull_request.merged'], filters: { labels: ['bug'] }, target: { kind: 'person' } }, ctx(person));
    await session.app.tools.create_subscription({ name: 'Wait for CI', scope: { kind: 'repos', repos: [REPO] }, events: ['ci.finished'], target: { kind: 'agent' } }, ctx({ kind: 'agent', id: 'agent-7' }));
    await session.app.tools.create_subscription({ scope: { kind: 'org', org: 'charioxai' }, events: ['mention', 'issue.opened'], target: { kind: 'room', agentId: 'agent-3', roomId: 'room-9' } }, ctx(person));
    await session.app.events.pull_request_opened(payload('pull_request_opened', pullRequest({ labels: ['bug'] })), { ...ctx(null), occurrenceId: 'o1' });
    await session.app.events.pull_request_opened(payload('pull_request_opened', pullRequest({ number: 3 })), { ...ctx(null), occurrenceId: 'o2' });
    await page.waitForSelector('[data-subscription]', { timeout: 12000 });
    await page.waitForFunction(() => document.querySelectorAll('[data-delivery]').length >= 2, null, { timeout: 12000 });
    await page.click('[data-subscription] summary');
    await page.waitForSelector('[data-subscription] [data-delivery]');
    await shot(session, 'subscriptions-list', dpr);
    await page.click('[data-new-subscription]');
    await page.click('summary:has-text("Only when")');
    await page.waitForSelector('form[data-form=create]');
    await shot(session, 'subscriptions-form', dpr);
  });
}
