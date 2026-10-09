import test from 'node:test';
import assert from 'node:assert/strict';
import { attachBrowserDisplay, IdleCredit } from './presenter.mjs';
test('MP-08/MP-10/MP-11 display requires protocol466 and a binary event decoder before subscribing',async()=>{
 for(const capability of [{},{kernelProtocolVersion:465,displayEventEncoding:'CXD1'},{kernelProtocolVersion:466},{kernelProtocolVersion:466,displayEventEncoding:'JSON'}]) {
  let requests=0;
  const transport={...capability,request:async()=>{requests++;return {KernelBrowser:{result:{}}}},onEvent:()=>()=>{}};
  await assert.rejects(attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1}),/binary display adapter/);
  assert.equal(requests,0,'incompatible adapters cannot start a display');
 }
});
test('MP-08/MP-10 empty local credits bound retries to eight milliseconds and input wakes them',async()=>{
 const idle=new IdleCredit();idle.observeRoundTrip(2);idle.wake();const wait=idle.wait();
 try{assert.equal(idle.delay,8)}finally{idle.wake();await wait}
 assert.equal(idle.parked.size,0);
});
test('MD-DISPLAY idle credits back off independently and input wakes all parked slots',async()=>{
 const idle=new IdleCredit();const waits=[idle.wait(),idle.wait(),idle.wait(),idle.wait()];
 assert.equal(idle.delay,100);assert.equal(idle.parked.size,4);
 idle.wake();await Promise.all(waits);assert.equal(idle.delay,0);assert.equal(idle.parked.size,0);
 const active=idle.wait();assert.ok(idle.delay<=8,'recent motion keeps the capture window responsive');idle.wake();await active;
});
test('MD-DISPLAY recently active empty credits retain a sub-frame retry cadence',async()=>{
 const idle=new IdleCredit();idle.observeRoundTrip?.(10);idle.wake();
 const waits=Array.from({length:5},()=>idle.wait());
 try{assert.ok(idle.delay<=8,'active retries must not add another32ms frame after native damage');}
 finally{idle.wake();await Promise.all(waits)}
});
test('MD-DISPLAY long credits retain WAN backoff instead of rapid empty polling',async()=>{
 const idle=new IdleCredit();idle.observeRoundTrip?.(100);idle.wake();
 const waits=Array.from({length:5},()=>idle.wait());
 try{assert.equal(idle.delay,33,'slow credited paths must retain bounded WAN backoff');}
 finally{idle.wake();await Promise.all(waits)}
});

test('MD-DISPLAY credit stays occupied through event-before-receipt and presentation', async () => {
  let listener, finishRequest, finishPresentation, presentationStarted;
  const receipt = new Promise(resolve => { finishRequest = resolve; });
  const presenting = new Promise(resolve => { presentationStarted = resolve; });
  const presentation = new Promise(resolve => { finishPresentation = resolve; });
  let calls = 0;
  const transport = {kernelProtocolVersion:466,displayEventEncoding:'CXD1',
    onEvent: callback => { listener = callback; return () => {}; },
    request: async ({ KernelBrowser: { command } }) => ({ KernelBrowser: { result:
      command.op === 'display_subscribe' ? { subscription_id: 's' } :
      command.op === 'display_next' && ++calls === 1 ? await receipt : { frame_sent: false },
    } }),
  };
  const stream = await attachBrowserDisplay({ width: 1, height: 1 }, transport, { tab_id: 't', generation: 1 });
  stream.presenter.present = async () => { presentationStarted(); await presentation; return true; };
  const first = stream.next();
  try {
    listener({ event: 'kernel_browser_frame', subscription_id: 's', frame: { sequence: 1 } });
    await assert.rejects(stream.next(), /credit outstanding/);
    await Promise.race([presenting,new Promise((_,reject)=>setTimeout(()=>reject(Error('MD-DISPLAY: receipt stalls presentation')),100))]);
    finishRequest({ frame_sent: true });
    await assert.rejects(stream.next(), /credit outstanding/);
  } finally {
    finishRequest({ frame_sent: true }); finishPresentation();
    await first; await stream.close();
  }
});

