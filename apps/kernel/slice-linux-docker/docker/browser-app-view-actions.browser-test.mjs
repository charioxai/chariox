// Apps v2 A1: the View SDK in real Chromium, through the real App bridge. The
// page registers actions, a state snapshot and changes; the kernel side reads
// them only through `__charioxViewAgent`, as the A2 bridge will. Supplementary
// controller regression; the agent drill with a real provider is A2/X1.
import test, { after, before } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'MP-11: explicit disposable root required');
const asset = (name, type, text) => ({ path: name, content_type: type, body_base64: Buffer.from(text).toString('base64') });
let root;
let host;
let requests = 0;

before(async () => {
  root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'app-view-actions-'));
  host = new KernelBrowserHost(root);
});
after(async () => {
  await host?.stop();
  if (root) await rm(root, { recursive: true, force: true });
});

const html = `<main>
  <form id="rename" data-chariox-action="rename_list" data-chariox-effect="write" data-chariox-description="Rename the list">
    <input name="title" required><select name="sort"><option value="a">a</option><option value="b">b</option></select>
    <input type="checkbox" name="pinned"><input type="password" name="pin"><button>Rename</button>
  </form>
  <p id="title">untitled</p>
</main><script src="app.js"></script>`;
const script = `
  const todos = [];
  const add = chariox.action({
    name: 'add_todo', description: 'Add a todo',
    inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'], additionalProperties: false },
    effect: 'write',
  }, async ({ text }) => { todos.push(text); chariox.view.changed({ added: text }); return { count: todos.length }; });
  chariox.action({
    name: 'wipe', description: 'Remove every todo', inputSchema: { type: 'object', additionalProperties: false },
    effect: 'approval', approvalReason: 'Deletes the list',
  }, async () => { throw Object.assign(new Error('refused by the app'), { code: 'APP_REFUSED' }); });
  chariox.view.state(() => ({ todos, title: document.getElementById('title').textContent }));
  rename.addEventListener('submit', event => { event.preventDefault(); document.getElementById('title').textContent = rename.title.value + '|' + rename.sort.value + '|' + rename.pinned.checked; });
  window.addTodo = add;`;

async function openApp(label, files = { 'index.html': html, 'app.js': script }) {
  const reply = await host.handle({ id: ++requests, method: 'browser.app.open', params: {
    origin_label: label, installation_id: label,
    assets: Object.entries(files).map(([name, text]) => asset(name, name.endsWith('.js') ? 'text/javascript' : 'text/html', text)) } });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const { connection, sessionId } = await host.browser.resolvePageTarget(reply.result.target_id);
  const evaluate = async expression => {
    const result = await connection.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }, sessionId);
    assert.ok(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  for (let i = 0; i < 200 && !await evaluate('document.readyState === "complete" && typeof window.addTodo === "function"'); i++) await new Promise(r => setTimeout(r, 25));
  return { evaluate, agent: (call) => evaluate(`__charioxViewAgent.${call}`) };
}

test('A1: the page registers declaration-shaped actions and form markup, password included', async () => {
  const app = await openApp('view-registrations');
  const registrations = await app.agent('registrations()');
  assert.deepEqual(registrations.map(r => r.name), ['add_todo', 'wipe', 'rename_list']);
  const rename = registrations[2];
  assert.deepEqual(rename.inputSchema, { type: 'object', additionalProperties: false, required: ['title'], properties: {
    title: { type: 'string' }, sort: { type: 'string', enum: ['a', 'b'] }, pinned: { type: 'boolean' }, pin: { type: 'string' } } });
  assert.equal(registrations[1].approvalReason, 'Deletes the list');
});

test('A1: invoke runs the view function, publishes state and changes, and reports app errors with their code', async () => {
  const app = await openApp('view-invoke');
  assert.deepEqual(await app.agent(`invoke('add_todo', { text: 'milk' })`), { ok: true, value: { count: 1 } });
  assert.deepEqual(await app.agent('state()'), { ok: true, value: { todos: ['milk'], title: 'untitled' } });
  assert.deepEqual(await app.agent('changes(0)'), { ok: true, value: { latest: 1, dropped: false, changes: [{ seq: 1, delta: { added: 'milk' } }] } });
  assert.deepEqual(await app.agent(`invoke('wipe', {})`), { ok: false, code: 'APP_REFUSED', message: 'refused by the app' });
  assert.deepEqual(await app.agent(`invoke('nothing', {})`), { ok: false, code: 'unknown_action', message: 'the view has no action "nothing"' });
});

