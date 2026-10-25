// MP-08/MP-10/MP-11: Room placement reuses the shared producer and 525 lifetime.
import test from 'node:test';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
import {RoomSharedDisplay} from './room-shared-display.mjs';
const policy={unknown:false,targets:[],values:[]};
const offer={op:'display_subscribe',observed_by:'room-lease-one',codecs:['avc1.420033'],bitrate:8000000,device_scale_factor:2};
function fixture(){
 let surface={surface_id:'slice:one',generation:'desktop-one',width:1920,height:1080},sources=0,closes=0,now=Date.now();
 const source={closed:false,sample:()=>null,valid:()=>!source.closed,close:async()=>{source.closed=true;closes++}};
 const display=new RoomSharedDisplay({binding:async()=>surface},{desktop:{createSource:async binding=>{assert.equal(binding,surface);sources++;return source}},sessions:{now:()=>now,clock:()=>now}});
 const request=params=>display.request({...params,policy});
 return {display,request,source,counts:()=>({sources,closes}),replace:()=>{surface={...surface,generation:'desktop-two'}},advance:ms=>now+=ms,syncClock:()=>{now=Date.now()}};
}
test('MP-08 Room display binds the existing desktop; two leases share its protected source',async()=>{
 const f=fixture();try{
  const a=await f.request(offer),b=await f.request({...offer,observed_by:'room-lease-two'});
  assert.equal(a.source.surface_id,'slice:one');assert.equal(a.source.width,1920);assert.equal(a.device_scale_factor,2);
  assert.equal(f.counts().sources,1);
  await assert.rejects(f.request({op:'display_attach',subscription_id:a.subscription_id,generation:a.generation,observed_by:'room-lease-two'}));
  await f.request({op:'unsubscribe',subscription_id:a.subscription_id,generation:a.generation,observed_by:'room-lease-one'});
  assert.equal(f.counts().closes,0);
  await f.request({op:'unsubscribe',subscription_id:b.subscription_id,generation:b.generation,observed_by:'room-lease-two'});
  assert.equal(f.counts().closes,1);
 }finally{await f.display.close()}
});
test('MP-11 Room Actions remain the only input authority and unknown protection refuses before capture',async()=>{
 const f=fixture();try{
  for(const op of ['input','takeover','release','start','launch_application'])await assert.rejects(f.request({...offer,op}));
  await assert.rejects(f.display.request({...offer,policy:{...policy,unknown:true}}));assert.equal(f.counts().sources,0);
 }finally{await f.display.close()}
});
test('MP-08 Room resumes through shared 525 sessions with a fresh delivery id and the same producer',async()=>{
 const f=fixture();try{
  const a=await f.request(offer);
  const command={subscription_id:a.subscription_id,generation:a.generation,observed_by:'room-lease-one'};
  await f.request({...command,op:'display_suspend'});f.advance(1000);
  const b=await f.request({...command,op:'display_resume'});
  assert.equal(b.resumed,true);assert.notEqual(b.subscription_id,a.subscription_id);assert.equal(f.counts().sources,1);
  await assert.rejects(f.request({...command,op:'display_attach'}));
  await f.request({...command,subscription_id:b.subscription_id,op:'display_attach'});
 }finally{await f.display.close()}
});
test('MP-11 Room surface replacement or protection changes retire old credits and resume hints',async()=>{
 for(const change of ['surface','policy','expired']){
  const f=fixture();try{
   const a=await f.request(offer),command={subscription_id:a.subscription_id,generation:a.generation,observed_by:'room-lease-one'};
   await f.request({...command,op:'display_suspend'});
   if(change==='surface')f.replace();if(change==='expired')f.advance(5000);
   await assert.rejects(f.display.request({...command,op:'display_resume',policy:change==='policy'?{...policy,targets:[{kind:'native',value_hash:'changed'}]}:policy}));
  }finally{await f.display.close()}
 }
});

test('MP-11 Room desktop target comes from the bound worker observation, never a viewer crop',async()=>{
 const f=fixture();try{
  for(const target of [{kind:'desktop',surface_id:'slice:foreign',generation:'desktop-one'},{kind:'desktop',surface_id:'slice:one',generation:'stale'},{kind:'region',binding_id:'foreign'}])await assert.rejects(f.request({...offer,target}));
  assert.equal(f.counts().sources,0);
  const result=await f.request({...offer,target:{kind:'desktop',surface_id:'slice:one',generation:'desktop-one'}});assert.equal(result.source.surface_id,'slice:one');
 }finally{await f.display.close()}
});

