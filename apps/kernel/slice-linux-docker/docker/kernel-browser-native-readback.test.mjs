// MP-08/MP-10/MP-11: retired captures do not become ownership failures or start workers.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm,mkdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {OwnedDisplay} from './kernel-browser-owned-display.mjs';
import {LinuxCapture} from './kernel-browser-native.mjs';
import {startNativeReadback} from './kernel-browser-native-readback.mjs';

async function displayFixture(run){
 const root=await mkdtemp(path.join(tmpdir(),'native-readback-')),display=new OwnedDisplay(root);
 try{await display.start();await run(display,root);}finally{await display.close();await rm(root,{recursive:true,force:true});}
}

test('MP-11 unowned displays and unsafe PIDs remain ownership failures',async()=>{
 await assert.rejects(startNativeReadback({display:{},valid:()=>false,pid:process.pid},{}),error=>error.code===undefined&&/source not owned/.test(error.message));
 await displayFixture(async display=>{
  for(const pid of [undefined,0,1,-1,NaN,2.1])await assert.rejects(startNativeReadback({display,valid:()=>false,pid},{}),error=>error.code===undefined&&/source not owned/.test(error.message));
 });
});

test('MP-08/MP-10 owned readback retired before startup cannot launch a worker',async()=>{
 await displayFixture(async display=>{
  const capture={display,pid:process.pid,valid:()=>false};
  await assert.rejects(startNativeReadback(capture,{}),/source not owned/);
  assert.equal(capture.child,undefined,'no worker is launched for the retired source');
 });
});

test('MP-08/MP-10 readback rechecks retirement after asynchronous codec setup before spawning',async()=>{
 await displayFixture(async(display,root)=>{
  const names=['CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER','CHARIOX_BROWSER_DISPLAY_PACKET_ROOT','CHARIOX_BROWSER_DISPLAY_CODEC_ROOT','CHARIOX_BROWSER_DISPLAY_OPENH264','CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER'];
  const previous=new Map(names.map(name=>[name,process.env[name]]));
  const capture=new LinuxCapture({display,pid:process.pid,policy:{},allowed:()=>true});
  try{
   process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER='/bin/true';
   process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=root;
   process.env.CHARIOX_BROWSER_DISPLAY_CODEC_ROOT=path.join(root,'codecs');
   delete process.env.CHARIOX_BROWSER_DISPLAY_OPENH264;delete process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER;
   capture.poolRoot=path.join(root,'pool');await mkdir(capture.poolRoot);
   let current=true;capture.valid=()=>current&&!capture.closed;
   queueMicrotask(()=>{current=false;});
   await assert.rejects(startNativeReadback(capture,{width:2,height:2}),/source not owned/);
   assert.equal(capture.child,undefined,'only the codec probe may run; the retired readback never starts');
  }finally{
   await capture.close();
   for(const[name,value]of previous){if(value===undefined)delete process.env[name];else process.env[name]=value;}
  }
 });
});
