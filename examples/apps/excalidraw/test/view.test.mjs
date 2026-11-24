// The Excalidraw window against a fake backend: what a person draws is saved with the
// backend's version check, and what an agent does in the backend shows up in the open window.
import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { openApp, closeEnvironment, fakeBackend, labelled, rect, sceneOf, text, waitUntil } from './window.mjs';

after(closeEnvironment);

const clone = value => JSON.parse(JSON.stringify(value));
const idle = ms => new Promise(done => setTimeout(done, ms));

async function withApp(options, body) {
  const app = await openApp(options);
  try { await app.load(); await body(app); app.assertClean(); } finally { await app.close(); }
}

for (const dpr of [1, 2]) test(`a drawn rectangle is saved with the version check and survives a reload at DPR${dpr}`, { timeout: 60_000 }, () => withApp({ dpr }, async app => {
  const { backend, page } = app;
  await idle(1500);
  // A diagram that was never saved is saved once, empty, when it is first shown.
  const opened = backend.saves.length;
  assert(opened <= 1, `opening saved ${opened} times`);
  if (opened) assert.deepEqual(backend.saves[0].scene.elements, []);
  await app.draw();
  await waitUntil(() => backend.saves.length === opened + 1, { what: 'the first save of the drawing' });
  assert.equal(backend.saves[opened].version, opened ? '1' : null, 'a save carries the version it read');
  assert.deepEqual(backend.live().map(element => element.type), ['rectangle']);
  const rectangle = backend.live()[0];
  const saved = backend.saves.length;
  const version = backend.version;

  // Selecting, hovering and scrolling change the canvas, not the diagram.
  await page.keyboard.press('v');
  await page.mouse.click(rectangle.x + 40, rectangle.y);
  await page.mouse.move(700, 500, { steps: 10 });
  for (let i = 0; i < 5; i++) { await page.mouse.wheel(0, 20); await page.mouse.move(600 + i * 10, 450); }
  await idle(1500);
  assert.equal(backend.saves.length, saved, 'selection, hover and scroll must not save');

  await page.reload();
  await page.waitForSelector('.excalidraw__canvas.interactive');
  await idle(1500);
  assert.equal(backend.saves.length, saved, 'an unchanged loaded diagram must not write');
  assert.equal(backend.version, version);
  assert.deepEqual((await app.elements()).map(element => element.type), ['rectangle'], 'the reload keeps the rectangle');

  await app.draw(600, 400);
  await waitUntil(() => backend.saves.length === saved + 1, { what: 'the next save' });
  assert.equal(backend.saves[saved].version, version, 'a save carries the version it read');
  assert.equal(backend.live().length, 2);
  await app.shot('canvas');
}));

test('an agent-made diagram opens without a write, bound labels included', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([...labelled('client', 'Client', 100, 100), ...labelled('kernel', 'Kernel', 400, 100)]),
}, async app => {
  const { backend } = app;
  await idle(2500);
  assert.equal(backend.count('save_scene'), 0, 'opening must not bump the version');
  assert.equal(backend.version, '1');
  assert.deepEqual(await app.ids(), ['client', 'client-text', 'kernel', 'kernel-text']);
}));

test('an agent adds a labelled box and edits another: the open window shows both without owner input', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('existing', 100, 400)]),
}, async app => {
  const { backend } = app;
  await app.canvas.waitFor();
  backend.agentEdit(scene => {
    scene.elements.push(...labelled('client', 'Client', 300, 100));
    scene.elements.find(element => element.id === 'existing').x = 700;
  });
  const took = await app.until(elements => elements.some(element => element.id === 'client-text')
    && elements.find(element => element.id === 'existing').x === 700, { what: 'the agent edit on the canvas' });
  assert(took < 3500, `the edit took ${took} ms to show`);
  const elements = await app.elements();
  const label = elements.find(element => element.id === 'client-text');
  assert.equal(label.containerId, 'client');
  assert.deepEqual(elements.find(element => element.id === 'client').boundElements.map(bound => bound.id), ['client-text']);
  // The label is drawn, not only present in the scene.
  await app.menu('Export SVG');
  await waitUntil(() => backend.files.has('diagram.svg'), { what: 'the SVG' });
  assert(backend.files.get('diagram.svg').toString().includes('Client'));
  assert.equal(backend.count('save_scene'), 0, 'merging what the backend saved must not save it back');
  await app.shot('agent-added');
}));

