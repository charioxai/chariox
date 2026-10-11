// MP-08/MP-10/MP-11: latest-frame rAF and exact overlays preserve ownership.
import test from 'node:test';import assert from 'node:assert/strict';import {VideoSink} from './video-sink.mjs';
test('MP-08/MP-10 rAF drops to latest within one callback and reports every skipped output',async()=>{
 let raf;const drawn=[],losses=[];
 const canvas={width:1280,height:800,ownerDocument:{defaultView:{requestAnimationFrame:cb=>{raf=cb;return 1},cancelAnimationFrame(){}}},getContext:()=>({drawImage:image=>drawn.push(image.n)})};
 const sink=new VideoSink(canvas,(stage)=>losses.push(stage)),images=[1,2,3].map(n=>({n,close(){}}));
 const jobs=images.map(image=>sink.commit({image,decodedAt:performance.timeOrigin+performance.now()},{width:1280,height:800}));assert.deepEqual(drawn,[]);raf();
 assert.deepEqual(drawn,[3]);assert.deepEqual((await Promise.all(jobs)).map(o=>o.drawn),[false,false,true]);assert.deepEqual(losses,['client_present_drop','client_present_drop']);sink.close();
});
test('MP-08/MP-10 native repairs cover only their pixels and remain through deltas',async()=>{
 const prior=globalThis.OffscreenCanvas;const drawn=[],canvas={n:'overlay',width:2560,height:1600,style:{opacity:'0'},getContext:()=>({drawImage:image=>drawn.push(image.n),clearRect:()=>drawn.push('clear')})};
 globalThis.OffscreenCanvas=class{constructor(){this.n='flattened'}getContext(){return {drawImage:image=>drawn.push(image.n)}}};
 const sink=new VideoSink(canvas,()=>{});sink.track={stop(){}};sink.canvasOpacity='';sink.canvasPosition='';sink.last={n:1,close(){}};
 try{
  sink.overlay({width:2560,height:1600});assert.deepEqual(drawn,['clear']);assert.equal(canvas.style.opacity,'');
  await sink.commit({image:{n:2,close(){},clone(){return {n:2,close(){}}}},decodedAt:0,committedAt:1},{key:false});assert.equal(canvas.style.opacity,'');
  sink.snapshot(canvas.getContext('2d'));assert.deepEqual(drawn,['clear',2,'overlay']);
  await sink.commit({image:{n:3,close(){},clone(){return {n:3,close(){}}}},decodedAt:0,committedAt:1},{key:true});assert.equal(canvas.style.opacity,'0');assert.equal(sink.overlayActive,false);
  sink.close({preserveFrame:true});assert.deepEqual(drawn,['clear',2,'overlay',3,'flattened']);assert.equal(canvas.style.opacity,'');
 }finally{globalThis.OffscreenCanvas=prior;}
});
test('MP-08/MP-10 hiding between decode and rAF retires pending outputs without stalling',async()=>{
 const document=new EventTarget();document.hidden=false;let cancelled=0;
 document.defaultView={requestAnimationFrame:()=>1,cancelAnimationFrame:()=>cancelled++};
 const sink=new VideoSink({ownerDocument:document},()=>{});let freed=0;
 const job=sink.commit({image:{close(){freed++}},decodedAt:0},{width:1280,height:800});document.hidden=true;document.dispatchEvent(new Event('visibilitychange'));
 assert.equal((await job).drawn,false);assert.equal(cancelled,1);assert.equal(freed,1);sink.close();
});
