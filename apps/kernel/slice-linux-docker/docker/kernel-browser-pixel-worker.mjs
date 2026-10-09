// MD-DISPLAY-02/04: bounded pixel work outside the host input event loop.
import {Worker,isMainThread,parentPort} from 'node:worker_threads';
import {decodePng,encodePng} from './kernel-browser-pixels.mjs';
import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
import {dirtyTiles} from './kernel-browser-tiles.mjs';
if(!isMainThread){
 parentPort.on('message',({id,kind,data,scale,previous,current,region,all,width,height})=>{
  try{
   let result;
   if(kind==='decode'){result=decodePng(data,scale);const shared=new SharedArrayBuffer(result.pixels.length);new Uint8Array(shared).set(result.pixels);result.pixels=new Uint8Array(shared);}
   else if(kind==='native'){
    if(!Number.isSafeInteger(width)||!Number.isSafeInteger(height)||width<1||height<1||width>geometry.width*2||height>geometry.height*2||data.length!==width*height*4)throw Error('native geometry');
    const shared=new SharedArrayBuffer(data.length),pixels=new Uint8Array(shared);
    for(let i=0;i<data.length;i+=4){pixels[i]=data[i+2];pixels[i+1]=data[i+1];pixels[i+2]=data[i];pixels[i+3]=255;}
    result={width,height,pixels,data_base64:encodePng(width,height,Buffer.from(shared))};
   }
   else if(kind==='tiles')result=dirtyTiles(previous&&{...previous,pixels:Buffer.from(previous.pixels.buffer,previous.pixels.byteOffset,previous.pixels.byteLength)}, {...current,pixels:Buffer.from(current.pixels.buffer,current.pixels.byteOffset,current.pixels.byteLength)},region,all);
   else throw Error('operation');
   parentPort.postMessage({id,result});
  }catch{parentPort.postMessage({id,error:'MD-DISPLAY: bounded pixel work failed'})}
 });
}
export class PixelWorker {
 constructor(){this.worker=null;this.pending=new Map();this.next=0;this.closed=false;}
 run(kind,parameters){
  if(this.closed||this.pending.size>=8)return Promise.reject(Error('MD-DISPLAY: pixel queue unavailable'));
  if(!this.worker){
   this.worker=new Worker(new URL(import.meta.url));
   this.worker.on('message',({id,result,error})=>{const p=this.pending.get(id);if(!p)return;this.pending.delete(id);clearTimeout(p.timer);if(error)p.reject(Error(error));else{if(result?.pixels)result.pixels=Buffer.from(result.pixels.buffer,result.pixels.byteOffset,result.pixels.byteLength);p.resolve(result)}});
   const fail=()=>{for(const p of this.pending.values()){clearTimeout(p.timer);p.reject(Error('MD-DISPLAY: pixel worker exited'))}this.pending.clear();this.closed=true};
   this.worker.on('error',fail);this.worker.on('exit',fail);this.worker.unref();
  }
  return new Promise((resolve,reject)=>{const id=++this.next,timer=setTimeout(()=>{this.pending.delete(id);reject(Error('MD-DISPLAY: pixel worker timeout'));void this.close()},5000);this.pending.set(id,{resolve,reject,timer});this.worker.postMessage({id,kind,...parameters})});
 }
 async close(){this.closed=true;for(const p of this.pending.values()){clearTimeout(p.timer);p.reject(Error('MD-DISPLAY: pixel worker closed'))}this.pending.clear();if(this.worker){const worker=this.worker;this.worker=null;await worker.terminate()}}
}