test('an owner drawing while an agent adds an element loses neither edit', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('existing', 100, 500)]),
}, async app => {
  const { backend } = app;
  await app.canvas.waitFor();
  await app.draw(300, 150);
  // The owner's save is still waiting for its 300 ms timer: the backend moves on first.
  backend.agentEdit(scene => scene.elements.push(rect('agent-box', 800, 150)));
  await waitUntil(() => backend.live().length === 3, { timeout: 5000, what: 'both edits in the saved diagram' });
  const saved = backend.live().map(element => element.id);
  assert(saved.includes('existing') && saved.includes('agent-box'));
  assert.equal(saved.filter(id => id !== 'existing' && id !== 'agent-box').length, 1, 'and the owner rectangle');
  await app.until(elements => elements.length === 3, { what: 'both edits on the canvas' });
  assert.deepEqual(await app.ids(), saved.slice().sort());
  assert.equal(await app.toast.filter({ hasText: /changed by another/ }).count(), 0, 'a refused save is merged, not reported');
}));

test('a save refused for a conflict is merged and saved again', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('existing', 100, 500)]),
}, async app => {
  const { backend } = app;
  await app.canvas.waitFor();
  backend.interleave = () => backend.agentEdit(scene => scene.elements.push(rect('agent-box', 800, 150)));
  await app.draw(300, 150);
  await waitUntil(() => backend.saves.length >= 2, { what: 'the retried save' });
  assert.equal(backend.saves.length, 2, 'one refused save, one retry');
  assert.equal(backend.saves[0].version, '1');
  assert.equal(backend.saves[1].version, '2', 'the retry carries the version the merge read');
  assert.equal(backend.live().length, 3);
  assert(backend.live().some(element => element.id === 'agent-box'), 'the agent edit stays saved');
  assert.equal(await app.toast.count(), 0, 'no notice for a conflict that was resolved');
  assert.equal((await app.elements()).length, 3);
  await idle(1500);
  assert.equal(backend.saves.length, 2, 'the merged diagram is saved once');
}));

test('a window that cannot catch up with the backend gives up after four refused saves and says so', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('existing', 100, 500)]),
}, async app => {
  const { backend } = app;
  await app.canvas.waitFor();
  let n = 0;
  const raceAgain = () => { backend.interleave = raceAgain; backend.agentEdit(scene => scene.elements.push(rect(`agent-${++n}`, 800, 100 + n * 30))); };
  backend.interleave = raceAgain;
  await app.draw(300, 150);
  await app.toast.waitFor();
  assert.match(await app.toast.innerText(), /changed by another window or an agent/);
  assert.equal(backend.saves.length, 4);
  backend.interleave = null;
  await idle(1500);
  assert.equal(backend.saves.length, 4, 'no further attempts after giving up');
  assert.equal(backend.live().filter(element => element.id.startsWith('agent-')).length, 4, 'every agent edit stays saved');
}));

test('an element the agent deleted leaves the window and does not come back when the window saves', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('a', 100, 100), rect('b', 300, 100), rect('c', 500, 100)]),
}, async app => {
  const { backend } = app;
  await app.canvas.waitFor();
  assert.deepEqual(await app.ids(), ['a', 'b', 'c']);
  backend.agentEdit(scene => { scene.elements = scene.elements.filter(element => element.id !== 'b'); });
  assert(backend.scene.elements.find(element => element.id === 'b').isDeleted, 'the backend keeps a marked element');
  await app.until(elements => elements.length === 2, { what: 'the deleted element to leave the canvas' });
  assert.deepEqual(await app.ids(), ['a', 'c']);
  assert.equal(backend.count('save_scene'), 0);

  await app.draw(300, 400);
  await waitUntil(() => backend.saves.length === 1, { what: 'the window save' });
  assert.equal(backend.live().length, 3);
  assert(!backend.live().some(element => element.id === 'b'), 'the saved diagram does not have b');
  await idle(2200);
  const ids = await app.ids();
  assert.equal(ids.length, 3);
  assert(!ids.includes('b'), 'b must not come back');
  // A later agent edit does not bring it back either.
  backend.agentEdit(scene => scene.elements.push(rect('d', 900, 100)));
  await app.until(elements => elements.some(element => element.id === 'd'), { what: 'the next agent element' });
  assert(!(await app.ids()).includes('b'));
}));

