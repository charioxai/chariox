// MP-08/MP-10/MP-11: temporal admission of native pixels around CDP capture.
import test from 'node:test';import assert from 'node:assert/strict';
import {CapturePresentationFence,fenceScreenshotPresentation} from './browser-capture-presentation.mjs';
const deferred=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b});return {promise,resolve,reject}};
test('MP-08/MP-10 capture and restoration both exclude native pixels, including delayed delivery',async()=>{
 let clock=100;const fence=new CapturePresentationFence(()=>clock),capture=deferred(),restore=deferred();
 const task=fence.run(()=>capture.promise,()=>restore.promise);
 assert.equal(fence.stable(99),true);assert.equal(fence.stable(100),false);
 clock=110;capture.resolve('png');await Promise.resolve();assert.equal(fence.stable(110),false);
 clock=120;restore.resolve();assert.equal(await task,'png');
 assert.equal(fence.stable(105,106),false,'a delayed readback from inside capture remains excluded');
 assert.equal(fence.stable(95,102),false,'a read straddling capture start remains excluded');
 assert.equal(fence.stable(122),true);
});
test('MP-08/MP-10 overlapping captures and failed restoration cannot admit transient pixels',async()=>{
 let clock=100;const fence=new CapturePresentationFence(()=>clock),first=deferred(),second=deferred();
 const a=fence.run(()=>first.promise,async()=>{});clock=105;const b=fence.run(()=>second.promise,async()=>{throw Error('unrestored')});
 clock=110;first.resolve();await a;assert.equal(fence.stable(111),false);
 second.resolve();await assert.rejects(b,/unrestored/);assert.equal(fence.stable(10000),false);
});
test('MP-08/MP-10 old readbacks cannot outrun bounded capture history',async()=>{
 let clock=100;const fence=new CapturePresentationFence(()=>clock);
 for(let i=0;i<100;i++){await fence.run(async()=>{},async()=>{});clock+=10;}
 assert.equal(fence.intervals.length,64);assert.equal(fence.stable(100),false);assert.equal(fence.stable(clock),true);
 for(const [start,end]of [[NaN,1],[1,Infinity],[2,1]])assert.equal(fence.stable(start,end),false);
});
test('MP-08/MP-10 only screenshots restore native presentation through an isolated world',async()=>{
 const sent=[];const connection={send:async(method,params)=>{sent.push({method,params});if(method==='Page.captureScreenshot')return {data:'png'};if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'top'}}};if(method==='Page.createIsolatedWorld')return {executionContextId:3};if(method==='Runtime.evaluate')return {result:{value:true}};return {ok:true};}};
 fenceScreenshotPresentation(connection);await connection.send('Input.dispatchKeyEvent',{});assert.equal(sent.length,1);
 assert.deepEqual(await connection.send('Page.captureScreenshot',{},'session'),{data:'png'});
 assert.deepEqual(sent.slice(1).map(x=>x.method),['Page.captureScreenshot','Page.getFrameTree','Page.createIsolatedWorld','Runtime.evaluate']);
 assert.equal(sent.at(-1).params.contextId,3);assert.equal(sent.at(-1).params.awaitPromise,true);
});

test('MP-08/MP-10 CSS activation and screenshot deadlines survive the presentation fence',async()=>{
 const sent=[];const connection={send:async(...args)=>{sent.push(args);if(args[0]==='Page.getFrameTree')return {frameTree:{frame:{id:'top'}}};if(args[0]==='Page.createIsolatedWorld')return {executionContextId:3};if(args[0]==='Runtime.evaluate')return {result:{value:true}};return {ok:true};}};
 fenceScreenshotPresentation(connection);
 const css={timeoutMs:30000},capture={timeoutMs:12000};
 await connection.send('CSS.enable',{},'page',css);
 await connection.send('Page.captureScreenshot',{format:'png'},'page',capture);
 assert.equal(sent[0][3],css,'A bounded CSS watcher must keep its own timeout');
 assert.equal(sent[1][3],capture,'Capture must keep its own timeout');
});


