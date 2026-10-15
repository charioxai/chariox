// MP-08/MP-10/MP-11:536 is the default-crop activation floor, not a519 region bump.
import test from 'node:test';import assert from 'node:assert/strict';
import {attachBrowserDisplay} from './presenter.mjs';
const result={source_kind:'region',binding_id:'r',document_id:'d',sequence:3,generation:2,subscription_id:'s',codec:'avc1.420033',bitrate:8000000,device_scale_factor:1,css_width:1264,css_height:712};
for(const version of [535,536])test(`MP-08/MP-10/MP-11 default crop floor${version}`,async()=>{
 const ops=[],document=Object.assign(new EventTarget(),{hidden:false});
 const canvas={ownerDocument:document,width:1,height:1,getContext:()=>({})};
 const transport={kernelProtocolVersion:version,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>{ops.push(command.op);return {KernelBrowser:{result:command.op==='display_subscribe'?result:{}}}}};
 const create=()=>attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:2},{codec:'avc1.420033',deviceScaleFactor:2});
 if(version===535){await assert.rejects(create(),/protocol536/);assert.deepEqual(ops,['display_subscribe','unsubscribe']);}
 else{const stream=await create();assert.equal(stream.binding.device_scale_factor,1);assert.equal(stream.binding.css_height,712);await stream.close();}
});
test('MP-11:536 default crop refuses malformed lease and unsubscribes',async()=>{
 const ops=[];const transport={kernelProtocolVersion:536,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>{ops.push(command.op);return {KernelBrowser:{result:{...result,binding_id:''}}}}};
 await assert.rejects(attachBrowserDisplay({},transport,{tab_id:'t',generation:2},{codec:'avc1.420033'}),/invalid region/);assert.deepEqual(ops,['display_subscribe','unsubscribe']);
});
