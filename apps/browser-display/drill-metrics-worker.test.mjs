// MD-DISPLAY-02/04: fidelity equivalence and awaited worker failure/ownership.
import test from 'node:test';
import assert from 'node:assert/strict';
import {EventEmitter} from 'node:events';
import {MetricsWorker} from './drill-metrics-worker.mjs';
import {encodePng} from '../kernel/slice-linux-docker/docker/kernel-browser-pixels.mjs';
class FakeWorker extends EventEmitter {
 ref(){} unref(){} postMessage(value){this.sent=value} async terminate(){this.terminated=true;return 0}
}
test('MD-DISPLAY PNG worker preserves exact RGB and numerical PSNR metrics',{skip:!process.env.MD_TOOLS},async()=>{
 const worker=new MetricsWorker(process.env.MD_TOOLS),pixels=Buffer.alloc(8*8*4,255);
 const source=Buffer.from(encodePng(8,8,pixels),'base64');
 try{
  const exact=await worker.compare(source,source);assert.equal(exact.lossless,true);assert.equal(exact.width,8);assert(Buffer.isBuffer(exact.diff));
  pixels[0]=225;const changed=await worker.compare(source,Buffer.from(encodePng(8,8,pixels),'base64'));
  assert.equal(changed.mse_rgb,900/(64*3));assert.equal(changed.changed_pixels_fraction,1/64);
 }finally{await worker.close()}
});
test('MD-DISPLAY PNG worker transfers only image bytes, never a pooled backing store',async()=>{
 const worker=new MetricsWorker('/public-tools',{WorkerClass:FakeWorker}),source=Buffer.from([1,2,3,4]);
 try{
  const pending=worker.compare(source,source);assert.equal(worker.worker.sent.reference.byteLength,4);
  worker.worker.emit('message',{id:worker.worker.sent.id,result:{lossless:true,diff:null}});assert((await pending).lossless);
 }finally{await worker.close()}
});
test('MD-DISPLAY asynchronous PNG worker errors reject the awaited operation',async()=>{
 const worker=new MetricsWorker('/public-tools',{WorkerClass:FakeWorker});
 const rejected=assert.rejects(worker.compare(Buffer.alloc(1),Buffer.alloc(1)),/PNG worker failed/);
 worker.worker.emit('error',Error('fixture'));await rejected;await worker.close();assert(worker.worker.terminated);
});
test('MD-DISPLAY malformed worker callback reaches awaited failure and cleanup',async()=>{
 const worker=new MetricsWorker('/public-tools',{WorkerClass:FakeWorker});
 const rejected=assert.rejects(worker.compare(Buffer.alloc(1),Buffer.alloc(1)),/malformed/);
 worker.worker.emit('message',{id:worker.worker.sent.id});await rejected;await worker.close();
});
test('MD-DISPLAY PNG job timeout terminates its own thread without process signals',async()=>{
 const worker=new MetricsWorker('/public-tools',{WorkerClass:FakeWorker,timeoutMs:5});
 await assert.rejects(worker.compare(Buffer.alloc(1),Buffer.alloc(1)),/timeout/);await worker.close();assert(worker.worker.terminated);
});