test('MP-08/MP-10/MP-11 restoration wake waits for overlapping captures and never bypasses a failed restore',async()=>{
 let clock=100,wakes=0;const fence=new CapturePresentationFence(()=>clock),first=deferred(),second=deferred();
 const a=fence.run(()=>first.promise,async()=>{}),b=fence.run(()=>second.promise,async()=>{throw Error('unrestored')});
 const cancel=fence.whenStable(()=>{wakes++});clock=200;first.resolve();await a;clock=202;await new Promise(resolve=>setTimeout(resolve,5));assert.equal(wakes,0);
 second.resolve();await assert.rejects(b,/unrestored/);clock=300;await new Promise(resolve=>setTimeout(resolve,5));assert.equal(wakes,0);assert.equal(fence.stable(clock),false);cancel();
});


test('MP-08/MP-10/MP-11 a failed restoration recovers only after repeating its original proof',async()=>{
 let clock=100,restored=false,proofs=0,wakes=0;const fence=new CapturePresentationFence(()=>clock);
 const restore=async()=>{proofs++;if(!restored)throw Error('temporarily busy')};
 await assert.rejects(fence.run(async()=>{},restore),/temporarily busy/);assert.equal(fence.stable(1000),false);
 const cancel=fence.whenStable(()=>{wakes++});clock=200;await assert.rejects(fence.recoverFailed(),/temporarily busy/);assert.equal(fence.stable(1000),false);assert.equal(wakes,0);
 restored=true;clock=300;await Promise.all([fence.recoverFailed(),fence.recoverFailed()]);clock=302;await new Promise(resolve=>setTimeout(resolve,5));assert.equal(proofs,3,'concurrent starts share one proof');assert.equal(wakes,1);assert.equal(fence.stable(302),true);assert.equal(fence.stable(250),false,'pixels from the failed interval remain excluded');cancel();
});

// MP-08/MP-10/MP-11: restoration reuses the same event-owned document, never a stale context.
function ownedPresentation(connection) {
 let frame={connection,sessionId:'session',frame:{id:'top',loaderId:'doc'}};
 const owner={connection,targetsBySession:new Map([['session','target']]),focusWorldsByTarget:new Map([['target',{documentId:'doc',frameId:'top',connection,sessionId:'session',contextId:7}]]),currentPageFrame:()=>frame};
 return {owner,replace:()=>{frame={...frame};owner.focusWorldsByTarget.set('target',{documentId:'doc',frameId:'top',connection,sessionId:'session',contextId:8});}};
}
test('MP-08/MP-10/MP-11 warm screenshot restoration uses the live owner world without renderer discovery',async()=>{
 const sent=[],connection={send:async(method,params)=>{sent.push({method,params});if(method==='Page.captureScreenshot')return {data:'png'};if(method==='Runtime.evaluate')return {result:{value:true}};assert.fail('no renderer discovery: '+method);}};
 fenceScreenshotPresentation(connection);const {owner}=ownedPresentation(connection);connection.presentationFence.bindOwner(owner);
 assert.deepEqual(await connection.send('Page.captureScreenshot',{},'session'),{data:'png'});
 assert.deepEqual(sent.map(x=>x.method),['Page.captureScreenshot','Runtime.evaluate']);assert.equal(sent[1].params.contextId,7);
});
test('MP-11 same-loader owner replacement cannot prove restoration; recovery resolves the new trusted world',async()=>{
 const proof=deferred(),contexts=[];let first=true;
 const connection={send:async(method,params)=>{if(method==='Page.captureScreenshot')return {data:'png'};if(method==='Runtime.evaluate'){contexts.push(params.contextId);if(first){first=false;return proof.promise;}return {result:{value:true}};}assert.fail('no renderer discovery: '+method);}};
 fenceScreenshotPresentation(connection);const {owner,replace}=ownedPresentation(connection),fence=connection.presentationFence;fence.bindOwner(owner);
 const capture=connection.send('Page.captureScreenshot',{},'session');await new Promise(resolve=>setImmediate(resolve));replace();proof.resolve({result:{value:true}});
 await assert.rejects(capture,/presentation owner retired/);const old=fence.intervals[0].start;assert.equal(fence.stable(Date.now()+100),false);
 await fence.recoverFailed();assert.deepEqual(contexts,[7,8]);assert.equal(fence.stable(old),false);assert.equal(fence.stable(Date.now()+100),true);
});

