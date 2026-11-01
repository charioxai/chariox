// MP-08/MP-10/MP-11: crops are admitted by the kernel, never by frame geometry.
import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserDisplayPresenter,attachBrowserDisplay} from './presenter.mjs';
const lease = {subscription_id:'s',tab_id:'t',generation:2,source_kind:'region',binding_id:'lease',
 document_id:'d',sequence:7,css_width:320,css_height:180,device_scale_factor:2};
const frame = {...lease,sequence:1,width:640,height:360};
const canvas=()=>({width:1,height:1,getContext:()=>({drawImage(){}})});
test('MP-08/MP-11 only kernel-negotiated region geometry admits a cropped frame',()=>{
 const presenter=new BrowserDisplayPresenter(canvas(),lease);
 try{
  assert.equal(presenter.validate(frame,0),true);
  for(const change of [{css_width:319,width:638},{css_height:179,height:358},{device_scale_factor:1,width:320,height:180},{document_id:'other'}])
   assert.throws(()=>presenter.validate({...frame,...change},0),/geometry\/binding/);
  const page=new BrowserDisplayPresenter(canvas(),{subscription_id:'s',tab_id:'t',generation:2});
  try{assert.throws(()=>page.validate(frame,0),/geometry\/binding/)}finally{page.close()}
 }finally{presenter.close()}
});
test('MP-08/MP-11 region input carries the resolved mirror epoch and opaque binding',()=>{
 const presenter=new BrowserDisplayPresenter(canvas(),lease);presenter.documentId='d';presenter.sequence=3;
 try{assert.deepEqual(presenter.input({kind:'click',x:10,y:20}),{op:'display_input',tab_id:'t',generation:2,document_id:'d',binding_id:'lease',sequence:7,input:{kind:'click',x:10,y:20}})}finally{presenter.close()}
});
test('MP-08/MP-11 malformed or unbound region negotiations fail before displaying pixels',()=>{
 for(const change of [{binding_id:''},{document_id:''},{sequence:0},{css_width:0},{css_height:801},{device_scale_factor:3}]){
  const presenter=new BrowserDisplayPresenter(canvas(),{...lease,...change});
  try{assert.throws(()=>presenter.validate(frame,0),/geometry\/binding/)}finally{presenter.close()}
 }
});
test('MP-08/MP-10 region subscriptions require protocol519 before sending a request',async()=>{
 let requests=0;
 await assert.rejects(attachBrowserDisplay(canvas(),{kernelProtocolVersion:517,displayEventEncoding:'CXD1',request:async()=>{requests++;throw Error('unexpected RPC')}},{tab_id:'t',generation:2},{target:{kind:'region',mirror_subscription_id:'m',document_id:'d',sequence:7,region_id:'n1'}}),/protocol519/);
 assert.equal(requests,0);
});
test('MP-08/MP-11 a region request cannot silently open a whole-page display',async()=>{
 const ops=[];
 const transport={kernelProtocolVersion:519,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>{
  ops.push(command);return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:2}: {unsubscribed:true}}};
 }};
 await assert.rejects(attachBrowserDisplay(canvas(),transport,{tab_id:'t',generation:2},{codec:'avc1.420033',target:{kind:'browser_chrome',window_generation:4}}),/region negotiation/);
 assert.deepEqual(ops.map(c=>c.op),['display_subscribe','unsubscribe']);
});
test('MP-08/MP-11 a two-device-pixel crop is valid at native DPR2',()=>{
 const binding={...lease,css_width:1,css_height:1};
 const presenter=new BrowserDisplayPresenter(canvas(),binding);
 try{assert.equal(presenter.validate({...binding,sequence:1,width:2,height:2},0),true)}finally{presenter.close()}
});
test('MP-08/MP-11 negotiated wide Chrome crops use the native physical-pixel bounds',()=>{
 const binding={...lease,css_width:1920,css_height:86,device_scale_factor:1};
 const presenter=new BrowserDisplayPresenter(canvas(),binding);
 try{assert.equal(presenter.validate({...binding,sequence:1,width:1920,height:86},0),true)}finally{presenter.close()}
});
test('MP-08/MP-10/MP-11 a Tab display binding admits the Tab\'s next document, never other geometry',()=>{
 const presenter=new BrowserDisplayPresenter(canvas(),{...lease,tab_display:true});
 try{
  assert.equal(presenter.validate({...frame,document_id:'next'},0),true);
  for(const change of [{css_width:319,width:638},{device_scale_factor:1,width:320,height:180}])
   assert.throws(()=>presenter.validate({...frame,document_id:'next',...change},0),/geometry\/binding/);
 }finally{presenter.close()}
});
