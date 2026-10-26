// Supplementary real-Chromium/A2 regression. The backend below is a fixture;
// this does not establish install, WASI files, provider or hosted acceptance.
import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const [bundle, evidence, runtime] = process.argv.slice(2).map(p => path.resolve(p));
assert(bundle && evidence && runtime, 'usage: drill-view.mjs <bundle> <evidence> <browser modules>');
assert(process.env.CHARIOX_HOME && process.env.TMPDIR, 'set disposable CHARIOX_HOME and TMPDIR');
await mkdir(evidence, { recursive: true });
const { KernelBrowserHost } = await import(pathToFileURL(path.join(runtime, 'kernel-browser-host.mjs')));
const host = new KernelBrowserHost(process.env.CHARIOX_HOME);
const files = new Map([['Budget.csv', 'Item,Qty,Price\r\nTea,2,3\r\nCoffee,3,4\r\nMilk,1,5\r\n']]);
const calls = [], errors = [];
let request = 0, target, running = true, stored = null, version = null;
const handle = (method, params) => host.handle({ id: ++request, method, params });
const fixture = async (name, input) => {
  if (name === 'load_workbook') return { workbook: stored, version };
  if (name === 'save_workbook') {
    assert.equal(input.version, version, 'optimistic save version');
    stored = structuredClone(input.workbook); version = String(Number(version ?? 0) + 1); return { version };
  }
  if (name === 'read_file') {
    assert.ok(files.has(input.name), `fixture file ${input.name}`);
    return { name: input.name, text: files.get(input.name) };
  }
  if (name === 'write_file') {
    files.set(input.name, input.text); await writeFile(path.join(evidence, input.name), input.text);
    return { name: input.name, bytes: Buffer.byteLength(input.text), grantId: null };
  }
  if (name === 'pick_file') return { grants: [] };
  throw new Error(`unexpected fixture backend call: ${name}`);
};
const serve = (async () => {
  while (running) {
    if (target) for (const call of (await host.browser.appTabs.takeCalls()).calls) {
      calls.push({ method: call.method, actor: call.actor ?? null });
      try {
        const result = await fixture(call.method, call.params);
        await handle('browser.app.respond', { target_id: target, call_id: call.call_id, result });
      } catch (error) {
        errors.push(error.message);
        await handle('browser.app.respond', { target_id: target, call_id: call.call_id, error: { code: 'FIXTURE_FAILED', message: error.message } });
      }
    }
    await delay(15);
  }
})();
try {
  const assets = [];
  for (const name of await readdir(path.join(bundle, 'ui'))) {
    assets.push({ path: name, content_type: name.endsWith('.html') ? 'text/html' : name.endsWith('.css') ? 'text/css' : 'text/javascript', body_base64: (await readFile(path.join(bundle, 'ui', name))).toString('base64') });
  }
  const opened = await handle('browser.app.open', { origin_label: 'univer-drill', installation_id: 'univer-drill', assets });
  assert.equal(opened.ok, true, JSON.stringify(opened)); target = opened.result.target_id;
  const { connection, sessionId } = await host.browser.resolvePageTarget(target);
  const cdp = (method, params = {}) => connection.send(method, params, sessionId);
  connection.subscribe(event => {
    if (event.method === 'Runtime.exceptionThrown') errors.push(String(event.params.exceptionDetails.exception?.description ?? event.params.exceptionDetails.text).slice(0, 1000));
  });
  await cdp('Runtime.enable');
  const evaluate = async expression => {
    const answer = await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (answer.exceptionDetails) throw new Error(JSON.stringify(answer.exceptionDetails));
    return answer.result.value;
  };
  const agent = async (call, params = {}) => {
    const reply = await handle('browser.app.agent', { target_id: target, agent: 'univer-regression-agent', call, ...params });
    assert.equal(reply.ok, true, JSON.stringify(reply)); return reply.result;
  };
  let registrations;
  for (let i = 0; i < 400; i++) {
    const found = await agent('registrations');
    if (found.ok && found.value.length) { registrations = found.value; break; }
    const failure = await evaluate('document.getElementById("app").textContent.slice(0,500)');
    if (failure.startsWith('Univer could not open:')) throw new Error(failure);
    await delay(50);
  }
  assert.ok(registrations, 'view registered its signed actions');
  const signed = JSON.parse(await readFile(path.join(bundle, 'schemas/ui-actions.json'), 'utf8')).uiActions;
  assert.deepEqual(registrations, signed, 'runtime registration equals the signed catalogue');
  const invoke = async (name, input = {}) => {
    const reply = await agent('invoke', { name, input, deadline_ms: 20000 });
    assert.equal(reply.ok, true, `${name}: ${JSON.stringify(reply)}`); return reply.value;
  };
  const log = [];
  log.push({ import: await invoke('import_csv', { name: 'Budget.csv' }) });
  log.push({ formula: await invoke('set_range', { range: 'D1:D4', values: [['Total'], ['=B2*C2'], ['=B3*C3'], ['=B4*C4']] }) });
  assert.deepEqual((await invoke('read_range', { range: 'D2:D4' })).values, [[6], [12], [5]]);
  log.push({ sorted: await invoke('sort_range', { range: 'A2:D4', column: 1, ascending: false }) });
  for (const axis of ['rows', 'columns']) {
    await invoke('edit_dimension', { axis, operation: 'insert', index: 5, count: 1 });
    await invoke('edit_dimension', { axis, operation: 'delete', index: 5, count: 1 });
  }
  const data = await invoke('read_range', { range: 'A2:D4' });
  assert.deepEqual(data.values.map(row => row[0]), ['Coffee', 'Tea', 'Milk']);
  for (const row of data.values) assert.equal(row[3], row[1] * row[2], 'independent formula recomputation after sort');
  const saved = await invoke('save_workbook'); assert.equal(saved.dirty, false);
  await invoke('export_csv', { name: 'Budget-export.csv' });
  await invoke('export_workbook', { name: 'Budget.univer.json' });
  await invoke('set_range', { range: 'B2', values: [[99]] });
  await invoke('open_workbook', { name: 'Budget.univer.json' });
  assert.deepEqual(await invoke('read_range', { range: 'A2:D4' }), data);
  const exported = JSON.parse(files.get('Budget.univer.json'));
  assert.equal(exported.workbook.sheets.sheet1.cellData[1][3].f, data.formulas[0][3]);
  const changes = await agent('changes', { since: 0 }); assert.ok(changes.ok && changes.value.latest > 0);
  const state = await agent('state'); assert.equal(state.ok, true);
  log.push({ state: state.value, data, changes: changes.value.latest, calls });
  for (const dpr of [1, 2]) {
    await cdp('Emulation.setDeviceMetricsOverride', { width: 1440, height: 900, deviceScaleFactor: dpr, mobile: false });
    await delay(500);
    const screenshot = await cdp('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(evidence, `canvas-dpr${dpr}.png`), Buffer.from(screenshot.data, 'base64'));
    assert.ok(await evaluate('document.querySelectorAll("canvas").length > 0'), 'canvas rendered');
    await writeFile(path.join(evidence, `controls-dpr${dpr}.json`), JSON.stringify(await evaluate(`[...document.querySelectorAll('button,[role="button"]')].map(e=>({text:e.textContent.trim().slice(0,100),label:e.getAttribute('aria-label')})).slice(0,150)`), null, 2));
    const bounds = await evaluate(`(() => { for(const el of document.querySelectorAll('button,[role="button"]')) { if(el.textContent.trim()!=='Files')continue; const r=el.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2; if(r.width&&r.height&&x>0&&y>0&&x<innerWidth&&y<innerHeight&&el.contains(document.elementFromPoint(x,y)))return {x,y}; } return null; })()`);
    assert.ok(bounds, 'existing Univer Files menu is visible');
    await cdp('Input.dispatchMouseEvent', { type: 'mouseMoved', ...bounds });
    await delay(300);
    if (!await evaluate('document.body.innerText.includes("Export workbook")')) {
      for (const type of ['mousePressed', 'mouseReleased']) await cdp('Input.dispatchMouseEvent', { type, ...bounds, button: 'left', clickCount: 1 });
    }
    await delay(250);
    const menu = await cdp('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(evidence, `files-menu-dpr${dpr}.png`), Buffer.from(menu.data, 'base64'));
    await writeFile(path.join(evidence, `menu-text-dpr${dpr}.txt`), await evaluate('document.body.innerText.slice(0,12000)'));
    await writeFile(path.join(evidence, `menu-button-dpr${dpr}.json`), JSON.stringify(await evaluate(`({bounds:${JSON.stringify(bounds)},html:document.querySelector('[data-u-command="chariox.files"]')?.outerHTML})`), null, 2));
    assert.ok(await evaluate('document.body.textContent.includes("Export workbook")'), 'file menu opened');
    await cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
    await cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
  }
  await writeFile(path.join(evidence, 'view-results.json'), JSON.stringify({ kind: 'supplementary-A2-browser-fixture', browser: await connection.send('Browser.getVersion'), log, errors }, null, 2));
  assert.deepEqual(errors, []);
  console.log('PASS: real Univer canvas, A2 declarations, CSV/formulas/sort/save/export/reopen; backend fixture only');
} finally {
  running = false; await serve; await host.stop();
}