test('MD-DISPLAY bounded window sends three credits before any receipt and drains before stop', async () => {
 let listener; const credits=[];
 const transport={kernelProtocolVersion:466,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{}},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s'}:command.op==='display_next'?await new Promise(resolve=>credits.push({command,resolve})):{}}})};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1});
 stream.presenter.present=async frame=>{stream.presenter.sequence=frame.sequence;return true};
 stream.start(); await new Promise(resolve=>setTimeout(resolve,10));
 assert.equal(credits.length,3);
 const stopped=stream.stop();
 for(const i of [2,0,1]){listener({event:'kernel_browser_frame',subscription_id:'s',frame:{sequence:i+1}});credits[i].resolve({frame_sent:true})}
 await stopped;assert.equal(credits.length,3);assert.equal(stream.presenter.sequence,3);
 await stream.close();
});

test('MD-DISPLAY persistent decoder accepts key/delta and CSS motion pixels; rejects lost delta base',async()=>{
 const {BrowserDisplayPresenter}=await import('./presenter.mjs');
 const oldDecoder=globalThis.VideoDecoder,oldChunk=globalThis.EncodedVideoChunk,oldCanvas=globalThis.OffscreenCanvas;
 let created=0,closed=0;const decoded=[];
 let draws=0;const context={drawImage:()=>{draws++}};
 globalThis.OffscreenCanvas=class{getContext(){return context}};
 globalThis.EncodedVideoChunk=class{constructor(value){Object.assign(this,value)}};
 globalThis.VideoDecoder=class{
   constructor(callbacks){this.callbacks=callbacks;this.state='configured';created++} configure(){}
   decode(chunk){decoded.push(chunk.type);queueMicrotask(()=>this.callbacks.output({displayWidth:1280,displayHeight:800,close:()=>{}}))}
   close(){this.state='closed';closed++}
 };
 const binding={subscription_id:'s',generation:1,tab_id:'t'},canvas={width:1,height:1,getContext:()=>context};
 const presenter=new BrowserDisplayPresenter(canvas,binding);
 const frame={...binding,document_id:'d',kind:'video',codec:'vp09.00.10.08',width:2560,height:1600,css_width:1280,css_height:800,device_scale_factor:2,data:new Uint8Array([97,98,99])};
 try{
   assert.equal(await presenter.present({...frame,key:true,sequence:1}),true);
   assert.equal(draws,1,'a validated full video frame commits atomically with one canvas draw');
   assert.equal(await presenter.present({...frame,key:false,sequence:2}),true);
   assert.equal(created,1);assert.deepEqual(decoded,['key','delta']);
   await assert.rejects(presenter.present({...frame,key:false,sequence:4}),/base lost/);
   assert.equal(await presenter.present({...frame,key:true,sequence:5}),true);
   assert.equal(created,2);assert.equal(canvas.width,2560);
   assert.equal(await presenter.present({...frame,key:false,sequence:6}),true);
   assert.equal(draws,4,'every accepted video commits its potential tile base');
   assert.equal(presenter.didDraw,true);assert.equal(presenter.sequence,6);
   assert.equal(await presenter.present({...frame,key:false,sequence:7}),true);
   assert.equal(await presenter.present({...frame,kind:'tiles',base_sequence:7,tiles:[],sequence:8}),true);
   assert.equal(await presenter.present({...frame,key:false,sequence:9}),true,'exact tiles retain the delivered decoder reference');
   assert.equal(presenter.didDraw,true);assert.equal(created,2);
 }finally{presenter.close();globalThis.VideoDecoder=oldDecoder;globalThis.EncodedVideoChunk=oldChunk;globalThis.OffscreenCanvas=oldCanvas}
 assert.equal(closed,2);
});
test('MD-DISPLAY native-DPR clients prefer a VP9 level that admits native geometry',async()=>{const prior=globalThis.VideoDecoder;globalThis.VideoDecoder=class{static async isConfigSupported({codec}){return {supported:['vp09.00.50.08','vp09.00.40.08'].includes(codec)}}};try{const {supportedCodecs}=await import('./presenter.mjs');assert.deepEqual(await supportedCodecs(),['vp09.00.50.08','vp09.00.40.08','png','chariox-video-dependencies-v1'])}finally{globalThis.VideoDecoder=prior}});

