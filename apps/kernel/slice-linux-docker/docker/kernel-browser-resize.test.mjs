// MP-08/MP-10/MP-11: the ordinary controller entry uses this layout mutation.
import test from 'node:test';import assert from 'node:assert/strict';
import {resizeBrowserPage} from './kernel-browser-resize.mjs';
import {KernelBrowserHost} from './kernel-browser-host.mjs';
const command={op:'resize',tab_id:'t',generation:1,width:900,height:600};
function fixture(){
 const calls=[],tab={tab_id:'t',target_id:'T'},old={width:1280,height:800},bounds={width:1280,height:888,windowState:'normal'};
 const stream={tab_id:'t',invalidate(){calls.push('stream_reset')}};
 const host={generation:1,tabs:new Map([['t',tab]]),displays:new Map([['s',stream]]),pageSize:()=>old,
  browser:{resolvePageTarget:async()=>({sessionId:'s',connection:{send:async(m,p)=>{calls.push([m,p]);if(m==='Browser.setWindowBounds')Object.assign(bounds,p.bounds);return {windowId:7,bounds:{...bounds}};}}})},
  chromium:{desktop:{binding:()=>({})}},chromeWindows:{resolve:async()=>{},current:()=>({tabbed:{scale:1,page_rect:{width:old.width,height:old.height}}}),retire:reason=>calls.push(reason)},closeCompositors:async()=>calls.push('close_source'),setLayout:async(t,owner,size)=>{calls.push(['layout',owner,size]);Object.assign(old,size)},stateSignal:{notify(){}},rebindWindows:async()=>calls.push('rebind')};
 return {host,tab,calls,stream};
}
test('MP-08/MP-10 resize changes native bounds and page layout without closing mirrors',async()=>{
 const {host,tab,calls,stream}=fixture();assert.deepEqual(await resizeBrowserPage(host,tab,command),{width:900,height:600});
 assert.equal(calls[1],'viewport_resize');assert.equal(calls[2],'close_source');assert.deepEqual(host.windowLayout,{width:900,height:600});
 assert.deepEqual(calls.find(call=>Array.isArray(call)&&call[0]==='Browser.setWindowBounds'),['Browser.setWindowBounds',{windowId:7,bounds:{width:900,height:688}}]);
 assert.deepEqual(calls.find(call=>Array.isArray(call)&&call[0]==='layout'),['layout','resize',{width:900,height:600}]);assert.equal(stream.css_width,900);assert.equal(stream.css_height,600);
 const count=calls.length;await resizeBrowserPage(host,tab,command);assert.equal(calls.length,count+1,'same size reads the current window but produces no intermediate resizes');
});
test('MP-08/MP-10 resize refreshes a cached page rectangle before measuring the native window inset',async()=>{
 const {host,tab,calls}=fixture();let refreshed=false;
 host.chromeWindows.current=()=>({tabbed:{scale:1,page_rect:refreshed?{width:1280,height:800}:{width:1600,height:1000}}});
 host.chromeWindows.resolve=async()=>{refreshed=true;assert(!calls.some(call=>Array.isArray(call)&&call[0]==='Browser.setWindowBounds'),'refresh precedes the resize mutation')};
 assert.deepEqual(await resizeBrowserPage(host,tab,command),{width:900,height:600});
 assert(refreshed);assert.deepEqual(calls.find(call=>Array.isArray(call)&&call[0]==='Browser.setWindowBounds'),['Browser.setWindowBounds',{windowId:7,bounds:{width:900,height:688}}]);
});
test('MP-08/MP-11 a target retired while refreshing geometry cannot resize its replacement',async()=>{
 const {host,tab,calls}=fixture();host.chromeWindows.resolve=async()=>{host.generation=2};
 await assert.rejects(resizeBrowserPage(host,tab,command),/target retired/);
 assert(!calls.some(call=>Array.isArray(call)&&call[0]==='Browser.setWindowBounds'));
});
for(const bad of [{width:0},{width:900.5},{height:NaN},{width:1921},{height:1081}])test('MP-08/MP-11 malformed or oversized resize fails before native mutation '+JSON.stringify(bad),async()=>{
 const {host,tab,calls}=fixture();await assert.rejects(resizeBrowserPage(host,tab,{...command,...bad}),/viewport between/);assert.deepEqual(calls,[]);
});
test('MP-08/MP-11 cancellation and retired target cannot move a replacement window',async()=>{
 const {host,tab,calls}=fixture(),abort=new AbortController();abort.abort();await assert.rejects(resizeBrowserPage(host,tab,command,abort.signal));assert.deepEqual(calls,[]);
 host.generation=2;await assert.rejects(resizeBrowserPage(host,tab,command),/target retired/);assert(!calls.some(c=>Array.isArray(c)&&c[0]==='Browser.setWindowBounds'));
});

