// MP-08/MP-10/MP-11: the ordinary controller entry uses this layout mutation.
import test from 'node:test';import assert from 'node:assert/strict';
import {resizeBrowserPage} from './kernel-browser-resize.mjs';
import {KernelBrowserHost} from './kernel-browser-host.mjs';
const command={op:'resize',tab_id:'t',generation:1,width:900,height:600};
function fixture(){
 const calls=[],tab={tab_id:'t',target_id:'T'},old={width:1280,height:800},bounds={width:1280,height:887,windowState:'normal'};
 const stream={tab_id:'t',invalidate(){calls.push('stream_reset')}};
 const host={generation:1,tabs:new Map([['t',tab]]),displays:new Map([['s',stream]]),pageSize:()=>old,
  browser:{resolvePageTarget:async()=>({sessionId:'s',connection:{send:async(m,p)=>{calls.push([m,p]);if(m==='Browser.setWindowBounds')Object.assign(bounds,p.bounds);return {windowId:7,bounds:{...bounds}};}}})},
  chromeWindows:{retire:reason=>calls.push(reason)},closeCompositors:async()=>calls.push('close_source'),setLayout:async(t,owner,size)=>{calls.push(['layout',owner,size]);Object.assign(old,size)},stateSignal:{notify(){}},rebindWindows:async()=>calls.push('rebind')};
 return {host,tab,calls,stream};
}
test('MP-08/MP-10 resize changes native bounds and page layout without closing mirrors',async()=>{
 const {host,tab,calls,stream}=fixture();assert.deepEqual(await resizeBrowserPage(host,tab,command),{width:900,height:600});
 assert.equal(calls[1],'viewport_resize');assert.equal(calls[2],'close_source');
 assert.deepEqual(calls[3],['Browser.setWindowBounds',{windowId:7,bounds:{width:900,height:687}}]);
 assert.deepEqual(calls[4],['layout','resize',{width:900,height:600}]);assert.equal(stream.css_width,900);assert.equal(stream.css_height,600);
 const count=calls.length;await resizeBrowserPage(host,tab,command);assert.equal(calls.length,count+1,'same size reads the current window but produces no intermediate resizes');
});
for(const bad of [{width:0},{width:900.5},{height:NaN},{width:1921},{height:1081}])test('MP-08/MP-11 malformed or oversized resize fails before native mutation '+JSON.stringify(bad),async()=>{
 const {host,tab,calls}=fixture();await assert.rejects(resizeBrowserPage(host,tab,{...command,...bad}),/viewport between/);assert.deepEqual(calls,[]);
});
test('MP-08/MP-11 cancellation and retired target cannot move a replacement window',async()=>{
 const {host,tab,calls}=fixture(),abort=new AbortController();abort.abort();await assert.rejects(resizeBrowserPage(host,tab,command,abort.signal));assert.deepEqual(calls,[]);
 host.generation=2;await assert.rejects(resizeBrowserPage(host,tab,command),/target retired/);assert(!calls.some(c=>Array.isArray(c)&&c[0]==='Browser.setWindowBounds'));
});

test('MP-08/MP-10 the ordinary host browser entry routes the resize mutation',async()=>{
 const {host,tab}=fixture();Object.assign(host,{start:async()=>{},protection:{unknown:false},streams:new Map(),chromium:{},target:async command=>{assert.equal(command.generation,1);return tab},displayTarget:async command=>{assert.equal(command.generation,1);return tab},appAgent:()=>({})});
 assert.deepEqual(await KernelBrowserHost.prototype.requestNow.call(host,command),{width:900,height:600});
});
test('MP-08/MP-10 failed layout application leaves the last committed size retryable',async()=>{
 const proto=KernelBrowserHost.prototype,host={layouts:new Map(),layoutRevisions:new Map(),browser:{setLayout:async()=>{throw Error('layout unavailable')}},pageSize:proto.pageSize,noteLayout:proto.noteLayout,layoutRevision:proto.layoutRevision};
 await assert.rejects(proto.setLayout.call(host,{tab_id:'t',target_id:'T'},'resize',{width:900,height:600}),/layout unavailable/);assert.equal(host.layouts.size,0);assert.equal(host.layoutRevisions.size,0);
});
