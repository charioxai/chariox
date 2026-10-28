import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

const asset = (name, type, body) => ({ path: name, content_type: type, body_base64: Buffer.from(body).toString('base64') });
const html = `<!doctype html><button id="human">Human call</button><button id="agent">Action</button>
<input id="filled"><form id="save" data-chariox-action="save_form" data-chariox-effect="write"><input name="title" value="Draft"><button>Save</button></form>
<form data-chariox-action="unregistered" data-chariox-effect="write"></form><script src="app.js"></script>`;
const script = `
  let retained;
  const spec = name => ({name, description:name, effect:'write', inputSchema:{type:'object'}});
  chariox.action(spec('hold'), async (input, caller) => { retained = caller; await caller.call('agent_hold', input); return {done:true}; });
  const clicked = chariox.action(spec('clicked'), async (input, caller) => { await caller.call('clicked'); await caller.call('after_click'); });
  document.querySelector('#agent').onclick = event => { window.lastInputTimestamp = event.timeStamp; return clicked({}); };
  chariox.action(spec('compete'), async (input, caller) => { await caller.call('compete'); return {done:true}; });
  chariox.action(spec('throws'), async (input, caller) => { retained = caller; await caller.call('before_throw'); throw new Error('handler refused'); });
  chariox.action(spec('late'), async () => retained.call('late'));
  const filled = chariox.action(spec('filled'), async (input, caller) => { await caller.call('filled'); await caller.call('after_fill'); });
  document.addEventListener('input', event => { if (event.target.id === 'filled') return filled({}); }, true);
  document.querySelector('#human').onclick = () => chariox.call('human');
  chariox.form(document.querySelector('#save'), async (input, caller) => { await caller.call('save', input); await caller.call('refresh'); });
  window.ready = true;`;

