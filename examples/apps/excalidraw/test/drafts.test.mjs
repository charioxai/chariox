// The drafts of replaced windows against a fake backend that enforces what the real App and the
// catalog enforce: at most four drafts of other windows, and 512 KiB per call and per result.
import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { openApp, closeEnvironment, fakeBackend, rect, sceneOf, waitUntil } from './window.mjs';

after(closeEnvironment);

/** A rectangle that carries `kib` KiB of data, so a scene of a few of them is near the 192 KiB scene limit. */
const bulky = (id, kib) => rect(id, 100, 100, 120, 80, { customData: { pad: 'x'.repeat(kib * 1024) } });

for (const dpr of [1, 2]) test(`three near-limit drafts are all recovered at DPR${dpr}`, { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  // Three windows that were replaced with a 174 KiB edit each: together over the catalog's 512 KiB call limit.
  for (const n of [1, 2, 3]) {
    backend.revisions.set(`older-${n}`, n);
    backend.drafts.set(`older-${n}`, sceneOf([bulky('big-a', 58), bulky('big-b', 58), bulky('big-c', 58), rect(`mark-${n}`, 100 * n, 400)]));
  }
  const app = await openApp({ backend, dpr });
  try {
    await app.load();
    await app.until(elements => ['mark-1', 'mark-2', 'mark-3'].every(id => elements.some(element => element.id === id)), { timeout: 15_000, what: 'the marks of all three drafts' });
    await waitUntil(() => backend.live().length === 6, { what: 'the recovered edits to be saved' });
    app.assertClean();
  } finally { await app.close(); }
});

// Each window edits, and is replaced before its scene save commits (the other windows) or after it (every third
// windows). Every window leaves a draft of its own behind until a later window recovers it.
for (const dpr of [1, 2]) test(`seven consecutive replacements lose no edit and always find room for a draft at DPR${dpr}`, { timeout: 240_000 }, async () => {
  const backend = fakeBackend();
  let app;
  try {
    for (let n = 1; n <= 7; n++) {
      app = await openApp({ backend, dpr });
      await app.load();
      await waitUntil(() => backend.live().length === n - 1, { what: 'the recovered edits to be saved' });
      const committed = n % 3 === 0;
      if (!committed) backend.holdSaves = 'lose';
      await app.draw(300 + 60 * n, 200);
      if (committed) await waitUntil(() => backend.live().length === n, { what: `window ${n}'s save` });
      else await waitUntil(() => [...backend.drafts.values()].some(scene => scene.elements.length === n), { what: `window ${n}'s draft` });
      await app.close();
      backend.holdSaves = null;
    }
    app = await openApp({ backend, dpr });
    await app.load();
    await app.until(elements => elements.length === 7, { timeout: 15_000, what: 'all seven rectangles' });
    await waitUntil(() => backend.live().length === 7, { what: 'all seven to be saved' });
    assert(backend.drafts.size <= 1, `${backend.drafts.size} drafts are kept`);
  } finally { await app?.close(); }
});

test('a refused draft write is shown to the person', { timeout: 60_000 }, async () => {
  const backend = fakeBackend();
  backend.failDraftSaves = '4 other windows already keep an unsaved diagram; save or close one, or wait until it is an hour old';
  const app = await openApp({ backend });
  try {
    await app.load();
    await app.draw();
    await waitUntil(async () => (await app.toast.allTextContents()).join(' ').includes('4 other windows already keep'), { what: 'the notice' });
  } finally { await app.close(); }
});

for (const dpr of [1, 2]) test(`a background-only edit is recovered with the replaced window at DPR${dpr}`, { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  const first = await openApp({ backend, dpr });
  let second;
  try {
    await first.load();
    backend.holdSaves = 'lose';
    await first.setBackground('#fffce8');
    await waitUntil(() => backend.drafts.size === 1, { what: 'the edit to be kept' });
    await first.close();
    backend.holdSaves = null;
    second = await openApp({ backend, dpr });
    await second.load();
    await waitUntil(async () => (await second.settings()).viewBackgroundColor === '#fffce8', { timeout: 15_000, what: 'the background to come back' });
    await waitUntil(() => backend.scene?.appState.viewBackgroundColor === '#fffce8', { what: 'the background to be saved' });
    second.assertClean();
  } finally { await first.close(); await second?.close(); }
});

test('a recovered setting overrides the same saved setting and keeps the other settings an agent saved meanwhile', { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  const first = await openApp({ backend });
  let second;
  try {
    await first.load();
    backend.holdSaves = 'lose';
    await first.setBackground('#fffce8');
    await waitUntil(() => backend.drafts.size === 1, { what: 'the edit to be kept' });
    await first.close();
    backend.holdSaves = null;
    backend.agentEdit(scene => Object.assign(scene.appState, { viewBackgroundColor: '#f5faff', gridModeEnabled: true }));
    second = await openApp({ backend });
    await second.load();
    await waitUntil(async () => (await second.settings()).viewBackgroundColor === '#fffce8', { timeout: 15_000, what: 'the person\'s background to win' });
    assert.equal((await second.settings()).gridModeEnabled, true, 'the agent\'s grid setting stays');
    await waitUntil(() => backend.scene.appState.viewBackgroundColor === '#fffce8' && backend.scene.appState.gridModeEnabled === true, { what: 'both settings to be saved' });
  } finally { await first.close(); await second?.close(); }
});
