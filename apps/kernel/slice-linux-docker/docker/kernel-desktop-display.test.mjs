// MP-08 / MP-10 / MP-11: desktop viewer authority/lifetime regressions.
import test from 'node:test';
import assert from 'node:assert/strict';
import { DesktopDisplay } from './kernel-desktop-display.mjs';
import { DesktopSource } from './kernel-desktop-source.mjs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
const target={surface_id:'desktop-one',generation:'native-one',width:1280,height:800};
function fixture(){
 let captures=0,closed=0;
 const source={closed:false,subscribe:()=>()=>{},sample:()=>null,start:async()=>{},close:async()=>{closed++;source.closed=true},wake:()=>{},valid:()=>!source.closed};
 const host={generation:1,protection:{values:[],targets:[],unknown:false},displays:new Map(),chromium:{desktop:{binding:()=>target}},armDisplayExpiry:()=>{},timing:()=>{}};
 const display=new DesktopDisplay(host,{createSource:async()=>{captures++;return source}});
 return {host,display,counts:()=>({captures,closed})};
}
const command={op:'display_subscribe',...target,codecs:['avc1.420033'],bitrate:8000000,device_scale_factor:1};
test('MP-11 stale desktop, agent video and unsupported codec fail before capture',async()=>{
 const {display,counts}=fixture();
 for(const request of [{...command,generation:'old'}, {...command,_agent_input:true}, {...command,codecs:['png']}, {...command,device_scale_factor:3}])await assert.rejects(display.subscribe(request,'terminal'));
 assert.equal(counts().captures,0);
});
test('MP-11 two desktop viewers share one source and last close releases it',async()=>{
 const {display,host,counts}=fixture();
 const first=await display.subscribe(command,'a'),second=await display.subscribe(command,'b');
 assert.equal(counts().captures,1);
 await assert.rejects(display.request({op:'display_attach',subscription_id:first.subscription_id,generation:1},'b'));
 await display.request({op:'unsubscribe',subscription_id:first.subscription_id,generation:1},'a');
 assert.equal(counts().closed,0);assert.equal(host.displays.size,1);
 await display.request({op:'unsubscribe',subscription_id:second.subscription_id,generation:1},'b');
 assert.equal(counts().closed,1);assert.equal(host.displays.size,0);
});
test('MP-11 protection retirement closes producers before returning and blocks old credits',async()=>{
 const {display,host,counts}=fixture();const subscribed=await display.subscribe(command,'a');
 await display.close();assert.equal(counts().closed,1);assert.equal(host.displays.size,0);
 await assert.rejects(display.request({op:'screenshot',display_subscription_id:subscribed.subscription_id,generation:1,after_sequence:0},'a'));
});

