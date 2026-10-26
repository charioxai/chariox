// MP-08/MP-10/MP-11: admission precedes receive-to-decode submission telemetry.
import test from 'node:test';import assert from 'node:assert/strict';
import {BrowserDisplayPresenter} from './presenter.mjs';
test('MP-08/MP-10/MP-11 received bytes are timed before worker submission, once per frame',async()=>{
 const saved=globalThis.Worker;globalThis.Worker=class{};const spans=[];
 const decoder={closed:false,close(){},async decode(_frame,data){assert.equal(spans[0]?.stage,'client_receive');assert.deepEqual(data,new Uint8Array([1]));return {image:{close(){}},decodedAt:2}}};
 const binding={subscription_id:'s',generation:1,tab_id:'t'},canvas={width:1280,height:800};
 const presenter=new BrowserDisplayPresenter(canvas,binding,(stage,start,metadata)=>spans.push({stage,start,...metadata}),decoder);
 presenter.videoSink.commit=async()=>({drawn:true,decodedAt:2,committedAt:3});
 const frame={...binding,sequence:1,document_id:'d',kind:'video',codec:'vp8',key:true,width:1280,height:800,css_width:1280,css_height:800,device_scale_factor:1,data:new Uint8Array([1])};
 Object.defineProperty(frame,Symbol.for('chariox.display.receivedAt'),{value:1234});
 try{
  presenter.prepareVideo(frame);presenter.prepareVideo(frame);await presenter.prepared.get(frame);
  const receive=spans.filter(s=>s.stage==='client_receive');assert.equal(receive.length,1);assert.equal(receive[0].start,1234);assert.equal(receive[0].sequence,1);assert(Number.isFinite(receive[0].decodeSubmittedAt));
 }finally{presenter.close();globalThis.Worker=saved}
});
