// The drafts of replaced windows against a fake backend that enforces what the catalog enforces:
// 512 KiB per call and per result. Each row of the draft's state table that the window decides has
// a test here or in view.test.mjs.
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

const QUOTA = 'The App\'s data directory is over its quota.';

test('a refused draft write is shown to the person', { timeout: 60_000 }, async () => {
  const backend = fakeBackend();
  backend.failDraftSaves = QUOTA;
  const app = await openApp({ backend });
  try {
    await app.load();
    await app.draw();
    await waitUntil(async () => (await app.toast.allTextContents()).join(' ').includes(QUOTA), { what: 'the notice' });
  } finally { await app.close(); }
});

// Review P2: a recovering window whose own draft write is refused still merges, saves and retires
// what it recovered; retiring waits for the scene save only.
for (const dpr of [1, 2]) test(`a refused draft write does not block recovery at DPR${dpr}`, { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  backend.holdSaves = 'lose';
  const first = await openApp({ backend, dpr });
  let second;
  try {
    await first.load();
    await first.draw();
    await waitUntil(() => backend.drafts.size === 1, { what: 'the edit to be kept' });
    const [replacedId] = backend.drafts.keys();
    await first.close();
    backend.holdSaves = null;
    backend.failDraftSaves = QUOTA;
    // The recovering window's save waits until its own draft write has been refused.
    let release;
    backend.gate = new Promise(resolve => { release = resolve; });
    const refused = backend.count('save_draft');
    second = await openApp({ backend, dpr });
    await second.load();
    await waitUntil(() => backend.count('save_draft') > refused, { what: 'the recovering window\'s own draft write to be refused' });
    await waitUntil(async () => (await second.toast.allTextContents()).join(' ').includes(QUOTA), { what: 'the notice' });
    await second.shot('quota-refused-during-recovery');
    backend.gate = null;
    release();
    await waitUntil(() => backend.live().length === 1, { what: 'the recovered edit to be saved' });
    await waitUntil(() => !backend.drafts.has(replacedId), { what: 'the replaced window\'s draft to be retired after the save' });
    assert.equal(backend.drafts.size, 0);
    second.assertClean();
  } finally { await first.close(); await second?.close(); }
});

// Review P1: the person reverts the background while the save of the changed one is in flight. That
// save then lands, and the window is replaced before it saves the revert: the revert must come back.
for (const dpr of [1, 2]) test(`a background reverted while a save is in flight survives the replacement at DPR${dpr}`, { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  const first = await openApp({ backend, dpr, scene: sceneOf([rect('a')], { viewBackgroundColor: '#ffffff' }) });
  let second, release;
  try {
    await first.load();
    backend.gate = new Promise(resolve => { release = resolve; });
    const saves = backend.saves.length;
    await first.setBackground('#fffce8');
    await waitUntil(() => backend.saves.length > saves, { what: 'the save of the new background to start' });
    const writes = backend.count('save_draft') + backend.count('discard_draft');
    await first.setBackground('#ffffff');
    await waitUntil(() => backend.count('save_draft') + backend.count('discard_draft') > writes, { what: 'the draft write after the revert' });
    // The save in flight lands; the save of the revert never does.
    backend.holdSaves = 'lose';
    backend.gate = null;
    release();
    await waitUntil(() => backend.scene.appState.viewBackgroundColor === '#fffce8', { what: 'the save in flight to land' });
    await waitUntil(() => backend.saves.length > saves + 1, { what: 'the save of the revert to start' });
    await first.shot('revert-in-flight-before-replacement');
    await first.close();
    backend.holdSaves = null;
    second = await openApp({ backend, dpr });
    await second.load();
    await waitUntil(async () => (await second.settings()).viewBackgroundColor === '#ffffff', { timeout: 15_000, what: 'the reverted background to come back' });
    await waitUntil(() => backend.scene.appState.viewBackgroundColor === '#ffffff', { what: 'the reverted background to be saved' });
    await waitUntil(() => backend.drafts.size === 0, { what: 'the replaced window\'s draft to be retired' });
    await second.shot('revert-in-flight-after-replacement');
    second.assertClean();
  } finally { release?.(); await first.close(); await second?.close(); }
});

// A second window: two windows open on the document after one was replaced, and both recover its
// draft. Both merge and save; whichever retires first removes it, and nothing is lost or left.
for (const dpr of [1, 2]) test(`two windows on one document both recover the replaced window's edit at DPR${dpr}`, { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  backend.holdSaves = 'lose';
  const replaced = await openApp({ backend, dpr });
  let left, right;
  try {
    await replaced.load();
    await replaced.setBackground('#fffce8');
    await replaced.draw(300, 200);
    await waitUntil(() => backend.drafts.size === 1 && [...backend.drafts.values()][0].elements.length === 1, { what: 'the edit to be kept' });
    await replaced.close();
    backend.holdSaves = null;
    left = await openApp({ backend, dpr });
    right = await openApp({ backend, dpr });
    await Promise.all([left.load(), right.load()]);
    for (const window of [left, right]) {
      await window.until(elements => elements.length === 1, { timeout: 15_000, what: 'the recovered rectangle in both windows' });
      assert.equal((await window.settings()).viewBackgroundColor, '#fffce8');
    }
    await waitUntil(() => backend.live().length === 1 && backend.scene.appState.viewBackgroundColor === '#fffce8', { what: 'the recovered edit to be saved' });
    await waitUntil(() => backend.drafts.size === 0, { what: 'every draft to be retired or discarded' });
    await left.shot('two-windows-recovered-left');
    left.assertClean(); right.assertClean();
  } finally { await replaced.close(); await left?.close(); await right?.close(); }
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

// The merge policy for document settings: they carry no versions, so a recovered draft's settings
// replace the saved ones, as the replaced window's own save would have; the newest draft's win.
test('a recovered draft\'s settings replace the saved ones and the newest draft\'s win', { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  backend.agentEdit(scene => Object.assign(scene.appState, { viewBackgroundColor: '#f5faff', gridModeEnabled: true }));
  backend.drafts.set('older', sceneOf([rect('o', 100, 100)], { viewBackgroundColor: '#fdf8f6', gridModeEnabled: false }));
  backend.revisions.set('older', 1);
  backend.drafts.set('newer', sceneOf([rect('n', 400, 100)], { viewBackgroundColor: '#fffce8', gridModeEnabled: false }));
  backend.revisions.set('newer', 2);
  const app = await openApp({ backend });
  try {
    await app.load();
    await app.until(elements => elements.length === 2, { timeout: 15_000, what: 'both drafts\' rectangles' });
    const settings = await app.settings();
    assert.equal(settings.viewBackgroundColor, '#fffce8', 'the newest draft\'s background');
    assert.equal(settings.gridModeEnabled, false, 'the drafts\' grid setting replaces the saved one');
    await waitUntil(() => backend.scene.appState.viewBackgroundColor === '#fffce8' && backend.scene.appState.gridModeEnabled === false, { what: 'the recovered settings to be saved' });
    await waitUntil(() => backend.drafts.size === 0, { what: 'both drafts to be retired' });
  } finally { await app.close(); }
});