test('MP-11 simultaneous viewers await one protected source',async()=>{
 const {host}=fixture();let captures=0,release;
 const started=new Promise(resolve=>{release=resolve});
 const source={closed:false,close:async()=>{},valid:()=>true};
 const display=new DesktopDisplay(host,{createSource:async()=>{captures++;await started;return source}});
 const first=display.subscribe(command,'a'),second=display.subscribe(command,'b');
 await new Promise(resolve=>setImmediate(resolve));assert.equal(captures,1);
 release();await Promise.all([first,second]);await display.close();
});
test('MP-11 canceling one credit preserves producer lifetime, retiring policy closes it',async()=>{
 const {host}=fixture();let valid,closed=0;
 const source={closed:false,sample:()=>null,close:async()=>{source.closed=true},valid:()=>!source.closed};
 const display=new DesktopDisplay(host,{createSource:async()=>source,createProducer:(_source,_encoder,options)=>{
  valid=options.valid;return {waitReady:async()=>{},take:()=>null,close:async()=>{closed++}};
 }});
 const subscription=await display.subscribe(command,'a'),controller=new AbortController();
 const request={op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:0};
 await display.request(request,'a',{signal:controller.signal});controller.abort();assert.equal(valid(),true);
 await display.request(request,'a');assert.equal(valid(),true);
 await display.retireSource();assert.equal(closed,1);assert.equal(valid(),false);
 await display.close();
});
test('MP-08/MP-10 a quiet lossy desktop refines to exact repair tiles of the same protected sample',async()=>{
 const {host}=fixture(),exact=[];
 const raw={serial:7,width:1280,height:800,retain(){},release(){},nativeExact:async request=>{exact.push(request);return {width:1280,height:800,native_exact:true,native_repair:true,repair_tiles:[{x:0,y:0,width:16,height:16,format:'png',data_base64:'iVBORw0KGgo='}]};}};
 const sample={raw,serial:7,width:1280,height:800,motion:true,data_base64:'masked-7'};
 const source={closed:false,changedAt:performance.now()-2000,sample:()=>sample,subscribe:()=>()=>{},close:async()=>{},valid:()=>true};
 const frames=[{...sample,encoded:{key:true,data_base64:'AAAA'}}];
 const display=new DesktopDisplay(host,{createSource:async()=>source,createProducer:()=>({waitReady:async()=>{},take:()=>frames.shift()??null,retireUnsent(){},invalidate(){},close:async()=>{}})});
 const subscription=await display.subscribe(command,'a');
 const credit=after=>display.request({op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:after},'a');
 assert.equal((await credit(0)).display_frame.kind,'video');
 let reply;for(let n=0;n<50&&!(reply=await credit(1)).frame_sent;n++)await new Promise(resolve=>setTimeout(resolve,10));
 assert.equal(reply.display_frame.kind,'tiles');assert.equal(reply.display_frame.tiles.length,1);
 assert.equal(exact.length,1);assert.equal(exact[0].repair_only,true);assert.equal(exact[0].patch,false);
 assert.equal(host.displays.get(subscription.subscription_id).exact,true);
 assert.equal((await credit(2)).frame_sent,false,'an exact canvas needs no further refinement');
 await display.close();
});
test('MP-08/MP-10 refinement waits for 1 s of quiet and a newer sample abandons a paced repair',async()=>{
 const {host}=fixture();let exact=0;
 const make=serial=>{const raw={serial,width:1280,height:800,retain(){},release(){},nativeExact:async()=>{exact++;return {width:1280,height:800,native_exact:true,native_repair:true,repair_tiles:Array.from({length:40},(_,i)=>({x:(i%10)*16,y:Math.floor(i/10)*16,width:16,height:16,format:'png',data_base64:'A'.repeat(60000)}))};}};return {raw,serial,width:1280,height:800,motion:true,data_base64:'masked-'+serial};};
 let current=make(7);
 const source={closed:false,changedAt:performance.now(),sample:()=>current,subscribe:()=>()=>{},close:async()=>{},valid:()=>true};
 const frames=[{...current,encoded:{key:true,data_base64:'AAAA'}}];
 const display=new DesktopDisplay(host,{createSource:async()=>source,createProducer:()=>({waitReady:async()=>{},take:()=>frames.shift()??null,retireUnsent(){},invalidate(){},close:async()=>{}})});
 const subscription=await display.subscribe({...command,bitrate:500000},'a');
 const credit=after=>display.request({op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:after},'a');
 await credit(0);
 await new Promise(resolve=>setTimeout(resolve,120));assert.equal((await credit(1)).frame_sent,false);assert.equal(exact,0,'no refinement within 1 s of motion');
 source.changedAt=performance.now()-2000;let reply;
 for(let n=0;n<50&&!(reply=await credit(1)).frame_sent;n++)await new Promise(resolve=>setTimeout(resolve,10));
 assert.equal(reply.display_frame.kind,'tiles');
 // The next repair batch is link paced (0.5 Mbit/s); a newer published sample abandons it.
 const pending=credit(2);await new Promise(resolve=>setTimeout(resolve,50));current=make(8);source.changedAt=performance.now();
 const started=Date.now();assert.equal((await pending).frame_sent,false);assert(Date.now()-started<1000,'paced repair abandoned promptly');
 await display.close();
});

// Vault (Miguel 2026-10-09): registered values never black out the desktop
// stream; they reach the capture helper for the best-effort AT-SPI text check.
test('MP-08/MP-11 Vault values reach the desktop capture without a whole-desktop mask',async()=>{
 const root=await mkdtemp(path.join(process.env.TMPDIR??tmpdir(),'desktop-source-'));
 const helper=path.join(root,'helper.sh'),seen=path.join(root,'config.json'),saved=process.env.CHARIOX_BROWSER_DISPLAY_PYTHON;
 await writeFile(helper,`#!/bin/sh\nhead -n 1 > ${seen}\n`,{mode:0o700});
 process.env.CHARIOX_BROWSER_DISPLAY_PYTHON=helper;
 try{
  for(const [policy,mask] of [[{values:['v-secret'],targets:[],unknown:false},false],[{values:[],targets:[],unknown:true},true]]){
   const source=new DesktopSource({...target,environment:{},ownedProcesses:async()=>[],browserProcesses:async()=>[],browser:()=>null},policy);
   await assert.rejects(source.start(),/unavailable/);
   const config=JSON.parse(await readFile(seen,'utf8'));
   assert.deepEqual([config.mask,config.values],[mask,policy.values]);
  }
 }finally{
  if(saved===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_PYTHON;else process.env.CHARIOX_BROWSER_DISPLAY_PYTHON=saved;
  await rm(root,{recursive:true,force:true});
 }
});
