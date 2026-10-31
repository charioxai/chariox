// MP-08 / MP-10 / MP-11: desktop viewer authority/lifetime regressions.
import test from 'node:test';
import assert from 'node:assert/strict';
import { DesktopDisplay } from './kernel-desktop-display.mjs';
import { DesktopSource } from './kernel-desktop-source.mjs';
import { encodePng } from './kernel-browser-pixels.mjs';
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
test('MP-08/MP-10 ACK-gated desktop exact frames do not pay a second link delay',async()=>{
 for(const push of [undefined,{reset:false,congested:false}]){
  const {host}=fixture();let now=0;
  const pixels=Buffer.alloc(target.width*target.height*4,255);
  for(let i=0;i<128*128;i++)for(let channel=0;channel<3;channel++)pixels[i*4+channel]=(i*37+(i>>8)*19+channel*71)%256;
  const frames=[{width:target.width,height:target.height,serial:1,native_exact:true,motion:false,force_lossless:true,data_base64:encodePng(target.width,target.height,pixels)}];
  const source={closed:false,sample:()=>null,close:async()=>{},valid:()=>true};
  const display=new DesktopDisplay(host,{createSource:async()=>source,createProducer:()=>({waitReady:async()=>{},take:()=>frames.shift()??null,invalidate(){},retireUnsent(){},feedback(){},close:async()=>{}})});
  try{
   const subscription=await display.subscribe({...command,bitrate:500000},'human');
   const stream=host.displays.get(subscription.subscription_id);
   stream.now=()=>now;stream.refillAt=0;stream.tokens=0;stream.wait=async ms=>{now+=ms};
   const reply=await display.request({op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:0,push},'human');
   assert.equal(reply.display_frame.kind,'png');
   if(push)assert.equal(now,0,'the kernel ACK pump already gates these exact bytes');
   else assert(now>0,'legacy display credits retain byte pacing');
  }finally{await display.close();}
 }
});
test('MP-08/MP-10 desktop motion receives ACK congestion and legacy credit lag',async()=>{
 for(const [push,expected] of [[{reset:false,congested:true},8],[{reset:false,congested:false},0],[undefined,3]]){
  const {host}=fixture(),feedback=[];
  const source={closed:false,sample:()=>null,close:async()=>{},valid:()=>true};
  const display=new DesktopDisplay(host,{createSource:async()=>source,createProducer:()=>({waitReady:async()=>{},take:()=>null,feedback:lag=>feedback.push(lag),invalidate(){},close:async()=>{}})});
  try{
   const subscription=await display.subscribe(command,'human');host.displays.get(subscription.subscription_id).sequence=3;
   await display.request({op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:0,push},'human');
   assert.deepEqual(feedback,[expected]);
  }finally{await display.close();}
 }
});
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
  valid=options.valid;return {feedback(){},waitReady:async()=>{},take:()=>null,close:async()=>{closed++}};
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
 const display=new DesktopDisplay(host,{createSource:async()=>source,createProducer:()=>({waitReady:async()=>{},take:()=>frames.shift()??null,feedback(){},retireUnsent(){},invalidate(){},close:async()=>{}})});
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
test('MP-08/MP-10/MP-11 idle caret updates permit exact refinement while recent human input waits',async()=>{
 const {host}=fixture();let exact=0;
 const raw={serial:7,width:1280,height:800,retain(){},release(){},nativeExact:async()=>{exact++;return {width:1280,height:800,native_exact:true,native_repair:true,repair_tiles:[{x:0,y:0,width:16,height:16,format:'png',data_base64:'iVBORw0KGgo='}]};}};
 const sample={raw,serial:7,width:1280,height:800,motion:true,data_base64:'masked-7'};
 const source={closed:false,changedAt:performance.now()-300,sample:()=>sample,wake(){},subscribe:()=>()=>{},close:async()=>{},valid:()=>true};
 const frames=[{...sample,encoded:{key:true,data_base64:'AAAA'}}];
 const display=new DesktopDisplay(host,{createSource:async()=>source,createProducer:()=>({waitReady:async()=>{},take:()=>frames.shift()??null,feedback(){},retireUnsent(){},invalidate(){},close:async()=>{}})});
 try{
  const subscription=await display.subscribe(command,'a');
  const credit=after=>display.request({op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:after},'a');
  await credit(0);display.wake();
  assert.equal((await credit(1)).frame_sent,false);assert.equal(exact,0,'repair must not compete with recent input');
  await new Promise(resolve=>setTimeout(resolve,1050));let reply;
  for(let n=0;n<10;n++){
   source.changedAt=performance.now()-300; // latest caret change, not a fully idle desktop
   reply=await credit(1);if(reply.frame_sent)break;
   await new Promise(resolve=>setTimeout(resolve,10));
  }
  assert.equal(reply.frame_sent,true,'an idle user field can refine between caret changes');
  assert.equal(reply.display_frame.kind,'tiles');assert.equal(exact,1);
 }finally{await display.close();}
});
test('MP-08/MP-10 refinement waits for fresh motion to settle and a newer sample abandons a paced repair',async()=>{
 const {host}=fixture();let exact=0;
 const make=serial=>{const raw={serial,width:1280,height:800,retain(){},release(){},nativeExact:async()=>{exact++;return {width:1280,height:800,native_exact:true,native_repair:true,repair_tiles:Array.from({length:40},(_,i)=>({x:(i%10)*16,y:Math.floor(i/10)*16,width:16,height:16,format:'png',data_base64:'A'.repeat(60000)}))};}};return {raw,serial,width:1280,height:800,motion:true,data_base64:'masked-'+serial};};
 let current=make(7);
 const source={closed:false,changedAt:performance.now(),sample:()=>current,subscribe:()=>()=>{},close:async()=>{},valid:()=>true};
 const frames=[{...current,encoded:{key:true,data_base64:'AAAA'}}];
 const display=new DesktopDisplay(host,{createSource:async()=>source,createProducer:()=>({waitReady:async()=>{},take:()=>frames.shift()??null,feedback(){},retireUnsent(){},invalidate(){},close:async()=>{}})});
 const subscription=await display.subscribe({...command,bitrate:500000},'a');
 const credit=after=>display.request({op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:after},'a');
 await credit(0);
 source.changedAt=performance.now();assert.equal((await credit(1)).frame_sent,false);assert.equal(exact,0,'no refinement immediately after motion');
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

test('MP-08/MP-10/MP-11 desktop sparse video uses only the negotiated shared stripe codec',async()=>{
 for(const [codecs,expected] of [[['avc1.420033','chariox-stripes-v1'],true],[['avc1.420033'],false],[['vp8','chariox-stripes-v1'],true],[['vp09.00.10.08','chariox-stripes-v1'],false]]) {
  const {host}=fixture();let options;
  const source={closed:false,sample:()=>null,close:async()=>{},valid:()=>true};
  const display=new DesktopDisplay(host,{createSource:async()=>source,createProducer:(_source,_encoder,value)=>{options=value;return {waitReady:async()=>{},take:()=>null,feedback(){},invalidate(){},close:async()=>{}}}});
  const sub=await display.subscribe({...command,codecs},'human');
  await display.request({op:'screenshot',display_subscription_id:sub.subscription_id,generation:1,after_sequence:0},'human');
  assert.equal(options.stripes,expected);
  await display.close();
 }
});
test('MP-08/MP-11 a credit in flight when protection changes asks the viewer to poll again instead of closing its stream',async()=>{
 const {host}=fixture();let release,producers=0;const waiting=new Promise(resolve=>{release=resolve});
 const display=new DesktopDisplay(host,{
  createSource:async()=>{const source={closed:false,sample:()=>null,close:async()=>{source.closed=true},valid:()=>!source.closed};return source},
  createProducer:()=>{producers++;return {feedback(){},waitReady:()=>producers===1?waiting:Promise.resolve(),take:()=>null,invalidate(){},retireUnsent(){},close:async()=>{}}}});
 try{
  const subscription=await display.subscribe(command,'a');
  const request={op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:0};
  const credit=display.request(request,'a');
  await new Promise(resolve=>setImmediate(resolve));
  host.protection={values:['synthetic-vault-value'],targets:[],unknown:false};await display.retireSource();release();
  assert.deepEqual(await credit,{generation:1,frame_sent:false,display_frame:null});
  assert.deepEqual(await display.request(request,'a'),{generation:1,frame_sent:false,display_frame:null});
  assert.equal(producers,2,'the next credit builds a producer over the new protected source');
 }finally{await display.close()}
});
test('MP-08/MP-11 only a kernel-named owner reads the unmasked feed; every other viewer reads the masked source',async()=>{
 const {host}=fixture();const fed=[];
 const feed={name:'owner-feed',sample:()=>null};
 const display=new DesktopDisplay(host,{
  createSource:async()=>{const source={closed:false,name:'masked',sample:()=>null,ownerFeed:()=>feed,close:async()=>{source.closed=true},valid:()=>!source.closed};return source},
  createProducer:source=>{fed.push(source.name);return {feedback(){},waitReady:()=>Promise.resolve(),take:()=>null,invalidate(){},retireUnsent(){},close:async()=>{}}}});
 try{
  const owner=await display.subscribe({...command,audience:'owner'},'lease-a'),other=await display.subscribe(command,'lease-b');
  for(const [subscription,scope] of [[owner,'lease-a'],[other,'lease-b']])
   await display.request({op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:0},scope);
  assert.deepEqual(fed,['owner-feed','masked']);
  await assert.rejects(display.subscribe({...command,audience:'masked-by-me'},'x'),{code:'user_domain_not_granted'});
 }finally{await display.close()}
});
// Review (PR 1219): an unwatched desktop must not keep reading the screen; its last watched viewer stops the shared source.
test('MP-08/MP-10 an unwatched desktop display pauses the shared source and a capture resumes it',async()=>{
 const {KernelBrowserHost}=await import('./kernel-browser-host.mjs');const {DisplaySessions}=await import('./kernel-display-resumption.mjs');
 const previous=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{
  const {host}=fixture();let sources=0,closed=0,producersClosed=0;
  const display=new DesktopDisplay(host,{createSource:async()=>{sources++;const source={closed:false,sample:()=>null,valid:()=>!source.closed,close:async()=>{closed++;source.closed=true}};return source},
   createProducer:()=>({waitReady:async()=>{},take:()=>null,feedback(){},invalidate(){},close:async()=>{producersClosed++}})});
  const real=Object.create(KernelBrowserHost.prototype);
  Object.assign(real,{generation:1,protection:host.protection,streams:new Map(),displays:host.displays,armDisplayExpiry(){},start:async()=>{},desktopDisplay:display});
  real.displaySessions=new DisplaySessions(real);
  const first=await display.subscribe(command,'a'),second=await display.subscribe(command,'b');
  const capture=(subscription,scope)=>display.request({op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:0,push:{reset:true,congested:false}},scope);
  await capture(first,'a');await capture(second,'b');assert.equal(sources,1);
  const park=(subscription,scope,parked)=>real.requestNow({op:'display_park',watch:true,subscription_id:subscription.subscription_id,generation:1,observed_by:scope,parked});
  assert.deepEqual(await park(first,'a',true),{generation:1,parked:true});
  assert.equal(closed,0,'another viewer still watches: the source keeps running');
  assert.deepEqual(await park(second,'b',true),{generation:1,parked:true});
  assert.equal(closed,1,'the last watched viewer stopped the source');assert.equal(producersClosed,2);
  assert.deepEqual(await park(second,'b',true),{generation:1,parked:true});assert.equal(closed,1,'a periodic park keeps it stopped');assert.equal(sources,1);
  await display.request({op:'display_attach',subscription_id:first.subscription_id,generation:1},'a');
  assert.equal(sources,1,'a keepalive attach starts no source');
  await park(first,'a',false);assert.equal(sources,1,'unparking alone reads nothing');
  await capture(first,'a');assert.equal(sources,2,'the independent frame starts the source again');
  assert.equal(host.displays.get(first.subscription_id).parked,false);
  await display.close();
 }finally{if(previous===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=previous;}
});
test('MP-08/MP-10 a sole unwatched desktop viewer stops the source and stays dormant',async()=>{
 const {host}=fixture();let sources=0,closed=0;
 const display=new DesktopDisplay(host,{createSource:async()=>{sources++;const source={closed:false,sample:()=>null,valid:()=>!source.closed,close:async()=>{closed++;source.closed=true}};return source},
  createProducer:()=>({waitReady:async()=>{},take:()=>null,feedback(){},invalidate(){},close:async()=>{}})});
 const only=await display.subscribe(command,'a');
 await display.park(host.displays.get(only.subscription_id),true);
 assert.equal(closed,1);assert.equal(display.source,null);
 await display.park(host.displays.get(only.subscription_id),true);assert.equal(sources,1,'repeated parks never restart it');
 await display.close();
});
