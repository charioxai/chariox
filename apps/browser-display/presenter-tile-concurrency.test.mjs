// MP-08/MP-10/MP-11: bounded parallel decode, atomic validated commit and cleanup.
import test from 'node:test';import assert from 'node:assert/strict';
const {BrowserDisplayPresenter}=await import(process.env.MD_PRESENTER_SOURCE??'./presenter.mjs');
test('MP-08/MP-10/MP-11 tile decoding overlaps four jobs and drains failures before cleanup',async()=>{
 const old=globalThis.createImageBitmap;
 for(const fail of [false,true]){
  const pending=[],drawn=[],closed=[];let active=0,peak=0,started=0;
  const canvas={width:1280,height:800,getContext:()=>({drawImage:image=>drawn.push(image.id)})};
  const p=new BrowserDisplayPresenter(canvas,{subscription_id:'s',tab_id:'t',generation:1});p.documentId='doc';p.sequence=1;
  globalThis.createImageBitmap=()=>{const id=started++;active++;peak=Math.max(peak,active);return new Promise((resolve,reject)=>pending.push(()=>{active--;if(fail&&id===1)reject(Error('decode failed'));else resolve({id,width:1,height:1,close(){closed.push(id)}})}))};
  const frame={subscription_id:'s',tab_id:'t',generation:1,sequence:2,base_sequence:1,document_id:'doc',kind:'tiles',device_scale_factor:1,width:1280,height:800,css_width:1280,css_height:800,tiles:Array.from({length:8},(_,x)=>({x,y:0,width:1,height:1,format:'png',data:new Uint8Array([1])}))};
  const result=p.present(frame);result.catch(()=>{});
  try{
   await new Promise(r=>setImmediate(r));
   assert.equal(started,4,'MP-10 independent tiles must overlap, bounded to four decodes');assert.equal(drawn.length,0);
   while(active){for(const release of pending.splice(0))release();await new Promise(r=>setImmediate(r));}
   if(fail){await assert.rejects(result,/decode failed/);assert.deepEqual(drawn,[]);assert.equal(p.sequence,1)}
   else {assert.equal(await result,true);assert.deepEqual(drawn,[0,1,2,3,4,5,6,7]);assert.equal(p.sequence,2)}
   assert.equal(peak,4);assert.equal(new Set(closed).size,started-(fail?1:0));assert.equal(closed.length,new Set(closed).size);
  }finally{while(active){for(const release of pending.splice(0))release();await new Promise(r=>setImmediate(r));}await result.catch(()=>{});p.close();globalThis.createImageBitmap=old}
 }
});
