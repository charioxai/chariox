// MP-08/MP-10/MP-11: ready push repairs use the admitted ACK window.
import test from 'node:test';
import assert from 'node:assert/strict';
import {randomBytes} from 'node:crypto';
import {DisplayStream} from './kernel-browser-display.mjs';
import {displayCredit} from './kernel-browser-display-credit.mjs';
import {encodePng} from './kernel-browser-pixels.mjs';

const binding={subscription_id:'s',tab_id:'t',device_scale_factor:1,bitrate:2_000_000,codec:'png'};
function setup(){
 let elapsed=0;
 const sample={generation:1,document_id:'d',width:256,height:256,data_base64:encodePng(256,256,randomBytes(256*256*4))};
 const connection={send:async method=>method==='Page.getFrameTree'?{frameTree:{frame:{id:'target',loaderId:'d'}}}:{cssVisualViewport:{pageX:0,pageY:0,scale:1}}};
 const stream=new DisplayStream(binding,{now:()=>elapsed,wait:async ms=>{elapsed+=ms},encoder:{close:async()=>{}}});
 stream.capture={document:'d',next:async()=>({...sample}),invalidate(){}};
 const host={generation:1,protection:{values:[],targets:[]},compositors:new Map(),inputEpochs:new Map(),inputChangedAt:new Map(),scrolling:new Map(),
  timing(){},displayTarget:async()=>({tab_id:'t',target_id:'target',document_id:'d'}),browser:{resolvePageTarget:async()=>({connection,sessionId:'page'})},compositorFor:async()=>null};
 return {stream,host,sample,elapsed:()=>elapsed};
}

test('MP-08/MP-10 a ready ACK-pump repair does not wait on the legacy byte pacer',async()=>{
 const f=setup();
 try{
  const reply=await displayCredit(f.host,f.stream,{tab_id:'t',generation:1,after_sequence:0,push:{}},new AbortController().signal);
  assert.equal(reply.display_frame.kind,'png');
  assert.equal(f.elapsed(),0,'network delivery and the ACK window must not be preceded by a second transfer delay');
  assert.ok(JSON.stringify(reply.display_frame).length<1024*1024);
  assert.equal('push_delivery' in reply.display_frame,false,'scheduling stays private');
 }finally{await f.stream.close()}
});

test('MP-08/MP-10 legacy credit repairs retain their negotiated byte pacing',async()=>{
 const f=setup();
 try{await displayCredit(f.host,f.stream,{tab_id:'t',generation:1,after_sequence:0},new AbortController().signal);assert.ok(f.elapsed()>500)}
 finally{await f.stream.close()}
});

test('MP-11 a ready push repair cannot publish a retired capture binding',async()=>{
 const f=setup();
 try{assert.equal(await f.stream.frame({...f.sample,push_delivery:true},'d',0,async()=>true,()=>false),null);assert.equal(f.stream.sequence,0)}
 finally{await f.stream.close()}
});

test('MP-11 a push repair rechecks its binding after asynchronous document validation',async()=>{
 const f=setup();let current=true;
 try{
  const frame=await f.stream.frame({...f.sample,push_delivery:true},'d',0,async()=>{current=false;return true},()=>current);
  assert.equal(frame,null);assert.equal(f.stream.sequence,0);
 }finally{await f.stream.close()}
});
