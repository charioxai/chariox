// MP-08 / MP-11: fail-first adapter protection, binding and keyboard contracts.
import test from 'node:test';
import assert from 'node:assert/strict';
import { NativeComputer, nativeInput, executeNative } from './native-computer.mjs';
import { LinuxOwnedDesktop } from './linux-owned-desktop.mjs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
const binding = {surface_id:'surface',generation:'generation',width:1280,height:800,environment:{DISPLAY:':999'}};
test('MP-08/MP-10 native injection timings stay private and contain only numbers', async () => {
  const events=[];
  const timing=()=>{};timing.event=(...event)=>events.push(event);
  const adapter=new NativeComputer({placement:'host',binding:()=>binding,timing,
    execute:async()=>({applied:true,_timing:{started_ms:100,ended_ms:102,text:'must not be logged'}})});
  const result=await adapter.request({op:'input',surface_id:'surface',generation:'generation',input:{kind:'key',key:'a'}},{});
  assert.deepEqual(result,{applied:true});
  const injection=events.find(([stage])=>stage==='computer_x11');
  assert(injection);assert.equal(injection[1].at,100);assert.equal(injection[1].injected_ms,102);
  assert(Object.values(injection[1]).every(Number.isFinite));
});
const publicDependencies=Object.fromEntries(['PYTHONPATH','LD_LIBRARY_PATH','GI_TYPELIB_PATH'].filter(key=>process.env[key]).map(key=>[key,process.env[key]]));
test('MP-11 native input rejects stale placement and invalid physical events', () => {
  for (const input of [{kind:'keycode',keycode:1,state:'down'},{kind:'keycode',keycode:38,state:'wrong'},{kind:'click',x:1280,y:1},{kind:'hold',key:'a',duration_ms:10001}]) assert.throws(() => nativeInput(input,binding));
  assert.deepEqual(nativeInput({kind:'keycode',keycode:38,state:'down'},binding),{kind:'keycode',keycode:38,state:'down'});
});
test('MP-11 unknown observation protection never invokes native capture', async () => {
  let calls=0;
  const adapter=new NativeComputer({placement:'host',binding:()=>binding,execute:async()=>{calls++;return {};}});
  await assert.rejects(adapter.request({op:'screenshot',surface_id:'surface',generation:'generation'}, {unknown:true}),/protection/);
  await assert.rejects(adapter.request({op:'screenshot',surface_id:'foreign',generation:'generation'}, {}),/stale/);
  assert.equal(calls,0);
});
test('MP-08 / MP-11 kernel-browser pixels are revealed only under a fenced CDP measurement', async () => {
  // A browser with no visible pages measures as {pages: []}; a broken one fails closed.
  const browser = layout => ({ ensureConnection: async () => ({ send: async method => {
    if (method !== 'Target.getTargets') throw new Error('unexpected');
    return { targetInfos: layout.shift() ?? [] };
  } }) });
  const run = async (connected, policy = {values:[],targets:[],unknown:false}) => {
    const sent = [];
    const bound = {...binding,browser:()=>connected};
    const adapter = new NativeComputer({placement:'host',binding:()=>bound,execute:async request=>{sent.push(request);return {data_base64:''};}});
    await adapter.request({op:'screenshot',surface_id:'surface',generation:'generation'}, policy);
    return sent.map(request => request.browser_protection ?? null);
  };
  assert.deepEqual(await run(browser([[], []])), [{pages:[]}]);
  assert.deepEqual(await run(null), [null]);
  assert.deepEqual(await run({ensureConnection:async()=>{throw Error('closed')}}),[{pages:[]}]);
  // A tracked fill whose page cannot be measured is covered whole by the helper (fill, no measurement), not refused.
  assert.deepEqual(await run({ensureConnection:async()=>{throw Error('closed')}},{unknown:false,values:[],targets:[{kind:'browser',target_id:'t',value_hash:'public-hash'}]}),[null]);
  // Owner 2026-10-09: Vault values never black out the desktop; browser windows
  // keep their per-field/value masks from the same measured protection.
  assert.deepEqual(await run(browser([[], []]), {values:['v'],targets:[],unknown:false}), [{pages:[]}]);
  const requests=[];
  const unmeasurable={ensureConnection:async()=>{throw Error('closed')}};
  const unmeasurableBinding={...binding,browser:()=>unmeasurable};
  const covering=new NativeComputer({placement:'host',binding:()=>unmeasurableBinding,execute:async request=>{requests.push(request);return {data_base64:''};}});
  await covering.request({op:'screenshot',surface_id:'surface',generation:'generation'},{values:[],targets:[{kind:'browser',target_id:'t',value_hash:'public-hash'}],unknown:false});
  assert.deepEqual([requests[0].fill,requests[0].browser_protection],[true,undefined],'the helper is told a fill is tracked');
  const plain=new NativeComputer({placement:'host',binding:()=>unmeasurableBinding,execute:async request=>{requests.push(request);return {data_base64:''};}});
  await plain.request({op:'screenshot',surface_id:'surface',generation:'generation'},{values:[],targets:[],unknown:false});
  assert.equal(requests[1].fill,undefined,'a capture without a Vault fill carries no fill flag');
  // A window AT-SPI has not yet bound is retried, and the count stays private.
  const withheld=[1,0],bound={...binding,browser:()=>browser([[],[],[],[]])};
  const adapter=new NativeComputer({placement:'host',binding:()=>bound,execute:async()=>({data_base64:'',browser_withheld:withheld.shift()})});
  assert.deepEqual(await adapter.request({op:'screenshot',surface_id:'surface',generation:'generation'},{values:[],targets:[],unknown:false}),{data_base64:'',surface_id:'surface',generation:'generation'});
  assert.deepEqual(withheld,[]);
});
// Vault (Miguel 2026-10-09): no desktop blackout; values reach the AT-SPI text
// check, OCR text is redacted, and a clipboard read stays withheld.
test('MP-08 / MP-11 Vault values never black out the desktop; OCR is redacted, clipboard withheld', async () => {
  const sent=[];
  const browser={ensureConnection:async()=>({send:async method=>{if(method!=='Target.getTargets')throw new Error('unexpected');return {targetInfos:[]};}})};
  const bound={...binding,browser:()=>browser};
  const adapter=new NativeComputer({placement:'host',binding:()=>bound,execute:async request=>{sent.push(request);
    return request.op==='ocr'?{text:'token v-secret here'}:request.op==='clipboard_read'?{text:'[protected]'}:{data_base64:''};}});
  const vault={values:['v-secret'],targets:[],unknown:false},at={surface_id:'surface',generation:'generation'};
  await adapter.request({op:'screenshot',...at},vault);
  assert.deepEqual([sent[0].mask,sent[0].values,sent[0].browser_protection],[false,['v-secret'],{pages:[]}]);
  assert.equal((await adapter.request({op:'ocr',...at},vault)).text,'token [redacted] here');
  await adapter.request({op:'clipboard_read',...at},vault);
  assert.equal(sent.at(-1).mask,true);
  await adapter.request({op:'clipboard_read',...at},{values:[],targets:[],unknown:false});
  assert.equal(sent.at(-1).mask,false);
});
test('MP-08 immediate physical key and text are distinct and wake capture after each event', async () => {
  const sent=[],wakes=[];
  const adapter=new NativeComputer({placement:'host',binding:()=>binding,execute:async request=>{sent.push(request);return {};},wakeCapture:event=>wakes.push(event)});
  for(const input of [{kind:'keycode',keycode:38,state:'down'},{kind:'keycode',keycode:38,state:'up'},{kind:'text',text:'Grüße 世界😀'}]) await adapter.request({op:'input',surface_id:'surface',generation:'generation',input},{});
  assert.equal(sent.length,3);assert.equal(wakes.length,3);assert.equal(sent[2].input.kind,'text');
});

