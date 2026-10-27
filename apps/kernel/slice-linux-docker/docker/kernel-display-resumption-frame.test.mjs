// MP-08/MP-10/MP-11: delivery replacement must fence outstanding protected work.
import test from 'node:test';
import assert from 'node:assert/strict';
const { DisplayStream }=await import(process.env.DISPLAY_MODULE??'./kernel-browser-display.mjs');
const binding={subscription_id:'old',tab_id:'tab',codec:'png',device_scale_factor:1,bitrate:4000000};
test('MP-11 a credit admitted before suspension cannot publish into the replacement stream',async()=>{
  const stream=new DisplayStream(binding,{encoder:{close:async()=>{}}});
  let finish,entered;
  const waiting=new Promise(resolve=>{finish=resolve});const started=new Promise(resolve=>{entered=resolve});
  stream.buildFrame=async(_source,_document,_sequence,valid)=>{entered();await waiting;return await valid()?{subscription_id:stream.subscription_id}:null};
  try{
    const old=stream.frame({},'doc',0);await started;
    stream.deliveryEpoch=1;stream.suspended=true;stream.subscription_id='new';
    stream.deliveryEpoch=2;stream.suspended=false;finish();
    assert.equal(await old,null);
  }finally{await stream.close()}
});
test('MP-08/MP-10 a resumed producer keeps its prepared independent frame through the first ACK',async()=>{
  const stream=new DisplayStream(binding,{encoder:{close:async()=>{}}});let resets=0;
  stream.producer={independentFrameReady:true,invalidate(){resets++},async close(){}};
  try{
    stream.resumed=true;stream.resetForDelivery();assert.equal(resets,0);
    stream.resetForDelivery();assert.equal(resets,1,'later loss still requests a fresh reference chain');
    stream.producer.independentFrameReady=false;stream.resumed=true;
    stream.resetForDelivery();assert.equal(resets,2,'a dependent cache is never a bootstrap');
  }finally{await stream.close()}
});
