// MP-08/MP-10/MP-11: latest-frame rAF and exact overlays preserve ownership.
import test from 'node:test';import assert from 'node:assert/strict';import {VideoSink} from './video-sink.mjs';
test('MP-08/MP-10 rAF drops to latest within one callback and reports every skipped output',async()=>{
 let raf;const drawn=[],losses=[];
 const canvas={width:1280,height:800,ownerDocument:{defaultView:{requestAnimationFrame:cb=>{raf=cb;return 1},cancelAnimationFrame(){}}},getContext:()=>({drawImage:image=>drawn.push(image.n)})};
 const sink=new VideoSink(canvas,(stage)=>losses.push(stage)),images=[1,2,3].map(n=>({n,close(){}}));
 const jobs=images.map(image=>sink.commit({image,decodedAt:performance.timeOrigin+performance.now()},{width:1280,height:800}));assert.deepEqual(drawn,[]);raf();
 assert.deepEqual(drawn,[3]);assert.deepEqual((await Promise.all(jobs)).map(o=>o.drawn),[false,false,true]);assert.deepEqual(losses,['client_present_drop','client_present_drop']);sink.close();
});
test('MP-08/MP-10 native repairs cover only their pixels and retire on the next complete video',async()=>{
 const prior=globalThis.OffscreenCanvas;const drawn=[],canvas={n:'overlay',width:2560,height:1600,style:{opacity:'0'},getContext:()=>({drawImage:image=>drawn.push(image.n),clearRect:()=>drawn.push('clear')})};
 globalThis.OffscreenCanvas=class{constructor(){this.n='flattened'}getContext(){return {drawImage:image=>drawn.push(image.n)}}};
 const sink=new VideoSink(canvas,()=>{});sink.track={stop(){}};sink.canvasOpacity='';sink.canvasPosition='';sink.last={n:1,close(){}};
 try{
  sink.overlay({width:2560,height:1600});assert.deepEqual(drawn,['clear']);assert.equal(canvas.style.opacity,'');
  await sink.commit({image:{n:2,close(){},clone(){return {n:2,close(){}}}},decodedAt:0,writtenAt:1},{key:false});assert.equal(canvas.style.opacity,'0');assert.equal(sink.overlayActive,false);
  sink.snapshot(canvas.getContext('2d'));assert.deepEqual(drawn,['clear','clear',2]);
  await sink.commit({image:{n:3,close(){},clone(){return {n:3,close(){}}}},decodedAt:0,writtenAt:1},{key:true});assert.equal(canvas.style.opacity,'0');assert.equal(sink.overlayActive,false);
  sink.close({preserveFrame:true});assert.deepEqual(drawn,['clear','clear',2,'clear',3,'flattened']);assert.equal(canvas.style.opacity,'');
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
 const canvas={width:1280,height:800,style:{opacity:''},ownerDocument:document,getContext:()=>({clearRect(){}})};
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
 const draws=[],canvas={width:1280,height:800,style:{opacity:''},ownerDocument:document,getContext:()=>({clearRect(){}})};
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
 assert.equal(sink.overlayActive,false);assert.equal(canvas.style.opacity,'0');
 sink.close();assert.equal(closed,clones);
});

test('MP-08/MP-10 hidden key retires old repairs, then a newer complete delta retires the post-key repair',async()=>{
 const document=new EventTarget();document.hidden=true;let raf,cleared=0;
 document.defaultView={requestAnimationFrame:cb=>{raf=cb;return 1},cancelAnimationFrame(){}};
 const canvas={width:1280,height:800,style:{opacity:''},ownerDocument:document,getContext:()=>({clearRect(){cleared++}})};
 const sink=new VideoSink(canvas,()=>{});sink.canvasOpacity='';sink.overlayActive=true;
 sink.video={width:1280,height:800,getContext:()=>({drawImage(){}}),remove(){}};
 const frame={width:1280,height:800},image={clone(){return {close(){}}}};
 await sink.commit(image,{...frame,key:true});
 sink.overlay(frame);assert.equal(cleared,2);
 document.hidden=false;document.dispatchEvent(new Event('visibilitychange'));
 const delta=sink.commit(image,{...frame,key:false});raf();await delta;
 assert.equal(sink.overlayActive,false);assert.equal(canvas.style.opacity,'0');assert.equal(cleared,3);sink.close();
});

// MP-08/MP-10/MP-11: the mirror's driver canvas is disconnected from layout;
// its accepted pixels are copied into the visible region's captureStream.
// Waiting for a display rAF here adds a second compositor boundary per frame.
test('MP-08/MP-10/MP-11 a disconnected region driver commits before the visible-surface animation frame',async()=>{
 const draws=[],canvas={isConnected:false,width:8,height:8,ownerDocument:{hidden:false,defaultView:{requestAnimationFrame(){assert.fail('an intermediate canvas has no presentation frame to wait for')}}},getContext:()=>({drawImage:image=>draws.push(image)})};
 const image={close(){},clone(){return {close(){}}}},sink=new VideoSink(canvas,()=>{});
 try{const outcome=await sink.commit({image,decodedAt:1},{width:8,height:8});assert.equal(outcome.drawn,true);assert.deepEqual(draws,[image]);assert(Number.isFinite(outcome.committedAt));}
 finally{sink.close()}
});
test('MP-08/MP-10/MP-11 hidden disconnected driver keeps its latest base without a visible-paint claim',async()=>{
 const draws=[],canvas={isConnected:false,width:8,height:8,ownerDocument:{hidden:true},getContext:()=>({drawImage:image=>draws.push(image)})};
 const image={close(){},clone(){return {close(){}}}},sink=new VideoSink(canvas,()=>{});
 try{const outcome=await sink.commit({image,decodedAt:1},{width:8,height:8});assert.equal(outcome.drawn,false);assert.deepEqual(draws,[image]);assert.equal(outcome.committedAt,undefined);}
 finally{sink.close()}
});

// MP-08/MP-10/MP-11: sparse native Chrome damage is an interactive image,
// not a paced movie. It must reach the next compositor paint without a media
// jitter buffer or an extra animation-frame boundary.
test('MP-08/MP-10/MP-11 direct Chrome video commits decoded pixels immediately',async()=>{
 const draws=[],canvas={isConnected:true,width:8,height:8,ownerDocument:{hidden:false,defaultView:{requestAnimationFrame(){assert.fail('interactive Chrome pixels must not wait for rAF')}}},getContext:()=>({drawImage:image=>draws.push(image)})};
 const image={close(){},clone(){return {close(){}}}},sink=new VideoSink(canvas,()=>{},{direct:true});
 try{const job=sink.commit({image,decodedAt:1},{width:8,height:8});job.catch(()=>{});assert.deepEqual(draws,[image]);assert.equal((await job).drawn,true);}
 finally{sink.close()}
});
test('MP-08/MP-10/MP-11 direct Chrome mode preserves hidden output without claiming visible paint',async()=>{
 const draws=[],canvas={width:8,height:8,ownerDocument:{hidden:true},getContext:()=>({drawImage:image=>draws.push(image)})};
 const image={close(){},clone(){return {close(){}}}},sink=new VideoSink(canvas,()=>{},{direct:true});
 try{const outcome=await sink.commit({image,decodedAt:1},{width:8,height:8});assert.equal(outcome.drawn,false);assert.equal(outcome.committedAt,undefined);assert.deepEqual(draws,[image]);}
 finally{sink.close()}
});
test('MP-08/MP-10/MP-11 direct Chrome mode bypasses the live media track',()=>{
 const priorGenerator=globalThis.MediaStreamTrackGenerator,priorStream=globalThis.MediaStream;
 let generated=0;const created=[];
 globalThis.MediaStreamTrackGenerator=class{constructor(){generated++;this.writable={}}stop(){}};
 globalThis.MediaStream=class{};
 const parent={style:{},insertBefore(){}},canvas={parentElement:parent,style:{},dataset:{},width:8,height:8,ownerDocument:{createElement(kind){created.push(kind);return {style:{},dataset:{},setAttribute(){},remove(){}}}}};
 const sink=new VideoSink(canvas,()=>{},{direct:true});
 try{assert.equal(sink.setup({setSink(){assert.fail('Chrome UI must not enter a media jitter buffer')}}),false);assert.equal(generated,0);assert.deepEqual(created,['canvas']);}
 finally{sink.close();globalThis.MediaStreamTrackGenerator=priorGenerator;globalThis.MediaStream=priorStream;}
});

test('MP-08/MP-10/MP-11 direct video pixels use the visible low-latency canvas',async()=>{
 const draws=[];let visible;
 const make=name=>{let context;const node={width:8,height:8,style:{},dataset:{},setAttribute(){},remove(){}};
  node.getContext=(_kind,options)=>context??=( {desynchronized:options?.desynchronized===true,clearRect(){},drawImage(){draws.push({name,desynchronized:this.desynchronized})}} );return node;};
 const document={defaultView:{getComputedStyle:()=>({position:'relative',objectFit:'fill'})},createElement:tag=>{assert.equal(tag,'canvas');return visible=make('visible');}};
 const canvas=make('overlay');canvas.ownerDocument=document;canvas.parentElement={style:{},insertBefore(){}};
 const sink=new VideoSink(canvas,()=>{},{direct:true});
 try{sink.setup({});await sink.commit({image:{clone:()=>({close(){}})}},{width:8,height:8});assert(visible);assert.deepEqual(draws,[{name:'visible',desynchronized:true}]);}
 finally{sink.close()}
});