test('caller attribution and registered form completion through Chromium', async () => {
  assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'explicit disposable root required');
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'app-caller-'));
  const host = new KernelBrowserHost(root);
  let request = 0;
  const handle = (method, params) => host.handle({ id: ++request, method, params });
  const observations = {};
  try {
    const opened = await handle('browser.app.open', { origin_label:'caller-test', installation_id:'caller-test', assets:[
      asset('index.html','text/html',html), asset('app.js','text/javascript',script)] });
    assert.equal(opened.ok, true, JSON.stringify(opened));
    const target = opened.result.target_id;
    const {connection,sessionId} = await host.browser.resolvePageTarget(target);
    const evaluate = async expression => {
      const reply = await connection.send('Runtime.evaluate', { expression, returnByValue:true, awaitPromise:true }, sessionId);
      assert.equal(reply.exceptionDetails, undefined, JSON.stringify(reply.exceptionDetails));
      return reply.result.value;
    };
    for(let i=0;i<200 && !await evaluate('window.ready === true');i++) await delay(20);
    assert.equal(await evaluate('window.ready === true'), true);
    const agent = (name, id='agent-one') => handle('browser.app.agent', { target_id:target, agent:id, call:'invoke', name, input:{}, deadline_ms:5000 });
    const queued = [];
    async function nextCall() {
      for(let i=0;i<100;i++) {
        queued.push(...(await host.browser.appTabs.takeCalls()).calls);
        if(queued.length) return queued.shift();
        await delay(10);
      }
      throw new Error('expected a backend call');
    }
    const respond = call => handle('browser.app.respond', {target_id:target,call_id:call.call_id,result:{done:true}});
    async function click(selector) {
      const pos = await evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return{x:r.x+r.width/2,y:r.y+r.height/2}})()`);
      for(const type of ['mousePressed','mouseReleased']) await connection.send('Input.dispatchMouseEvent',{type,...pos,button:'left',clickCount:1},sessionId);
    }
    async function browserInput(input, id) {
      const tab = [...host.tabs.values()].find(tab => tab.target_id === target);
      const reply = await handle('host.browser', {op:'input', tab_id:tab.tab_id, generation:host.generation,
        document_id:tab.document_id, input, ...(id ? {_agent_input:true, observed_by:'agent:'+id, _app_view_tab:tab.tab_id} : {})});
      assert.equal(reply.ok, true, JSON.stringify(reply));
    }
    async function browserClick(selector, id) {
      const pos = await evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return{x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2)}})()`);
      await browserInput({kind:'click',...pos},id);
    }
    for (const [input, firstMethod, secondMethod] of [['click','clicked','after_click'],['keyboard','save','refresh'],['form_click','save','refresh'],['room_click','clicked','after_click'],['room_submit','save','refresh'],['room_fill_submit','save','refresh'],['room_fill','filled','after_fill']]) {
      if (input.startsWith('room_')) {
        const selector = input === 'room_click' ? '#agent' : input === 'room_fill' ? '#filled' : '#save input';
        const { root } = await connection.send('DOM.getDocument', {}, sessionId);
        const { nodeId } = await connection.send('DOM.querySelector', { nodeId: root.nodeId, selector }, sessionId);
        const { node } = await connection.send('DOM.describeNode', { nodeId }, sessionId);
        const { frameTree } = await connection.send('Page.getFrameTree', {}, sessionId);
        const action = input === 'room_click' ? {kind:'click'} : input === 'room_submit' ? {kind:'submit'} : {kind:'fill',text:'Edited',append:false,submit:input !== 'room_fill'};
        await host.browser.performAction({ target_id:target, document_id:frameTree.frame.loaderId,
          node_ref:'backend:'+node.backendNodeId, action, timeout_ms:500, agent_id:'agent-one' });
      } else if (input === 'click') await browserClick('#agent','agent-one');
      else if (input === 'form_click') await browserClick('#save button','agent-one');
      else {
        await browserClick('#save input');
        await browserInput({kind:'key',key:'Enter'},'agent-one');
      }
      if (input === 'click' || input === 'room_click') assert.equal(await evaluate('lastInputTimestamp <= performance.now()'),true,'input labels must not change App event clocks');
      const firstInput = await nextCall();
      assert.equal(firstInput.method, firstMethod);
      assert.deepEqual(firstInput.actor,{kind:'agent',id:'agent-one'}, 'ordinary browser input must carry the invoking agent');
      await browserClick('#human');
      const humanInput = await nextCall(); assert.equal(humanInput.method,'human'); assert.equal(humanInput.actor,undefined);
      await respond(humanInput); await respond(firstInput);
      const afterInput = await nextCall(); assert.equal(afterInput.method,secondMethod);
      assert.deepEqual(afterInput.actor,{kind:'agent',id:'agent-one'}, 'the input caller survives an awaited backend reply');
      await respond(afterInput);
      for (let i=0;i<100 && [...host.browser.appTabs.apps.values()].some(app=>app.agentRunning);i++) await delay(10);
      observations[input+'Input'] = {agent:firstInput.actor,human:humanInput.actor??null,after:afterInput.actor};
      const retiredInput = firstInput.call_id.split(':')[1];
      assert.equal((await evaluate(`__charioxViewAgent.invoke('hold',{},${JSON.stringify(retiredInput)})`)).code,'APP_INVOCATION_EXPIRED');
    }
    const holding = agent('hold');
    const first = await nextCall(); assert.equal(first.method,'agent_hold');
    await click('#human');
    const human = await nextCall(); assert.equal(human.method,'human');
    observations.concurrentHuman = {agent:first.actor??null,human:human.actor??null};
    assert.deepEqual(first.actor,{kind:'agent',id:'agent-one'});
    assert.equal(human.actor,undefined);
    assert.equal((await agent('compete','agent-two')).result.code,'view_busy');
    await respond(human); await respond(first); assert.equal((await holding).result.ok,true);
    assert.equal((await agent('late')).result.code,'APP_INVOCATION_EXPIRED');
    const retiredToken = first.call_id.split(':')[1];
    const reused = await evaluate(`__charioxViewAgent.invoke('hold',{},${JSON.stringify(retiredToken)})`);
    assert.equal(reused.code,'APP_INVOCATION_EXPIRED');
    const unknown = await evaluate(`__charioxViewAgent.invoke('hold',{},'unknown-token')`);
    assert.equal(unknown.code,'APP_INVOCATION_EXPIRED');
    observations.retiredToken = reused.code;
    observations.unknownToken = unknown.code;

    let completed = false;
    const submitting = agent('save_form').then(reply => {completed=true;return reply;});
    const saving = await nextCall(); assert.equal(saving.method,'save');
    await delay(50);
    const competing = await agent('compete','agent-two');
    observations.asyncForm = {returnedBeforeSaveReply:completed,competingAccepted:competing.result?.ok===true,first:saving.actor??null};
    assert.equal(completed,false); assert.equal(competing.result.code,'view_busy');
    await respond(saving);
    const refreshing = await nextCall(); assert.equal(refreshing.method,'refresh');
    observations.asyncForm.second = refreshing.actor??null;
    assert.deepEqual(refreshing.actor,{kind:'agent',id:'agent-one'});
    assert.equal(completed,false);
    await respond(refreshing); assert.equal((await submitting).result.ok,true);
    const secondAgent = agent('compete','agent-two');
    const second = await nextCall(); assert.deepEqual(second.actor,{kind:'agent',id:'agent-two'});
    await respond(second); assert.equal((await secondAgent).result.ok,true);
    observations.secondAgent = second.actor;
    assert.equal((await agent('unregistered')).result.code,'form_unregistered');
    observations.unregistered = 'form_unregistered';
    const throwing = agent('throws');
    const beforeThrow = await nextCall(); assert.equal(beforeThrow.method,'before_throw');
    await respond(beforeThrow); assert.equal((await throwing).result.code,'action_failed');
    assert.equal((await agent('late')).result.code,'APP_INVOCATION_EXPIRED');
    observations.throwingHandler = 'action_failed';
    await click('#save button');
    const humanSave = await nextCall(); assert.equal(humanSave.method,'save'); assert.equal(humanSave.actor,undefined);
    await respond(humanSave);
    const humanRefresh = await nextCall(); assert.equal(humanRefresh.method,'refresh'); assert.equal(humanRefresh.actor,undefined);
    await respond(humanRefresh);
    observations.humanForm = [humanSave.actor??null,humanRefresh.actor??null];
    if(process.env.CHARIOX_CALLER_EVIDENCE) {
      await writeFile(path.join(process.env.CHARIOX_CALLER_EVIDENCE,'actor-repro.json'),JSON.stringify(observations,null,2));
      await writeFile(path.join(process.env.CHARIOX_CALLER_EVIDENCE,'actor-form.png'),Buffer.from((await connection.send('Page.captureScreenshot',{format:'png'},sessionId)).data,'base64'));
    }
  } finally { await host.stop(); await rm(root,{recursive:true,force:true}); }
});
