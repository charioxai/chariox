// MP-08/MP-10/MP-11: native default actions keep delegated App behavior.
import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

for (const key of [false, true]) for (const behavior of ['shadow-name', 'shadow-id']) {
  test(`registered form ${key ? 'Enter' : 'click'} preserves ${behavior}`, async () => {
    assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'explicit disposable root required');
    const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'form-input-'));
    const host = new KernelBrowserHost(root);
    let request = 0;
    const handle = (method, params) => host.handle({id:++request,method,params});
    try {
      const html = `<div id="outer"><form id="form"><input name="title" value="Draft">${behavior.startsWith('shadow-') ? `<input ${behavior === 'shadow-id' ? 'id' : 'name'}="requestSubmit">` : ''}<button>Save</button></form></div><script src="app.js"></script>`;
      const script = `
        const form = document.querySelector('form');
        chariox.form(form, async (input, caller) => { await caller.call('save', input); await caller.call('refresh'); });
        outer.addEventListener(${JSON.stringify(key ? 'keydown' : 'click')}, event => {
          if (${JSON.stringify(behavior)} === 'cancel') event.preventDefault();
          if (${JSON.stringify(behavior)} === 'update') form.elements.title.value = 'Delegated';
        }); window.ready = true;`;
      const opened = await handle('browser.app.open', {origin_label:'form-input',installation_id:'form-input',assets:[
        ['index.html','text/html',html],['app.js','text/javascript',script],
      ].map(([path,content_type,body]) => ({path,content_type,body_base64:Buffer.from(body).toString('base64')}))});
      assert.equal(opened.ok, true, JSON.stringify(opened));
      const target = opened.result.target_id;
      const {connection,sessionId} = await host.browser.resolvePageTarget(target);
      const evaluate = async expression => {
        const reply = await connection.send('Runtime.evaluate',{expression,returnByValue:true},sessionId);
        assert.equal(reply.exceptionDetails,undefined,JSON.stringify(reply.exceptionDetails));
        return reply.result.value;
      };
      for(let i=0;i<200 && !await evaluate('window.ready === true');i++) await delay(10);
      assert.equal(await evaluate('window.ready === true'),true);
      await evaluate(`document.querySelector('input').focus()`);
      const position = await evaluate(`(()=>{const r=document.querySelector('button').getBoundingClientRect();return{x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2)}})()`);
      const tab = [...host.tabs.values()].find(tab => tab.target_id === target);
      const reply = await handle('host.browser',{op:'input',tab_id:tab.tab_id,generation:host.generation,document_id:tab.document_id,
        input:key ? {kind:'key',key:'Enter'} : {kind:'click',...position},_agent_input:true,observed_by:'agent:one',_app_view_tab:tab.tab_id});
      assert.equal(reply.ok,true,JSON.stringify(reply));
      const calls = [];
      for(let i=0;i<10 && !calls.length;i++) { calls.push(...(await host.browser.appTabs.takeCalls()).calls); await delay(10); }
      if (behavior === 'cancel') {
        assert.equal(calls.length,0,'delegated preventDefault must cancel native submission');
      } else {
        assert.equal(calls.length,1,'native submission must reach the registered handler');
        const [saving] = calls;
        assert.equal(saving.method,'save');
        assert.equal(saving.params.title,behavior === 'update' ? 'Delegated' : 'Draft');
        assert.deepEqual(saving.actor,{kind:'agent',id:'one'});
        await handle('browser.app.respond',{target_id:target,call_id:saving.call_id,result:{}});
        let refreshing;
        for(let i=0;i<100 && !refreshing;i++) { [refreshing] = (await host.browser.appTabs.takeCalls()).calls; await delay(10); }
        assert.equal(refreshing?.method,'refresh');
        assert.deepEqual(refreshing.actor,{kind:'agent',id:'one'});
        await handle('browser.app.respond',{target_id:target,call_id:refreshing.call_id,result:{}});
      }
    } finally { await host.stop(); await rm(root,{recursive:true,force:true}); }
  });
}
