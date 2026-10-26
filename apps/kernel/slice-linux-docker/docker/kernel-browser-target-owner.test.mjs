// MP-08/MP-10/MP-11: tab-state preflight reuses the kernel's event-owned document without renderer discovery.
import test from 'node:test';
import assert from 'node:assert/strict';
import {KernelBrowserHost} from './kernel-browser-host.mjs';
function setup(warm=true){
 const calls=[],tab={tab_id:'tab',target_id:'target',document_id:'old',url:'https://www.wikipedia.org/',title:'Wikipedia'};
 let info=async()=>({targetInfo:{url:'https://html.spec.whatwg.org/',title:'HTML'}}),proof;
 const connection={isOpen:()=>true,send:async method=>{calls.push(method);if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame',loaderId:'new'}}};if(method==='Target.getTargetInfo')return info();assert.fail('unexpected method');}};
 proof={connection,sessionId:'s',frame:{id:'frame',loaderId:'new'}};
 const browser={connection,currentPageFrame:()=>warm?proof:null,resolvePageTarget:async()=>{assert(!warm,'warm tab-state never resolves the target again');return {connection,sessionId:'s'};}};
 let saves=0;const host={browser,generation:1,tabs:new Map([['tab',tab]]),save:async()=>{saves++;}};
 return {host,calls,tab,saves:()=>saves,holdInfo:p=>{info=()=>p;},replace:()=>{proof={...proof};}};
}
test('MP-08/MP-10/MP-11 warm tab-state reads browser metadata without asking the loading renderer',async()=>{
 const f=setup();const result=await KernelBrowserHost.prototype.displayTarget.call(f.host,{generation:1,tab_id:'tab'});
 assert.equal(result.document_id,'new');assert.equal(f.tab.document_id,'new');assert.equal(result.url,'https://html.spec.whatwg.org/');assert.deepEqual(f.calls,['Target.getTargetInfo']);assert.equal(f.saves(),1);
});
test('MP-11 replacement during the metadata read cannot publish or save an old owner',async()=>{
 const f=setup(),ready=Promise.withResolvers();f.holdInfo(ready.promise);
 const pending=KernelBrowserHost.prototype.displayTarget.call(f.host,{generation:1,tab_id:'tab'});const rejected=assert.rejects(pending,e=>e.code==='stale_document_reference');
 await new Promise(setImmediate);f.replace();ready.resolve({targetInfo:{url:'https://html.spec.whatwg.org/',title:'HTML'}});await rejected;assert.equal(f.tab.document_id,'old');assert.equal(f.saves(),0);
});
test('MP-08/MP-11 cold tab-state retains its fresh frame query',async()=>{
 const f=setup(false);const result=await KernelBrowserHost.prototype.displayTarget.call(f.host,{generation:1,tab_id:'tab'});
 assert.equal(result.document_id,'new');assert.deepEqual(f.calls,['Page.getFrameTree','Target.getTargetInfo']);
});
