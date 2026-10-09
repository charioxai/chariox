// MP-08/MP-10/MP-11: bounded ACK ownership across hidden-page heartbeats.
import test from 'node:test';import assert from 'node:assert/strict';
const {attachBrowserDisplay}=await import(process.env.MD_PRESENTER_SOURCE??'./presenter.mjs');
const pause=ms=>new Promise(r=>setTimeout(r,ms));
async function fixture(hidden=false,hold=false){
 const document=new EventTarget();document.hidden=hidden;
 const canvas={width:1280,height:800,ownerDocument:document};const ops=[],replies=[];
 const transport={kernelProtocolVersion:475,displayEventEncoding:'CXD1',onEvent:()=>()=>{},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},request:async({KernelBrowser:{command}})=>{
  ops.push(command);if(command.op==='display_ack'&&hold)return new Promise(resolve=>replies.push(()=>resolve({KernelBrowser:{result:{push:'running'}}})));
  return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:{push:'running'}}};
 }};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1},{heartbeatMs:5});stream.start();
 return {stream,document,ops,replies};
}
test('MP-08/MP-10 hidden heartbeats send no ACK and resume one current ACK',async()=>{
 const h=await fixture(true);
 try{await pause(40);assert.equal(h.ops.filter(c=>c.op==='display_ack').length,0);
  h.stream.presenter.sequence=12;h.document.hidden=false;h.document.dispatchEvent(new Event('visibilitychange'));
  await pause(1);assert.deepEqual(h.ops.filter(c=>c.op==='display_ack').map(c=>c.sequence),[12]);
 }finally{await h.stream.close()}
});
test('MP-08/MP-10 slow ACK remains one in flight with one latest sequence',async()=>{
 const h=await fixture(false,true);
 try{h.stream.presenter.sequence=12;await pause(40);assert.equal(h.ops.filter(c=>c.op==='display_ack').length,1);
  h.replies.shift()();await pause(1);assert.deepEqual(h.ops.filter(c=>c.op==='display_ack').map(c=>c.sequence),[0,12]);
 }finally{for(const reply of h.replies)reply();await h.stream.close()}
});
