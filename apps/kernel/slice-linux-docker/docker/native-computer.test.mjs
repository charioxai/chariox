// MP-08 / MP-11: fail-first adapter protection, binding and keyboard contracts.
import test from 'node:test';
import assert from 'node:assert/strict';
import { NativeComputer, nativeInput, executeNative } from './native-computer.mjs';
const binding = {surface_id:'surface',generation:'generation',width:1280,height:800,environment:{DISPLAY:':999'}};
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
  await assert.rejects(run({ ensureConnection: async () => { throw new Error('closed'); } }), /fence unavailable/);
  // Vault policy never blacks out the desktop: browser windows are measured with the values.
  assert.deepEqual(await run(browser([[], []]), {values:['v'],targets:[],unknown:false}), [{pages:[]}]);
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
  let current = { ...binding, environment: { ...publicDependencies, PATH: '/usr/bin:/bin', LANG: 'C.UTF-8' } };
  const adapter = new NativeComputer({ placement: 'host', binding: () => current });
  try {
    await adapter.primeKeyboard();
    const first = adapter.keyboard.child;
    current = { ...current, generation: 'replacement' };
    await adapter.primeKeyboard();
    assert.notEqual(adapter.keyboard.child.pid, first.pid);
    assert(first.exitCode !== null || first.signalCode !== null);
    assert.equal(adapter.held.size, 0);
  } finally { await adapter.close(); }
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

test('MP-11 finding 1 agents cannot lend native clipboard, repeats or middle-button paste', async () => {
  let calls=0;
  const adapter=new NativeComputer({placement:'host',binding:()=>binding,execute:async()=>{calls++;return {};}});
  for(const input of [{kind:'clipboard_write',text:'ordinary'}, {kind:'hold',key:'a',duration_ms:500},
    {kind:'drag',x:10,y:10,to_x:20,to_y:20}, {kind:'click',button:2,x:10,y:10}, {kind:'pointer_hold',button:2,x:10,y:10,duration_ms:1}]) {
    await assert.rejects(adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input},{}));
  }
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


test('MP-11 R1 agent pointer holds require the same human admission as keyboard holds',async()=>{
  let calls=0;
  const adapter=new NativeComputer({placement:'host',binding:()=>binding,execute:async()=>{calls++;return {};}});
  await assert.rejects(adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input:{kind:'pointer_hold',button:1,x:10,y:10,duration_ms:100}},{}));
  assert.equal(calls,0);
});

test('MP-11 R2 unknown clipboard never reaches an agent paste shortcut',async()=>{
  const calls=[];
  const adapter=new NativeComputer({placement:'host',binding:()=>binding,execute:async request=>{calls.push(request.op);return {text:'[protected]'};}});
  for(const key of ['CTRL+V','SHIFT+Insert','Control_L+Shift+v','Super+v']){
    await assert.rejects(adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input:{kind:'key',key}},{}),error=>error.code==='user_domain_sensitive_requires_focus');
  }
  assert.deepEqual(calls,['clipboard_read','clipboard_read','clipboard_read','clipboard_read']);
});

test('MP-11 review R1 agent clicks cannot invoke Paste with an unproved clipboard',async()=>{
  const calls=[];
  const adapter=new NativeComputer({placement:'host',binding:()=>binding,execute:async request=>{calls.push(request.op);return {text:'[protected]'};}});
  for(const button of [1,3])await assert.rejects(adapter.request({op:'input',surface_id:'surface',generation:'generation',_agent_input:true,input:{kind:'click',button,x:10,y:10}},{}),error=>error.code==='user_domain_sensitive_requires_focus');
  assert.deepEqual(calls,['clipboard_read','clipboard_read']);
  await adapter.request({op:'input',surface_id:'surface',generation:'generation',input:{kind:'click',x:10,y:10}},{});
  assert.equal(calls.at(-1),'input','MP-11 human click preserves existing admission');
});
