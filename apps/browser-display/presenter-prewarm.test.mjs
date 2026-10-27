// MP-08/MP-10/MP-11: overlap code-only worker startup with existing admission.
import test from'node:test';import assert from'node:assert/strict';
const{attachBrowserDisplay}=await import(process.env.MD_PRESENTER_SOURCE??'./presenter.mjs');
test('MP-08/MP-10/MP-11 worker warms before registration and retires on refused admission',async()=>{
 const prior={Worker:globalThis.Worker,VideoDecoder:globalThis.VideoDecoder};
 for(const refused of [false,true]){
  let created=0,retired=0,finish;const pending=new Promise((resolve,reject)=>{finish=()=>refused?reject(Error('admission refused')):resolve({subscription_id:'s',generation:1})});
  globalThis.Worker=class{constructor(){created++}terminate(){retired++}postMessage(){}};globalThis.VideoDecoder=class{};
  const transport={kernelProtocolVersion:466,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?await pending:{}}})};
  const result=attachBrowserDisplay({width:1280,height:800},transport,{tab_id:'t',generation:1},{codec:'vp8'});result.catch(()=>{});let stream;
  try{await new Promise(r=>setImmediate(r));assert.equal(created,1,'MP-10 do not serialize code-only worker startup after remote admission');finish();if(refused)await assert.rejects(result,/admission refused/);else{stream=await result;assert.equal(created,1,'MP-10 transfer the warm worker without constructing another')}}
  finally{finish();stream??=await result.catch(()=>null);await stream?.close();assert.equal(retired,created,'MP-11 warm worker must not survive close or refused admission');Object.assign(globalThis,prior)}
 }
});

test('MP-08/MP-10/MP-11 refused crop floor or binding retires its prewarmed worker',async()=>{
 const prior={Worker:globalThis.Worker,VideoDecoder:globalThis.VideoDecoder};
 try{for(const version of [535,536]){
  let created=0,retired=0,released=0;
  globalThis.Worker=class{constructor(){created++}terminate(){retired++}postMessage(){}};globalThis.VideoDecoder=class{};
  const transport={kernelProtocolVersion:version,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>{
   if(command.op==='unsubscribe')released++;
   return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1,source_kind:'region'}:{}}};
  }};
  await assert.rejects(attachBrowserDisplay({width:1280,height:800},transport,{tab_id:'t',generation:1},{codec:'vp8'}),version===535?/protocol536/:/invalid region negotiation/);
  assert.equal(created,1);assert.equal(retired,1);assert.equal(released,1);
 }}finally{Object.assign(globalThis,prior)}
});
