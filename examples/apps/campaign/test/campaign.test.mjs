import assert from 'node:assert/strict';
import test from 'node:test';
import { createHash } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import { harness, viewNodes } from './harness.mjs';
import { FRESH_MS, OFFLINE_MS, REFRESH_MS, SOURCE } from '../bundle/runtime/main.mjs';
import { mountCampaignView } from '../bundle/ui/app.mjs';

async function releaseDigest() {
  const hash = createHash('sha256');
  async function visit(url) {
    for (const entry of (await readdir(url, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      const child = new URL(entry.name + (entry.isDirectory() ? '/' : ''), url);
      if (entry.isDirectory()) await visit(child);
      else { hash.update(child.pathname); hash.update(await readFile(child)); }
    }
  }
  await visit(new URL('../bundle/', import.meta.url));
  hash.update(await readFile(new URL('../app.json', import.meta.url)));
  return hash.digest('hex');
}

test('opening refresh and timed open-view refresh preserve edits and installed code', async t => {
  const h = await harness(t);
  const before = await releaseDigest();
  const ui = viewNodes();
  const view = mountCampaignView({ ...ui, bridge: { call: h.read }, now: () => h.at });
  t.after(() => view.close());
  await view.refresh();
  assert.equal(ui.nodes.get('campaign-title').textContent, 'Autumn campaign');
  const draft = ui.nodes.get('draft');
  draft.value = 'Keep my unfinished reply';
  draft.selectionStart = 5; draft.selectionEnd = 7;
  draft.listeners.get('input')();
  h.at += REFRESH_MS;
  h.data = { ...h.data, title: 'Winter campaign', body: '<script>untrusted()</script>' };
  await ui.runTimer(5000);
  assert.equal(ui.nodes.get('campaign-title').textContent, 'Winter campaign');
  assert.equal(ui.nodes.get('campaign-body').textContent, '<script>untrusted()</script>');
  assert.equal(ui.nodes.get('draft'), draft);
  assert.equal(draft.value, 'Keep my unfinished reply');
  assert.equal(draft.selectionStart, 5); assert.equal(draft.selectionEnd, 7);
  assert.equal(ui.document.activeElement, draft);
  assert.equal(h.attempts, 2);
  assert.equal(await releaseDigest(), before);
  view.close();
  const reopened = mountCampaignView({ ...ui, bridge: { call: h.read }, now: () => h.at });
  t.after(() => reopened.close());
  assert.equal(draft.value, 'Keep my unfinished reply', 'Session draft survives reopening');
});

test('offline cache survives worker restart with explicit freshness and bounded expiry', async t => {
  const h = await harness(t);
  const first = await h.read();
  assert.equal(first.status, 'fresh');
  assert.equal(first.freshUntilMs, h.at + FRESH_MS);
  assert.equal(first.offlineUntilMs, h.at + OFFLINE_MS);
  h.at += FRESH_MS;
  h.offline = true;
  await h.restart();
  const offline = await h.read();
  assert.equal(offline.status, 'stale');
  assert.equal(offline.refreshFailed, true);
  assert.deepEqual(offline.campaign, first.campaign);
  assert.equal(offline.fetchedAtMs, first.fetchedAtMs);
  h.at = first.offlineUntilMs;
  const expired = await h.read();
  assert.equal(expired.status, 'expired');
  assert.equal(expired.campaign, null);
  assert.equal(expired.offlineUntilMs, first.offlineUntilMs, 'Failures never extend expiry');
});

test('campaign starts and ends apply to offline data at the exact boundaries', async t => {
  const h = await harness(t);
  const start = h.at + 10_000;
  const end = start + 10_000;
  h.data = { ...h.data, startsAtMs: start, endsAtMs: end };
  assert.equal((await h.read()).status, 'scheduled');
  h.offline = true;
  h.at = start;
  assert.equal((await h.read()).campaign.id, 'autumn');
  h.at = end;
  const expired = await h.read();
  assert.equal(expired.status, 'expired');
  assert.equal(expired.campaign, null);
  assert.equal(expired.offlineUntilMs, end);
  assert.equal(h.attempts, 1);
});

test('failure retries, open views, agents and notifications share a durable rate bound', async t => {
  const h = await harness(t);
  h.offline = true;
  assert.equal((await h.read()).status, 'unavailable');
  for (let i = 0; i < 8; i++) await h.read();
  await h.notify(); await h.restart(); await h.read();
  assert.equal(h.attempts, 1);
  h.at += REFRESH_MS;
  h.offline = false;
  await h.notify();
  assert.equal((await h.read()).status, 'fresh');
  assert.equal(h.attempts, 2);
  assert.ok(!h.methods.some(method => method.startsWith('schedule.')));
});

test('concurrent human and agent reads plus notification delivery make one fetch', async t => {
  const h = await harness(t);
  let release;
  h.gate = new Promise(resolve => { release = resolve; });
  const calls = [h.read(), h.read(), h.notify()];
  await new Promise(resolve => setImmediate(resolve));
  release();
  const [human, agent, notification] = await Promise.all(calls);
  assert.deepEqual(human, agent);
  assert.equal(notification, null);
  assert.equal(h.attempts, 1);
  assert.equal(h.streams.size, 0, 'Buffered SDK request closes its stream');
});

test('state conflicts retry before HTTP and never discard the previous cache', async t => {
  const h = await harness(t);
  h.conflicts = 2;
  assert.equal((await h.read()).status, 'fresh');
  assert.equal(h.attempts, 1);
  h.at += REFRESH_MS;
  h.conflicts = 5;
  await assert.rejects(h.read(), { code: 'CONFLICT' });
  assert.equal(h.attempts, 1);
  h.offline = true;
  assert.equal((await h.read()).campaign.title, 'Autumn campaign');
});

test('invalid, oversized, executable and non-200 responses retain the last good data', async t => {
  const h = await harness(t);
  const first = await h.read();
  for (const raw of ['{broken', JSON.stringify({ ...h.data, script: 'run()' }),
    JSON.stringify({ ...h.data, endsAtMs: h.data.startsAtMs }),
    JSON.stringify({ ...h.data, body: 'x'.repeat(4001) }), 'x'.repeat(16 * 1024 + 1),
    Buffer.from([0xff])]) {
    h.at += REFRESH_MS; h.raw = raw;
    const result = await h.read();
    assert.equal(result.refreshFailed, true);
    assert.deepEqual(result.campaign, first.campaign);
    assert.equal(result.offlineUntilMs, first.offlineUntilMs);
    assert.equal(h.streams.size, 0);
  }
  h.at += REFRESH_MS; h.raw = undefined; h.status = 503;
  assert.equal((await h.read()).refreshFailed, true);
  h.at += REFRESH_MS; h.status = 200; h.denied = true;
  assert.equal((await h.read()).refreshFailed, true);
});

test('offline view shows cache status, hides expired data and retains its draft', async t => {
  const h = await harness(t);
  const ui = viewNodes();
  const view = mountCampaignView({ ...ui, bridge: { call: h.read }, now: () => h.at });
  t.after(() => view.close());
  await view.refresh();
  const draft = ui.nodes.get('draft');
  draft.value = 'Unsent draft';
  h.offline = true; h.at += FRESH_MS;
  await view.refresh();
  assert.match(ui.nodes.get('status').textContent, /Offline cache/);
  assert.equal(ui.nodes.get('campaign-title').textContent, 'Autumn campaign');
  h.at += OFFLINE_MS;
  await view.refresh();
  assert.equal(ui.nodes.get('campaign-body').textContent, '');
  assert.match(ui.nodes.get('status').textContent, /expired/);
  assert.equal(draft.value, 'Unsent draft');
});

test('backend failure clears content and closing suppresses late responses and polling', async () => {
  const ui = viewNodes();
  ui.nodes.get('campaign-body').textContent = 'Previously cached campaign';
  const failed = mountCampaignView({ ...ui, bridge: { call: async () => { throw new Error('disconnected'); } } });
  await failed.refresh();
  assert.equal(ui.nodes.get('campaign-body').textContent, '');
  failed.close();
  let finish;
  const view = mountCampaignView({ ...ui, bridge: { call: () => new Promise(resolve => { finish = resolve; }) } });
  const pending = view.refresh();
  await Promise.resolve();
  view.close();
  finish({ campaign: { title: 'late', body: 'late' }, status: 'fresh' });
  await pending;
  assert.notEqual(ui.nodes.get('campaign-title').textContent, 'late');
  assert.equal(ui.timers.length, 0);
});

test('hidden views stop work and repeated refreshes join the current view request', async () => {
  const ui = viewNodes();
  ui.document.hidden = true;
  let calls = 0;
  const view = mountCampaignView({ ...ui, bridge: { call: async () => {
    calls += 1; return { campaign: null, status: 'unavailable' };
  } } });
  await ui.runTimer(5000);
  assert.equal(calls, 0);
  ui.document.hidden = false;
  await Promise.all([view.refresh(), view.refresh()]);
  assert.equal(calls, 1);
  view.close();
});

test('displayed campaign expires during an unresolved bridge call without touching the draft', async t => {
  const h = await harness(t);
  h.data.endsAtMs = h.at + 1000;
  const ui = viewNodes();
  let stalled = false;
  const view = mountCampaignView({ ...ui, now: () => h.at, bridge: { call: () =>
    stalled ? new Promise(() => {}) : h.read() } });
  t.after(() => view.close());
  await view.refresh();
  const draft = ui.nodes.get('draft');
  draft.value = 'Keep this draft'; draft.selectionStart = 2; draft.selectionEnd = 6;
  stalled = true;
  const pending = view.refresh();
  h.at += 1000;
  await ui.runTimer(1000);
  assert.equal(ui.nodes.get('campaign-body').textContent, '');
  assert.match(ui.nodes.get('status').textContent, /expired/);
  assert.equal(draft.value, 'Keep this draft');
  assert.equal(draft.selectionStart, 2); assert.equal(draft.selectionEnd, 6);
  assert.equal(ui.document.activeElement, draft);
  await ui.runTimer(10_000);
  await pending;
});

test('visibility resume expires content even if browser timers were suspended', async t => {
  const h = await harness(t);
  h.data.endsAtMs = h.at + 1000;
  const ui = viewNodes();
  let stalled = false;
  const view = mountCampaignView({ ...ui, now: () => h.at, bridge: { call: () =>
    stalled ? new Promise(() => {}) : h.read() } });
  await view.refresh();
  stalled = true; h.at += 2000;
  ui.listeners.get('visibilitychange')();
  assert.equal(ui.nodes.get('campaign-body').textContent, '');
  assert.match(ui.nodes.get('status').textContent, /expired/);
  view.close();
  assert.equal(ui.listeners.size, 0);
});

test('bridge waits time out, recover after one lost reply, and cap unresolved requests', async () => {
  const ui = viewNodes();
  ui.document.hidden = true;
  let calls = 0;
  let healthy = false;
  const view = mountCampaignView({ ...ui, now: () => 1000, bridge: { call: () => {
    calls += 1;
    return healthy ? Promise.resolve({ campaign: { title: 'Recovered', body: 'Current', endsAtMs: 10_000 },
      offlineUntilMs: 10_000, status: 'fresh' }) : new Promise(() => {});
  } } });
  let pending = view.refresh();
  await Promise.resolve();
  await ui.runTimer(10_000); await pending;
  assert.match(ui.nodes.get('status').textContent, /Could not contact/);
  healthy = true; ui.document.hidden = false;
  await ui.runTimer(5000);
  assert.equal(ui.nodes.get('campaign-title').textContent, 'Recovered');
  healthy = false;
  pending = view.refresh();
  await Promise.resolve();
  await ui.runTimer(10_000); await pending;
  const before = calls;
  await view.refresh(); await view.refresh();
  assert.equal(calls, before, 'At most two lost bridge calls may remain pending');
  view.close();
});

test('signed declarations bind the fixed source and both registered handlers', async () => {
  const manifest = JSON.parse(await readFile(new URL('../app.json', import.meta.url)));
  const tools = JSON.parse(await readFile(new URL('../bundle/schemas/tools.json', import.meta.url)));
  const events = JSON.parse(await readFile(new URL('../bundle/schemas/events.json', import.meta.url)));
  assert.deepEqual(manifest.capabilities.network, [{ origin: new URL(SOURCE).origin, methods: ['GET'] }]);
  assert.equal(manifest.sdkVersion, '0.8.0');
  assert.equal(manifest.runtime.entry, 'runtime/main.mjs');
  assert.deepEqual(tools.tools.map(tool => tool.name), ['read_campaign']);
  assert.equal(tools.tools[0].inputSchema.additionalProperties, false);
  assert.equal(events.events[0].name, 'campaign_changed');
  assert.equal(events.events[0].direction, 'incoming');
});
