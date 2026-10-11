// MD-DISPLAY-04: decode ownership, tile batch pinning and prediction restoration.
import test from 'node:test';import assert from 'node:assert/strict';import{WorkerVideoDecoder}from'./decoder-worker.mjs';import{ScrollPrediction}from'./scroll-prediction.mjs';
test('MD-DISPLAY worker decoder bounds jobs and closes late transferred frames',async()=>{let worker;class W{constructor(){worker=this}postMessage(data){this.sent=data}terminate(){this.stopped=true}}const d=new WorkerVideoDecoder(W),f={codec:'vp09.00.40.08',key:true,sequence:1,width:2560,height:1600};const pending=d.decode(f,new Uint8Array([1]));await assert.rejects(d.decode(f,new Uint8Array([1])),/busy/);let closed=0;const frame={close(){closed++}};worker.onmessage({data:{id:worker.sent.id,frame}});assert.equal(await pending,frame);const closing=d.decode({...f,sequence:2,key:false},new Uint8Array([2]));d.close();await assert.rejects(closing,/closed/);worker.onmessage({data:{id:worker.sent.id,frame}});assert.equal(closed,1);assert.equal(worker.stopped,true)});
test('MD-DISPLAY optional scroll prediction restores real pixels and requires an explicit rectangle',()=>{const old=globalThis.OffscreenCanvas;const draws=[];globalThis.OffscreenCanvas=class{constructor(w,h){this.width=w;this.height=h}getContext(){return {drawImage(...args){draws.push(args)}}}};try{const canvas={width:2560,height:1600,getContext:()=>({save(){},restore(){},beginPath(){},rect(){},clip(){},drawImage(...args){draws.push(args)}})},input={kind:'scroll',delta_x:0,delta_y:40};assert.equal(new ScrollPrediction(canvas,null).predict(input,2),false);const p=new ScrollPrediction(canvas,{x:0,y:100,width:1280,height:700});assert.equal(p.predict(input,2),true);assert.equal(draws.length,2);p.restore();assert.equal(draws.length,3);assert.equal(p.saved,null);p.close()}finally{globalThis.OffscreenCanvas=old}});


// MP-08/MP-10/MP-11: multiplexing retains ownership and per-chain bounds.
test('MP-08/MP-10/MP-11 shared decoder channels reset independently and close late output',async()=>{
 let worker;class W{constructor(){worker=this;this.messages=[]}postMessage(data){this.messages.push(data)}terminate(){this.stopped=true}}
 const d=new WorkerVideoDecoder(W),f={codec:'vp8',key:true,sequence:1,width:1280,height:100};
 const row=d.decode(f,new Uint8Array([1]),0),video=d.decode(f,new Uint8Array([2]),8);row.catch(()=>{});
 await assert.rejects(d.decode(f,new Uint8Array([3]),0),/busy/);
 await assert.rejects(d.decode(f,new Uint8Array([3]),9),/busy/);
 d.reset(0);await assert.rejects(row,/reset/);assert.equal(d.closed,false);
 let closed=0;worker.onmessage({data:{id:worker.messages[0].id,frame:{close(){closed++}}}});assert.equal(closed,1);
 const frame={close(){closed++}};worker.onmessage({data:{id:worker.messages[1].id,frame}});assert.equal(await video,frame);
 const restarted=d.decode(f,new Uint8Array([4]),0);d.close();await assert.rejects(restarted,/closed/);assert(worker.stopped);
});