const tombstone = (backend, id) => backend.scene.elements.find(element => element.id === id && element.isDeleted);

test('a window that saves keeps the deletion an agent made, so another window still learns of it', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('a', 100, 100), rect('b', 300, 100)]),
}, async app => {
  const { backend } = app;
  await app.canvas.waitFor();
  backend.agentEdit(scene => { scene.elements = scene.elements.filter(element => element.id !== 'b'); });
  await app.until(elements => elements.length === 1, { what: 'the deleted element to leave the canvas' });
  await app.draw(300, 400);
  await waitUntil(() => backend.saves.length === 1, { what: 'the window save' });
  assert(tombstone(backend, 'b'), 'the saved scene still carries b as deleted');
}));

test('an element the owner deletes is saved as deleted, so another window removes it', { timeout: 60_000 }, () => withApp({}, async app => {
  const { backend, page } = app;
  await app.canvas.waitFor();
  await idle(1500);
  await app.draw(300, 200);
  await waitUntil(() => backend.live().length === 1, { what: 'the drawing to be saved' });
  const id = backend.live()[0].id;
  await page.keyboard.press('Delete');
  await waitUntil(() => backend.live().length === 0, { what: 'the deletion to be saved' });
  assert(tombstone(backend, id), 'the deletion is saved as a marked element, not as an absence');
}));

// Excalidraw restores every saved element, deleted ones too, and reads the points of a linear
// one before anything else: a compact deletion record must still look like a valid element.
const stroke = (id, type, x, y) => ({
  ...rect(id, x, y, 120, 60), type, points: [[0, 0], [60, 20], [120, 60]], lastCommittedPoint: null,
  ...(type === 'freedraw' ? { pressures: [], simulatePressure: true } : { startBinding: null, endBinding: null, startArrowhead: null, endArrowhead: type === 'arrow' ? 'arrow' : null }),
});

const shapes = {
  arrow: () => stroke('s', 'arrow', 200, 200),
  line: () => stroke('s', 'line', 200, 200),
  freedraw: () => stroke('s', 'freedraw', 200, 200),
  text: () => text('s', 'Note', 200, 200),
  frame: () => ({ ...rect('s', 200, 200, 300, 200), type: 'frame', name: null }),
};
for (const [type, shape] of Object.entries(shapes)) {
  test(`a deleted ${type} is saved as a record that reopens and that another window merges`, { timeout: 90_000 }, async () => {
    const backend = fakeBackend();
    const scene = sceneOf([shape(), rect('keep', 600, 400)]);
    const first = await openApp({ scene, backend });
    const second = await openApp({ backend });
    try {
      await first.load(); await second.load();
      assert.deepEqual(await second.ids(), ['keep', 's']);
      // Excalidraw handles keys only while it has focus: a click on empty canvas gives it focus.
      await first.page.mouse.click(1000, 700);
      await first.page.keyboard.press('Control+a');
      await first.page.keyboard.press('Delete');
      await waitUntil(() => backend.live().length === 0, { what: 'the deletion to be saved' });
      const record = tombstone(backend, 's');
      assert(record, 'the deletion is saved as a marked element');
      if (type in { arrow: 1, line: 1, freedraw: 1 }) assert(record.points.length >= 2, 'a deleted linear element keeps the points Excalidraw reads');
      assert(JSON.stringify(record).length < 600, 'the record stays compact');

      await second.until(elements => elements.length === 0, { what: 'the second window to merge the deletion' });
      await first.page.reload();
      await first.page.waitForSelector('.excalidraw__canvas.interactive');
      await idle(1500);
      assert.deepEqual(await first.ids(), [], 'the reopened diagram shows no deleted element');
      first.assertClean(); second.assertClean();
    } finally { await first.close(); await second.close(); }
  });
}