test('MP-08 / MP-11 real warm keyboard is reaped and replaced with the desktop generation', async () => {
  const root=await mkdtemp(path.join(os.tmpdir(),'chariox-warm-generation-'));
  const desktop=new LinuxOwnedDesktop(root,{environment:{PATH:process.env.PATH,HOME:root,LANG:'C.UTF-8'}});
  const owned=await desktop.start();
  let current = { ...binding, environment: { ...owned.environment, ...publicDependencies } };
  const adapter = new NativeComputer({ placement: 'host', binding: () => current });
  try {
    await adapter.primeKeyboard();
    const first = adapter.keyboard.child;
    current = { ...current, generation: 'replacement' };
    await adapter.primeKeyboard();
    assert.notEqual(adapter.keyboard.child.pid, first.pid);
    assert(first.exitCode !== null || first.signalCode !== null);
    assert.equal(adapter.held.size, 0);
  } finally { await adapter.close(); await desktop.stop(); await rm(root,{recursive:true,force:true}); }
});

test('MP-08 native text is rejected before dispatch when it cannot fit the RPC budget', async () => {
  let calls = 0;
  const adapter = new NativeComputer({ placement: 'host', binding: () => binding, execute: async () => { calls++; return {}; } });
  await assert.rejects(adapter.request({ op: 'input', surface_id: binding.surface_id, generation: binding.generation,
    input: { kind: 'text', text: 'a'.repeat(129) } }, {}), /128/);
  assert.equal(calls, 0);
  assert.doesNotThrow(() => nativeInput({ kind: 'composition', text: '😀'.repeat(128) }, binding));
});