for (const fresh of [true, false]) test(`MD-DISPLAY-04 credited video/tile/video preserves ${fresh?'fresh':'existing'} canvas base`, async () => {
 const prior={VideoDecoder:globalThis.VideoDecoder,EncodedVideoChunk:globalThis.EncodedVideoChunk,createImageBitmap:globalThis.createImageBitmap};
 let listener,closed=0;const credits=[],draws=[];let raster=fresh?null:'older';
 const context={drawImage(image){if(image.patch){assert.equal(raster,'video-1','tiles must commit over their actual accepted video base');draws.push('tiles')}else{raster=image.raster;draws.push(raster)}}};
 const canvas={width:fresh?1:2560,height:fresh?1:1600,getContext:()=>context};
 globalThis.EncodedVideoChunk=class{constructor(value){Object.assign(this,value)}};
 globalThis.VideoDecoder=class{
  constructor(callbacks){this.callbacks=callbacks} configure(){} close(){}
  decode(chunk){queueMicrotask(()=>this.callbacks.output({displayWidth:1280,displayHeight:800,raster:'video-'+chunk.timestamp/33333,close(){closed++}}))}
 };
 globalThis.createImageBitmap=async()=>({width:8,height:8,patch:true,close(){}});
 const transport={kernelProtocolVersion:466,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{}},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s'}:command.op==='display_next'?await new Promise(resolve=>credits.push(resolve)):{}}})};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1},{codec:'vp09.00.10.08'});
 const common={...stream.binding,document_id:'d',width:2560,height:1600,css_width:1280,css_height:800,device_scale_factor:2};
 try{
  stream.start();assert.equal(credits.length,3);const stopped=stream.stop();
  const frames=[{...common,kind:'video',codec:'vp09.00.10.08',key:true,data:new Uint8Array([97,98,99]),sequence:1},{...common,kind:'tiles',base_sequence:1,tiles:[{x:0,y:0,width:8,height:8,format:'png',data:new Uint8Array([97,98,99])}],sequence:2},{...common,kind:'video',codec:'vp09.00.10.08',key:true,data:new Uint8Array([97,98,99]),sequence:3}];
  // Last video is already queued before the first decoder callback completes.
  for(const frame of frames)listener({event:'kernel_browser_frame',subscription_id:'s',frame});
  for(const resolve of credits)resolve({frame_sent:true});
  await stopped;
  assert.deepEqual(draws,['video-1','tiles','video-3']);assert.equal(stream.presenter.sequence,3);assert.equal(raster,'video-3');assert.equal(closed,2);
 }finally{await stream.close();Object.assign(globalThis,prior)}
});

// MP-08/MP-10: only the explicitly bounded motion raster can be upscaled.
test('MP-08/MP-10 1080p motion admits 720p video while rejecting arbitrary decoded dimensions',async()=>{
 const {BrowserDisplayPresenter}=await import('./presenter.mjs');
 const oldDecoder=globalThis.VideoDecoder,oldChunk=globalThis.EncodedVideoChunk;
 let size=[1280,720],draws=0;
 globalThis.EncodedVideoChunk=class{constructor(v){Object.assign(this,v)}};
 globalThis.VideoDecoder=class{constructor(c){this.c=c;this.state='configured'}configure(){}close(){}decode(){queueMicrotask(()=>this.c.output({displayWidth:size[0],displayHeight:size[1],close(){}}))}};
 const binding={subscription_id:'s',generation:1,tab_id:'t'},canvas={width:1920,height:1080,getContext:()=>({drawImage(){draws++}})};
 const presenter=new BrowserDisplayPresenter(canvas,binding),frame={...binding,document_id:'d',kind:'video',codec:'avc1.420033',width:1920,height:1080,css_width:1920,css_height:1080,device_scale_factor:1,data:new Uint8Array([97,98,99]),key:true};
 try{assert.equal(await presenter.present({...frame,sequence:1}),true);assert.equal(draws,1);size=[960,540];await assert.rejects(presenter.present({...frame,sequence:2}),/decoded geometry/);assert.equal(draws,1);assert.equal(presenter.sequence,1)}
 finally{presenter.close();globalThis.VideoDecoder=oldDecoder;globalThis.EncodedVideoChunk=oldChunk}
});

test('MP-08/MP-10 #893 default viewer offers DPR1 for a 1080p host',async()=>{
 let command;
 const transport={kernelProtocolVersion:466,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async request=>{
  if(request.KernelBrowser.command.op==='display_subscribe')command=request.KernelBrowser.command;
  return{KernelBrowser:{result:{subscription_id:'s',generation:1,codec:'png'}}};
 }};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1});
 try{assert.equal(command.device_scale_factor,1)}finally{await stream.close()}
});