test('an agent deleting a labelled box removes its label from the window too', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([...labelled('client', 'Client', 100, 100), rect('other', 500, 100)]),
}, async app => {
  const { backend } = app;
  await app.canvas.waitFor();
  backend.agentEdit(scene => { scene.elements = scene.elements.filter(element => element.id !== 'client' && element.containerId !== 'client'); });
  await app.until(elements => elements.length === 1, { what: 'the labelled box to leave' });
  assert.deepEqual(await app.ids(), ['other']);
}));

test('an agent selection is applied in the window once', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('a', 100, 100), rect('b', 400, 100)]),
}, async app => {
  const { backend, page } = app;
  await app.canvas.waitFor();
  // Excalidraw handles keys only while it has focus: a click on empty canvas gives it focus.
  await page.mouse.click(1000, 700);
  backend.agentEdit(scene => { scene.appState.selectedElementIds = { b: true }; });
  // The selection is observable through what it does: Delete removes the selected element.
  await idle(1800);
  await page.keyboard.press('Delete');
  await waitUntil(() => backend.live().length === 1, { what: 'the selected element to be deleted' });
  assert.deepEqual(backend.live().map(element => element.id), ['a']);
  assert.deepEqual(await app.ids(), ['a']);
}));

test('an agent selection is not applied again after the owner cleared it', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('a', 100, 100), rect('b', 400, 100)]),
}, async app => {
  const { backend, page } = app;
  await app.canvas.waitFor();
  // Excalidraw handles keys only while it has focus: a click on empty canvas gives it focus.
  await page.mouse.click(1000, 700);
  backend.agentEdit(scene => { scene.appState.selectedElementIds = { b: true }; });
  await idle(1800);
  await page.mouse.click(1000, 600);
  await idle(2500);
  await page.keyboard.press('Delete');
  await idle(800);
  assert.deepEqual(await app.ids(), ['a', 'b'], 'nothing is selected, so Delete deletes nothing');
  assert.equal(backend.count('save_scene'), 0);
}));

test('a save that fails for a reason that is not a conflict is reported once and does not loop', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('existing', 100, 500)]),
}, async app => {
  const { backend } = app;
  await app.canvas.waitFor();
  backend.failSaves = 'The data directory is full.';
  await app.draw(300, 150);
  await app.toast.waitFor();
  assert.match(await app.toast.innerText(), /data directory is full/);
  assert.equal(await app.toast.count(), 1, 'one notice');
  assert.equal(await app.page.locator('.Modal').count(), 0, 'no modal traps the canvas');
  assert.equal(backend.saves.length, 1);
  await app.toast.locator('button').click();
  await idle(2500);
  assert.equal(await app.toast.count(), 0, 'the closed notice is not shown again');
  assert.equal(backend.saves.length, 1, 'the failed save is not retried in a loop');

  // The person keeps editing: the unsaved state is reported again, without asking the backend.
  await app.draw(600, 150);
  await app.toast.waitFor();
  assert.match(await app.toast.innerText(), /data directory is full/);
  assert.equal(backend.saves.length, 1);
  await app.toast.locator('button').click();

  // Reloading recovers automatic saving once the backend works again, and the edits that could
  // not be saved come back with the window.
  backend.failSaves = null;
  await app.page.reload();
  await app.canvas.waitFor();
  await app.draw(300, 400);
  await waitUntil(() => backend.live().length === 4, { what: 'saving after the reload' });
  assert.equal(backend.saves.at(-1).scene.elements.length, 4, 'the existing rectangle, the two drawn before the reload and the new one');
  await waitUntil(() => backend.drafts.size === 0, { what: 'the replaced window\'s draft to be retired and the recovering window\'s own discarded once saved' });
  await app.shot('failure-recovery');
}));

