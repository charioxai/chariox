// MP-08/MP-10/MP-11: loss fails before atomic draw; fresh row keys recover.
import test from 'node:test';import assert from 'node:assert/strict';
import {StripePresenter} from './stripe-presenter.mjs';
import {BrowserDisplayPresenter} from './presenter.mjs';
class Worker {
 postMessage(data){queueMicrotask(()=>this.onmessage({data:{id:data.id,frame:{displayWidth:data.width,displayHeight:data.height,close(){}}}}))}
 terminate(){}
}
const row=(id,key=true,sequence=1)=>({row:id,y:id*100,height:100,codec:'avc1.420033',key,sequence,reference_sequence:key?null:sequence-1,data:new Uint8Array([0])});
const frame=()=>({kind:'stripes',base_sequence:0,sequence:1,document_id:'d',subscription_id:'s',generation:1,tab_id:'t',width:1280,height:800,css_width:1280,css_height:800,device_scale_factor:1,stripes:Array.from({length:8},(_,id)=>row(id))});
test('MP-08/MP-10 producer independent stripe recovery establishes a lost outer canvas cursor',async()=>{
 const {DisplayStream}=await import('../kernel/slice-linux-docker/docker/kernel-browser-display.mjs');
 const previous=globalThis.Worker;globalThis.Worker=Worker;const draws=[];
 const canvas={width:1280,height:800,getContext:()=>({drawImage:()=>draws.push(true)})};
 const binding={subscription_id:'s',generation:1,tab_id:'t',codec:'avc1.420033',dependencies:true,stripes:true,device_scale_factor:1,bitrate:8000000};
 const producer=new DisplayStream(binding,{encoder:{close:async()=>{}},now:()=>0,wait:async()=>{}});
 const presenter=new BrowserDisplayPresenter(canvas,binding);
 try{
  assert(await presenter.present(frame()));presenter.sequence=10;producer.sequence=20;
  // The kernel's protocol 466 projection turns base64 segments into raw views.
  const wire=packet=>({...packet,stripes:packet.stripes.map(({data_base64,...row})=>({...row,data:new Uint8Array(Buffer.from(data_base64,'base64'))}))});
  const recovered=wire(await producer.frame({generation:1,motion:true,width:1280,height:800,data_base64:'source',encoded:{stripes:frame().stripes.map(({data,...row})=>({...row,data_base64:'AA=='}))}},'d',10));
  assert.equal(recovered.base_sequence,20);assert.equal(recovered.sequence,21);
  assert(await presenter.present(recovered));assert.equal(presenter.sequence,21);assert.equal(draws.length,16);
  await assert.rejects(presenter.present({...recovered,base_sequence:30,sequence:31,stripes:[row(0)]}),/base lost/);
  await assert.rejects(presenter.present({...recovered,base_sequence:30,sequence:31,stripes:recovered.stripes.map(r=>({...r,y:0}))}),/geometry|base lost/);
  assert.equal(presenter.sequence,21);assert.equal(draws.length,16);
 }finally{presenter.close();await producer.close();globalThis.Worker=previous}
});
test('MP-10 row reference loss draws nothing, row IDR recovers and navigation requires full cover',async()=>{
 const previous=globalThis.Worker;globalThis.Worker=Worker;const draws=[];
 const canvas={width:1280,height:800,getContext:()=>({drawImage:(...args)=>draws.push(args)})};
 const p=new BrowserDisplayPresenter(canvas,{subscription_id:'s',generation:1,tab_id:'t'});
 try{
  const first=frame();assert(await p.present(first));assert.equal(draws.length,8);
  const lost={...first,base_sequence:1,sequence:2,stripes:[row(0,false,3)]};
  await assert.rejects(p.present(lost),/reference lost/);assert.equal(draws.length,8);assert.equal(p.sequence,1);
  const recovery={...lost,stripes:[row(0,true,1)]};assert(await p.present(recovery));assert.equal(draws.length,9);
  await assert.rejects(p.present({...recovery,base_sequence:2,sequence:3,document_id:'next'}),/base lost/);
  assert(await p.present({...first,base_sequence:2,sequence:3,document_id:'next'}));assert.equal(draws.length,17);
 }finally{p.close();globalThis.Worker=previous}
});
test('MP-11 partial bootstrap and overlapping rows reject before decoding',async()=>{
 const p=new StripePresenter();const f=frame();
 await assert.rejects(p.decode({...f,stripes:[row(0)]},()=>new Uint8Array()),/incomplete/);
 await assert.rejects(p.decode({...f,stripes:[row(0),{...row(1),y:0}]},()=>new Uint8Array()),/geometry/);
});

test('MP-08/MP-10 independent rows decode together; failure closes all outputs before any draw',async()=>{
 const previous=globalThis.Worker,workers=[],closed=[];
 class HeldWorker {
  constructor(){workers.push(this)}
  postMessage(data){this.data=data}
  finish(error=false){this.onmessage({data:{id:this.data.id,...(error?{error:'decode failed'}:{frame:{displayWidth:this.data.width,displayHeight:this.data.height,close:()=>closed.push(this)}})}})}
  terminate(){}
 }
 globalThis.Worker=HeldWorker;
 const draws=[],canvas={width:1280,height:800,getContext:()=>({drawImage:()=>draws.push(true)})};
 const p=new BrowserDisplayPresenter(canvas,{subscription_id:'s',generation:1,tab_id:'t'});
 try {
  const pending=p.present(frame());pending.catch(()=>{});
  await new Promise(r=>setImmediate(r));
  const queued=workers.length;
  // Release the sequential base too so a RED assertion leaves no decode timers.
  for(let i=0;i<8;i++){
   while(!workers[i])await new Promise(r=>setImmediate(r));
   workers[i].finish(i===7);await new Promise(r=>setImmediate(r));
  }
  await assert.rejects(pending,/decode failed/);
  assert.equal(queued,8,'independent row chains should not serialize worker round trips');
  assert.equal(draws.length,0);assert.equal(closed.length,7);assert.equal(p.sequence,0);
 } finally {p.close();globalThis.Worker=previous}
});