// MP-08/MP-10: protocol 466 lossless scroll: moves read one canvas snapshot,
// then WebP/PNG residuals; invalid moves are refused before any draw.
test('MP-08/MP-10 lossless scroll frame copies a snapshot then draws WebP residuals',async()=>{
 const {BrowserDisplayPresenter}=await import('./presenter.mjs');
 const prior={OffscreenCanvas:globalThis.OffscreenCanvas,createImageBitmap:globalThis.createImageBitmap};
 const draws=[],types=[];
 globalThis.OffscreenCanvas=class{constructor(w,h){this.width=w;this.height=h;this.name='scratch'}getContext(){return {drawImage:(...a)=>draws.push(['scratch',a[0].name??'bitmap',...a.slice(1)])}}};
 globalThis.createImageBitmap=async blob=>{types.push(blob.type);return {width:1920,height:20,close(){}}};
 const canvas={name:'canvas',width:1920,height:1080,getContext:()=>({drawImage:(...a)=>draws.push(['canvas',a[0].name??'bitmap',...a.slice(1)])})};
 const binding={subscription_id:'s',generation:1,tab_id:'t'};
 const presenter=new BrowserDisplayPresenter(canvas,binding);presenter.sequence=4;presenter.documentId='d';
 const frame={...binding,document_id:'d',kind:'tiles',base_sequence:4,sequence:5,width:1920,height:1080,css_width:1920,css_height:1080,device_scale_factor:1,
  moves:[[0,0,1920,1060,-20]],tiles:[{x:0,y:1060,width:1920,height:20,format:'webp',data:new Uint8Array([1])}]};
 try{
  await assert.rejects(presenter.present({...frame,moves:[[0,0,1920,1070,-20]]}),/move geometry/);
  await assert.rejects(presenter.present({...frame,tiles:[{...frame.tiles[0],format:'gif'}]}),/tile geometry/);
  assert.equal(draws.length,0);
  assert.equal(await presenter.present(frame),true);
  assert.deepEqual(types,['image/webp']);
  assert.deepEqual(draws,[['scratch','canvas',0,0],['canvas','scratch',0,20,1920,1060,0,0,1920,1060],['canvas','bitmap',0,1060]]);
  assert.equal(presenter.sequence,5);
 }finally{presenter.close();Object.assign(globalThis,prior)}
});

test('MP-08/MP-11 desktop subscription binds native identity and DPR geometry in the shared viewer',async()=>{
 const commands=[],target={surface_id:'desktop-one',generation:'native-one'};
 const transport={kernelProtocolVersion:474,relayProtocolVersion:96,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>{
  commands.push(command);return {KernelBrowser:{result:{subscription_id:'s',generation:4,device_scale_factor:2,source:{kind:'desktop',...target,width:1280,height:800}}}};
 }};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,target,{desktop:true,codec:'avc1.420033',deviceScaleFactor:2});
 try{
  assert.equal(commands[0].op,'computer');assert.equal(commands[0].command.op,'display_subscribe');
  assert.deepEqual(commands[0].command.target,target);assert.equal(stream.binding.tab_id,target.surface_id);
  stream.presenter.documentId=target.generation;
  assert.deepEqual(stream.presenter.input({kind:'click',x:12,y:23}),{op:'computer',command:{op:'input',target,input:{kind:'click',x:24,y:46,button:1}}});
  await stream.takeover();assert.equal(commands.at(-1).command.op,'takeover');
 }finally{await stream.close()}
});

test('MP-11 desktop frame geometry is exact to the admitted physical surface at DPR 2',async()=>{
 const {BrowserDisplayPresenter}=await import('./presenter.mjs');
 const oldCanvas=globalThis.OffscreenCanvas,oldBitmap=globalThis.createImageBitmap;
 const context={drawImage(){}};globalThis.OffscreenCanvas=class{getContext(){return context}};
 globalThis.createImageBitmap=async()=>({width:1280,height:800,close(){}});
 const source={kind:'desktop',surface_id:'desk',generation:'native-one',width:1280,height:800};
 const binding={subscription_id:'s',generation:4,tab_id:'desk',device_scale_factor:2,source};
 const presenter=new BrowserDisplayPresenter({width:1,height:1,getContext:()=>context},binding);
 const frame={subscription_id:'s',generation:4,tab_id:'desk',sequence:1,document_id:'native-one',kind:'png',data:new Uint8Array([1]),width:1280,height:800,css_width:640,css_height:400,device_scale_factor:2};
 try{
  assert.equal(await presenter.present(frame),true);
  await assert.rejects(presenter.present({...frame,sequence:2,document_id:'stale'}),/geometry\/binding/);
  await assert.rejects(presenter.present({...frame,sequence:2,width:2560,height:1600,css_width:1280,css_height:800}),/geometry\/binding/);
 }finally{presenter.close();globalThis.OffscreenCanvas=oldCanvas;globalThis.createImageBitmap=oldBitmap}
});

