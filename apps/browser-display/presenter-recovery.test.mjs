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
 const canvas={width:1280,height:800};const transport={kernelProtocolVersion:491,displayEventEncoding:'CXD1',subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:{push:'running'}}})};
 const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1});
 await stream.close({preserveFrame:true});assert.deepEqual([canvas.width,canvas.height],[1280,800]);
});
test('MP-08/MP-10 failed replacement registration preserves previous paint',async()=>{
 const canvas={width:1280,height:800};const transport={kernelProtocolVersion:491,displayEventEncoding:'CXD1',subscribeDisplay:async()=>{throw Error('relay disconnected')},onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:{}}})};
 await assert.rejects(attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1}),/relay disconnected/);
 assert.deepEqual([canvas.width,canvas.height],[1280,800]);
});
test('MP-08/MP-10 protocol 525 transient teardown parks the producer and returns a bounded resume hint',async()=>{
 const commands=[],hints=[],canvas={width:1280,height:800};
 const transport={kernelProtocolVersion:525,displayEventEncoding:'CXD1',subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>{commands.push(command);return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'old',generation:7}:{push:'running'}}}}};
 const display=await attachBrowserDisplay(canvas,transport,{tab_id:'tab',generation:7},{onResumeHint:hint=>hints.push(hint)});
 await display.close({preserveFrame:true});
 try{
  assert(commands.some(c=>c.op==='display_suspend'));assert(!commands.some(c=>c.op==='unsubscribe'));
  assert.equal(hints.at(-1).subscription_id,'old');assert.equal(hints.at(-1).generation,7);
  assert(hints.at(-1).expiresAt>performance.now()&&hints.at(-1).expiresAt<=performance.now()+5000);
  assert.deepEqual([canvas.width,canvas.height],[1280,800]);
 }finally{await display.close()}
});
test('MP-08/MP-11 replacement delivery repeats admission instead of replaying an opening grant',async()=>{
 const commands=[],registered=[],canvas={width:1280,height:800};
 const transport={kernelProtocolVersion:525,displayEventEncoding:'CXD1',subscribeDisplay:async binding=>registered.push(binding.subscription_id),unsubscribeDisplay:async()=>{},onEvent:()=>()=>{},request:async({KernelBrowser:{command}})=>{commands.push(command);return {KernelBrowser:{result:command.op==='display_resume'?{subscription_id:'fresh',generation:7,resumed:true}:{push:'running'}}}}};
 const display=await attachBrowserDisplay(canvas,transport,{tab_id:'tab',generation:7},{resumeHint:{subscription_id:'old',generation:7,targetKey:JSON.stringify(['tab',7]),desktop:false,device_scale_factor:1,expiresAt:performance.now()+5000}});
 try{assert.equal(commands[0].op,'display_resume');assert.equal(commands[0].subscription_id,'old');
 assert.deepEqual(registered,['fresh']);}finally{await display.close()}
});

test('MP-08/MP-10/MP-11 first independent push during registration reaches the presenter',async()=>{
 const previous=BrowserDisplayPresenter.prototype.present,seen=[],listeners=new Set();
 BrowserDisplayPresenter.prototype.present=async function(frame){this.sequence=frame.sequence;return true};
 const binding={subscription_id:'fresh',generation:7},frame={subscription_id:'fresh',generation:7,sequence:1,kind:'png',width:1280,height:800,data:new Uint8Array([1])};
 const transport={kernelProtocolVersion:535,displayEventEncoding:'CXD1',onEvent:fn=>{listeners.add(fn);return()=>listeners.delete(fn)},
  subscribeDisplay:async()=>{for(const listener of listeners)listener({event:'kernel_browser_frame',subscription_id:'fresh',frame})},unsubscribeDisplay:async()=>{},
  request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?binding:{push:'running'}}})};
 let stream;
 try{
  stream=await attachBrowserDisplay({width:1280,height:800},transport,{tab_id:'tab',generation:7},{onPresented:frame=>seen.push(frame.sequence)});
  await new Promise(resolve=>setTimeout(resolve,20));
  assert.deepEqual(seen,[1],'MP-10 do not lose the first independently decoded frame in the registration listener gap');
 }finally{await stream?.close();BrowserDisplayPresenter.prototype.present=previous}
});
