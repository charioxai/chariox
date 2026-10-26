// Apps v2 A2: the kernel's agent bridge into an App view, in real Chromium
// through the real App bridge. Supplementary controller regression; the agent
// drill with a real provider is run through the runtime MCP catalog.
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
  root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'app-agent-'));
  host = new KernelBrowserHost(root);
});
after(async () => {
  await host?.stop();
  if (root) await rm(root, { recursive: true, force: true });
});

const script = `
  const todos = [];
  const settle = ms => new Promise(resolve => setTimeout(resolve, ms));
  chariox.action({ name: 'add_todo', description: 'Add a todo', effect: 'write',
    inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'], additionalProperties: false } },
    async ({ text }) => { await chariox.call('todo.add', { text }); todos.push(text); chariox.view.changed({ added: text }); return { count: todos.length }; });
  chariox.action({ name: 'slow', description: 'Takes a while', effect: 'read', inputSchema: { type: 'object', additionalProperties: false } },
    async () => { await settle(700); return { done: true }; });
  chariox.action({ name: 'hang', description: 'Never finishes', effect: 'read', inputSchema: { type: 'object', additionalProperties: false } },
    () => new Promise(() => {}));
  chariox.view.state(() => ({ todos }));
  window.ready = true;`;

async function openApp(label) {
  const reply = await host.handle({ id: ++requests, method: 'browser.app.open', params: {
    origin_label: label, installation_id: label, assets: [
      asset('index.html', 'text/html', '<main></main><script src="app.js"></script>'), asset('app.js', 'text/javascript', script)] } });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const target = reply.result.target_id;
  const { connection, sessionId } = await host.browser.resolvePageTarget(target);
  const evaluate = async expression => (await connection.send('Runtime.evaluate', { expression, returnByValue: true }, sessionId)).result.value;
  for (let i = 0; i < 200 && !await evaluate('window.ready === true'); i++) await new Promise(r => setTimeout(r, 25));
  const agent = async (params, id = 'agent-7') => {
    const answer = await host.handle({ id: ++requests, method: 'browser.app.agent', params: { target_id: target, agent: id, ...params } });
    return answer.ok ? answer.result : answer;
  };
  // Answer the page's kernel calls like the kernel does, recording who each ran as.
  const calls = [];
  const serve = async () => {
    for (let i = 0; i < 100; i++) {
      for (const call of (await host.browser.appTabs.takeCalls()).calls.filter(c => c.target_id === target)) {
        calls.push({ method: call.method, actor: call.actor ?? null });
        await host.handle({ id: ++requests, method: 'browser.app.respond', params: { target_id: target, call_id: call.call_id, result: { ok: true } } });
      }
    }
  };
  return { agent, calls, serve, target };
}

test('A2: an agent invoke runs the view function as that agent and the state and changes show the result', async () => {
  const app = await openApp('agent-invoke');
  const serving = app.serve();
  assert.deepEqual(await app.agent({ call: 'invoke', name: 'add_todo', input: { text: 'milk' } }), { ok: true, value: { count: 1 } });
  assert.deepEqual(app.calls, [{ method: 'todo.add', actor: { kind: 'agent', id: 'agent-7' } }], 'the handler\'s kernel call is the agent\'s, never the person\'s');
  assert.deepEqual(await app.agent({ call: 'state' }), { ok: true, value: { todos: ['milk'] } });
  assert.deepEqual(await app.agent({ call: 'changes', since: 0 }), { ok: true, value: { latest: 1, dropped: false, changes: [{ seq: 1, delta: { added: 'milk' } }] } });
  assert.deepEqual((await app.agent({ call: 'registrations' })).value.map(r => r.name), ['add_todo', 'slow', 'hang']);
  await serving;
});

test('A2: a call is refused for an unknown view and for input the bridge does not accept', async () => {
  const app = await openApp('agent-invalid');
  const refused = await host.handle({ id: ++requests, method: 'browser.app.agent', params: { target_id: 'nope', agent: 'agent-7', call: 'state' } });
  assert.equal(refused.ok, false);
  for (const params of [{ agent: 'a b', call: 'state' }, { agent: 'agent-7', call: 'eval' }, { agent: 'agent-7', call: 'invoke', name: 'Bad-Name' },
    { agent: 'agent-7', call: 'invoke', name: 'slow', input: [] }, { agent: 'agent-7', call: 'changes', wait_ms: -1 }, { agent: 'agent-7', call: 'state', deadline_ms: 0 }]) {
    const reply = await app.agent(params, params.agent);
    assert.equal(reply.ok, false, JSON.stringify(params));
  }
});

test('A2: a handler that outlives its deadline keeps the view single-flight until it settles', async () => {
  const app = await openApp('agent-deadline');
  const first = await app.agent({ call: 'invoke', name: 'slow', deadline_ms: 150 });
  assert.equal(first.code, 'deadline');
  assert.equal((await app.agent({ call: 'state' })).code, 'view_busy', 'a second handler must not interleave with the running one');
  await new Promise(r => setTimeout(r, 800));
  assert.deepEqual(await app.agent({ call: 'invoke', name: 'slow' }), { ok: true, value: { done: true } });
});

test('A2: a reload while an action runs ends the call with view_reloaded', async () => {
  const app = await openApp('agent-reload');
  const running = app.agent({ call: 'invoke', name: 'hang', deadline_ms: 20000 });
  await new Promise(r => setTimeout(r, 300));
  const reload = await host.handle({ id: ++requests, method: 'browser.app.reload', params: { target_id: app.target, entry: 'index.html', assets: [
    asset('index.html', 'text/html', '<main></main><script src="app.js"></script>'), asset('app.js', 'text/javascript', script)] } });
  assert.equal(reload.ok, true);
  assert.equal((await running).code, 'view_reloaded');
});
