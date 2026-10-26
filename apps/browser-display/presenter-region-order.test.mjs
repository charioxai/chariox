// MP-08/MP-10/MP-11: each opaque consumer copies the exact ordered frame.
import test from 'node:test';import assert from 'node:assert/strict';
import {BrowserDisplayPresenter} from './presenter.mjs';
test('MP-08/MP-10/MP-11 detached region decode-ahead cannot paint over an earlier consumer callback',async()=>{
 const Worker=globalThis.Worker;globalThis.Worker=class{};
 let pixel=0,draws=0;const closed=[];
 const image=id=>({id,displayWidth:4,displayHeight:4,close(){closed.push(id)},clone(){return {id,close(){}}}});
 const decoder={closed:false,decode:async f=>({image:image(f.sequence),decodedAt:f.sequence}),close(){this.closed=true}};
 const canvas={width:4,height:4,isConnected:false,ownerDocument:{hidden:false,addEventListener(){},removeEventListener(){}},getContext:()=>({drawImage(i){pixel=i.id;draws++},clearRect(){}})};
 const binding={source_kind:'region',binding_id:'lease',document_id:'d',sequence:7,subscription_id:'s',tab_id:'t',generation:1,css_width:4,css_height:4,device_scale_factor:1};
 const frame=sequence=>({...binding,width:4,height:4,document_id:'d',sequence,kind:'video',codec:'avc1.420033',key:sequence===1,data:new Uint8Array([1])});
 const presenter=new BrowserDisplayPresenter(canvas,binding,()=>{},decoder,{direct:true}),a=frame(1),b=frame(2);
 try{
  presenter.prepareVideo(a);presenter.prepareVideo(b);await Promise.all([presenter.prepared.get(a),presenter.prepared.get(b)]);
  assert.equal(draws,0,'preparation decodes without changing the ordered consumer surface');
  await presenter.present(a);assert.equal(pixel,1,'the first callback must copy frame1, even when frame2 is decoded');
  await presenter.present(b);assert.equal(pixel,2);assert.equal(draws,2);assert.deepEqual(closed,[1,2]);
 }finally{presenter.close();globalThis.Worker=Worker}
});
