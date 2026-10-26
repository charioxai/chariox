// MP-08/MP-10/MP-11: real page bridge coverage, supplementary to live D1 drills.
import test, { after, before } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'explicit disposable root required');
let root, host;
before(async () => { root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'inspection-')); host = new KernelBrowserHost(root); });
after(async () => { await host?.stop(); if (root) await rm(root, { recursive: true, force: true }); });
const asset = (name, text) => ({ path: name, content_type: name.endsWith('.js') ? 'text/javascript' : 'text/html', body_base64: Buffer.from(text).toString('base64') });

test('A4: inspect covers form fields and annotated actions, warns on an undeclared button without reading field values', async () => {
  const html = `<form id="rename" data-chariox-action="rename" data-chariox-effect="write" data-chariox-description="Rename">
    <input name="title" value="private-field"><input name="password" type="password" value="private-password"><button>Save</button>
    <input type="file" id="unsupported"><button type="reset" id="reset">Reset</button>
  </form><button data-chariox-action="add" id="add">Add</button><button id="missing">Uncovered</button>
  <button hidden id="hidden">Hidden</button><button style="visibility:hidden" id="invisible">Invisible</button>
  <script src="app.js"></script>`;
  const script = `chariox.action({ name:'add', description:'Add', inputSchema:{type:'object'}, effect:'write' }, () => {}); chariox.view.state(() => ({ count: 1 })); window.ready = true;`;
  const opened = await host.handle({ id: 1, method: 'browser.app.open', params: { origin_label: 'inspection', installation_id: 'inspection', assets: [asset('index.html', html), asset('app.js', script)] } });
  assert.equal(opened.ok, true, JSON.stringify(opened));
  const { connection, sessionId } = await host.browser.resolvePageTarget(opened.result.target_id);
  const evaluate = async expression => {
    const reply = await connection.send('Runtime.evaluate', { expression, awaitPromise:true, returnByValue:true }, sessionId);
    assert.ok(!reply.exceptionDetails, reply.exceptionDetails?.text);
    return reply.result.value;
  };
  for (let i=0; i<200 && !await evaluate('window.ready === true'); i++) await new Promise(r => setTimeout(r, 25));
  await evaluate(`(() => { const widget=document.createElement('div'); document.body.append(widget); widget.attachShadow({mode:'open'}).innerHTML='<button id="shadow-add" data-chariox-action="add">Add</button><button id="shadow-missing">Missing</button>'; })()`);
  const inspected = await host.handle({ id: 2, method: 'browser.app.inspect', params: { target_id: opened.result.target_id } });
  assert.equal(inspected.ok, true, JSON.stringify(inspected));
  const result = inspected.result;
  const unbound = await host.handle({ id: 3, method: 'browser.app.inspect', params: { target_id: 'not-an-app' } });
  assert.equal(unbound.ok, false);
  assert.deepEqual(result.registrations.map(r => r.name), ['add', 'rename']);
  assert.deepEqual(result.state, {ok:true, value:{count:1}});
  assert.equal(result.controls.filter(c => c.action === 'rename').length, 3);
  assert.equal(result.controls.filter(c => c.action === 'add').length, 2);
  assert.deepEqual(result.controls.filter(c => c.action === null).map(c => c.control), ['input#unsupported', 'button#reset', 'button#missing', 'button#shadow-missing']);
  assert.ok(!JSON.stringify(result).includes('private-'));
  await evaluate('chariox.view.state(() => "x".repeat(65537))');
  const oversized = await evaluate('__charioxViewAgent.inspect()');
  assert.equal(oversized.state.code, 'state_too_large');
  await evaluate('chariox.view.state(() => new Promise(() => {}))');
  const timed = await evaluate('__charioxViewAgent.inspect()');
  assert.equal(timed.state.code, 'state_timeout');
  await evaluate('chariox.view.state(() => null); for (let i=0;i<513;i++) document.body.append(document.createElement("button"))');
  const excessive = await host.handle({id:4,method:'browser.app.inspect',params:{target_id:opened.result.target_id}});
  assert.equal(excessive.ok,false);
  assert.equal(excessive.error.code,'app_view_coverage_too_large');
});
