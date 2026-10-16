// MP-08/MP-10/MP-11: bounded ACK ownership across hidden-page heartbeats.
import test from 'node:test';import assert from 'node:assert/strict';
const {attachBrowserDisplay}=await import(process.env.MD_PRESENTER_SOURCE??'./presenter.mjs');
const pause=ms=>new Promise(r=>setTimeout(r,ms));
async function fixture(hidden=false,hold=false,leaseMs=Infinity){
 const document=new EventTarget();document.hidden=hidden;
 const canvas={width:1280,height:800,ownerDocument:document};const ops=[],replies=[];let expires=performance.now()+leaseMs;
 const transport={kernelProtocolVersion:491,displayEventEncoding:'CXD1',onEvent:()=>()=>{},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},request:async({KernelBrowser:{command}})=>{
  ops.push(command);if(command.op==='display_ack'){const now=performance.now();if(now>expires)return {KernelBrowser:{result:{push:'display_subscription_required'}}};expires=now+leaseMs;}if(command.op==='display_ack'&&hold)return new Promise(resolve=>replies.push(()=>resolve({KernelBrowser:{result:{push:'running'}}})));
  return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:{push:'running'}}};
 }};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1},{heartbeatMs:5});stream.start();
 return {stream,document,ops,replies};
}
test('MP-08/MP-10 hidden heartbeats renew the lease beyond its expiry and resume',async()=>{
 const h=await fixture(true,false,60); // scaled 60-second relay lease
 try{
  await pause(150); // more than two leases, no presented frames
  assert(h.ops.filter(c=>c.op==='display_ack').length>=2,'MP-10 hidden heartbeat renews lease');
  h.stream.presenter.sequence=12;h.document.hidden=false;h.document.dispatchEvent(new Event('visibilitychange'));
  await pause(1);assert.equal(h.ops.filter(c=>c.op==='display_ack').at(-1).sequence,12);
  assert.equal(h.stream.error,null,'MP-10 visible return keeps the running binding');
 }finally{await h.stream.close()}
});
// MP-08/MP-10/MP-11: C1 pipelines replies; hidden renewal obeys the same bound.
for(const hidden of [true,false])test(`MP-08/MP-10/MP-11 ${hidden?'hidden':'visible'} slow ACK caps requests and releases the newest cursor`,async()=>{
 const h=await fixture(hidden,true),acks=()=>h.ops.filter(c=>c.op==='display_ack');
 try{
  const until=performance.now()+1000;while(acks().length<8&&performance.now()<until)await pause(5);
  assert.equal(acks().length,8);h.stream.presenter.sequence=12;await pause(20);
  assert.equal(acks().length,8,'held replies must not grow the ACK window');
  h.replies.shift()();await pause(1);assert.equal(acks().length,9);assert.equal(acks().at(-1).sequence,12);
 }finally{await h.stream.close();for(const reply of h.replies)reply()}
});
