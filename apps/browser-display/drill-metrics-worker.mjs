// MD-DISPLAY-02/04: PNG work must not block draining owned kernel diagnostics.
import {Worker, isMainThread, parentPort, workerData} from 'node:worker_threads';
import {createRequire} from 'node:module';
import path from 'node:path';
import {compare} from './drill-metrics.mjs';

if(!isMainThread){
 const {PNG}=createRequire(path.join(workerData.tools,'package.json'))('pngjs');
 parentPort.on('message',({id,reference,actual})=>{
  try{
   const result=compare(Buffer.from(reference),Buffer.from(actual),PNG);
   const diff=result.diff?Uint8Array.from(result.diff).buffer:null;
   parentPort.postMessage({id,result:{...result,diff}},diff?[diff]:[]);
  }catch{parentPort.postMessage({id,error:'MD-DISPLAY: PNG comparison failed'})}
 });
}

export class MetricsWorker {
 constructor(tools,{WorkerClass=Worker,timeoutMs=30000}={}){
  if(!path.isAbsolute(tools))throw Error('MD-DISPLAY: absolute public tools path required');
  this.timeoutMs=timeoutMs;this.next=0;this.pending=null;this.closed=false;
  this.worker=new WorkerClass(new URL('./drill-metrics-worker.mjs',import.meta.url),{workerData:{tools}});
  this.worker.on('message',message=>{
   const p=this.pending;if(!p||message.id!==p.id)return;
   try{
    const result=message.error?null:{...message.result,diff:message.result.diff?Buffer.from(message.result.diff):undefined};
    clearTimeout(p.timer);this.pending=null;this.worker.unref();
    message.error?p.reject(Error(message.error)):p.resolve(result);
   }catch{this.fail(Error('MD-DISPLAY: malformed PNG worker result'))}
  });
  this.worker.on('error',()=>this.fail(Error('MD-DISPLAY: PNG worker failed')));
  this.worker.on('exit',()=>{if(!this.closed)this.fail(Error('MD-DISPLAY: PNG worker exited'))});
  this.worker.unref();
 }
 fail(error){this.failure=error;const p=this.pending;this.pending=null;if(p){clearTimeout(p.timer);p.reject(error)}this.worker.unref()}
 async compare(reference,actual){
  if(this.closed||this.failure||this.pending)throw this.failure??Error('MD-DISPLAY: PNG worker closed or busy');
  if(!Buffer.isBuffer(reference)||!Buffer.isBuffer(actual)||reference.length>4*1024*1024||actual.length>4*1024*1024)throw Error('MD-DISPLAY: PNG comparison bound');
  // Never transfer a pooled Buffer backing store: it can contain unrelated bytes.
  const a=Uint8Array.from(reference).buffer,b=Uint8Array.from(actual).buffer;
  this.worker.ref();
  return new Promise((resolve,reject)=>{
   const id=++this.next,timer=setTimeout(()=>{this.fail(Error('MD-DISPLAY: PNG worker timeout'));void this.close().catch(()=>{})},this.timeoutMs);
   this.pending={id,timer,resolve,reject};
   try{this.worker.postMessage({id,reference:a,actual:b},[a,b])}catch{this.fail(Error('MD-DISPLAY: PNG worker dispatch failed'))}
  });
 }
 close(){
  if(this.closing)return this.closing;this.closed=true;this.fail(Error('MD-DISPLAY: PNG worker closed'));
  this.closing=Promise.resolve().then(()=>this.worker.terminate()).then(()=>{});return this.closing;
 }
}
