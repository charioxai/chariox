// MP-08/MP-10/MP-11: screenshot ownership uses the existing event authority; masking and cold checks remain shared.
import test from 'node:test';
import assert from 'node:assert/strict';
import {KernelBrowserHost} from './kernel-browser-host.mjs';
function setup(warm=true,replaceDuringCapture=false){
 const calls=[],tab={tab_id:'tab',target_id:'target',document_id:'doc'};let frame;
 const connection={send:async method=>{calls.push(method);if(method==='Page.getFrameTree')return {frameTree:{frame:{loaderId:'doc'}}};if(method==='Page.captureScreenshot'){if(replaceDuringCapture)frame={...frame};return {data:'cG5n'};}assert.fail('unexpected CDP');}};
 frame={connection,sessionId:'s',frame:{id:'top',loaderId:'doc'}};
 const browser={connection,currentPageFrame:warm?()=>frame:undefined,resolvePageTarget:async()=>{assert(!warm,'a warm capture never resolves the same target again');return {connection,sessionId:'s'};},fillTargets:new Map(),inputCapture:{run:async(_c,_s,read)=>read()}};
 const host={browser,generation:1,scales:new Map([['tab',1]]),viewport:()=>({device_scale_factor:1}),protection:{unknown:false,values:[],targets:[]},compositors:new Map(),sampleLane:()=>({run:(_kind,read)=>read()}),timing:()=>{}};
 return {host,tab,calls};
}
test('MP-08/MP-10/MP-11 a warm protected screenshot avoids duplicate target/frame discovery',async()=>{
 const {host,tab,calls}=setup();const shot=await KernelBrowserHost.prototype.screenshot.call(host,tab,null,true);
 assert.equal(shot.document_id,'doc');assert.deepEqual(shot.protected_regions,[]);assert.deepEqual(calls,['Page.captureScreenshot']);
});
test('MP-11 replacing the capture owner with the same loader refuses the screenshot',async()=>{
 const {host,tab}=setup(true,true);await assert.rejects(KernelBrowserHost.prototype.screenshot.call(host,tab,null,true),error=>error.code==='stale_document_reference');
});
test('MP-08/MP-11 an unknown screenshot owner retains fresh renderer checks before and after capture',async()=>{
 const {host,tab,calls}=setup(false);await KernelBrowserHost.prototype.screenshot.call(host,tab,null,true);
 assert.deepEqual(calls,['Page.getFrameTree','Page.captureScreenshot','Page.getFrameTree']);
});
