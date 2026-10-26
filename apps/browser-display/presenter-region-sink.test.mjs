// MP-08/MP-10/MP-11: cropped page pixels must reach the paint surface without
// a second animation-frame wait; ordinary full-page/desktop policy is separate.
import test from 'node:test';import assert from 'node:assert/strict';
import {attachBrowserDisplay} from './presenter.mjs';
for(const explicit of [true,false])test(`MP-08/MP-10/MP-11 ${explicit?'opaque region':'visible page crop'} commits decoded pixels before another rAF`,async()=>{
 const binding={subscription_id:'s',tab_id:'t',generation:2,source_kind:'region',binding_id:'lease',document_id:'d',sequence:7,css_width:320,css_height:180,device_scale_factor:1};
 let draws=0,raf=0;const canvas={width:320,height:180,isConnected:true,getContext:()=>({drawImage(){draws++}}),ownerDocument:{hidden:false,defaultView:{requestAnimationFrame(){raf++;return 1},cancelAnimationFrame(){}}}};
 const transport={kernelProtocolVersion:536,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?binding:{push:'running'}}})};
 const options={codec:'avc1.420033',...(explicit?{target:{kind:'region',mirror_subscription_id:'m',document_id:'d',sequence:7,region_id:'n1'}}:{})};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:2},options);
 try{
  const pending=stream.presenter.videoSink.commit({image:{close(){},clone(){return {close(){}}}},decodedAt:1},{...binding,sequence:1,width:320,height:180});
  assert.equal(draws,1,'a crop must draw before another viewer animation frame');assert.equal(raf,1,'the only animation frame requested marks the end of this frame; the draw never waited for it');assert.equal((await pending).drawn,true);
 }finally{await stream.close()}
});

// The Chrome UI band is region-bound too, but its keystroke echo must never wait for a compositor frame.
for(const kind of ['browser_chrome','browser_auxiliary'])test(`MP-08/MP-10/MP-11 the ${kind} band draws immediately and is never paced`,async()=>{
 const binding={subscription_id:'s',tab_id:'t',generation:2,source_kind:'region',binding_id:'lease',document_id:'d',sequence:7,css_width:320,css_height:180,device_scale_factor:1};
 const canvas={width:320,height:180,isConnected:true,getContext:()=>({drawImage(){}}),ownerDocument:{hidden:false,defaultView:{requestAnimationFrame(){assert.fail('the band must not wait for a compositor frame')},cancelAnimationFrame(){}}}};
 const transport={kernelProtocolVersion:536,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?binding:{push:'running'}}})};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:2},{codec:'avc1.420033',target:{kind,window_generation:1}});
 try{assert.equal(stream.presenter.videoSink.paced,false);assert.equal(stream.presenter.videoSink.direct,true)}finally{await stream.close()}
});
for(const [name,options,paced] of [['visible page crop',{},true],['mirror region',{target:{kind:'region',mirror_subscription_id:'m',document_id:'d',sequence:7,region_id:'n1'}},true]])test(`MP-08/MP-10/MP-11 the ${name} is paced`,async()=>{
 const binding={subscription_id:'s',tab_id:'t',generation:2,source_kind:'region',binding_id:'lease',document_id:'d',sequence:7,css_width:320,css_height:180,device_scale_factor:1};
 const canvas={width:320,height:180,isConnected:true,getContext:()=>({drawImage(){}}),ownerDocument:{hidden:false,defaultView:{requestAnimationFrame(){return 1},cancelAnimationFrame(){}}}};
 const transport={kernelProtocolVersion:536,displayEventEncoding:'CXD1',onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?binding:{push:'running'}}})};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:2},{codec:'avc1.420033',...options});
 try{assert.equal(stream.presenter.videoSink.paced,paced)}finally{await stream.close()}
});
