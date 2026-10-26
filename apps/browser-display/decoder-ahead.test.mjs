// MP-08/MP-10/MP-11: decode-ahead bounds and pre-track geometry fences.
import test from 'node:test';import assert from 'node:assert/strict';import vm from 'node:vm';import {readFileSync} from 'node:fs';
import {WorkerVideoDecoder} from './decoder-worker.mjs';
const source=readFileSync(new URL('./decoder-worker.mjs',import.meta.url),'utf8');
function fixture(){
 const decoders=[],messages=[],writes=[];class Scope{static[Symbol.hasInstance](){return true}}
 const scope=new Scope();Object.assign(scope,{performance:{now:()=>100,timeOrigin:0},WorkerGlobalScope:Scope,self:scope,postMessage:data=>messages.push(data),EncodedVideoChunk:class{constructor(v){Object.assign(this,v)}},VideoDecoder:class{constructor(c){this.c=c;this.state='configured';decoders.push(this)}configure(){}decode(chunk){this.chunks??=[];this.chunks.push(chunk)}close(){this.state='closed'}}});
 vm.runInNewContext(source.slice(0,source.indexOf('export class')),scope);
 const send=overrides=>scope.onmessage({data:{id:1,channel:8,codec:'vp8',key:true,sequence:1,width:1280,height:800,cssWidth:1280,cssHeight:800,sink:true,bytes:new Uint8Array([1]),...overrides}});
 const output=(n,width=1280)=>decoders.at(-1).c.output({timestamp:decoders.at(-1).chunks[n-1].timestamp,displayWidth:width,displayHeight:800,close(){},clone(){return {timestamp:decoders.at(-1).chunks[n-1].timestamp,close(){}}}});
 scope.onmessage({data:{op:'sink',writable:{getWriter:()=>({ready:Promise.resolve(),write:async f=>writes.push(f.timestamp)})}}});
 return {send,output,decoders,messages,writes};
}
test('MP-08/MP-10 worker decodes three ahead and correlates output by timestamp',async()=>{
 const f=fixture();for(let n=1;n<=3;n++)f.send({id:n,sequence:n,key:n===1});assert.equal(f.decoders.length,1);assert.equal(f.decoders[0].chunks.length,3);
 for(let n=1;n<=3;n++)f.output(n);await new Promise(r=>setImmediate(r));
 assert.deepEqual(f.messages.map(m=>m.id),[1,2,3]);assert.deepEqual(f.writes,[100000,100001,100002]);
});
test('MP-08/MP-10 fourth pending video reports all lost jobs and retires decoder',()=>{
 const f=fixture();for(let n=1;n<=4;n++)f.send({id:n,sequence:n,key:n===1});
 assert.equal(f.decoders[0].chunks.length,3);assert.equal(f.decoders[0].state,'closed');assert.deepEqual(f.messages.map(m=>m.id),[1,2,3,4]);assert(f.messages.every(m=>m.error));
});
test('MP-11 invalid decoded geometry never reaches native pixels',async()=>{
 const f=fixture();f.send({});f.output(1,640);await new Promise(r=>setImmediate(r));assert.deepEqual(f.writes,[]);assert(f.messages[0].error);
});
test('MP-08/MP-10 main-side whole-video bound is three and reset rejects every job',async()=>{
 let worker;class W{constructor(){worker=this}postMessage(){}terminate(){}}
 const d=new WorkerVideoDecoder(W),frame={sequence:1,codec:'vp8',key:true,width:1280,height:800};
 const jobs=[1,2,3].map(sequence=>d.decode({...frame,sequence},new Uint8Array([1]),8));for(const job of jobs)job.catch(()=>{});
 await assert.rejects(d.decode({...frame,sequence:4},new Uint8Array([1]),8),/busy/);d.reset(8);for(const job of jobs)await assert.rejects(job,/reset/);assert.equal(d.jobs.size,0);assert(worker);d.close();
});