test('A1: a form action fills the real controls and submits through the real submit event', async () => {
  const app = await openApp('view-form');
  assert.deepEqual(await app.agent(`invoke('rename_list', { title: 'Groceries', sort: 'b', pinned: true })`), { ok: true, value: { submitted: true } });
  assert.equal(await app.evaluate('document.getElementById("title").textContent'), 'Groceries|b|true');
  await app.evaluate('rename.title.value = ""');
  assert.equal((await app.agent(`invoke('rename_list', { sort: 'a' })`)).code, 'invalid_form', 'the required title is empty');
  await app.evaluate('rename.title.value = "Kept"');
  assert.equal((await app.agent(`invoke('rename_list', { pin: '1234' })`)).ok, true);
  assert.equal(await app.evaluate('document.querySelector("[name=pin]").value'), '1234', 'an agent may enter a password it knows');
});

test('A1: the bridge object stays frozen and the agent handle cannot be replaced by the page', async () => {
  const app = await openApp('view-frozen');
  assert.equal(await app.evaluate(`(() => { try { chariox.action = null } catch {} ; return typeof chariox.action })()`), 'function');
  assert.equal(await app.evaluate(`(() => { try { __charioxViewAgent = null } catch {} ; return typeof __charioxViewAgent.invoke })()`), 'function');
});

// A controlled form the way React builds it: a value tracker on each control
// swallows direct `.value =` and `.checked =` writes, and `onChange` runs (on
// `input` for text, on `click` for checkbox and radio) only when the DOM value
// differs from the tracked one. The submit handler reads component state only.
const controlledHtml = `<form id="f" data-chariox-action="save_profile" data-chariox-effect="write" data-chariox-description="Save the profile">
  <input name="nick" required><textarea name="bio"></textarea><select name="lang"><option value="en">en</option><option value="es">es</option></select>
  <input type="checkbox" name="news"><input type="radio" name="tier" value="free" checked><input type="radio" name="tier" value="pro">
  <input type="password" name="pin"><button>Save</button></form><p id="saved"></p><script src="app.js"></script>`;
const controlledScript = `
  const state = { nick: '', bio: '', lang: 'en', news: false, tier: 'free', pin: '' };
  const tracked = new WeakMap();
  const field = node => (node.type === 'checkbox' || node.type === 'radio' ? 'checked' : 'value');
  for (const node of document.querySelectorAll('[name]')) {
    const key = field(node);
    const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(node), key);
    let last = '' + node[key];
    tracked.set(node, { get: () => last, set: value => { last = value; } });
    Object.defineProperty(node, key, { configurable: true, get() { return descriptor.get.call(this); },
      set(value) { last = '' + value; descriptor.set.call(this, value); } });
  }
  const changed = node => { const t = tracked.get(node); const next = '' + node[field(node)]; if (next === t.get()) return false; t.set(next); return true; };
  const onChange = node => { if (node.type === 'checkbox') state.news = node.checked; else if (node.type === 'radio') { if (node.checked) state.tier = node.value; } else state[node.name] = node.value; };
  document.addEventListener('input', e => { if (changed(e.target)) onChange(e.target); });
  document.addEventListener('change', e => { if (e.target.tagName === 'SELECT' && changed(e.target)) onChange(e.target); });
  document.addEventListener('click', e => { if ((e.target.type === 'checkbox' || e.target.type === 'radio') && changed(e.target)) onChange(e.target); });
  f.addEventListener('submit', e => { e.preventDefault(); document.getElementById('saved').textContent = JSON.stringify(state); });
  window.addTodo = () => {};`;

test('A1: a form action reaches a controlled component: its state updates before the submit handler reads it', async () => {
  const app = await openApp('view-controlled', { 'index.html': controlledHtml, 'app.js': controlledScript });
  const answer = await app.agent(`invoke('save_profile', { nick: 'Ada', bio: 'maths', lang: 'es', news: true, tier: 'pro', pin: '1234' })`);
  assert.deepEqual(answer, { ok: true, value: { submitted: true } });
  assert.deepEqual(JSON.parse(await app.evaluate('document.getElementById("saved").textContent')),
    { nick: 'Ada', bio: 'maths', lang: 'es', news: true, tier: 'pro', pin: '1234' });
});
