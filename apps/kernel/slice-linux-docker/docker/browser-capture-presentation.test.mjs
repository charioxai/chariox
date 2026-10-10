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