test('MP-08 provider shortcut casing resolves to physical base keys', () => {
  for(const [key,expected] of [['SUPER+D','super+d'],['CTRL+SHIFT+A','ctrl+shift+a'],['ALT+ENTER','alt+Return'],['Escape','Escape'],['F12','F12']])
    assert.equal(nativeInput({kind:'key',key},binding).key,expected);
  assert.equal(nativeInput({kind:'hold',key:'CTRL+A',duration_ms:5},binding).key,'ctrl+a');
});

test('agents drag, hold, write the clipboard and middle-click like any other input', async () => {
  const sent=[];
  const adapter=new NativeComputer({placement:'host',binding:()=>binding,execute:async request=>{sent.push(request);return {};}});
  adapter.writeClipboard=async()=>({applied:true});
  const inputs=[{kind:'hold',key:'a',duration_ms:500},{kind:'drag',x:10,y:10,to_x:20,to_y:20},
    {kind:'click',button:2,x:10,y:10},{kind:'pointer_hold',button:2,x:10,y:10,duration_ms:1}];
  for(const input of inputs)await adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input},{});
  assert.deepEqual(sent.map(request=>request.input.kind),['hold','drag','click','pointer_hold']);
  assert.deepEqual(await adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input:{kind:'clipboard_write',text:'ordinary'}},{}),{applied:true});
});

test('MP-11 agents still cannot hold raw physical keycodes', async () => {
  let calls=0;
  const adapter=new NativeComputer({placement:'host',binding:()=>binding,execute:async()=>{calls++;return {};}});
  await assert.rejects(adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input:{kind:'keycode',keycode:38,state:'down'}},{}));
  assert.equal(calls,0);
});

test('MP-11 finding 1 each agent text/composition/chord dispatch carries live focus admission', async () => {
  const sent=[];
  const scoped={...binding,ownedProcesses:async()=>[{pid:200,started:'1'}]};
  const adapter=new NativeComputer({placement:'host',binding:()=>scoped,execute:async request=>{sent.push(request);return {};}});
  for(const input of [{kind:'text',text:'public'},{kind:'composition',text:'public'},{kind:'key',key:'CTRL+s'}]) {
    await adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input},{});
  }
  assert(sent.every(request=>request.agent_input===true && request.processes?.[0]?.pid===200));
});

test('MP-11 finding 1 native helper uses the same typed Browser/Vault refusal for unproved focus',async()=>{
  await assert.rejects(executeNative({op:'input',agent_input:true,processes:[],input:{kind:'text',text:'public'}},
    {...publicDependencies,PATH:'/usr/bin:/bin',DISPLAY:':invalid',DBUS_SESSION_BUS_ADDRESS:'unix:path=/chariox-missing-owned-bus'}),error=>error.code==='user_domain_sensitive_requires_focus');
});