test('MP-11 close detaches and unsubscribes before parked credits settle',async()=>{
 let listener;const ops=[];let detached=0;
 const transport={kernelProtocolVersion:466,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{detached++}},request:async({KernelBrowser:{command}})=>{ops.push(command.op);return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s'}:command.op==='display_next'?{frame_sent:true}:{}}}}};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1});
 stream.start();await new Promise(resolve=>setTimeout(resolve,10));
 // Every credit was sent but its window event never arrives (dead lane).
 const closing=stream.close();await new Promise(resolve=>setTimeout(resolve,20));
 assert.equal(detached,1);assert.equal(stream.presenter.closed,true);assert(ops.includes('unsubscribe'),'close cannot wait for the window timeout');
 await closing;
});
test('MP-11 an errored main-thread decoder is replaced without a second close',async()=>{
 const {BrowserDisplayPresenter}=await import('./presenter.mjs');
 const prior={VideoDecoder:globalThis.VideoDecoder,EncodedVideoChunk:globalThis.EncodedVideoChunk};
 let fail=true;const context={drawImage(){}};
 globalThis.EncodedVideoChunk=class{constructor(value){Object.assign(this,value)}};
 globalThis.VideoDecoder=class{constructor(c){this.c=c;this.state='configured'}configure(){}
  close(){if(this.state==='closed')throw Error('InvalidStateError');this.state='closed'}
  decode(){queueMicrotask(()=>{if(fail){this.state='closed';this.c.error(Error('synthetic decode error'))}else this.c.output({displayWidth:1280,displayHeight:800,close(){}})})}};
 const binding={subscription_id:'s',generation:1,tab_id:'t'},presenter=new BrowserDisplayPresenter({width:1,height:1,getContext:()=>context},binding);
 const frame={...binding,document_id:'d',kind:'video',codec:'vp09.00.10.08',key:true,width:1280,height:800,css_width:1280,css_height:800,device_scale_factor:1,data:new Uint8Array([1])};
 try{
  await assert.rejects(presenter.present({...frame,sequence:1}),/synthetic decode error/);
  fail=false;assert.equal(await presenter.present({...frame,sequence:2}),true);
  fail=true;await assert.rejects(presenter.present({...frame,sequence:3}),/synthetic decode error/);
  presenter.close();
 }finally{Object.assign(globalThis,prior)}
});
// MP-08/MP-10: protocol 475 push. No credits; one ack per presented frame,
// reordered events decode in sequence, lost frames request a key.
test('MP-08/MP-10 push display acknowledges each presented frame in sequence without credits',async()=>{
 let listener;const ops=[];
 const transport={kernelProtocolVersion:475,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{}},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},
  request:async({KernelBrowser:{command}})=>{ops.push(command);return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?{acknowledged:true,push:'running'}:{}}}}};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1},{idleMs:20});
 const presented=[];stream.presenter.present=async frame=>{presented.push(frame.sequence);stream.presenter.sequence=frame.sequence;return true};
 try{
  assert.ok(stream.push);assert.deepEqual(ops.filter(c=>c.op==='display_ack').map(c=>[c.sequence,c.lost]),[[0,false]],'the first ack starts the kernel pump');
  for(const sequence of [2,1,3])listener({event:'kernel_browser_frame',subscription_id:'s',frame:{sequence}});
  assert.equal((await stream.next()).sequence,1);assert.equal((await stream.next()).sequence,2);assert.equal((await stream.next()).sequence,3);
  assert.equal(await stream.next(),null,'a quiet stream reads as settled');
  assert.deepEqual(presented,[1,2,3]);
  assert.deepEqual(ops.filter(c=>c.op==='display_ack').map(c=>c.sequence),[0,1,2,3]);
  assert.equal(ops.filter(c=>c.op==='display_next').length,0,'push mode issues no credits');
  stream.requestKey();assert.deepEqual(ops.at(-1),{op:'display_ack',subscription_id:'s',generation:1,sequence:3,lost:true});
 }finally{await stream.close()}
});
test('MP-08/MP-10 push display reports a stopped kernel pump as a stream error',async()=>{
 const transport={kernelProtocolVersion:475,displayEventEncoding:'CXD1',onEvent:()=>()=>{},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},
  request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?{acknowledged:true,push:'failed'}:{}}})};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1},{idleMs:20});
 try{await new Promise(r=>setTimeout(r,5));await assert.rejects(stream.next(),/push pump failed/);}finally{await stream.close()}
});
test('MP-08/MP-10 credit transports without a display subscription keep display_next',async()=>{
 const ops=[];const transport={kernelProtocolVersion:475,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>{ops.push(command.op);return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s'}:{frame_sent:false}}}}};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1});
 try{assert.equal(stream.push,undefined);assert.equal(await stream.next(),null);assert.deepEqual(ops,['display_subscribe','display_next'])}finally{await stream.close()}
});
// MP-08/MP-10 (1.5): recover in place; never fall back to images.
function pushFixture(){
 let listener;const ops=[];
 const transport={kernelProtocolVersion:475,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{}},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},
  request:async({KernelBrowser:{command}})=>{ops.push(command);return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?{acknowledged:true,push:'running'}:{}}}}};
 return {transport,ops,emit:frame=>listener({event:'kernel_browser_frame',subscription_id:'s',frame})};
}
test('MP-08/MP-10 push display recovers a lost base in place with one key request',async()=>{
 const f=pushFixture();const stream=await attachBrowserDisplay({width:1,height:1},f.transport,{tab_id:'t',generation:1},{idleMs:20});
 const shown=[];stream.presenter.present=async frame=>{if(frame.fail)throw Error('MD-DISPLAY: video base lost; subscribe afresh');shown.push(frame.sequence);stream.presenter.sequence=frame.sequence;return true};
 try{
  f.emit({sequence:1,kind:'video',key:true});f.emit({sequence:2,kind:'video',key:false,fail:true});f.emit({sequence:3,kind:'video',key:false});f.emit({sequence:4,kind:'tiles'});f.emit({sequence:5,kind:'video',key:true});f.emit({sequence:6,kind:'video',key:false});
  for(let i=0;i<4;i++)await stream.next();
  assert.deepEqual(shown,[1,5,6],'dependent frames after a lost base are skipped until the key');
  const acks=f.ops.filter(c=>c.op==='display_ack');
  assert.deepEqual(acks.filter(c=>c.lost).map(c=>c.sequence),[1],'one rate-limited key request, from the last good base');
  assert.deepEqual(acks.filter(c=>!c.lost).map(c=>c.sequence),[0,1,2,3,4,5,6],'skipped frames are still acknowledged so the gate stays open');
  assert.equal(stream.error,null);
 }finally{await stream.close()}
});
test('MP-08/MP-10 push display skips a receive gap and fails only on binding mismatch',async()=>{
 const f=pushFixture();const stream=await attachBrowserDisplay({width:1,height:1},f.transport,{tab_id:'t',generation:1},{idleMs:20});
 const shown=[];stream.presenter.present=async frame=>{if(frame.foreign)throw Error('MD-DISPLAY: invalid geometry/binding');shown.push(frame.sequence);stream.presenter.sequence=frame.sequence;return true};
 try{
  for(let n=2;n<=12;n++)f.emit({sequence:n,kind:n===11?'png':'tiles'});
  for(let i=0;i<2;i++)await stream.next();assert.deepEqual(shown,[11,12],'missing sequence 1 is skipped and recovery waits for an independent frame');
  assert.ok(f.ops.some(c=>c.op==='display_ack'&&c.lost));
  f.emit({sequence:13,kind:'png',foreign:true});await assert.rejects(async()=>{for(let i=0;i<10;i++)await stream.next()},/invalid geometry\/binding/);
 }finally{await stream.close()}
});
// MP-08/MP-10: frames pushed into a dropped relay socket never arrive. On a
// quiet page the viewer must not wait for eight more frames to skip the gap.
test('MP-08/MP-10 push display presents an independent frame across a lost-frame gap at once',async()=>{
 const f=pushFixture();const stream=await attachBrowserDisplay({width:1,height:1},f.transport,{tab_id:'t',generation:1},{idleMs:20});
 const shown=[];stream.presenter.present=async frame=>{shown.push(frame.sequence);stream.presenter.sequence=frame.sequence;return true};
 try{
  f.emit({sequence:1,kind:'video',key:true});await stream.next();
  f.emit({sequence:4,kind:'video',key:true});f.emit({sequence:5,kind:'video',key:false});
  await stream.next();await stream.next();
  assert.deepEqual(shown,[1,4,5],'lost 2 and 3 are not waited for');
  f.emit({sequence:3,kind:'tiles'});assert.equal(await stream.next(),null,'a late frame below the key is dropped');
 }finally{await stream.close()}
});
test('MP-08/MP-10 push display skips a gap that stays open and asks for a key',async()=>{
 const f=pushFixture();const stream=await attachBrowserDisplay({width:1,height:1},f.transport,{tab_id:'t',generation:1},{idleMs:20,heartbeatMs:30});
 const shown=[];stream.presenter.present=async frame=>{shown.push(frame.sequence);stream.presenter.sequence=frame.sequence;return true};
 try{
  f.emit({sequence:1,kind:'video',key:true});await stream.next();
  f.emit({sequence:3,kind:'video',key:false});
  await new Promise(r=>setTimeout(r,120));
  assert.ok(f.ops.some(c=>c.op==='display_ack'&&c.lost&&c.sequence===1),'a key is requested from the last good base');
  f.emit({sequence:4,kind:'video',key:true});await stream.next();
  assert.deepEqual(shown,[1,4],'the dependent frame after the gap is skipped');
 }finally{await stream.close()}
});
// MP-08/MP-10 #933 review @215371f67: a lost sequence followed by only a few
// frames (the kernel ACK gate stops sending) must not wait for queue overflow.
test('MP-08/MP-10 push display bounds a receive gap in time, requests a key and resumes at the independent frame',async()=>{
 const f=pushFixture();const stream=await attachBrowserDisplay({width:1,height:1},f.transport,{tab_id:'t',generation:1},{idleMs:20,gapMs:60});
 const shown=[];stream.presenter.present=async frame=>{shown.push(frame.sequence);stream.presenter.sequence=frame.sequence;return true};
 try{
  f.emit({sequence:2,kind:'tiles'});f.emit({sequence:3,kind:'tiles'});
  await new Promise(resolve=>setTimeout(resolve,30));assert.deepEqual(shown,[],'a short reorder window still waits');
  await new Promise(resolve=>setTimeout(resolve,80));
  assert.ok(f.ops.some(c=>c.op==='display_ack'&&c.lost),'the gap deadline requests an independent frame');
  assert.deepEqual(shown,[],'dependent frames after the gap are not presented');
  f.emit({sequence:4,kind:'video',key:true});f.emit({sequence:5,kind:'video',key:false});
  await stream.next();await stream.next();assert.deepEqual(shown,[4,5]);
  // Once recovery started, an independent frame behind a new gap establishes the cursor at once.
  f.emit({sequence:7,kind:'tiles'});await new Promise(resolve=>setTimeout(resolve,100));assert.deepEqual(shown,[4,5]);
  const started=Date.now();f.emit({sequence:9,kind:'png'});
  await stream.next();assert.deepEqual(shown,[4,5,9]);assert(Date.now()-started<50,'no second gap deadline');
 }finally{await stream.close()}
});
test('MP-08/MP-10 a stopped push display neither presents nor acknowledges, and restarts from an independent frame',async()=>{
 const f=pushFixture();const stream=await attachBrowserDisplay({width:1,height:1},f.transport,{tab_id:'t',generation:1},{idleMs:20,heartbeatMs:20});
 const shown=[];stream.presenter.present=async frame=>{shown.push(frame.sequence);stream.presenter.sequence=frame.sequence;return true};
 try{
  stream.start();f.emit({sequence:1,kind:'video',key:true});await stream.next();
  await stream.stop();const acks=f.ops.filter(c=>c.op==='display_ack').length;
  f.emit({sequence:2,kind:'video',key:false});await new Promise(resolve=>setTimeout(resolve,80));
  assert.deepEqual(shown,[1]);assert.equal(f.ops.filter(c=>c.op==='display_ack').length,acks,'no acknowledgements or heartbeats while stopped');
  stream.start();assert.ok(f.ops.at(-1).op==='display_ack'&&f.ops.at(-1).lost,'restart asks for an independent frame');
  f.emit({sequence:3,kind:'video',key:false});f.emit({sequence:4,kind:'video',key:true});
  await stream.next();assert.deepEqual(shown,[1,4]);
 }finally{await stream.close()}
});
