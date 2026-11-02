// MP-08/MP-09/MP-10/MP-11 BROWSER09: real Chromium + existing host dialog path.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

const root = process.env.CHARIOX_MDACCESS_DRILL_ROOT;
assert(root, 'explicit disposable drill root required');
const delay = ms => new Promise(r => setTimeout(r, ms));

test('MP-08/09/10/11 permission requests share state, answer, timeout, session and cleanup', {timeout:90000}, async () => {
  const home = await mkdtemp(path.join(root, 'permissions-'));
  const host = new KernelBrowserHost(home);
  const server = createServer((_q, r) => r.end('<!doctype html><title>Permissions</title><p>Permission regression</p>'));
  try {
    await new Promise(r => server.listen(0, '127.0.0.1', r));
    const url = `http://127.0.0.1:${server.address().port}/`;
    const opened = await host.request({op:'open',url});
    let tab = host.tabs.get(opened.tabs[0].tab_id);
    let {connection,sessionId} = await host.browser.resolvePageTarget(tab.target_id);
    const evaluate = async expression => {
      const response = await connection.send('Runtime.evaluate', {expression,returnByValue:true,userGesture:true,awaitPromise:true}, sessionId);
      assert.ok(!response.exceptionDetails, 'fixture evaluation threw'); return response.result?.value;
    };
    const pending = async (permission, origin = new URL(url).origin) => {
      for (let i=0;i<100;i++) {
        const state = await host.request({op:'state'});
        const dialog = state.dialogs?.find(d => d.kind === 'permission' && d.message === permission);
        if (dialog) {assert.equal(dialog.origin,origin);return dialog;}
        await delay(20);
      }
      assert.fail(`${permission} did not surface in shared state`);
    };
    const answer = (d,action) => host.request({op:'dialog',tab_id:d.tab_id,generation:d.generation,document_id:d.document_id,dialog_id:d.dialog_id,action});
    assert.equal(await evaluate('navigator.permissions.query({name:"geolocation"}).then(s=>s.state)'), 'prompt');
    await evaluate('navigator.geolocation.getCurrentPosition(()=>{window.geo="ok"},e=>{window.geo=e.code});undefined');
    const geo = await pending('geolocation'); await answer(geo,'accept');
    assert.equal(await evaluate('navigator.permissions.query({name:"geolocation"}).then(s=>s.state)'), 'granted');
    assert.equal(await evaluate('Permissions.prototype.query.call(navigator.permissions,{name:"geolocation"}).then(s=>s.state)'), 'granted', 'Chromium must receive the real native grant');
    await assert.rejects(answer(geo,'accept'), {code:'browser_dialog_invalid'});
    await evaluate('Notification.requestPermission().then(s=>window.notified=s);undefined');
    const notifications = await pending('notifications'); await answer(notifications,'dismiss');
    for(let i=0;i<100 && await evaluate('window.notified')!=='denied';i++) await delay(10);
    assert.equal(await evaluate('window.notified'), 'denied');
    assert.equal(await evaluate('Notification.permission'), 'denied');
    assert.match(await evaluate('navigator.mediaDevices.getUserMedia({video:true}).catch(e=>e.message)'), /unavailable/);
    assert.match(await evaluate('navigator.mediaDevices.getDisplayMedia({video:true}).catch(e=>e.message)'), /unavailable/);
    // Real 30-second default, no test-only shortened clock.
    const started=Date.now();
    await evaluate('navigator.clipboard.readText().then(()=>window.clip="allowed",e=>window.clip=e.name);undefined');
    await pending('clipboard-read');
    for(let i=0;i<350 && await evaluate('window.clip')!=='NotAllowedError';i++) await delay(100);
    assert.equal(await evaluate('window.clip'), 'NotAllowedError'); assert.ok(Date.now()-started>=29000);
    assert.deepEqual((await host.request({op:'state'})).dialogs,[]);
    // New origins cannot reuse a grant. Closing the tab cancels its timer/dialog.
    const other=await host.request({op:'open',url:url.replace('127.0.0.1','localhost')});
    tab=host.tabs.get(other.active_tab_id); ({connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id));
    await evaluate('navigator.geolocation.getCurrentPosition(()=>{},()=>{});undefined');
    const isolated=await pending('geolocation',new URL(url.replace('127.0.0.1','localhost')).origin); assert.equal(isolated.origin,new URL(url.replace('127.0.0.1','localhost')).origin);
    await host.request({op:'close',tab_id:tab.tab_id,generation:other.generation});
    assert.equal(host.browser.pagePermissions.requests.size,0);
    await host.stop();
    const restarted=await host.request({op:'open',url});
    tab=host.tabs.get(restarted.active_tab_id); ({connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id));
    assert.equal(await evaluate('navigator.permissions.query({name:"geolocation"}).then(s=>s.state)'), 'prompt');
    await evaluate('navigator.geolocation.getCurrentPosition(()=>{},()=>{});undefined');
    await answer(await pending('geolocation'),'dismiss');
  } finally {await host.stop();await new Promise(r=>server.close(r));await rm(home,{recursive:true,force:true});}
});
