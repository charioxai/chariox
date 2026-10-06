// MP-08/MP-10/MP-11: loss fails before atomic draw; fresh row keys recover.
import test from 'node:test';import assert from 'node:assert/strict';
import {StripePresenter} from './stripe-presenter.mjs';
import {BrowserDisplayPresenter} from './presenter.mjs';
class Worker {
 postMessage(data){queueMicrotask(()=>this.onmessage({data:{id:data.id,frame:{displayWidth:data.width,displayHeight:data.height,close(){}}}}))}
 terminate(){}
}
const row=(id,key=true,sequence=1)=>({row:id,y:id*100,height:100,codec:'avc1.420033',key,sequence,reference_sequence:key?null:sequence-1,data_base64:'AA=='});
const frame=()=>({kind:'stripes',base_sequence:0,sequence:1,document_id:'d',subscription_id:'s',generation:1,tab_id:'t',width:1280,height:800,css_width:1280,css_height:800,device_scale_factor:1,stripes:Array.from({length:8},(_,id)=>row(id))});
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