test('MP-11 a foreign session, connection, document or invalid world uses fresh isolated discovery',async()=>{
 for(const kind of ['session','connection','document','context','world_session','world_connection','world_frame']){
  const sent=[],connection={send:async(method,params)=>{sent.push({method,params});if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'top'}}};if(method==='Page.createIsolatedWorld')return {executionContextId:3};if(method==='Runtime.evaluate')return {result:{value:true}};return {data:'png'};}};
  const {owner}=ownedPresentation(connection),frame=owner.currentPageFrame(),world=owner.focusWorldsByTarget.get('target');
  if(kind==='session')frame.sessionId='foreign';if(kind==='connection')frame.connection={};if(kind==='document')world.documentId='old';if(kind==='context')world.contextId=0;if(kind==='world_session')world.sessionId='foreign';if(kind==='world_connection')world.connection={};if(kind==='world_frame')world.frameId='foreign';
  fenceScreenshotPresentation(connection);connection.presentationFence.bindOwner(owner);await connection.send('Page.captureScreenshot',{},'session');
  assert.deepEqual(sent.map(x=>x.method),['Page.captureScreenshot','Page.getFrameTree','Page.createIsolatedWorld','Runtime.evaluate'],kind);assert.equal(sent.at(-1).params.contextId,3);
 }
});

test('MP-08/MP-10/MP-11 a first preview creates its world through the known document owner without querying its frame tree',async()=>{
 const sent=[],connection={send:async(method,params)=>{sent.push({method,params});if(method==='Page.captureScreenshot')return {data:'png'};if(method==='Page.createIsolatedWorld')return {executionContextId:9};if(method==='Runtime.evaluate')return {result:{value:true}};assert.fail('owner already knows the frame: '+method);}};
 const {owner}=ownedPresentation(connection);owner.focusWorldsByTarget.clear();let created=0;
 owner.ensureFocusWorld=async(c,s,target,frame)=>{assert.equal(c,connection);assert.equal(s,'session');assert.equal(frame.id,'top');created++;const result=await c.send('Page.createIsolatedWorld',{frameId:frame.id,worldName:'trusted-focus'},s),world={documentId:frame.loaderId,frameId:frame.id,connection:c,sessionId:s,contextId:result.executionContextId};owner.focusWorldsByTarget.set(target,world);return world;};
 fenceScreenshotPresentation(connection);connection.presentationFence.bindOwner(owner);await connection.send('Page.captureScreenshot',{},'session');
 assert.equal(created,1);assert.deepEqual(sent.map(x=>x.method),['Page.captureScreenshot','Page.createIsolatedWorld','Runtime.evaluate']);assert.equal(sent.at(-1).params.contextId,9);
 await connection.send('Page.captureScreenshot',{},'session');assert.equal(created,1,'subsequent restoration reuses the exact owner world');
});

test('MP-11 owner replacement while its first isolated world is created keeps presentation fenced',async()=>{
 const created=deferred();let evaluated=false;
 const connection={send:async method=>{if(method==='Page.captureScreenshot')return {data:'png'};if(method==='Runtime.evaluate')evaluated=true;assert.fail('no evaluation or discovery after replacement');}};
 const {owner,replace}=ownedPresentation(connection);owner.focusWorldsByTarget.clear();owner.ensureFocusWorld=()=>created.promise;
 fenceScreenshotPresentation(connection);const fence=connection.presentationFence;fence.bindOwner(owner);
 const capture=connection.send('Page.captureScreenshot',{},'session');await new Promise(resolve=>setImmediate(resolve));replace();created.resolve({documentId:'doc',contextId:9});
 await assert.rejects(capture,/presentation owner retired/);assert.equal(evaluated,false);assert.equal(fence.stable(Date.now()+100),false);
});
