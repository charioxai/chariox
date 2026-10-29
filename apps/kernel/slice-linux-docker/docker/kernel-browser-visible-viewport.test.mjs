// MP-08/MP-10/MP-11: native publication reuses geometry only under existing document/window observers.
import test from 'node:test';
import { DisplaySessions } from './kernel-display-resumption.mjs';
import assert from 'node:assert/strict';
import {subscribeVisiblePage} from './kernel-browser-page-display.mjs';
import {NativeTabState,nativeWindowChangedBinding} from './kernel-browser-native-tab-state.mjs';
import {visiblePageGeometry} from './kernel-browser-visible-page.mjs';
import {BrowserCdpClient} from './browser-controller-cdp.mjs';
function fixture(){
 let reads=0,active=true,frame;
 const connection={isOpen:()=>true,subscribe:()=>()=>{},send:async method=>{assert.equal(method,'Page.getLayoutMetrics');if(++reads>1)throw new Error('loading renderer must not be re-queried');return{cssLayoutViewport:{clientWidth:1265,clientHeight:800},cssVisualViewport:{zoom:1,scale:1}};}};
 const state=new NativeTabState(connection);state.session='native';state.ready=true;frame={connection,sessionId:'page',frame:{loaderId:'doc'}};
 const tab={tab_id:'tab',target_id:'target',document_id:'doc'},window={role:'tabbed',xid:42,pid:42,scale:1,window_generation:1,browser_generation:1,bounds:{width:1280,height:800},page_rect:{left:0,top:87,width:1280,height:713},isCurrent:()=>true,isPageCurrent:async()=>active,isPageCurrentCached:()=>active};
 const browser={connection,viewportByTarget:new Map([['target',{key:'canonical',metrics:{}}]]),currentPageFrame:()=>frame};
 const host={browser,generation:1,protection:{unknown:false,targets:[],values:[]},tabs:new Map([['tab',tab]]),displays:new Map(),mirrorTarget:async()=>tab,tabGroups:{nativeStateGuard:()=>state.guard?.()??null},chromium:{display:{},scale:1,browserPid:42},chromeWindows:{current:()=>({tabbed:window}),inWindowOverNow:async()=>false},timing:()=>{},armDisplayExpiry:()=>{}};
 class Capture{constructor(options){this.options=options;this.region=visiblePageGeometry(options.window,options.viewport)}async start(){return this}async close(){}}
 const command={op:'display_subscribe',tab_id:'tab',generation:1,codecs:['png','avc1.420033'],bitrate:8000000,device_scale_factor:1};
 return{host,state,command,Capture,reads:()=>reads,inactive:()=>{active=false;},replace:()=>{frame={...frame};}};
}
test('MP-08/MP-10/MP-11 known native viewport publishes without a renderer query per frame',async()=>{
 const f=fixture();await subscribeVisiblePage(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});const stream=[...f.host.displays.values()][0];
 for(let i=0;i<3;i++)assert.equal(await stream.regionCurrent(),true);assert.equal(f.reads(),1);f.inactive();assert.equal(await stream.regionCurrent(),false);await stream.close();
});
for(const change of ['resize','zoom','document','metrics','observer-detach','browser-closed'])test('MP-11 cached visible geometry retires on '+change,async()=>{
 const f=fixture();await subscribeVisiblePage(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});const stream=[...f.host.displays.values()][0];
 if(change==='resize')f.state.onCdp({method:'Page.frameResized',sessionId:'page'});
 if(change==='zoom')f.state.onCdp({method:'Runtime.bindingCalled',sessionId:'native',params:{name:nativeWindowChangedBinding,payload:''}});
 if(change==='observer-detach')f.state.detach();if(change==='browser-closed')f.host.browser.connection.isOpen=()=>false;
 if(change==='document')f.replace();if(change==='metrics')f.host.browser.viewportByTarget.set('target',{key:'canonical',metrics:{}});
 assert.equal(await stream.regionCurrent(),false);assert.equal(stream.regionFrameCurrent(),false);assert.equal(f.reads(),1);await stream.close();
});

test('MP-11 a window bounds event keeps the lease and re-reads no layout',async()=>{
 const f=fixture();await subscribeVisiblePage(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});const stream=[...f.host.displays.values()][0];
 f.state.onCdp({method:'Runtime.bindingCalled',sessionId:'native',params:{name:nativeWindowChangedBinding,payload:'bounds'}});
 assert.equal(await stream.regionCurrent(),true);assert.equal(stream.regionFrameCurrent(),true);assert.equal(f.reads(),1);await stream.close();
});

test('MP-11 a viewport event during the first renderer measurement refuses the lease',async()=>{
 const f=fixture(),ready=Promise.withResolvers();f.host.browser.connection.send=()=>ready.promise;
 const pending=subscribeVisiblePage(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});await new Promise(setImmediate);f.state.onCdp({method:'Page.frameResized',sessionId:'page'});
 ready.resolve({cssLayoutViewport:{clientWidth:1265,clientHeight:800},cssVisualViewport:{zoom:1,scale:1}});await assert.rejects(pending);assert.equal(f.host.displays.size,0);
});

for(const cold of ['observer','frame','metrics'])test('MP-11 cold '+cold+' retains fresh renderer geometry checks',async()=>{
 const f=fixture();if(cold==='observer')f.state.ready=false;
 if(cold==='frame'){f.host.browser.currentPageFrame=()=>null;f.host.browser.resolvePageTarget=async()=>({connection:f.host.browser.connection,sessionId:'page'});}
 if(cold==='metrics')f.host.browser.viewportByTarget.clear();
 await assert.rejects(subscribeVisiblePage(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture}),/loading renderer must not be re-queried/);assert.equal(f.reads(),2);assert.equal(f.host.displays.size,0);
});

