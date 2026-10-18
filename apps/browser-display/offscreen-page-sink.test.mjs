// vidlat r37: the visible-page crop is drawn in the decoder worker on an OffscreenCanvas; the viewer main thread only composites.
import test from 'node:test';import assert from 'node:assert/strict';import {VideoSink} from './video-sink.mjs';
test('worker draws decoded page frames on the transferred canvas and posts the frame without a track write',async()=>{
 const names=['WorkerGlobalScope','self','VideoDecoder','EncodedVideoChunk'],prior=Object.fromEntries(names.map(k=>[k,globalThis[k]]));
 const posted=[],draws=[],closed=[];
 globalThis.WorkerGlobalScope=class{static [Symbol.hasInstance](v){return v===globalThis}};
 globalThis.self={postMessage:(data,transfer)=>posted.push({data,transfer})};
 globalThis.EncodedVideoChunk=class{constructor(value){Object.assign(this,value)}};
 globalThis.VideoDecoder=class{constructor(callbacks){this.callbacks=callbacks;this.state='configured'}configure(){}close(){this.state='closed'}decode(chunk){queueMicrotask(()=>this.callbacks.output({timestamp:chunk.timestamp,displayWidth:16,displayHeight:8,close(){closed.push('frame')}}))}};
 try{
  await import('./decoder-worker.mjs?offscreen-draw');
  const canvas={width:1,height:1,getContext:(kind,options)=>({drawImage:(frame,x,y,w,h)=>draws.push({kind,options,w,h,canvasWidth:canvas.width,canvasHeight:canvas.height})})};
  self.onmessage({data:{op:'canvas',canvas}});
  self.onmessage({data:{id:1,channel:8,codec:'vp8',key:true,sequence:1,width:16,height:8,cssWidth:16,cssHeight:8,sink:true,bytes:new Uint8Array([1])}});
  await new Promise(r=>setTimeout(r,0));
  assert.deepEqual(draws.map(d=>[d.kind,d.options,d.w,d.h,d.canvasWidth,d.canvasHeight]),[['2d',{desynchronized:true},16,8,16,8]]);
  assert.equal(posted.length,1);assert.equal(posted[0].data.id,1);assert(Number.isFinite(posted[0].data.writtenAt));assert.equal(posted[0].transfer.length,1);assert.deepEqual(closed,[]);
 }finally{for(const [key,value]of Object.entries(prior))if(value===undefined)delete globalThis[key];else Object.defineProperty(globalThis,key,{configurable:true,writable:true,value})}
});
const harness=options=>{
 const mainDraws=[],canvasCalls=[],sinkCalls=[];
 const sibling={style:{},setAttribute(){},remove(){},getContext:(...args)=>{canvasCalls.push(args);return {drawImage:()=>mainDraws.push('sibling')}},transferControlToOffscreen:()=>({offscreen:true})};
 const parent={insertBefore(){}},document={createElement:()=>sibling,defaultView:{getComputedStyle:()=>({position:'relative'})}};
 parent.ownerDocument=document;
 const canvas={width:1280,height:800,style:{},parentElement:parent,ownerDocument:document,getContext:()=>({drawImage:()=>mainDraws.push('main'),clearRect(){}})};
 const decoder={setCanvas:c=>sinkCalls.push(c),setSink:()=>sinkCalls.push('track')};
 return {sink:new VideoSink(canvas,()=>{},options),decoder,mainDraws,canvasCalls,sinkCalls,canvas};
};
test('page sink hands its sibling canvas to the worker and never draws it on the main thread',async()=>{
 const h=harness({direct:true,offscreen:true});h.sink.setup(h.decoder);
 assert.deepEqual(h.sinkCalls,[{offscreen:true}]);assert.deepEqual(h.canvasCalls,[],'a transferred canvas has no main-thread context');
 const timings=[],image={close(){},clone:()=>({close(){}})};h.sink.onTiming=(stage,at,detail)=>timings.push([stage,detail.sequence]);
 const outcome=await h.sink.commit({image,decodedAt:1,writtenAt:2},{sequence:7,width:1280,height:800});
 assert.equal(outcome.drawn,true);assert.deepEqual(h.mainDraws,[]);assert.deepEqual(timings,[['client_present',7]]);
 h.sink.close();
});
test('band and non-offscreen sinks keep the main-thread desynchronized canvas',()=>{
 const h=harness({direct:true});h.sink.setup(h.decoder);
 assert.deepEqual(h.sinkCalls,[]);assert.deepEqual(h.canvasCalls,[['2d',{desynchronized:true}]]);h.sink.close();
});