test('MP-08/MP-10 the ordinary host browser entry routes the resize mutation',async()=>{
 const {host,tab}=fixture();Object.assign(host,{start:async()=>{},protection:{unknown:false},streams:new Map(),target:async command=>{assert.equal(command.generation,1);return tab},displayTarget:async command=>{assert.equal(command.generation,1);return tab},appAgent:()=>({})});
 assert.deepEqual(await KernelBrowserHost.prototype.requestNow.call(host,command),{width:900,height:600});
});
test('MP-08/MP-10 failed layout application leaves the last committed size retryable',async()=>{
 const proto=KernelBrowserHost.prototype,host={layouts:new Map(),layoutRevisions:new Map(),browser:{setLayout:async()=>{throw Error('layout unavailable')}},pageSize:proto.pageSize,noteLayout:proto.noteLayout,layoutRevision:proto.layoutRevision};
 await assert.rejects(proto.setLayout.call(host,{tab_id:'t',target_id:'T'},'resize',{width:900,height:600}),/layout unavailable/);assert.equal(host.layouts.size,0);assert.equal(host.layoutRevisions.size,0);
});

test('MP-08/MP-10 resize refuses odd video dimensions before mutating the window',async()=>{const {host,tab,calls}=fixture();await assert.rejects(resizeBrowserPage(host,tab,{...command,width:901}),/even/);assert.deepEqual(calls,[])});

// A Tab opened in the resized window starts at the window's size: the person's view needs no second resize (which retires the band under a click).
test('MP-08/MP-10 resize lays out every Tab of the shared window',async()=>{
 const {host,tab,calls}=fixture();const other={tab_id:'u',target_id:'U'};host.tabs.set('u',other);
 await resizeBrowserPage(host,tab,command);
 assert.deepEqual(calls.filter(c=>Array.isArray(c)&&c[0]==='layout').map(c=>c[2]),[{width:900,height:600},{width:900,height:600}]);
});

// #1073 P2: Tab layouts commit one by one; a failure after the first must not let a retry of the same size take the
// same-size shortcut while the other Tab, the streams and the window bindings are still at the old size.
test('MP-08/MP-10 a retried resize completes a partially applied one across every Tab, stream and binding',async()=>{
 const proto=KernelBrowserHost.prototype,calls=[],bounds={width:1280,height:888,windowState:'normal'};
 const tabs=new Map([['t',{tab_id:'t',target_id:'T'}],['u',{tab_id:'u',target_id:'U'}]]);
 const streams=[{tab_id:'t',invalidate(){calls.push('reset t')}},{tab_id:'u',invalidate(){calls.push('reset u')}}];
 let failU=1;
 const host={generation:1,tabs,displays:new Map(streams.map((s,i)=>[String(i),s])),layouts:new Map(),layoutRevisions:new Map(),windowLayout:null,
  pageSize:proto.pageSize,noteLayout:proto.noteLayout,layoutRevision:proto.layoutRevision,setLayout:proto.setLayout,
  browser:{setLayout:async target=>{if(target==='U'&&failU-->0)throw Error('layout unavailable');calls.push('layout '+target)},
   resolvePageTarget:async()=>({sessionId:'s',connection:{send:async(m,p)=>{if(m==='Browser.setWindowBounds')Object.assign(bounds,p.bounds);return {windowId:7,bounds:{...bounds}};}}})},
  chromium:{desktop:{binding:()=>({})}},chromeWindows:{resolve:async()=>{},current:()=>({tabbed:{scale:1,page_rect:{width:bounds.width,height:bounds.height-88}}}),retire:()=>{}},
  closeCompositors:async()=>{},stateSignal:{notify(){}},rebindWindows:async()=>calls.push('rebind')};
 host.browser.resolvePageTarget=host.browser.resolvePageTarget.bind(host.browser);
 await assert.rejects(resizeBrowserPage(host,tabs.get('t'),command),/layout unavailable/);
 assert.deepEqual(host.pageSize('t'),{width:900,height:600},'the first Tab already committed');
 assert.deepEqual(host.pageSize('u'),{width:1280,height:800});assert(!calls.includes('rebind'));
 calls.length=0;
 assert.deepEqual(await resizeBrowserPage(host,tabs.get('t'),command),{width:900,height:600});
 for(const id of ['t','u'])assert.deepEqual(host.pageSize(id),{width:900,height:600},`Tab ${id} at the window size`);
 for(const s of streams)assert.deepEqual([s.css_width,s.css_height],[900,600],`stream of ${s.tab_id} at the window size`);
 assert(calls.includes('reset t')&&calls.includes('reset u')&&calls.includes('rebind'),'streams invalidated and windows re-bound: '+calls);
 // Once complete, the same size is a no-op again.
 calls.length=0;await resizeBrowserPage(host,tabs.get('t'),command);assert.deepEqual(calls,[]);
});

