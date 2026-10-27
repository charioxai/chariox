// MP-08/MP-10/MP-11: accepted geometry is independent of visible paint.
import test from 'node:test';
import assert from 'node:assert/strict';
const {attachBrowserDisplay}=await import(process.env.MD_PRESENTER_SOURCE??'./presenter.mjs');

for(const mode of ['next','window','push'])test(`MP-08/MP-10 ${mode} publishes hidden accepted geometry without a visible presentation`,async()=>{
 const prior={VideoDecoder:globalThis.VideoDecoder,EncodedVideoChunk:globalThis.EncodedVideoChunk};
 globalThis.EncodedVideoChunk=class{constructor(value){Object.assign(this,value)}};
 globalThis.VideoDecoder=class{
  constructor(callbacks){this.callbacks=callbacks}configure(){}close(){}
  decode(){queueMicrotask(()=>this.callbacks.output({displayWidth:1280,displayHeight:720,close(){}}))}
 };
 const document=Object.assign(new EventTarget(),{hidden:true}),canvas={width:1,height:1,ownerDocument:document,getContext:()=>({drawImage(){}})};
 let listener,first=true;const accepted=[],shown=[],ops=[];
 const transport={kernelProtocolVersion:mode==='push'?491:466,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{}},
  ...(mode==='push'?{subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{}}:{}),
  request:async({KernelBrowser:{command}})=>{
   ops.push(command);
   if(command.op==='display_next'&&first){first=false;queueMicrotask(()=>listener({event:'kernel_browser_frame',subscription_id:'s',frame}));return {KernelBrowser:{result:{frame_sent:true}}}}
   return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?{push:'running'}:{frame_sent:false}}};
  }};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1},{codec:'vp8',creditWindow:1,onAccepted:f=>accepted.push(f),onPresented:f=>shown.push(f)});
 const frame={...stream.binding,sequence:1,document_id:'d',kind:'video',codec:'vp8',key:true,width:1920,height:1080,css_width:1920,css_height:1080,device_scale_factor:1,data:new Uint8Array([1])};
 try{
  if(mode==='next')await stream.next();
  else if(mode==='push'){listener({event:'kernel_browser_frame',subscription_id:'s',frame});await stream.next()}
  else{stream.start();while(stream.presenter.sequence!==1&&!stream.error)await new Promise(r=>setTimeout(r,1));await stream.stop()}
  assert.equal(stream.error,null);assert.equal(stream.presenter.sequence,1);
  assert.deepEqual(accepted,[frame]);assert.deepEqual(shown,[]);
  document.hidden=false;document.dispatchEvent(new Event('visibilitychange'));
  assert.deepEqual(accepted,[frame],'quiet return requires no new frame or duplicate acceptance');
  assert.deepEqual(shown,[],'geometry must not count a hidden output as visibly presented');
 }finally{await stream.close();Object.assign(globalThis,prior)}
});