test('MP-08/MP-10/MP-11 each native agent mutation carries the current browser process identity',async()=>{
 const sent=[],processes=[{pid:200,started:'1'},{pid:300,started:'2'}];
 const scoped={...binding,ownedProcesses:async()=>processes,browserProcesses:async()=>[processes[1]]};
 const adapter=new NativeComputer({placement:'host',binding:()=>scoped,execute:async request=>{sent.push(request);return {applied:true}}});
 await adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input:{kind:'click',x:10,y:10}},{});
 assert.deepEqual(sent[0].processes,[{...processes[0],browser:false},{...processes[1],browser:true}]);
 assert(!processes.some(process=>Object.hasOwn(process,'browser')),'never mutate a shared ownership inventory');
});


test('MP-11 an agent click carries the CDP-placed active page for the X11 hit test, other inputs do not',async()=>{
 const sent=[],page={url:'https://example.test/',window:[0,0,900,700],native:[892,553]},requests=[];
 const browser={nativeViewport:async()=>({target_id:'t',width:892,height:553}),async ensureConnection(){return {async send(method){requests.push(method);if(method==='Target.getTargets')return {targetInfos:[]};return method==='Target.getTargetInfo'?{targetInfo:{url:page.url}}:{bounds:{left:0,top:0,width:900,height:700}};}};}};
 const scoped={...binding,ownedProcesses:async()=>[{pid:300,started:'2'}],browser:()=>browser};
 const adapter=new NativeComputer({placement:'host',binding:()=>scoped,execute:async request=>{sent.push(request);return {applied:true}}});
 await adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input:{kind:'click',x:10,y:10}},{});
 assert.deepEqual(sent[0].browser_page,{...page,overlay:false});
 await adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input:{kind:'key',key:'CTRL+s'}},{});
 assert.equal(Object.hasOwn(sent[1],'browser_page'),false);
 scoped.browser=()=>({nativeViewport:async()=>{throw Error('extension gone');}});
 await adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input:{kind:'click',x:10,y:10}},{});
 assert.equal(sent[2].browser_page,null,'an unplaceable page is browser UI to the helper');
});

test('MP-11 #904 review 1/3 agent clicks and keys reach the helper without a clipboard read',async()=>{
  const calls=[];
  const scoped={...binding,ownedProcesses:async()=>[{pid:200,started:'1'}]};
  const adapter=new NativeComputer({placement:'host',binding:()=>scoped,execute:async request=>{calls.push(request);return request.op==='clipboard_read'?{text:'[protected]'}:{applied:true};}});
  for(const input of [{kind:'click',button:1,x:10,y:10},{kind:'click',button:3,x:10,y:10},{kind:'key',key:'CTRL+V'},{kind:'key',key:'p'},{kind:'key',key:'alt+e'},{kind:'text',text:'p'}])
    await adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input},{});
  assert.deepEqual(calls.map(request=>request.op),Array(6).fill('input'));
  assert(calls.every(request=>request.agent_input===true && request.processes?.[0]?.pid===200));
});

test('MP-11 an agent clipboard write does not stop its next paste, typing or unrelated click',async()=>{
  const dir=await mkdtemp(path.join(os.tmpdir(),'fake-xclip-'));
  const calls=[];
  try {
    // A headless selection owner like the real xclip: no window, no accessibility nodes.
    await writeFile(path.join(dir,'xclip'),'#!/bin/sh\ncat >/dev/null\nexec sleep 30\n',{mode:0o755});
    const scoped={...binding,environment:{...binding.environment,PATH:`${dir}:${process.env.PATH}`},ownedProcesses:async()=>[{pid:200,started:'1'}]};
    const adapter=new NativeComputer({placement:'host',binding:()=>scoped,execute:async request=>{calls.push(request);return {applied:true};}});
    const send=input=>adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input},{});
    assert.deepEqual(await send({kind:'clipboard_write',text:'ordinary agent text'}),{applied:true});
    for(const input of [{kind:'key',key:'CTRL+V'},{kind:'text',text:'typed'},{kind:'click',button:1,x:10,y:10}])
      assert.deepEqual(await send(input),{applied:true});
    assert.deepEqual(calls.map(request=>request.input.kind),['key','text','click']);
    assert(calls.every(request=>request.agent_input===true));
    await adapter.close();
  } finally {await rm(dir,{recursive:true,force:true});}
});

