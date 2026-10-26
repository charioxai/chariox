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

