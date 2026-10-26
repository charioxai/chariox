// MP-08/MP-10/MP-11 supplementary event-order and loader fence regressions.
import test from 'node:test';import assert from 'node:assert/strict';
import {DisplayDocuments,resolveDisplayTarget} from './kernel-browser-display-document.mjs';
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

// MP-08/MP-10/MP-11: cancelled navigation and download retain the loader.
test('MP-10 stopped top-level load recovers an unchanged document off the credit path',async()=>{
 const f=fixture(),old=await f.cache.target(f.browser,f.stored);
 f.event('Page.frameStartedLoading',{frameId:'root'});
 assert.equal(f.cache.current(old),false);
 f.event('Page.frameStoppedLoading',{frameId:'child'});
 await assert.rejects(f.cache.target(f.browser,f.stored),/unavailable/);
 let finish;f.block(new Promise(r=>finish=r));f.calls.length=0;
 f.event('Page.frameStoppedLoading',{frameId:'root'});
 assert.deepEqual(f.calls,['Page.getFrameTree']);
 await assert.rejects(f.cache.target(f.browser,f.stored),/unavailable/);
 finish();await new Promise(r=>setImmediate(r));
 const fresh=await f.cache.target(f.browser,f.stored);
 assert.equal(fresh.document_id,'a');assert.equal(f.cache.current(fresh),true);
 assert.equal(f.cache.current(old),false);f.cache.clear();
});
test('MP-11 stopped-load refresh cannot overwrite a newer navigation',async()=>{
 const f=fixture();await f.cache.target(f.browser,f.stored);
 f.event('Page.frameStartedLoading',{frameId:'root'});
 let finish;f.block(new Promise(r=>finish=r));
 f.event('Page.frameStoppedLoading',{frameId:'root'});
 f.event('Page.frameNavigated',{frame:{id:'root',loaderId:'b'}});
 finish();await new Promise(r=>setImmediate(r));
 assert.equal((await f.cache.target(f.browser,f.stored)).document_id,'b');f.cache.clear();
});

for(const [method,params]of [
 ['Target.targetDestroyed',{targetId:'t'}],['Target.detachedFromTarget',{sessionId:'s'}],
 ['Inspector.detached',{}],['Chariox.browserDisconnected',{}]
])test(`MP-10 ${method} evicts closed targets and unsubscribes immediately`,async()=>{
 const f=fixture();
 for(let n=0;n<32;n++){
  const tab=await f.cache.target(f.browser,f.stored);
  assert.equal(f.cache.entries.size,1);assert.equal(f.observers.size,1);
  f.event(method,params);
  assert.equal(f.cache.current(tab),false);
  assert.equal(f.cache.entries.size,0);assert.equal(f.observers.size,0);
 }
 f.cache.clear();
});
test('MP-11 old target retirement cannot evict its replacement binding',async()=>{
 const f=fixture(),old=await f.cache.target(f.browser,f.stored),observer=[...f.observers][0];
 f.cache.clear();const fresh=await f.cache.target(f.browser,f.stored);
 observer({method:'Target.targetDestroyed',params:{targetId:'t'}});
 assert.equal(f.cache.current(old),false);assert.equal(f.cache.current(fresh),true);
 assert.equal(f.cache.entries.size,1);assert.equal(f.observers.size,1);f.cache.clear();
});
test('MP-11 clearing during target resolution cannot leak a late subscription',async()=>{
 const f=fixture();let finish;
 const browser={resolvePageTarget:()=>new Promise(r=>finish=()=>r({connection:f.connection,sessionId:'s'}))};
 const pending=f.cache.target(browser,f.stored);f.cache.clear();finish();
 await assert.rejects(pending,/unavailable/);assert.equal(f.observers.size,0);assert.equal(f.cache.entries.size,0);
});

