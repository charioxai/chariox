// MP-08/MP-10/MP-11: sequence numbers order dependencies, not the live track clock.
import test from 'node:test';import assert from 'node:assert/strict';
test('MP-08/MP-10/MP-11 decoder uses monotonic arrival time across quiet gaps and same-tick bursts',async()=>{
 const names=['WorkerGlobalScope','self','VideoDecoder','EncodedVideoChunk','performance'],prior=Object.fromEntries(names.map(k=>[k,globalThis[k]]));
 const posted=[],timestamps=[];let clock=100;
 globalThis.WorkerGlobalScope=class{static [Symbol.hasInstance](v){return v===globalThis}};
 globalThis.self={postMessage:data=>posted.push(data)};
 Object.defineProperty(globalThis,'performance',{configurable:true,value:{timeOrigin:1000,now:()=>clock}});
 globalThis.EncodedVideoChunk=class{constructor(value){Object.assign(this,value)}};
 globalThis.VideoDecoder=class{constructor(callbacks){this.callbacks=callbacks;this.state='configured'}configure(){}close(){this.state='closed'}decode(chunk){timestamps.push(chunk.timestamp);queueMicrotask(()=>this.callbacks.output({timestamp:chunk.timestamp,close(){}}))}};
 try{
  await import('./decoder-worker.mjs?arrival-clock');
  const send=sequence=>self.onmessage({data:{id:sequence,channel:8,codec:'vp8',key:sequence===1,sequence,width:8,height:8,bytes:new Uint8Array([1])}});
  send(1);await new Promise(r=>setTimeout(r,0));clock=250;send(2);send(3);await new Promise(r=>setTimeout(r,0));
  assert.deepEqual(posted.map(j=>j.id),[1,2,3]);assert(posted.every(j=>!j.error));
  assert.equal(timestamps[1]-timestamps[0],150000,'a150ms idle gap must not become a fabricated60Hz clock');
  assert.equal(timestamps[2]-timestamps[1],1,'same-tick frames have unique monotonic microsecond timestamps');
 }finally{for(const [key,value]of Object.entries(prior))if(value===undefined)delete globalThis[key];else Object.defineProperty(globalThis,key,{configurable:true,writable:true,value})}
});
