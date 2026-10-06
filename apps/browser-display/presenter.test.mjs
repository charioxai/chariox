import test from 'node:test';
import assert from 'node:assert/strict';
import { attachBrowserDisplay, IdleCredit } from './presenter.mjs';
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
  const transport = {
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
 const transport={onEvent:fn=>{listener=fn;return()=>{}},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s'}:command.op==='display_next'?await new Promise(resolve=>credits.push({command,resolve})):{}}})};
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
 const frame={...binding,document_id:'d',kind:'video',codec:'vp09.00.10.08',width:2560,height:1600,css_width:1280,css_height:800,device_scale_factor:2,data_base64:'YWJj'};
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
 const transport={onEvent:fn=>{listener=fn;return()=>{}},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s'}:command.op==='display_next'?await new Promise(resolve=>credits.push(resolve)):{}}})};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1},{codec:'vp09.00.10.08'});
 const common={...stream.binding,document_id:'d',width:2560,height:1600,css_width:1280,css_height:800,device_scale_factor:2};
 try{
  stream.start();assert.equal(credits.length,3);const stopped=stream.stop();
  const frames=[{...common,kind:'video',codec:'vp09.00.10.08',key:true,data_base64:'YWJj',sequence:1},{...common,kind:'tiles',base_sequence:1,tiles:[{x:0,y:0,width:8,height:8,data_base64:'YWJj'}],sequence:2},{...common,kind:'video',codec:'vp09.00.10.08',key:true,data_base64:'YWJj',sequence:3}];
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
 const presenter=new BrowserDisplayPresenter(canvas,binding),frame={...binding,document_id:'d',kind:'video',codec:'avc1.420033',width:1920,height:1080,css_width:1920,css_height:1080,device_scale_factor:1,data_base64:'YWJj',key:true};
 try{assert.equal(await presenter.present({...frame,sequence:1}),true);assert.equal(draws,1);size=[960,540];await assert.rejects(presenter.present({...frame,sequence:2}),/decoded geometry/);assert.equal(draws,1);assert.equal(presenter.sequence,1)}
 finally{presenter.close();globalThis.VideoDecoder=oldDecoder;globalThis.EncodedVideoChunk=oldChunk}
});

test('MP-08/MP-10 #893 default viewer offers DPR1 for a 1080p host',async()=>{
 let command;
 const transport={onEvent:()=>()=>{},request:async request=>{
  if(request.KernelBrowser.command.op==='display_subscribe')command=request.KernelBrowser.command;
  return{KernelBrowser:{result:{subscription_id:'s',generation:1,codec:'png'}}};
 }};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1});
 try{assert.equal(command.device_scale_factor,1)}finally{await stream.close()}
});
