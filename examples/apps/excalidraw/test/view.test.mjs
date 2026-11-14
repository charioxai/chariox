// The Excalidraw window against a fake backend: what a person draws is saved with the
// backend's version check, and what an agent does in the backend shows up in the open window.
import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { openApp, closeEnvironment, labelled, rect, sceneOf, waitUntil } from './window.mjs';

after(closeEnvironment);

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

  // Reloading recovers automatic saving once the backend works again.
  backend.failSaves = null;
  await app.page.reload();
  await app.canvas.waitFor();
  await app.draw(300, 400);
  await waitUntil(() => backend.saves.length === 2, { what: 'saving after the reload' });
  assert.equal(backend.live().length, 2, 'the two rectangles drawn before the reload were never saved');
  await app.shot('failure-recovery');
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
