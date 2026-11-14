// Shared harness for the Excalidraw window tests: the built page runs in real Chrome against a
// FAKE BACKEND that has the contract of the App's Rust backend, as far as the page sees it
// (window.chariox.call). The fake is not the backend; the backend's own tests cover that.
//
//   load_scene   -> { scene, version }     scene is null and version null until the first save
//   save_scene   -> { version }            refused with an error unless `version` is the saved version
//   view_changes -> { latest, dropped, changes }   `latest` rises on every write, window saves too
//   write_file / read_file / pick_file     files; an offered copy returns a grant
//
// `agentEdit(fn)` edits the saved scene the way the backend does for an agent action.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, extname, join } from 'node:path';
import { chromium } from 'playwright-core';

const bundle = process.env.CHARIOX_EXCALIDRAW_BUNDLE;
const evidence = process.env.CHARIOX_EXCALIDRAW_EVIDENCE;
assert(bundle && evidence, 'set CHARIOX_EXCALIDRAW_BUNDLE and CHARIOX_EXCALIDRAW_EVIDENCE outside the repository');

const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2' };
let shared;
async function environment() {
  shared ??= (async () => {
    const server = createServer(async (req, res) => {
      try {
        const file = resolve(bundle, 'ui', '.' + new URL(req.url, 'http://localhost').pathname);
        if (!file.startsWith(resolve(bundle, 'ui') + '/')) throw Error('invalid path');
        res.setHeader('Content-Type', types[extname(file)] ?? 'application/octet-stream');
        res.end(await readFile(file));
      } catch { res.statusCode = 404; res.end(); }
    });
    await new Promise(done => server.listen(0, '127.0.0.1', done));
    const browser = await chromium.launch({ executablePath: process.env.CHARIOX_TEST_CHROME ?? '/usr/bin/google-chrome', headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    await mkdir(evidence, { recursive: true });
    return { server, browser, origin: `http://127.0.0.1:${server.address().port}` };
  })();
  return shared;
}
export async function closeEnvironment() {
  if (!shared) return;
  const { server, browser } = await shared;
  shared = undefined;
  await browser.close();
  await new Promise(done => server.close(done));
}

const clone = value => JSON.parse(JSON.stringify(value));
const hash = text => [...text].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 2_000_000_000, 7);

// Fractional indices a0..a9, aA..aZ, aa..az: enough for the elements a test makes.
const DIGITS = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
const indexAfter = previous => {
  if (!previous) return 'a0';
  const next = DIGITS.indexOf(previous[1]) + 1;
  assert(previous.length === 2 && next < DIGITS.length, 'a test scene has at most 62 elements');
  return 'a' + DIGITS[next];
};

// ---- elements as the backend writes them (version 1) ----
export const rect = (id, x = 100, y = 100, width = 120, height = 80, extra = {}) => ({
  id, type: 'rectangle', x, y, width, height, angle: 0, strokeColor: '#1e1e1e', backgroundColor: 'transparent',
  fillStyle: 'solid', strokeWidth: 2, strokeStyle: 'solid', roughness: 1, opacity: 100, groupIds: [], frameId: null,
  index: null, roundness: null, seed: hash(id), version: 1, versionNonce: hash(id) + 1, isDeleted: false,
  boundElements: null, updated: 1, link: null, locked: false, ...extra,
});
export const text = (id, value, x, y, extra = {}) => ({
  ...rect(id, x, y, Math.max(20, value.length * 11), 25), type: 'text', text: value, originalText: value,
  fontSize: 20, fontFamily: 1, textAlign: 'left', verticalAlign: 'top', containerId: null, autoResize: true,
  lineHeight: 1.25, ...extra,
});
/** A rectangle with a label bound to it: the container and its text element. */
export const labelled = (id, label, x, y, width = 160, height = 80) => {
  const label_ = text(`${id}-text`, label, x + width / 2 - label.length * 5.5, y + height / 2 - 12.5, {
    containerId: id, textAlign: 'center', verticalAlign: 'middle',
  });
  return [rect(id, x, y, width, height, { boundElements: [{ id: label_.id, type: 'text' }] }), label_];
};
export const sceneOf = (elements = [], appState = {}) =>
  ({ type: 'excalidraw', version: 2, source: 'test', elements, appState, files: {} });

export function fakeBackend() {
  const backend = {
    scene: null, version: null, counter: 0, feed: 0,
    calls: [], saves: [], writes: [], files: new Map(), picked: null,
    /** Message of the error every save_scene throws, with the saved version unchanged. */
    failSaves: null,
    /** Run once at the start of the next save_scene; the save is then refused when the version moved. */
    interleave: null,
    count: name => backend.calls.filter(call => call === name).length,
    /** Offer `name` (a file in `files`) to the next pick_file, as a person's choice. */
    pick: name => { backend.picked = name; },
    write(scene) {
      backend.scene = clone(scene);
      backend.version = String(++backend.counter);
      backend.feed++;
    },
    /**
     * An agent action: `fn` edits a copy of the saved scene (push, mutate, splice, set
     * appState.selectedElementIds), then the edit is stamped as the backend stamps it: a changed
     * element gets version + 1, a removed one stays in the scene as isDeleted:true.
     */
    agentEdit(fn) {
      const before = backend.scene ?? sceneOf();
      const next = clone(before);
      fn(next);
      const previous = new Map(before.elements.map(element => [element.id, element]));
      for (const element of next.elements) {
        const old = previous.get(element.id);
        if (!old) { element.version ??= 1; element.versionNonce ??= hash(element.id) + 1; continue; }
        if (JSON.stringify(old) !== JSON.stringify(element)) {
          element.version = old.version + 1;
          element.versionNonce = (old.versionNonce + 7919) % 2_000_000_000;
        }
      }
      // The backend gives an element without an index one after the last indexed element.
      let last = null;
      for (const element of next.elements) {
        if (!element.index) element.index = indexAfter(last);
        last = element.index;
      }
      const kept = new Set(next.elements.map(element => element.id));
      for (const old of before.elements) {
        if (!kept.has(old.id)) next.elements.push({ ...old, isDeleted: true, version: old.version + 1, versionNonce: (old.versionNonce + 104729) % 2_000_000_000 });
      }
      backend.write(next);
    },
    /** The saved scene's live elements. */
    live: () => (backend.scene?.elements ?? []).filter(element => !element.isDeleted),
    async call(name, input = {}) {
      backend.calls.push(name);
      if (name === 'load_scene') return { scene: clone(backend.scene), version: backend.version };
      if (name === 'view_changes') return { latest: backend.feed, dropped: false, changes: [] };
      if (name === 'save_scene') {
        backend.saves.push(clone(input));
        const interleave = backend.interleave;
        backend.interleave = null;
        interleave?.();
        if (backend.failSaves) throw Error(backend.failSaves);
        if (input.version !== backend.version) throw Error('the diagram was changed by another window or an agent; load it again and retry');
        backend.write(input.scene);
        return { version: backend.version };
      }
      if (name === 'write_file') {
        const bytes = input.base64 ? Buffer.from(input.base64, 'base64') : Buffer.from(input.text);
        backend.writes.push(clone(input));
        backend.files.set(input.name, bytes);
        return { name: input.name, bytes: bytes.length, grantId: input.offer ? input.name : null };
      }
      if (name === 'read_file') {
        const file = backend.files.get(input.name ?? input.grantId);
        if (!file) throw Error('no such file: ' + (input.name ?? input.grantId));
        return { name: input.name ?? input.grantId, scene: JSON.parse(file.toString()) };
      }
      if (name === 'pick_file') return { grants: backend.picked ? [backend.picked] : [] };
      throw Error('unknown backend tool: ' + name);
    },
  };
  return backend;
}

export async function waitUntil(check, { timeout = 5000, interval = 100, what = 'the condition' } = {}) {
  const deadline = Date.now() + timeout;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() > deadline) throw Error(`timed out waiting for ${what}`);
    await new Promise(done => setTimeout(done, interval));
  }
}

