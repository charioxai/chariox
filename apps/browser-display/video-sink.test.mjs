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
test('MP-08/MP-10 hiding between decode and rAF leaves output retirement to the presenter',async()=>{
 const document=new EventTarget();document.hidden=false;let cancelled=0;
 document.defaultView={requestAnimationFrame:()=>1,cancelAnimationFrame:()=>cancelled++};
 const sink=new VideoSink({ownerDocument:document,getContext:()=>({drawImage(){}})},()=>{});let freed=0;
 const job=sink.commit({image:{close(){freed++}},decodedAt:0},{width:1280,height:800});document.hidden=true;document.dispatchEvent(new Event('visibilitychange'));
 assert.equal((await job).drawn,false);assert.equal(cancelled,1);assert.equal(freed,0);sink.close();
});

test('MP-08/MP-10 repair overlay saves and restores the CSS background',()=>{
 const canvas={width:1280,height:800,style:{opacity:'0',background:'var(--surface)'},getContext:()=>({clearRect(){}})};
 const sink=new VideoSink(canvas,()=>{});sink.video={remove(){},style:{}};sink.canvasBackground=canvas.style.background;
 sink.overlay({width:1280,height:800});assert.equal(canvas.style.background,'transparent');
 sink.close();assert.equal(canvas.style.background,'var(--surface)');
});

test('MP-08/MP-10 hidden key retires previous repairs on visible delta resume',async()=>{
 const document=new EventTarget();document.hidden=true;let raf;
 document.defaultView={requestAnimationFrame:cb=>{raf=cb;return 1},cancelAnimationFrame(){}};
 const canvas={width:1280,height:800,style:{opacity:''},ownerDocument:document};
 const sink=new VideoSink(canvas,()=>{});sink.overlayActive=true;
 sink.video={width:1280,height:800,getContext:()=>({drawImage(){}}),remove(){}};
 const frame={width:1280,height:800},image={close(){},clone(){return {close(){}}}};
 assert.equal((await sink.commit(image,{...frame,key:true})).drawn,false);
 document.hidden=false;document.dispatchEvent(new Event('visibilitychange'));
 const resumed=sink.commit(image,{...frame,key:false});raf();assert.equal((await resumed).drawn,true);
 assert.equal(canvas.style.opacity,'0');assert.equal(sink.overlayActive,false);sink.close();
});

for(const key of [false,true])test(`MP-08/MP-10 hidden ${key?'key':'delta'} preserves its base without another visible video`,async()=>{
 const document=new EventTarget();document.hidden=false;let raf;
 document.defaultView={requestAnimationFrame:cb=>{raf=cb;return 1},cancelAnimationFrame(){}};
 const draws=[],canvas={width:1280,height:800,style:{opacity:''},ownerDocument:document};
 const sink=new VideoSink(canvas,()=>{});sink.canvasOpacity='';sink.overlayActive=true;
 sink.video={width:1280,height:800,getContext:()=>({drawImage:image=>draws.push(image.n)}),remove(){}};
 let clones=0,closed=0;
 const image=n=>({n,clone(){clones++;return {n,close(){closed++}}}}),frame={width:1280,height:800};
 const first=sink.commit(image(1),{...frame,key:false});raf();await first;
 const hidden=sink.commit(image(2),{...frame,key});document.hidden=true;document.dispatchEvent(new Event('visibilitychange'));
 assert.equal((await hidden).drawn,false);
 // A second hidden output must replace the retained base, without waiting for rAF.
 assert.equal((await sink.commit(image(3),{...frame,key:false})).drawn,false);
 document.hidden=false;document.dispatchEvent(new Event('visibilitychange'));
 assert.equal(draws.at(-1),3);assert.equal(sink.last.n,3);
 assert.equal(sink.overlayActive,!key);assert.equal(canvas.style.opacity,key?'0':'');
 sink.close();assert.equal(closed,clones);
});

test('MP-08/MP-10 hidden key retires old repairs before new repairs and a visible delta',async()=>{
 const document=new EventTarget();document.hidden=true;let raf,cleared=0;
 document.defaultView={requestAnimationFrame:cb=>{raf=cb;return 1},cancelAnimationFrame(){}};
 const canvas={width:1280,height:800,style:{opacity:''},ownerDocument:document,getContext:()=>({clearRect(){cleared++}})};
 const sink=new VideoSink(canvas,()=>{});sink.canvasOpacity='';sink.overlayActive=true;
 sink.video={width:1280,height:800,getContext:()=>({drawImage(){}}),remove(){}};
 const frame={width:1280,height:800},image={clone(){return {close(){}}}};
 await sink.commit(image,{...frame,key:true});
 sink.overlay(frame);assert.equal(cleared,1);
 document.hidden=false;document.dispatchEvent(new Event('visibilitychange'));
 const delta=sink.commit(image,{...frame,key:false});raf();await delta;
 assert.equal(sink.overlayActive,true);assert.equal(canvas.style.opacity,'');sink.close();
});