test('MP-08/MP-10 resize waits for a native fit to settle before measuring shared window geometry',async()=>{
 const {withWindowGeometry}=await import('./kernel-browser-window-geometry.mjs');
 const {fitNativeViewport}=await import('./kernel-browser-native-viewport.mjs');
 const {host,tab}=fixture();let bounds={width:1280,height:888,windowState:'normal'},fitting=false,release;
 const held=new Promise(resolve=>{release=resolve}),calls=[];
 const connection={async send(method,params){
  calls.push(method);
  if(method==='Browser.setWindowBounds')bounds={...bounds,...params.bounds};
  return {windowId:7,bounds:{...bounds}};
 }};
 host.browser.resolvePageTarget=async()=>({connection,sessionId:'s'});
 host.chromeWindows.current=()=>({tabbed:{scale:1,page_rect:{width:1280,height:800}}});
 const fit=withWindowGeometry(connection,()=>fitNativeViewport({connection,windowId:7,sessionId:'s',width:1280,height:800,wait:async()=>{},measure:async()=>{fitting=true;await held;return {width:1280,height:800}}}));
 while(!fitting)await new Promise(resolve=>setImmediate(resolve));
 const before=calls.length,resize=resizeBrowserPage(host,tab,command);resize.catch(()=>{});
 await new Promise(resolve=>setImmediate(resolve));
 try{assert.equal(calls.length,before,'resize must not measure a fitting window')}finally{release()}
 await fit;assert.deepEqual(await resize,{width:900,height:600});
});

test('MP-08/MP-10 compositor shutdown runs outside the window geometry lane',async()=>{
 const {withWindowGeometry}=await import('./kernel-browser-window-geometry.mjs');
 const {host,tab}=fixture(),{connection}=await host.browser.resolvePageTarget(tab.target_id);
 host.closeCompositors=()=>withWindowGeometry(connection,async()=>{});
 const timeout=new Promise((_,reject)=>{const timer=setTimeout(()=>reject(Error('geometry lane deadlocked during close')),1000);timer.unref()});
 await Promise.race([resizeBrowserPage(host,tab,command),timeout]);
});

test('MP-08/MP-11 a native capture retired while waiting cannot fit a stale layout',async()=>{
 const {withWindowGeometry}=await import('./kernel-browser-window-geometry.mjs');
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 let release,current=true,calls=0;
 const connection={send:async()=>{calls++;return {}}};
 const held=withWindowGeometry(connection,()=>new Promise(resolve=>{release=resolve}));
 while(!release)await new Promise(resolve=>setImmediate(resolve));
 const capture={connection,pageWindow:null,valid:()=>current};
 const read=LinuxCapture.prototype.prepareReadback.call(capture);read.catch(()=>{});
 current=false;release();await held;
 await assert.rejects(read,/native attestation retired/);assert.equal(calls,0);
});

for(const resized of [false,true])test('MP-08/MP-10 headless resize measures the shared window rather than a Tab layout: '+(resized?'resized':'launch'),async()=>{
 const tab={tab_id:'new',target_id:'NEW'},bounds={width:resized?506:1280,height:resized?709:887,windowState:'normal'},layouts=[];
 const connection={send:async(method,params)=>{if(method==='Page.getLayoutMetrics')return{cssLayoutViewport:{clientWidth:resized?1280:506,clientHeight:resized?800:622}};if(method==='Browser.setWindowBounds')Object.assign(bounds,params.bounds);return{windowId:7,bounds:{...bounds}}}};
 const host={start:async()=>{},generation:1,protection:{unknown:false},streams:new Map(),target:async()=>tab,displayTarget:async()=>tab,appAgent:()=>({}),tabs:new Map([['new',tab]]),displays:new Map(),windowLayout:resized?{width:506,height:622}:null,pageSize:()=>({width:1280,height:800}),browser:{resolvePageTarget:async()=>({connection,sessionId:'s'})},chromium:{desktop:{binding:()=>null}},chromeWindows:{retire:()=>{}},closeCompositors:async()=>{},setLayout:async(t,owner,size)=>layouts.push(size),stateSignal:{notify(){}},rebindWindows:async()=>{}};
 assert.deepEqual(await KernelBrowserHost.prototype.requestNow.call(host,{op:'resize',tab_id:'new',generation:1,width:900,height:600}),{width:900,height:600});
 assert.equal(bounds.width,900);assert.equal(bounds.height,687);assert.deepEqual(layouts,[{width:900,height:600}]);
});
