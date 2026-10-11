// MP-08/MP-10/MP-11 supplementary event-order and loader fence regressions.
import test from 'node:test';import assert from 'node:assert/strict';
import {DisplayDocuments} from './kernel-browser-display-document.mjs';
function fixture(){
 const observers=new Set(),calls=[];let document='a',blocked;
 const connection={subscribe:fn=>(observers.add(fn),()=>observers.delete(fn)),isOpen:()=>true,send:async method=>{calls.push(method);await blocked;return {frameTree:{frame:{id:'root',loaderId:document}}}}};
 const browser={resolvePageTarget:async()=>({connection,sessionId:'s'})},cache=new DisplayDocuments(),stored={target_id:'t',document_id:'a'};
 return {cache,connection,calls,observers,browser,stored,event:(method,params={})=>{for(const fn of observers)fn({method,params,sessionId:'s'})},navigate:d=>document=d,block:p=>blocked=p};
}
test('MP-10 admitted native document and steady validation make zero CDP calls',async()=>{
 const f=fixture(),tab=await f.cache.target(f.browser,f.stored);f.calls.length=0;
 for(let n=0;n<20;n++){assert.equal(f.cache.current(tab),true);assert.equal(await f.cache.validate(tab),true);await f.cache.target(f.browser,f.stored);}
 assert.deepEqual(f.calls,[]);f.cache.clear();assert.equal(f.observers.size,0);assert.equal(f.cache.current(tab),false);
});
test('MP-11 document changes between readback and commit discard the old frame',async()=>{
 const f=fixture(),tab=await f.cache.target(f.browser,f.stored);f.navigate('b');
 f.event('Page.frameNavigated',{frame:{id:'root',loaderId:'b'}});
 assert.equal(f.cache.current(tab),false);await assert.rejects(f.cache.validate(tab),e=>e.code==='stale_document_reference');
 const fresh=await f.cache.target(f.browser,f.stored);assert.equal(fresh.document_id,'b');assert.equal(f.cache.current(fresh),true);f.cache.clear();
});
test('MP-11 change away and back cannot re-admit a readback of the old revision',async()=>{
 const f=fixture(),tab=await f.cache.target(f.browser,f.stored);
 f.event('Page.frameNavigated',{frame:{id:'root',loaderId:'b'}});f.event('Page.frameNavigated',{frame:{id:'root',loaderId:'a'}});
 assert.equal(await f.cache.validate(tab),false);f.cache.clear();
});
test('MP-11 load-event refresh runs off the credit path and retires in-flight pixels',async()=>{
 const f=fixture(),tab=await f.cache.target(f.browser,f.stored);let resolve;
 f.block(new Promise(r=>resolve=r));f.navigate('b');f.event('Page.loadEventFired');
 assert.equal(f.cache.current(tab),false);await assert.rejects(f.cache.target(f.browser,f.stored),/unavailable/);
 resolve();await new Promise(r=>setImmediate(r));
 assert.equal((await f.cache.target(f.browser,f.stored)).document_id,'b');assert.equal(f.cache.current(tab),false);f.cache.clear();
});
test('MP-11 loading and session retirement refuse cached document pixels',async()=>{
 for(const [method,params]of [['Page.frameStartedLoading',{frameId:'root'}],['Target.detachedFromTarget',{sessionId:'s'}],['Target.targetDestroyed',{targetId:'t'}]]){
  const f=fixture(),tab=await f.cache.target(f.browser,f.stored);f.event(method,params);assert.equal(f.cache.current(tab),false);f.cache.clear();
 }
});

test('MP-10 unchanged loader revision retires pixels without a credit-path query',async()=>{
 const f=fixture(),tab=await f.cache.target(f.browser,f.stored);f.calls.length=0;
 f.event('Page.frameStartedLoading',{frameId:'root'});
 assert.equal(await f.cache.validate(tab),false);assert.deepEqual(f.calls,[]);f.cache.clear();
});

test('MP-11 connection retirement invalidates cached pixels through the event seam',async()=>{
 const f=fixture(),tab=await f.cache.target(f.browser,f.stored);f.calls.length=0;
 f.event('Chariox.browserDisconnected');
 assert.equal(f.cache.current(tab),false);assert.equal(await f.cache.validate(tab),false);assert.deepEqual(f.calls,[]);f.cache.clear();
});