test('MP-08/MP-10/MP-11 a viewport configured by the browser client keeps the native guard without renderer reads',async()=>{
 const f=fixture(),browser=new BrowserCdpClient();
 browser.connection=f.host.browser.connection;browser.canonical={css_width:1280,css_height:800,device_scale_factor:1,desktop_pixel_width:1280,desktop_pixel_height:800};
 browser.sessionsByTarget.set('target','page');browser.pageFramesByTarget.set('target',{connection:browser.connection,sessionId:'page',frame:{loaderId:'doc'}});browser.documentIdsByTarget.set('target','doc');
 const send=f.host.browser.connection.send;f.host.browser.connection.close=async()=>{};f.host.browser.connection.send=async(method,...rest)=>method==='Page.getLayoutMetrics'?send(method,...rest):{};
 try{
  await browser.applyLayout('target');assert.equal(typeof browser.viewportByTarget.get('target')?.key,'string');
  f.host.browser=browser;
  await subscribeVisiblePage(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});const stream=[...f.host.displays.values()][0];
  for(let i=0;i<3;i++)assert.equal(await stream.regionCurrent(),true);assert.equal(f.reads(),1);
  f.state.onCdp({method:'Page.frameResized',sessionId:'page'});assert.equal(await stream.regionCurrent(),false);assert.equal(f.reads(),1);await stream.close();
 }finally{await browser.close();}
});
// MP-08/MP-10/MP-11 (P1): a Tab display keeps its lease for the Tab's later documents on the same session.
test('MP-08/MP-10/MP-11 a Tab display lease follows the committed document and unparks on the commit',async()=>{
 const f=fixture();let committed={document_id:'doc',sessionId:'page'},follow=null;const parks=[];
 f.host.browser.committedPageDocument=()=>({connection:f.host.browser.connection,...committed});
 class Capture extends f.Capture{onDocument(fn){follow=fn;return()=>{};}park(parked){parks.push(parked);}}
 const result=await subscribeVisiblePage(f.host,{...f.command,tab_display:true},'terminal:a',undefined,{Capture});
 const stream=f.host.displays.get(result.subscription_id);
 assert.equal(result.tab_display,true);assert.equal(stream.tabDisplay,true);
 stream.park(true);assert.deepEqual(parks,[true]);
 // Navigation started: the frame record is suspended, the page still shows the committed document.
 f.host.browser.currentPageFrame=()=>null;
 assert.equal(await stream.regionCurrent(),true);
 // Commit: the source follows, the lease follows the source and the display unparks.
 committed={document_id:'doc2',sessionId:'page'};follow('doc2');
 assert.equal(stream.regionLease.document_id,'doc2');assert.equal(await stream.regionCurrent(),true);assert.equal(stream.regionFrameCurrent(),true);
 assert.deepEqual(parks,[true,false]);assert.equal(stream.parked,false);assert.equal(f.reads(),1,'no renderer layout read per document');
 // A commit the source did not follow (another session) retires the binding.
 committed={document_id:'doc3',sessionId:'other'};
 assert.equal(await stream.regionCurrent(),false);
 await stream.close();
});
// Review (PR 1166): resume replies omitted tab_display, so a resumed viewer rebuilt a plain binding and refused the
// Tab's next document and its park.
test('MP-08/MP-10/MP-11 a resumed Tab display stays a Tab display: it parks and follows the next commit',async()=>{
 const f=fixture();let committed={document_id:'doc',sessionId:'page'},follow=null;const parks=[];
 f.host.browser.committedPageDocument=()=>({connection:f.host.browser.connection,...committed});
 class Capture extends f.Capture{onDocument(fn){follow=fn;return()=>{};}park(parked){parks.push(parked);}}
 const result=await subscribeVisiblePage(f.host,{...f.command,tab_display:true},'terminal:a',undefined,{Capture});
 const stream=f.host.displays.get(result.subscription_id),source={},binding=()=>source;
 f.host.armDisplayExpiry??=()=>{};
 const sessions=new DisplaySessions(f.host,{id:()=>'resumed'});
 await sessions.suspend(result.subscription_id,stream.observed_by,stream.generation??f.host.generation,binding);
 const resumed=await sessions.resume(result.subscription_id,stream.observed_by,stream.generation??f.host.generation,binding);
 assert.equal(resumed.tab_display,true);assert.equal(f.host.displays.get('resumed'),stream);
 stream.park(true);committed={document_id:'doc2',sessionId:'page'};follow('doc2');
 assert.deepEqual(parks,[true,false]);assert.equal(stream.regionLease.document_id,'doc2');
 await stream.close();
});
test('MP-11 without the Tab display binding a commit still retires the page display',async()=>{
 const f=fixture();let committed={document_id:'doc',sessionId:'page'};
 f.host.browser.committedPageDocument=()=>({connection:f.host.browser.connection,...committed});
 const result=await subscribeVisiblePage(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const stream=f.host.displays.get(result.subscription_id);assert.equal(result.tab_display,undefined);
 committed={document_id:'doc2',sessionId:'page'};f.replace();
 assert.equal(await stream.regionCurrent(),false);await stream.close();
});