/**
 * Open the built page against a fresh fake backend. `scene` seeds the saved scene (as an agent
 * would have left it) before the window loads it.
 */
export async function openApp({ dpr = 1, scene, backend = fakeBackend() } = {}) {
  if (scene) backend.agentEdit(saved => { Object.assign(saved, clone(scene)); });
  const { browser, origin } = await environment();
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: dpr });
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  const page = await context.newPage();
  page.setDefaultTimeout(5000);
  const errors = [], network = [];
  page.on('pageerror', error => errors.push(error.message));
  await context.route('**/*', route => {
    if (route.request().url().startsWith(origin)) return route.continue();
    network.push(route.request().url());
    return route.abort();
  });
  await context.exposeFunction('broker', (name, input) => backend.call(name, input));
  await context.addInitScript(() => { window.chariox = { call: (name, input) => window.broker(name, input) }; });
  const app = {
    page, backend, errors, network, dpr, toast: page.locator('.Toast'),
    canvas: page.locator('.excalidraw__canvas.interactive'),
    async load() {
      await page.goto(origin + '/index.html');
      await page.waitForSelector('.excalidraw__canvas.interactive');
      await page.evaluate(() => document.fonts.ready);
    },
    /** A person draws a rectangle with the toolbar and the pointer. */
    async draw(x = 300, y = 200, width = 100, height = 80) {
      // The toolbar button is a radio input under its label: click the label a person sees.
      await page.getByTestId('toolbar-rectangle').locator('xpath=..').click();
      await page.mouse.move(x, y); await page.mouse.down();
      await page.mouse.move(x + width, y + height, { steps: 5 });
      await page.waitForTimeout(50);
      await page.mouse.up();
    },
    async menu(item) {
      await page.getByTestId('main-menu-trigger').click();
      await page.getByText(item, { exact: true }).click();
    },
    /**
     * What the canvas shows. The page exposes no handle on its Excalidraw instance, so this reads
     * the canvas the way a person does: through the main menu's Export JSON, which serializes the
     * visible (not deleted) elements through write_file. It reads, it does not edit.
     */
    async elements() {
      const before = backend.writes.length;
      await app.menu('Export JSON');
      await waitUntil(() => backend.writes.length > before, { what: 'Export JSON' });
      return JSON.parse(backend.files.get('diagram.excalidraw').toString()).elements;
    },
    async ids() { return (await app.elements()).map(element => element.id).sort(); },
    /** Read the canvas until `check(elements)` holds; returns how long the canvas took to show it. */
    async until(check, { timeout = 4000, what = 'the canvas to change' } = {}) {
      const started = Date.now();
      await waitUntil(async () => check(await app.elements()), { timeout, interval: 150, what });
      return Date.now() - started;
    },
    async shot(name) { await page.screenshot({ path: join(evidence, `${name}-dpr${dpr}.png`) }); },
    assertClean() {
      assert.deepEqual(errors, [], 'the page threw');
      assert.deepEqual(network, [], 'all fonts, workers and code must load from App assets');
    },
    close: () => context.close(),
  };
  return app;
}