// The poll loop never stops and never hides a failure: the window shows the error in its own
// notice, the console logs each distinct error once however many polls meet it, and the missed
// change is merged once the backend answers again.
const malformedScene = { scene: { type: 'excalidraw', version: 2, elements: 5, appState: {}, files: {} }, version: '99' };
for (const [what, tool, outage] of [
  ['the change feed cannot be read', 'view_changes', { message: 'The feed is unreachable.' }],
  ['the saved scene cannot be loaded', 'load_scene', { message: 'The scene is unreachable.' }],
  ['the saved scene cannot be merged', 'load_scene', { reply: malformedScene }],
]) test(`when ${what}, the window says so once, keeps polling and recovers`, { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('a', 100, 100)]),
}, async app => {
  const { backend, logs } = app;
  await app.canvas.waitFor();
  const call = backend.call.bind(backend);
  let current = null, attempts = 0;
  backend.call = async (name, input) => {
    if (name === tool && current) {
      attempts++;
      if (current.message) throw Error(current.message);
      return clone(current.reply);
    }
    return call(name, input);
  };
  current = outage;
  backend.agentEdit(scene => scene.elements.push(rect('c', 700, 100)));
  await app.toast.waitFor();
  const notice = await app.toast.innerText();
  assert.match(notice, /Could not update from the saved diagram/);
  if (outage.message) assert.match(notice, new RegExp(outage.message));
  const before = attempts;
  await idle(3500);
  assert(attempts >= before + 2, 'polling continues while the error stands');
  assert.equal(await app.toast.count(), 1, 'one notice');
  const lines = () => logs.filter(line => line.includes('Could not update from the saved diagram'));
  assert.equal(lines().length, 1, `the error is logged once, not on every poll: ${JSON.stringify(logs)}`);

  // A different error is a new notice and a new log line.
  if (outage.message) {
    current = { message: 'The backend moved.' };
    await waitUntil(async () => /backend moved/.test(await app.toast.innerText().catch(() => '')), { what: 'the new error in the notice' });
    assert.equal(lines().length, 2);
  }

  // The backend answers again: the notice goes and the missed change is merged.
  current = null;
  await waitUntil(async () => (await app.toast.count()) === 0, { timeout: 6000, what: 'the notice to clear' });
  await app.until(elements => elements.some(element => element.id === 'c'), { timeout: 6000, what: 'the agent element after the outage' });
}));

// Known gaps in the page (src/persistence.js); these fail today and are reported, not hidden.
test('a change whose load failed once is still merged', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('a', 100, 100)]),
}, async app => {
  const { backend } = app;
  await app.canvas.waitFor();
  const call = backend.call.bind(backend);
  let failed = false;
  backend.call = async (name, input) => {
    if (name === 'load_scene' && !failed) { failed = true; throw Error('transient'); }
    return call(name, input);
  };
  backend.agentEdit(scene => scene.elements.push(rect('c', 700, 100)));
  await app.until(elements => elements.some(element => element.id === 'c'), { timeout: 6000, what: 'the agent element after a failed load' });
}));

test('an agent selection that the owner cleared is not applied again by a later agent edit', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('a', 100, 100), rect('b', 400, 100)]),
}, async app => {
  const { backend, page } = app;
  await app.canvas.waitFor();
  await page.mouse.click(1000, 700);
  backend.agentEdit(scene => { scene.appState.selectedElementIds = { b: true }; });
  await idle(2000);
  await page.mouse.click(1000, 700);
  backend.agentEdit(scene => scene.elements.push(rect('c', 700, 100)));
  await idle(2200); // reading the canvas through the menu would take the focus away from the keys
  await page.keyboard.press('Delete');
  await idle(1000);
  assert.deepEqual(await app.ids(), ['a', 'b', 'c']);
}));

// An App update (or a reconnect) replaces the window; its page keeps nothing, so what the person
// drew and has not saved yet must wait in the App and come back with the new window.
// What a window sends while its save never lands: the canvas cannot be read through Export then,
// which waits for the save, so the scene the window tried to save stands for it.
const triedToSave = async (backend, since, what) => {
  await waitUntil(() => backend.saves.length > since, { what });
  return backend.saves.at(-1).scene.elements.filter(element => !element.isDeleted);
};

const replaced = async (old, backend) => {
  await old.close();
  const next = await openApp({ backend });
  await next.load();
  return next;
};

