// MD-DISPLAY-04: decode ownership, tile batch pinning and prediction restoration.
import test from 'node:test';import assert from 'node:assert/strict';import{WorkerVideoDecoder}from'./decoder-worker.mjs';import{ScrollPrediction}from'./scroll-prediction.mjs';
test('MD-DISPLAY worker decoder bounds jobs and closes late transferred frames',async()=>{let worker;class W{constructor(){worker=this}postMessage(data){this.sent=data}terminate(){this.stopped=true}}const d=new WorkerVideoDecoder(W),f={codec:'vp09.00.40.08',key:true,sequence:1,width:2560,height:1600};const pending=d.decode(f,new Uint8Array([1]));await assert.rejects(d.decode(f,new Uint8Array([1])),/busy/);let closed=0;const frame={close(){closed++}};worker.onmessage({data:{id:worker.sent.id,frame}});assert.equal(await pending,frame);const closing=d.decode({...f,sequence:2,key:false},new Uint8Array([2]));d.close();await assert.rejects(closing,/closed/);worker.onmessage({data:{id:worker.sent.id,frame}});assert.equal(closed,1);assert.equal(worker.stopped,true)});
test('MD-DISPLAY optional scroll prediction restores real pixels and requires an explicit rectangle',()=>{const old=globalThis.OffscreenCanvas;const draws=[];globalThis.OffscreenCanvas=class{constructor(w,h){this.width=w;this.height=h}getContext(){return {drawImage(...args){draws.push(args)}}}};try{const canvas={width:2560,height:1600,getContext:()=>({drawImage(...args){draws.push(args)}})},input={kind:'scroll',delta_x:0,delta_y:40};assert.equal(new ScrollPrediction(canvas,null).predict(input,2),false);const p=new ScrollPrediction(canvas,{x:0,y:100,width:1280,height:700});assert.equal(p.predict(input,2),true);assert.equal(draws.length,2);p.restore();assert.equal(draws.length,3);assert.equal(p.saved,null);p.close()}finally{globalThis.OffscreenCanvas=old}});

test('MP-11 the decoder worker replaces an errored decoder on the next key frame',async()=>{
 const prior={WorkerGlobalScope:globalThis.WorkerGlobalScope,self:globalThis.self,VideoDecoder:globalThis.VideoDecoder,EncodedVideoChunk:globalThis.EncodedVideoChunk};
 const posted=[];let fail=true;
 globalThis.WorkerGlobalScope=class{static [Symbol.hasInstance](value){return value===globalThis}};
 globalThis.self={postMessage:data=>posted.push(data)};
 globalThis.EncodedVideoChunk=class{constructor(value){Object.assign(this,value)}};
 globalThis.VideoDecoder=class{constructor(c){this.c=c;this.state='configured'}configure(){}
  close(){if(this.state==='closed')throw Error('InvalidStateError');this.state='closed'}
  decode(){queueMicrotask(()=>{if(fail){this.state='closed';this.c.error(Error('decode'))}else this.c.output({close(){}})})}};
 try{
  await import('./decoder-worker.mjs?errored-decoder');
  const send=id=>globalThis.self.onmessage({data:{id,codec:'vp8',key:true,sequence:id,width:8,height:8,bytes:new Uint8Array([1])}});
  send(1);await new Promise(resolve=>setTimeout(resolve,0));
  fail=false;send(2);await new Promise(resolve=>setTimeout(resolve,0));
  assert.deepEqual(posted.map(m=>[m.id,Boolean(m.error)]),[[1,true],[2,false]]);
 }finally{for(const [key,value] of Object.entries(prior))if(value===undefined)delete globalThis[key];else globalThis[key]=value}
});
