import test from 'node:test';
import assert from 'node:assert/strict';
import { attachBrowserDisplay } from './presenter.mjs';

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

test('MD-DISPLAY bounded window sends four credits before any receipt and drains before stop', async () => {
 let listener; const credits=[];
 const transport={onEvent:fn=>{listener=fn;return()=>{}},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s'}:command.op==='display_next'?await new Promise(resolve=>credits.push({command,resolve})):{}}})};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1});
 stream.presenter.present=async frame=>{stream.presenter.sequence=frame.sequence;return true};
 stream.start(); await new Promise(resolve=>setTimeout(resolve,10));
 assert.equal(credits.length,4);
 const stopped=stream.stop();
 for(let i=0;i<4;i++){listener({event:'kernel_browser_frame',subscription_id:'s',frame:{sequence:i+1}});credits[i].resolve({frame_sent:true})}
 await stopped;assert.equal(credits.length,4);assert.equal(stream.presenter.sequence,4);
 await stream.close();
});

test('MD-DISPLAY persistent decoder accepts key/delta and CSS motion pixels; rejects lost delta base',async()=>{
 const {BrowserDisplayPresenter}=await import('./presenter.mjs');
 const oldDecoder=globalThis.VideoDecoder,oldChunk=globalThis.EncodedVideoChunk,oldCanvas=globalThis.OffscreenCanvas;
 let created=0,closed=0;const decoded=[];
 const context={drawImage:()=>{}};
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
   assert.equal(await presenter.present({...frame,key:false,sequence:2}),true);
   assert.equal(created,1);assert.deepEqual(decoded,['key','delta']);
   await assert.rejects(presenter.present({...frame,key:false,sequence:4}),/base lost/);
   assert.equal(await presenter.present({...frame,key:true,sequence:5}),true);
   assert.equal(created,2);assert.equal(canvas.width,2560);
   assert.equal(await presenter.present({...frame,key:false,sequence:6},()=>false),true);
   assert.equal(presenter.didDraw,false);assert.equal(presenter.sequence,6);
   assert.equal(await presenter.present({...frame,key:false,sequence:7}),true);
   assert.equal(presenter.didDraw,true);assert.equal(created,2);
 }finally{presenter.close();globalThis.VideoDecoder=oldDecoder;globalThis.EncodedVideoChunk=oldChunk;globalThis.OffscreenCanvas=oldCanvas}
 assert.equal(closed,2);
});
