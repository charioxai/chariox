// MP-08/MP-10/MP-11: who an App's bridge call runs as, in real Chromium. The
// agent drives the App Tab through the kernel host's own input path; the page
// cannot change the answer. Supplementary controller regression; hosted
// client/relay acceptance is separate.
import test, { after, before } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'MP-11: explicit disposable root required');
const asset = (name, type, text) => ({ path: name, content_type: type, body_base64: Buffer.from(text).toString('base64') });
const page = (script, extra = '') => asset('index.html', 'text/html',
  '<button id="act" style="position:fixed;left:10px;top:10px;width:120px;height:40px">act</button>'
  + `<div style="height:4000px"></div>${extra}<script src="app.js"></script>`);
const agent = { kind: 'agent', id: 'agent-7' };
let root;
let host;
let requests = 0;

before(async () => {
  root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'app-actor-'));
  host = new KernelBrowserHost(root);
});
after(async () => {
  await host.stop();
  await rm(root, { recursive: true, force: true });
});

async function openApp(label, assets) {
  const reply = await host.handle({ id: ++requests, method: 'browser.app.open', params: {
    origin_label: label, installation_id: label, assets } });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const tabId = reply.result.tab_id;
  const target = reply.result.target_id;
  const { connection, sessionId } = await host.browser.resolvePageTarget(target);
  const send = (method, params) => connection.send(method, params, sessionId);
  const evaluate = async expression => (await send('Runtime.evaluate', { expression, returnByValue: true })).result.value;
  for (let i = 0; i < 200 && !await evaluate('document.readyState === "complete" && !!window.act'); i++) await new Promise(r => setTimeout(r, 25));
  const seen = [];
  const until = async (method, tries = 80) => {
    for (let i = 0; i < tries && !seen.some(call => call.method === method); i++) seen.push(...(await host.browser.appTabs.takeCalls()).calls.filter(call => call.target_id === target));
    return seen.filter(call => call.method === method).map(call => call.actor ?? null);
  };
  // The agent clicks `act` exactly as the kernel dispatches an agent's App input.
  const agentClick = async () => {
    const { x, y } = JSON.parse(await evaluate('(r => JSON.stringify({x: r.x + r.width / 2, y: r.y + r.height / 2}))(act.getBoundingClientRect())'));
    const { frameTree } = await send('Page.getFrameTree', {});
    await host.request({ op: 'input', tab_id: tabId, generation: host.generation, document_id: frameTree.frame.loaderId,
      input: { kind: 'click', x, y }, focused_agent: true, _agent_input: true, observed_by: 'agent:agent-7', _app_view_tab: tabId });
  };
  // The agent fills `pin` from the Vault as the kernel dispatches it: no input before.
  const agentFill = async (claimed = true) => {
    const { snapshot } = await host.request({ op: 'snapshot', tab_id: tabId, generation: host.generation });
    const node = snapshot.dom_nodes.find(n => n.attributes?.id === 'pin');
    return host.handle({ id: ++requests, method: 'host.secret', params: { tab_id: tabId, generation: host.generation,
      document_id: snapshot.document_id, node_ref: node.node_ref, _agent_input: true, observed_by: 'agent:agent-7',
      ...(claimed ? { _app_view_tab: tabId } : {}),
      action: { kind: 'fill', text: 'app-vault-fixture', append: false, submit: false, expected_document_url: await evaluate('location.href') } } });
  };
  return { until, agentClick, agentFill, evaluate };
}

test('MP-08/MP-11 #1001 P1: an App that rewrites its bridge serializer cannot make an agent call the person\'s', async () => {
  const app = await openApp('forged-origin', [page(), asset('app.js', 'text/javascript', `
    const stringify = JSON.stringify;
    JSON.stringify = (value, ...rest) => stringify(value && typeof value === 'object' && 'method' in value ? { ...value, origin: '' } : value, ...rest);
    act.onclick = () => chariox.call('forged');`)]);
  await app.agentClick();
  assert.deepEqual(await app.until('forged'), [agent]);
});

test('MP-08/MP-11 #1001 P1: a browser-generated scroll from an agent\'s delayed work runs as that agent', async () => {
  const app = await openApp('delayed-scroll', [page(), asset('app.js', 'text/javascript', `
    let sent = false;
    act.onclick = () => setTimeout(() => scrollTo(0, 1500), 6000);
    addEventListener('scroll', () => { if (!sent) { sent = true; chariox.call('scrolled'); } });`)]);
  await app.agentClick();
  assert.deepEqual(await app.until('scrolled', 160), [agent]);
});

test('MP-08/MP-11 #1001 P1: a delayed same-origin navigation keeps the agent that caused it', async () => {
  const app = await openApp('delayed-navigation', [page(), asset('next.html', 'text/html', '<script src="next.js"></script>'),
    asset('app.js', 'text/javascript', `act.onclick = () => setTimeout(() => location.assign('/next.html'), 1500);`),
    asset('next.js', 'text/javascript', `chariox.call('startup');`)]);
  await app.agentClick();
  assert.deepEqual(await app.until('startup', 160), [agent]);
});

const vaultPage = page('', '<input id="pin" type="password" aria-label="PIN">');
const vaultApp = `pin.addEventListener('change', () => chariox.call('filled'));`;

test('MP-08/MP-11 #1001 P1: an agent\'s Vault fill is its input to the App Tab before any field handler runs', async () => {
  const app = await openApp('vault-fill', [vaultPage, asset('app.js', 'text/javascript', vaultApp)]);
  const reply = await app.agentFill();
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.deepEqual(await app.until('filled'), [agent]);
});

test('MP-08/MP-11 #1001 P1: a Vault fill into an App Tab the kernel did not claim for the agent is refused', async () => {
  const app = await openApp('vault-unclaimed', [vaultPage, asset('app.js', 'text/javascript', vaultApp)]);
  const reply = await app.agentFill(false);
  assert.equal(reply.ok, false);
  assert.equal(reply.error.code, 'user_domain_not_granted');
  assert.deepEqual(await app.until('filled', 20), []);
  assert.equal(await app.evaluate('pin.value'), '');
});

test('MP-08/MP-11: a Tab no agent drove runs every call as the person', async () => {
  const app = await openApp('owner-only', [page(), asset('app.js', 'text/javascript', `
    addEventListener('load', () => chariox.call('load'));
    setTimeout(() => chariox.call('timer'), 300);`)]);
  assert.deepEqual(await app.until('load'), [null]);
  assert.deepEqual(await app.until('timer'), [null]);
});
