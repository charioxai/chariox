// MP-08/MP-10/MP-11: use real presenter teardown, not a no-op close fixture.
import test from 'node:test';import assert from 'node:assert/strict';
const {BrowserDisplayPresenter,attachBrowserDisplay}=await import(process.env.MD_PRESENTER_SOURCE??'./presenter.mjs');
test('MP-08/MP-10 recovery frees decoders while retaining the painted canvas',()=>{
 const canvas={width:1280,height:800};const p=new BrowserDisplayPresenter(canvas,{generation:1,tab_id:'t',subscription_id:'s'});let freed=0;
 p.decoder={close:()=>freed++};p.close({preserveFrame:true});
 assert.equal(freed,1);assert.equal(p.closed,true);assert.deepEqual([canvas.width,canvas.height],[1280,800]);
 p.close();assert.deepEqual([canvas.width,canvas.height],[1,1]);
});
test('MP-08/MP-10 subscription recovery forwards preserveFrame to actual presenter',async()=>{
 const canvas={width:1280,height:800};const transport={kernelProtocolVersion:475,displayEventEncoding:'CXD1',subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:{push:'running'}}})};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1});
 await stream.close({preserveFrame:true});assert.deepEqual([canvas.width,canvas.height],[1280,800]);
});
test('MP-08/MP-10 failed replacement registration preserves previous paint',async()=>{
 const canvas={width:1280,height:800};const transport={kernelProtocolVersion:475,displayEventEncoding:'CXD1',subscribeDisplay:async()=>{throw Error('relay disconnected')},onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:{}}})};
 await assert.rejects(attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1}),/relay disconnected/);
 assert.deepEqual([canvas.width,canvas.height],[1280,800]);
});