test('MP-08/MP-10/MP-11 native input guard never promotes a different requested document',async()=>{
 const f=fixture();try{
  const valid=await f.cache.guard(f.browser,f.stored);assert.equal(valid(),true);
  const stale=await f.cache.guard(f.browser,{...f.stored,document_id:'other'});assert.equal(stale(),false);
  f.stored.document_id='promoted';assert.equal(valid(),false,'caller mutation cannot change the pinned request');
  f.stored.document_id='a';f.event('Page.frameStartedLoading',{frameId:'root'});assert.equal(valid(),false);
 }finally{f.cache.clear()}
});

for(const retirement of ['loader','session','connection'])test(`MP-08/MP-10/MP-11 cached metadata refuses ${retirement} retirement during its read`,async()=>{
 const stored={tab_id:'t',target_id:'T',document_id:'old'},saved=[];
 const connection={isOpen:()=>true,async send(method){
  assert.equal(method,'Target.getTargetInfo','a named loader needs no renderer query');
  if(retirement==='loader')browser.documentIdsByTarget.set('T','next');
  if(retirement==='session')browser.sessionsByTarget.set('T','replacement');
  if(retirement==='connection')browser.connection={};
  return {targetInfo:{url:'https://public.example/',title:'Public'}};
 }};
 const browser={connection,sessionsByTarget:new Map([['T','S']]),documentIdsByTarget:new Map([['T','current']]),resolvePageTarget:async()=>({connection,sessionId:'S'})};
 const host={generation:1,tabs:new Map([['t',stored]]),browser,save:async()=>saved.push(true)};
 await assert.rejects(resolveDisplayTarget(host,{tab_id:'t',generation:1}),error=>error.code==='user_domain_stale_reference');
 assert.equal(stored.document_id,'old');assert.deepEqual(saved,[]);
});

// MP-08/MP-10/MP-11: the first native wheel/key admits the document while the renderer is still parsing; the
// event-bound frame of this live session is the document a query would return, so admission does not wait for load.
test('MP-10 first native admission does not wait for a busy renderer when the live frame is known',async()=>{
 const f=fixture();f.block(new Promise(()=>{}));
 f.browser.currentPageFrame=()=>({connection:f.connection,sessionId:'s',frame:{id:'root',loaderId:'a'}});
 const tab=await Promise.race([f.cache.target(f.browser,f.stored),new Promise((_,r)=>setTimeout(()=>r(Error('admission waited for the renderer')),200))]);
 assert.equal(tab.document_id,'a');assert.deepEqual(f.calls,[]);assert.equal(f.cache.current(tab),true);
 f.event('Page.frameNavigated',{frame:{id:'root',loaderId:'b'}});assert.equal(f.cache.current(tab),false);
 assert.equal((await f.cache.target(f.browser,f.stored)).document_id,'b');f.cache.clear();
});
test('MP-11 a known frame from another session is not trusted at admission',async()=>{
 const f=fixture();let finish;f.block(new Promise(r=>finish=r));
 f.browser.currentPageFrame=()=>({connection:f.connection,sessionId:'other',frame:{id:'root',loaderId:'a'}});
 const pending=f.cache.target(f.browser,f.stored);await new Promise(r=>setImmediate(r));
 assert.deepEqual(f.calls,['Page.getFrameTree']);finish();assert.equal((await pending).document_id,'a');f.cache.clear();
});

test('MP-10 input arriving while the document is being identified waits for it, then is admitted; a document that never settles is still refused',async()=>{
 const f=fixture(),tab=await f.cache.target(f.browser,f.stored);let release;
 f.block(new Promise(r=>release=r));f.navigate('b');f.event('Page.loadEventFired');
 assert.equal(f.cache.current(tab),false);
 const waiting=f.cache.target(f.browser,f.stored,2000);setTimeout(release,120);
 assert.equal((await waiting).document_id,'b','the click is delivered to the identified document, not refused as loading');
 f.block(new Promise(()=>{}));f.event('Page.loadEventFired');
 await assert.rejects(f.cache.target(f.browser,f.stored,150),/unavailable while loading/);f.cache.clear();
});