test('MP-08/MP-10/MP-11 535 registration resumes and attaches through the sole 525 session authority',async()=>{
 const f=fixture();try{
  const old=await f.request({...offer,lease_id:'room-display-'+'a'.repeat(32),observed_by:'room-display-'+'a'.repeat(32)});
  await f.request({op:'display_attach',subscription_id:old.subscription_id,observed_by:old.subscription_id,generation:old.generation});
  f.syncClock();
  const resumed=await f.request({op:'display_resume_attach',subscription_id:old.subscription_id,observed_by:old.subscription_id,generation:old.generation});
  assert.equal(resumed.resumed,true);assert.notEqual(resumed.subscription_id,old.subscription_id);
  assert.deepEqual(f.counts(),{sources:1,closes:0});
  const stream=f.display.displays.get(resumed.subscription_id);assert.equal(stream.observed_by,resumed.subscription_id);assert.equal(stream.suspended,false);
  await assert.rejects(f.request({op:'display_resume_attach',subscription_id:old.subscription_id,observed_by:old.subscription_id,generation:old.generation}));
  await f.request({op:'display_attach',subscription_id:resumed.subscription_id,observed_by:resumed.subscription_id,generation:resumed.generation});
 }finally{await f.display.close()}
});

// MP-08/MP-10/MP-11: anonymous numeric delivery markers join a worker's
// queued request, producer wait and first frame without recording identities.
test('MP-08/MP-10/MP-11 worker trace joins each viewer and resumed delivery without authority data',async()=>{
 const f=fixture(),events=[];let flushes=0;f.display.timing.event=(stage,fields)=>events.push({stage,...fields});f.display.timing.flush=()=>flushes++;
 f.display.display.createProducer=()=>({feedback(){},invalidate(){},close:async()=>{},waitReady:async()=>{},take:()=>({motion:true,width:1920,height:1080,data_base64:'protected-sample',encoded:{key:true,data_base64:'AAAA'}})});
 try{
  const a=await f.request(offer),b=await f.request({...offer,observed_by:'private-viewer-two'});
  for(const [binding,owner] of [[a,offer.observed_by],[b,'private-viewer-two']]){
   await f.request({op:'display_attach',subscription_id:binding.subscription_id,observed_by:owner,generation:1});
   for(const after_sequence of [0,1])await f.request({op:'screenshot',subscription_id:binding.subscription_id,observed_by:owner,generation:1,after_sequence,push:{reset:false,congested:false}});
  }
  f.syncClock();
  const resumed=await f.request({op:'display_resume_attach',subscription_id:a.subscription_id,observed_by:offer.observed_by,generation:1});
  await f.request({op:'screenshot',subscription_id:resumed.subscription_id,observed_by:offer.observed_by,generation:1,after_sequence:0,push:{reset:true,congested:false}});
  const join=id=>Number(createHash('sha256').update(id).digest().readBigUInt64BE(0)>>12n);
  const subscriptions=events.filter(e=>e.stage==='room_display_subscribed');
  assert.deepEqual(subscriptions.map(e=>[e.viewer,e.delivery,e.stream_join]),[[1,1,join(a.subscription_id)],[2,1,join(b.subscription_id)]],'MP-10 concurrent initial opens need an exact opaque stream join, never order inference');
  const recoveries=events.filter(e=>e.stage==='room_display_resumed');
  assert.deepEqual(recoveries.map(e=>[e.viewer,e.delivery,e.stream_join]),[[1,2,join(resumed.subscription_id)]],'MP-10 a resumed delivery joins its fresh public stream hint to the same anonymous viewer');
  assert.notEqual(recoveries[0].stream_join,subscriptions[0].stream_join);
  const first=events.filter(e=>e.stage==='desktop_delivery_first_frame');
  assert.deepEqual(first.map(e=>[e.viewer,e.delivery,e.sequence]),[[1,1,1],[2,1,1],[1,2,1]],'MP-10 anonymous worker first-frame joins missing');
  const outgoing=events.filter(e=>e.stage==='desktop_frame_out'&&e.sequence===1);
  assert.deepEqual(outgoing.map(e=>[e.viewer,e.delivery,e.sequence]),first.map(e=>[e.viewer,e.delivery,e.sequence]),'MP-10 every first outgoing frame must join its anonymous viewer and delivery');
  for(const stage of ['room_display_queue','room_display_binding','room_display_operation','desktop_producer_wait','desktop_frame_build'])assert(events.some(e=>e.stage===stage),'MP-10 missing stage '+stage);
  for(const event of events)for(const [key,value]of Object.entries(event))if(key!=='stage'&&value!==undefined)assert(Number.isFinite(value),'MP-11 timing fields must be numeric');
  const text=JSON.stringify(events);for(const value of [a.subscription_id,b.subscription_id,resumed.subscription_id,'private-viewer-two'])assert(!text.includes(value));
 }finally{await f.display.close()}
 assert.equal(flushes,1,'MP-10 shutdown must flush the last worker delivery markers');
});

