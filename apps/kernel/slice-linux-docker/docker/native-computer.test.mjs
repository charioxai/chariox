// MP-08 / MP-11: fail-first adapter protection, binding and keyboard contracts.
import test from 'node:test';
import assert from 'node:assert/strict';
import { NativeComputer, nativeInput } from './native-computer.mjs';
const binding = {surface_id:'surface',generation:'generation',width:1280,height:800,environment:{DISPLAY:':999'}};
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
test('MP-08 immediate physical key and text are distinct and wake capture after each event', async () => {
  const sent=[],wakes=[];
  const adapter=new NativeComputer({placement:'host',binding:()=>binding,execute:async request=>{sent.push(request);return {};},wakeCapture:event=>wakes.push(event)});
  for(const input of [{kind:'keycode',keycode:38,state:'down'},{kind:'keycode',keycode:38,state:'up'},{kind:'text',text:'Grüße 世界😀'}]) await adapter.request({op:'input',surface_id:'surface',generation:'generation',input},{});
  assert.equal(sent.length,3);assert.equal(wakes.length,3);assert.equal(sent[2].input.kind,'text');
});
