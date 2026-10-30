// A display stream runs only while a person can see it: unwatched = no ACKs, watched again = one independent-frame request.
import test from 'node:test';import assert from 'node:assert/strict';
const {attachBrowserDisplay}=await import(process.env.MD_PRESENTER_SOURCE??'./presenter.mjs');
const pause=ms=>new Promise(r=>setTimeout(r,ms));
function gate(value){const listeners=new Set();return {get value(){return value},set(next){value=next;for(const listener of [...listeners])listener(next)},subscribe:listener=>{listeners.add(listener);return()=>listeners.delete(listener)},listeners}}
async function fixture(watched){
 const document=new EventTarget();document.hidden=false;
 const canvas={width:1280,height:800,ownerDocument:document};const ops=[];
 const transport={kernelProtocolVersion:491,displayEventEncoding:'CXD1',onEvent:()=>()=>{},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},request:async({KernelBrowser:{command}})=>{
  ops.push(command);return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:{push:'running'}}};
 }};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1},{heartbeatMs:5,watched});stream.start();
 const acks=()=>ops.filter(c=>c.op==='display_ack');
 return {stream,ops,acks};
}
test('an unwatched display sends no ACKs and asks for one independent frame when watched again',async()=>{
 const watched=gate(true),h=await fixture(watched);
 try{
  await pause(40);assert.ok(h.acks().length>=3,'watched: heartbeats run');
  watched.set(false);await pause(10);const stopped=h.acks().length;
  await pause(60);assert.equal(h.acks().length,stopped,'unwatched: no ACK leaves');
  watched.set(true);await pause(10);
  const resumed=h.acks().slice(stopped);
  assert.ok(resumed.length>=1&&resumed[0].lost===true,'watched again: the first ACK asks for a key frame');
  const after=h.acks().length;await pause(40);assert.ok(h.acks().length>after,'heartbeats continue');
 }finally{await h.stream.close();assert.equal(watched.listeners.size,0,'close drops the subscription')}
});
test('a display attached while unwatched stays silent until watched',async()=>{
 const watched=gate(false),h=await fixture(watched);
 try{
  await pause(60);assert.equal(h.acks().length,0);
  watched.set(true);await pause(20);assert.ok(h.acks().length>=1);assert.equal(h.acks()[0].lost,true);
 }finally{await h.stream.close()}
});
test('an explicit stop is not undone by becoming watched',async()=>{
 const watched=gate(true),h=await fixture(watched);
 try{
  await h.stream.stop();watched.set(false);watched.set(true);await pause(10);const stopped=h.acks().length;
  await pause(50);assert.equal(h.acks().length,stopped);
 }finally{await h.stream.close()}
});