test('MP-08 / MP-10 / MP-11 human text and committed composition use the warm channel while every agent press keeps admission', async () => {
  const warm=[],oneShot=[];
  const channel=()=>({start:async()=>{},close:async()=>{},send:async request=>{warm.push(request);return {applied:true};}});
  const bound={...binding,ownedProcesses:async()=>[]};
  const adapter=new NativeComputer({placement:'host',binding:()=>bound,execute:async request=>{oneShot.push(request);return {applied:true};},channel});
  const send=(input,agent)=>adapter.request({op:'input',surface_id:'surface',generation:'generation',...(agent?{_agent_input:true}:{}),input},{});
  for(const input of [{kind:'click',x:5,y:5},{kind:'move',x:6,y:6},{kind:'scroll',x:5,y:5,steps:2},{kind:'key',key:'PageDown'},{kind:'keycode',keycode:38,state:'down'},{kind:'keycode',keycode:38,state:'up'},{kind:'text',text:'hi'},{kind:'composition',text:'Grüße 世界😀'}])await send(input);
  assert.deepEqual(warm.map(request=>request.input.kind),['click','move','scroll','key','keycode','keycode','text','text']);
  assert(warm.every(request=>request.op==='input'&&!('agent_input' in request)&&!('processes' in request)));
  await send({kind:'text',text:'hi'},true);await send({kind:'composition',text:'世界'},true);await send({kind:'click',x:5,y:5},true);await send({kind:'key',key:'PageDown'},true);
  assert.deepEqual(oneShot.map(request=>[request.input.kind,request.agent_input??false]),[['text',true],['text',true],['click',true],['key',true]]);
  assert.equal(warm.length,8);
  await adapter.close();
});

// MP-08 / MP-11 (owner 2026-10-09): registered Vault values are masked best effort
// where they appear as accessible text and are redacted from OCR and clipboard text.
test('MP-08 / MP-11 Vault values never black out the desktop and are redacted from OCR and clipboard text', async () => {
  const sent=[];
  const replies={screenshot:{data_base64:'',protected:false},ocr:{text:'header synthetic-vault-value footer',targets:[{text:'synthetic-vault-value',x:1,y:1},{text:'Save',x:2,y:2}]},clipboard_read:{text:'copied synthetic-vault-value'},input:{}};
  const bound={...binding,ownedProcesses:async()=>[]};
  const adapter=new NativeComputer({placement:'host',binding:()=>bound,execute:async request=>{sent.push(request);return structuredClone(replies[request.op]);}});
  const policy={values:['synthetic-vault-value'],targets:[{kind:'native',target:{focus_window:1,active_window:2}}],unknown:false};
  for(const op of ['screenshot','ocr','clipboard_read'])await adapter.request({op,surface_id:'surface',generation:'generation'},policy).then(result=>replies[op].result=result);
  assert(sent.every(request=>request.mask===(request.op==='clipboard_read')&&JSON.stringify(request.values)===JSON.stringify(policy.values)),'best-effort boxes leave desktop visible; clipboard stays withheld');
  assert.equal(replies.ocr.result.text,'header [redacted] footer');
  assert.deepEqual(replies.ocr.result.targets.map(target=>target.text),['Save'],'OCR targets that echo a value are dropped');
  assert.equal(replies.clipboard_read.result.text,'copied [redacted]');
  // UX-first: a registered Vault value never stops the agent clicking or typing elsewhere.
  sent.length=0;
  for(const input of [{kind:'click',x:1,y:1},{kind:'text',text:'Landsmall Tester'}])await adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input},policy);
  assert.deepEqual(sent.map(request=>request.op),['input','input'],'agent input still reaches the native helper under a registry');
});