// MP-08/MP-10: an idle producer wait of one viewer's frame pump must not hold
// the Room queue, or a recovering viewer waits behind every other pump.
test('MP-08/MP-10 one viewer idle producer wait never delays another viewer or its own resume',async()=>{
 const f=fixture(),waiters=new Set(),wake=()=>{for(const done of waiters)done();waiters.clear()};
 f.display.display.createProducer=()=>({feedback(){},invalidate(){},close:async()=>{},take:()=>null,
  waitReady:()=>new Promise(resolve=>{waiters.add(resolve);setTimeout(resolve,400)}),wakeReady:wake});
 const within=async(promise,ms,label)=>{let timer;try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error(label)),ms)})])}finally{clearTimeout(timer)}};
 try{
  const a=await f.request(offer),b=await f.request({...offer,observed_by:'room-lease-two'});
  const pump=(binding,owner)=>f.request({op:'screenshot',subscription_id:binding.subscription_id,observed_by:owner,generation:1,after_sequence:0,push:{reset:false,congested:false}});
  for(const [binding,owner] of [[a,offer.observed_by],[b,'room-lease-two']])await f.request({op:'display_attach',subscription_id:binding.subscription_id,observed_by:owner,generation:1});
  const waitingA=pump(a,offer.observed_by),waitingB=pump(b,'room-lease-two');
  await new Promise(resolve=>setTimeout(resolve,20));
  f.syncClock();
  const resumed=await within(f.request({op:'display_resume_attach',subscription_id:a.subscription_id,observed_by:offer.observed_by,generation:1}),200,'resume queued behind idle producer waits');
  assert.equal(resumed.resumed,true);
  await within(f.request({op:'display_attach',subscription_id:b.subscription_id,observed_by:'room-lease-two',generation:1}),200,'attach queued behind an idle producer wait');
  await Promise.all([waitingA,waitingB]);
 }finally{await f.display.close()}
});

// MP-08/MP-10/MP-11: only the authenticated home control may change the shared
// source's audience policy. A viewer offer never supplies this authority.
test('MP-11 Room audience defaults masked and ignores viewer mask flags',async()=>{
 const f=fixture(),protectedPolicy={unknown:false,values:['vault-canary'],targets:[{kind:'native',value_hash:'vault-hash'}]};
 try{
  await f.display.request({...offer,policy:protectedPolicy,mask:false});
  assert.deepEqual(f.display.protection,protectedPolicy);
  assert.equal(f.display.audienceMask,true);
 }finally{await f.display.close()}
});
test('MP-08 Room home audience control retires shared pixels before revealing or masking',async()=>{
 const f=fixture(),protectedPolicy={unknown:false,values:['vault-canary'],targets:[{kind:'native',value_hash:'vault-hash'}]};
 try{
  await f.display.request({...offer,policy:protectedPolicy});
  const stale=f.display.protection;
  const result=await f.display.request({op:'display_mask',mask:false,policy:protectedPolicy});
  assert.equal(result.mask,false);
  assert.equal(f.display.audienceMask,false);
  assert.deepEqual(f.display.protection,protectedPolicy);
  assert.equal(f.display.protection,stale);
  assert.equal(f.counts().closes,0,'keep the already warm masked capture source');
  assert.equal(f.display.display.source,f.source);
  await f.display.request({op:'display_mask',mask:true,policy:protectedPolicy});
  assert.equal(f.display.audienceMask,true);
  assert.deepEqual(f.display.protection,protectedPolicy);
  await assert.rejects(f.display.request({op:'display_mask',mask:'false',policy:protectedPolicy}));
 }finally{await f.display.close()}
});
test('MP-11 Room audience changes invalidate an in-flight frame before retirement waits',async()=>{
 const f=fixture(),protectedPolicy={unknown:false,values:['vault-canary'],targets:[{kind:'native',value_hash:'vault-hash'}]};
 let unblock,closing;
 try{
  await f.display.request({...offer,policy:protectedPolicy});
  const previous=f.display.audienceEpoch;
  const stream=[...f.display.displays.values()][0];
  closing=new Promise(resolve=>unblock=resolve);
  stream.producer={close:async()=>{assert.notEqual(f.display.audienceEpoch,previous);await closing},invalidate(){}};
  const update=f.display.request({op:'display_mask',mask:false,policy:protectedPolicy});
  await new Promise(resolve=>setImmediate(resolve));
  assert.notEqual(f.display.audienceEpoch,previous,'queued old frames must lose validity synchronously');
  unblock();await update;
 }finally{unblock?.();await f.display.close()}
});

test('MP-11 Room unmask grant expires masked when the home link stops renewing',async t=>{
 t.mock.timers.enable({apis:['setTimeout']});
 const f=fixture();
 try{
  await f.display.request({op:'display_mask',mask:false,policy});
  const epoch=f.display.audienceEpoch;
  t.mock.timers.tick(2999);assert.equal(f.display.audienceMask,false);
  await f.display.request({op:'display_mask',mask:false,policy});
  assert.equal(f.display.audienceEpoch,epoch,'renewal must preserve the encoder and its references');
  t.mock.timers.tick(2);assert.equal(f.display.audienceMask,false);
  t.mock.timers.tick(2998);assert.equal(f.display.audienceMask,true,'link expiry revokes frame validity immediately');
  assert.notEqual(f.display.audienceEpoch,epoch);
  await f.display.pending;
  assert.equal(f.display.audienceApplied,true);
  await f.display.request({op:'display_mask',mask:true,policy:{unknown:true,targets:[],values:[]}});
  assert.equal(f.display.protection.unknown,true,'unknown policy must still allow fail-closed revocation');
  await assert.rejects(f.display.request({op:'display_mask',mask:false,policy:{unknown:true,targets:[],values:[]}}));
 }finally{await f.display.close()}
});