test('an edit still waiting for its automatic save comes back when the App update replaces the window', { timeout: 60_000 }, async () => {
  const backend = fakeBackend();
  backend.holdSaves = 'lose';
  const first = await openApp({ backend });
  let second;
  try {
    await first.load();
    await first.draw();
    await waitUntil(() => backend.drafts.size === 1, { what: 'the edit to be kept' });
    const [replacedId] = backend.drafts.keys();
    assert.equal(backend.live().length, 0, 'nothing of the drawing reached the saved diagram');
    backend.holdSaves = null;
    second = await replaced(first, backend);
    assert.deepEqual((await second.elements()).map(element => element.type), ['rectangle'], 'the drawn rectangle comes back');
    await waitUntil(() => backend.live().length === 1, { what: 'the recovered edit to be saved' });
    await waitUntil(() => backend.drafts.size === 0, { what: 'the replaced window\'s draft to be retired and the recovering window\'s own discarded once saved' });
    second.assertClean();
  } finally { await first.close(); await second?.close(); }
});

test('an unacknowledged automatic save does not lose newer edits when the App update replaces the window', { timeout: 60_000 }, async () => {
  const backend = fakeBackend();
  backend.holdSaves = 'commit';
  const first = await openApp({ backend });
  let second;
  try {
    await first.load();
    await first.draw(300, 200);
    await waitUntil(() => backend.live().length === 1, { what: 'the first save to reach the App' });
    await first.draw(300, 400);
    await first.draw(600, 400);
    await waitUntil(() => backend.drafts.size === 1 && [...backend.drafts.values()][0].elements.length === 3, { what: 'all three edits to be kept' });
    backend.holdSaves = null;
    second = await replaced(first, backend);
    assert.equal((await second.elements()).length, 3, 'the saved rectangle and the two drawn while the save waited');
    await waitUntil(() => backend.live().length === 3, { what: 'the recovered edits to be saved' });
    await waitUntil(() => backend.drafts.size === 0, { what: 'the replaced window\'s draft to be retired and the recovering window\'s own discarded once saved' });
  } finally { await first.close(); await second?.close(); }
});

test('an edit kept for a replaced window is merged with what an agent saved meanwhile, not saved over it', { timeout: 60_000 }, async () => {
  const backend = fakeBackend();
  backend.holdSaves = 'lose';
  const first = await openApp({ backend, scene: sceneOf([rect('existing', 100, 500)]) });
  let second;
  try {
    await first.load();
    await first.draw();
    await waitUntil(() => backend.drafts.size === 1, { what: 'the edit to be kept' });
    backend.holdSaves = null;
    backend.agentEdit(scene => scene.elements.push(rect('agent-box', 800, 150)));
    second = await replaced(first, backend);
    await waitUntil(() => backend.live().length === 3, { what: 'the recovered edit and the agent edit to be saved together' });
    assert(backend.live().some(element => element.id === 'agent-box'), 'the agent edit stays saved');
    assert.equal((await second.elements()).length, 3);
  } finally { await first.close(); await second?.close(); }
});

test('a refused write of the recovering window\'s draft leaves the replaced window\'s edit in the App', { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  backend.holdSaves = 'lose';
  const first = await openApp({ backend });
  let second, third;
  try {
    await first.load();
    await first.draw();
    await waitUntil(() => backend.drafts.size === 1, { what: 'the edit to be kept' });
    const [replacedId] = backend.drafts.keys();
    // The App refuses the replacement draft, accepts the discard, and the scene save never commits.
    backend.failDraftSaves = 'the draft store is unavailable';
    const writes = backend.count('save_draft');
    const saves = backend.saves.length;
    second = await replaced(first, backend);
    assert.equal((await triedToSave(backend, saves, 'the recovering window\'s save')).length, 1, 'the recovered rectangle is on the canvas');
    await waitUntil(() => backend.count('save_draft') > writes, { what: 'the replacement draft to be refused' });
    await new Promise(resolve => setTimeout(resolve, 500));
    assert(backend.drafts.has(replacedId), 'the replaced window\'s draft is still kept');
    assert.equal(backend.live().length, 0, 'the scene save did not commit');
    // The view is replaced once more: the edit is still there to recover.
    backend.failDraftSaves = null;
    backend.holdSaves = null;
    third = await replaced(second, backend);
    assert.equal((await third.elements()).length, 1, 'the rectangle comes back again');
    await waitUntil(() => backend.live().length === 1, { what: 'the recovered edit to be saved' });
    await waitUntil(() => !backend.drafts.has(replacedId), { what: 'the replaced window\'s draft to be retired once the third window holds its edit' });
  } finally { await first.close(); await second?.close(); await third?.close(); }
});

