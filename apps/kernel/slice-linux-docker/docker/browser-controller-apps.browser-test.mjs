// MP-08/MP-10/MP-11: an App's delayed bridge calls keep the origin of the real
// CDP input that started them, in real Chromium. Supplementary controller
// regression; hosted client/relay acceptance is separate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'MP-11: explicit disposable root required');
const asset = (name, type, text) => ({ path: name, content_type: type, body_base64: Buffer.from(text).toString('base64') });
const assets = [
  asset('index.html', 'text/html', '<button id="later" style="position:fixed;left:10px;top:10px;width:120px;height:40px">later</button>'
    + '<button id="chain" style="position:fixed;left:10px;top:70px;width:120px;height:40px">chain</button>'
    + '<div style="height:4000px"></div><script src="app.js"></script>'),
  asset('app.js', 'text/javascript', `
    later.onclick = () => setTimeout(() => chariox.call("delayed"), 6000);
    chain.onclick = async () => { await chariox.call('first'); chariox.call('second'); };
    addEventListener('load', () => chariox.call('load'));`),
];

test('MP-08/MP-11: delayed and chained App calls keep their originating actor in real Chromium', async () => {
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'app-actor-'));
  const host = new KernelBrowserHost(root);
  try {
    const reply = await host.handle({ id: 1, method: 'browser.app.open', params: {
      origin_label: 'actor-app', installation_id: 'actor-app', assets } });
    assert.equal(reply.ok, true, JSON.stringify(reply));
    const tabs = host.browser.appTabs;
    const target = host.tabs.get(reply.result.tab_id).target_id;
    const { connection, sessionId } = await host.browser.resolvePageTarget(target);
    const send = (method, params) => connection.send(method, params, sessionId);
    const evaluate = async expression => (await send('Runtime.evaluate', { expression, returnByValue: true })).result.value;
    for (let i = 0; i < 200 && !await evaluate('document.readyState === "complete" && !!window.later'); i++) await new Promise(r => setTimeout(r, 25));
    const center = async id => JSON.parse(await evaluate(`(r => JSON.stringify({x: r.x + r.width / 2, y: r.y + r.height / 2}))(document.getElementById('${id}').getBoundingClientRect())`));
    const click = async id => {
      const { x, y } = await center(id);
      for (const type of ['mousePressed', 'mouseReleased']) await send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1 });
    };
    const drain = async () => {
      const calls = [];
      for (let i = 0; i < 4; i++) calls.push(...(await tabs.takeCalls()).calls);
      return calls;
    };
    const agent = { kind: 'agent', id: 'agent-7' };
    const methods = calls => calls.map(call => [call.method, call.actor ?? null]);
    // Page load and a person's click: the person.
    await click('chain');
    let seen = [];
    for (let i = 0; i < 20 && !seen.some(call => call.method === 'first'); i++) seen.push(...await drain());
    const first = seen.find(call => call.method === 'first');
    await tabs.respond({ target_id: target, call_id: first.call_id, result: {} });
    for (let i = 0; i < 20 && !seen.some(call => call.method === 'second'); i++) seen.push(...await drain());
    assert.deepEqual(methods(seen), [['load', null], ['first', null], ['second', null]]);

    // The agent clicks; the owner scrolls; the handler's timer then calls as the agent.
    const done = await tabs.beginInput(target, 'agent-7');
    await click('later');
    await done();
    await send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: 300, y: 300, deltaX: 0, deltaY: 300 });
    seen = [];
    for (let i = 0; i < 60 && !seen.some(call => call.method === "delayed"); i++) seen.push(...await drain());
    assert.deepEqual(methods(seen), [['delayed', agent]]);

    // The same agent-driven page: the person's click and its awaited second call stay the person's.
    await click('chain');
    seen = [];
    for (let i = 0; i < 20 && !seen.some(call => call.method === 'first'); i++) seen.push(...await drain());
    await tabs.respond({ target_id: target, call_id: seen.find(call => call.method === 'first').call_id, result: {} });
    for (let i = 0; i < 20 && !seen.some(call => call.method === 'second'); i++) seen.push(...await drain());
    assert.deepEqual(methods(seen), [['first', null], ['second', null]]);
  } finally {
    await host.stop();
    await rm(root, { recursive: true, force: true });
  }
});
