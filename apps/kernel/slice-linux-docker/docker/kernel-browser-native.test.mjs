// MD-DISPLAY-02/04: native admission fails before opening any foreign display.
import test from 'node:test';
import assert from 'node:assert/strict';
import { OwnedDisplay, ownsDisplay } from './kernel-browser-owned-display.mjs';
import { selectNativeCapture } from './kernel-browser-native.mjs';
for(const display of [null,{}, {display:':0',pid:200}, {display:':99',owned:true}])
 test('MD-DISPLAY refuses unowned display '+JSON.stringify(display),async()=>{
  let launched=false;
  assert.equal(ownsDisplay(display),false);
  assert.equal(await selectNativeCapture({platform:'linux',display,create:()=>{launched=true}}),null);
  assert.equal(launched,false);
 });
test('MD-DISPLAY macOS unsupported falls back without opening display',async()=>{
 let called=false;assert.equal(await selectNativeCapture({platform:'darwin',create:()=>called=true}),null);assert.equal(called,false);
});
test('MD-DISPLAY owned display refuses unsafe IDs and environment adoption',()=>{
 const display=new OwnedDisplay('/tmp/not-started');
 for(const pid of [0,1,-1,undefined,NaN,2.1]){display.child={pid,exitCode:null,signalCode:null};assert.equal(ownsDisplay(display),false)}
 assert.equal(ownsDisplay(new OwnedDisplay('/tmp/other',{environment:{DISPLAY:':0'}})),false);
});
test('MD-DISPLAY selection admits only a live kernel-created server; retirement fences it',async()=>{
 const {mkdtemp,rm}=await import('node:fs/promises');const {tmpdir}=await import('node:os');
 const root=await mkdtemp(tmpdir()+'/chariox-md-native-test-'),display=new OwnedDisplay(root);
 try{
  await display.start();assert.equal(ownsDisplay(display),true);
  let called=false;const chosen=await selectNativeCapture({platform:'linux',display,create:()=>{called=true;return 'native'}});
  assert.equal(chosen,'native');assert.equal(called,true);
  assert.equal(await selectNativeCapture({platform:'linux',display,create:()=>{throw Error('XShm absent')}}),null);
  await display.close();assert.equal(ownsDisplay(display),false);
 }finally{await display.close();await rm(root,{recursive:true,force:true})}
});
test('MD-DISPLAY raw capture rejects a hidden or replaced source before publishing',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 for(const change of ['hidden','navigation','policy']){
  let allowed=true;
  const connection={send:async method=>method==='Page.getFrameTree'?{frameTree:{frame:{id:'frame',loaderId:change==='navigation'?'new':'document'}}}:{result:{value:'hidden'}}};
  const source=new LinuxCapture({connection,sessionId:'session',tab:{target_id:'target',tab_id:'tab',document_id:'document'},policy:{},allowed:()=>allowed});source.valid=()=>!source.closed&&allowed;source.attested=true;source.contextId=1;
  source.pending={serial:1,signature:'a'.repeat(64),width:1280,height:800};
  let emitted=false;source.subscribe(()=>emitted=true);
  if(change==='policy')allowed=false;
  await source.publish();assert.equal(emitted,false);assert.equal(source.sample(),null);await source.close();
 }
});

test('MP-10 input wake never writes to a fenced capture or destroyed pipe',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const writes=[],capture=new LinuxCapture({});let valid=true;capture.valid=()=>valid;
 capture.child={stdin:{destroyed:false,write:text=>writes.push(JSON.parse(text))}};
 capture.wake();valid=false;capture.wake();valid=true;capture.child.stdin.destroyed=true;capture.wake();
 assert.deepEqual(writes,[{wake:true}]);
});
