// A Vault fill into an App view runs in an isolated world that cannot see the
// view's caller bridge. The fill must still land, stay masked, and run the
// view's handlers as the agent that filled it. Supplementary controller
// regression in real Chromium; the Room flow is checked separately.
import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

const asset = (name, type, body) => ({ path: name, content_type: type, body_base64: Buffer.from(body).toString('base64') });
const html = `<form id="login"><input id="pin" type="password" name="pin" aria-label="PIN"><button>Sign in</button></form><script src="app.js"></script>`;
const script = `
  const spec = name => ({name, description:name, effect:'write', inputSchema:{type:'object'}});
  const filled = chariox.action(spec('filled'), async (input, caller) => { await caller.call('filled'); });
  pin.addEventListener('change', () => filled({}));
  chariox.form(login, async (input, caller) => { await caller.call('signed_in'); });
  window.ready = true;`;
const SECRET = 'vault-canary-7731';

test('a Vault fill into an App view lands and its handlers run as the filling agent', async () => {
  assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'explicit disposable root required');
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'app-secret-fill-'));
  const host = new KernelBrowserHost(root);
  let request = 0;
  try {
    const opened = await host.handle({ id: ++request, method: 'browser.app.open', params: {
      origin_label: 'secret-fill', installation_id: 'secret-fill',
      assets: [asset('index.html', 'text/html', html), asset('app.js', 'text/javascript', script)] } });
    assert.equal(opened.ok, true, JSON.stringify(opened));
    const { target_id: target, tab_id: tabId } = opened.result;
    const { connection, sessionId } = await host.browser.resolvePageTarget(target);
    const evaluate = async expression => {
      const reply = await connection.send('Runtime.evaluate', { expression, returnByValue: true }, sessionId);
      assert.equal(reply.exceptionDetails, undefined, JSON.stringify(reply.exceptionDetails));
      return reply.result.value;
    };
    for (let i = 0; i < 200 && !await evaluate('window.ready === true'); i++) await delay(20);
    const { snapshot } = await host.request({ op: 'snapshot', tab_id: tabId, generation: host.generation });
    const node = snapshot.dom_nodes.find(candidate => candidate.attributes?.id === 'pin');
    const filled = await host.browser.performAction({
      target_id: target, document_id: snapshot.document_id, node_ref: node.node_ref, timeout_ms: 5000, agent_id: 'agent-one',
      action: { kind: 'fill', text: SECRET, append: false, submit: true, expected_document_url: await evaluate('location.href') },
    });
    assert.equal(filled.action_kind, 'fill');
    assert.equal(JSON.stringify(filled).includes(SECRET), false);
    assert.equal(await evaluate('pin.value'), SECRET);
    assert.equal(await evaluate('pin.type'), 'password');
    const calls = [];
    for (let i = 0; i < 100 && !['filled', 'signed_in'].every(method => calls.some(call => call.method === method)); i++) {
      calls.push(...(await host.browser.appTabs.takeCalls()).calls);
      await delay(10);
    }
    for (const method of ['filled', 'signed_in']) {
      assert.deepEqual(calls.find(call => call.method === method)?.actor, { kind: 'agent', id: 'agent-one' }, method);
    }
  } finally { await host.stop(); await rm(root, { recursive: true, force: true }); }
});
