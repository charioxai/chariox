// MP-08/MP-10/MP-11: Room placement reuses the shared producer and 525 lifetime.
import test from 'node:test';
import assert from 'node:assert/strict';
import {RoomSharedDisplay} from './room-shared-display.mjs';
const policy={unknown:false,targets:[],values:[]};
const offer={op:'display_subscribe',observed_by:'room-lease-one',codecs:['avc1.420033'],bitrate:8000000,device_scale_factor:2};
function fixture(){
 let surface={surface_id:'slice:one',generation:'desktop-one',width:1920,height:1080},sources=0,closes=0,now=Date.now();
 const source={closed:false,sample:()=>null,valid:()=>!source.closed,close:async()=>{source.closed=true;closes++}};
 const display=new RoomSharedDisplay({binding:async()=>surface},{desktop:{createSource:async binding=>{assert.equal(binding,surface);sources++;return source}},sessions:{now:()=>now,clock:()=>now}});
 const request=params=>display.request({...params,policy});
 return {display,request,source,counts:()=>({sources,closes}),replace:()=>{surface={...surface,generation:'desktop-two'}},advance:ms=>now+=ms};
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