// The review's second repro: a window that is still open advances its draft after another window read it,
// and nothing the recovering window does may remove the newer draft.
test('a draft that advances after a recovering window read it is never removed by that window', { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  backend.holdSaves = 'lose';
  const open = await openApp({ backend });
  let recovering, next;
  try {
    await open.load();
    await open.draw(300, 200);
    await waitUntil(() => backend.drafts.size === 1, { what: 'the open window\'s edit to be kept' });
    const [openId] = backend.drafts.keys();
    // The recovering window has its copy of that draft when the open window keeps a second rectangle.
    backend.afterReadDraft = async () => {
      backend.afterReadDraft = null;
      await open.draw(700, 500);
      await waitUntil(() => backend.drafts.get(openId).elements.length === 2, { what: 'the open window\'s second rectangle to be kept' });
    };
    const saves = backend.saves.length;
    recovering = await openApp({ backend });
    await recovering.load();
    assert.equal((await triedToSave(backend, saves, 'the recovering window\'s save')).length, 1, 'the recovering window shows what the draft held when it read it');
    // The recovering window's own writes finish; its scene save never commits (holdSaves).
    await idle(1500);
    assert.equal(backend.drafts.get(openId)?.elements.length, 2, 'the open window\'s newer draft is still kept');
    // Both views are replaced; the next window finds both rectangles.
    await open.close(); await recovering.close();
    backend.holdSaves = null;
    next = await openApp({ backend });
    await next.load();
    assert.equal((await next.elements()).length, 2, 'the rectangle kept after the other window read the draft is back');
    await waitUntil(() => backend.live().length === 2, { what: 'both rectangles to be saved' });
  } finally { await open.close(); await recovering?.close(); await next?.close(); }
});

test('two windows replaced together each get their edit back', { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  const left = await openApp({ backend });
  const right = await openApp({ backend });
  let third;
  try {
    await left.load(); await right.load();
    backend.holdSaves = 'lose';
    await left.draw(300, 200);
    await right.draw(700, 500);
    await waitUntil(() => backend.drafts.size === 2, { what: 'both edits to be kept' });
    backend.holdSaves = null;
    await left.close(); await right.close();
    third = await openApp({ backend });
    await third.load();
    assert.equal((await third.elements()).length, 2, 'both windows\' rectangles come back');
    await waitUntil(() => backend.live().length === 2, { what: 'both edits to be saved' });
    await waitUntil(() => backend.drafts.size === 0, { what: 'both replaced windows\' drafts to be retired and the recovering window\'s own discarded once saved' });
  } finally { await left.close(); await right.close(); await third?.close(); }
});

test('opening a different diagram in one window replaces it in the other window and in the saved diagram', { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  const first = await openApp({ backend, scene: sceneOf([...labelled('client', 'Client', 100, 100), rect('shared', 600, 400)]) });
  const second = await openApp({ backend });
  try {
    await first.load(); await second.load();
    assert.deepEqual(await second.ids(), ['client', 'client-text', 'shared']);
    backend.files.set('other.excalidraw', Buffer.from(JSON.stringify(sceneOf([rect('replacement', 200, 200), rect('shared', 700, 300)]))));
    backend.pick('other.excalidraw');
    await first.menu('Open');
    await waitUntil(() => backend.saves.length >= 1, { what: 'the replacement to be saved' });
    await second.until(elements => elements.length === 2 && elements.some(element => element.id === 'replacement'), { what: 'the other window to show only the replacement' });
    assert.deepEqual(await second.ids(), ['replacement', 'shared']);
    assert.deepEqual(await first.ids(), ['replacement', 'shared']);
    assert.deepEqual(backend.live().map(element => element.id).sort(), ['replacement', 'shared']);
    assert.equal(backend.live().find(element => element.id === 'shared').x, 700, 'the replacement\'s version of an element both diagrams have wins');
    assert.equal((await second.elements()).find(element => element.id === 'shared').x, 700);
    // Neither window saves the old diagram back.
    await idle(2500);
    assert.deepEqual(backend.live().map(element => element.id).sort(), ['replacement', 'shared']);
    first.assertClean(); second.assertClean();
  } finally { await first.close(); await second.close(); }
});
