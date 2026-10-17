// MP-08/MP-10/MP-11: a whole-page viewer gets only the owned window's visible page.
import test from 'node:test';
import assert from 'node:assert/strict';
import {VisiblePageCapture,visiblePageGeometry} from './kernel-browser-visible-page.mjs';
import {encodePng,decodePng,displayMaskRegions} from './kernel-browser-pixels.mjs';
const binding=(scale=1)=>({role:'tabbed',xid:42,scale,bounds:{width:1280*scale,height:800*scale},page_rect:{left:0,top:87*scale,width:1280*scale,height:713*scale},isCurrent:()=>true});
for(const scale of [1,2])test(`MP-08/MP-10/MP-11 visible page capture never resizes the owned desktop at DPR${scale}`,async()=>{
 const window=binding(scale),r=visiblePageGeometry(window);
 assert.equal(r.width,1280*scale);assert.equal(r.height,713*scale-(713*scale)%2);
 assert.equal(r.window_region.top,87*scale);assert.equal(r.window_region.height,800*scale);
 const source=new VisiblePageCapture({window,viewport:{clientWidth:1280,clientHeight:800,zoom:1,scale:1},scale,validateCurrent:async()=>true,connection:{send:()=>assert.fail('capture must not resize or activate a window')}});
 source.valid=()=>true;
 assert.deepEqual(await source.prepareReadback(),{width:r.width,height:r.height,window_region:r.window_region});
 source.validateCurrent=async()=>false;await assert.rejects(source.prepareReadback(),/retired/);
});
test('MP-08/MP-10/MP-11 visible page reference keeps protected pixels and their masks',async()=>{
 const pixels=Buffer.alloc(1280*800*4,255);pixels.fill(0,4*(1280*700+10),4*(1280*700+14));
 const shot={data_base64:encodePng(1280,800,pixels),[displayMaskRegions]:[{x:10,y:700,width:4,height:20}]};
 const source=new VisiblePageCapture({window:binding(),viewport:{clientWidth:1280,clientHeight:800,zoom:1,scale:1},scale:1,screenshot:async()=>shot});
 const cropped=await source.screenshot(),r=decodePng(cropped.data_base64,1);
 assert.equal(r.height,712);assert.deepEqual(cropped[displayMaskRegions],[{x:10,y:700,width:4,height:12}]);
 assert.deepEqual(r.pixels.subarray(4*(1280*700+10),4*(1280*700+14)),Buffer.alloc(16));
});
test('MP-11 visible page refuses missing, overflowing or auxiliary window geometry',()=>{
 for(const change of [w=>w.role='auxiliary',w=>w.page_rect.top=0,w=>w.page_rect.height=800,w=>w.page_rect.left=-1,w=>w.page_rect.width=1281,w=>w.xid=1,w=>w.isCurrent=()=>false]){
  const w=binding();change(w);assert.throws(()=>visiblePageGeometry(w));
 }
});

test('MP-08/MP-10/MP-11 visible-page lease uses the shared region pump and retires before delivery',async()=>{
 const {subscribeVisiblePage}=await import('./kernel-browser-page-display.mjs');
 let active=true,closed=0,covered=false,checkedRect;
 const tab={tab_id:'t',target_id:'target',document_id:'d'},window={...binding(),window_generation:2,browser_generation:1,pid:42,isPageCurrent:async target=>target==='target'&&active};
 const host={generation:1,protection:{unknown:false},tabs:new Map([['t',tab]]),displays:new Map(),displayTarget:async()=>tab,
  chromium:{display:{},scale:1,browserPid:42},chromeWindows:{current:()=>({tabbed:window}),inWindowOverNow:async rect=>{checkedRect=rect;return covered;}},arbiter:{isActiveTab:async()=>active},
  browser:{resolvePageTarget:async()=>({connection:{send:async()=>({cssLayoutViewport:{clientWidth:1280,clientHeight:800},cssVisualViewport:{zoom:1,scale:1}})},sessionId:'s'})},timing:()=>{},armDisplayExpiry:()=>{}};
 class Capture {constructor(options){this.options=options;this.region=visiblePageGeometry(options.window,options.viewport);}async start(){return this;}async close(){closed++;}}
 const command={op:'display_subscribe',tab_id:'t',generation:1,codecs:['png','avc1.420033'],bitrate:8_000_000,device_scale_factor:2};
 active=false;await assert.rejects(subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture}));assert.equal(host.displays.size,0);
 active=true;const result=await subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture});
 assert.equal(result.css_height,712);assert.equal(result.device_scale_factor,1);assert.equal(result.source_kind,'region');assert.equal(result.document_id,'d');
 const stream=[...host.displays.values()][0];assert.equal(await stream.regionCurrent(),true);
 assert.equal(await stream.regionSource.options.occluded(),false);covered=true;assert.equal(await stream.regionSource.options.occluded(),true);
 assert.deepEqual(checkedRect,{left:0,top:87,width:1280,height:712});
 active=false;assert.equal(await stream.regionCurrent(),false);await stream.close();assert.equal(host.displays.size,0);assert.equal(closed,1);
 active=true;class Retired extends Capture {async start(){active=false;return this;}}
 await assert.rejects(subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture:Retired}));assert.equal(host.displays.size,0);assert.equal(closed,2);
});

test('MP-11 visible-page input keeps ordinary dispatch and the final live crop guard',async()=>{
 const {inputVisiblePage}=await import('./kernel-browser-page-display.mjs');
 let current=true,request;
 const stream={subscription_id:'crop',tab_id:'t',generation:1,observed_by:'terminal:a',expires:Date.now()+60000,
  regionTarget:{kind:'visible_page'},regionLease:{document_id:'d'},regionWindow:{isCurrent:()=>true},
  regionSource:{attested:true,valid:()=>true},css_width:1280,css_height:712,regionCurrent:async()=>current};
 const host={generation:1,displays:new Map([['crop',stream]]),tabs:new Map([['t',{tab_id:'t',document_id:'d'}]]),request:async(command,options)=>{request={command,options};}};
 const command={op:'input',tab_id:'t',generation:1,document_id:'d',binding_id:'crop',sequence:2,input:{kind:'click',x:12,y:700}};
 await inputVisiblePage(host,stream,command,'terminal:a');
 assert.equal(request.command.binding_id,undefined);assert.equal(request.command.document_id,'d');assert.equal(request.command.observed_by,'terminal:a');
 assert.equal(request.options.nativeDisplay,stream);
 await request.options.inputGuard();current=false;await assert.rejects(request.options.inputGuard());
 current=true;const guard=request.options.inputGuard;host.displays.delete('crop');await assert.rejects(guard());host.displays.set('crop',stream);
 const original=stream.regionCurrent;stream.regionCurrent=async()=>{host.displays.delete('crop');return true};await assert.rejects(guard());host.displays.set('crop',stream);stream.regionCurrent=original;
 for(const patch of [{observed_by:'terminal:b'},{suspended:true},{expires:0},{generation:0}]){const saved={...stream};Object.assign(stream,patch);await assert.rejects(guard());Object.assign(stream,saved);}
 for(const input of [{kind:'click',x:12,y:712},{kind:'drag',x:12,y:700,to_x:12,to_y:799},{kind:'mirror'}]){
  request=undefined;await assert.rejects(inputVisiblePage(host,stream,{...command,input},'terminal:a'));assert.equal(request,undefined);
 }
});

test('MP-08/MP-10/MP-11 visible page crops browser scrollbars out of the source, without an RGB exemption',()=>{
 const r=visiblePageGeometry(binding(),{clientWidth:1265,clientHeight:800,zoom:1,scale:1});
 assert.equal(r.width,1264);assert.equal(r.height,712);assert.equal(r.window_region.left,0);assert.equal(r.window_region.top,87);
 for(const viewport of [{clientWidth:0,clientHeight:800,zoom:1,scale:1},{clientWidth:1280,clientHeight:800,zoom:2,scale:1},{clientWidth:1280,clientHeight:800,zoom:1,scale:2}])assert.throws(()=>visiblePageGeometry(binding(),viewport));
});


test('MP-08/MP-10/MP-11 visible-page screenshot requires a live same-observer crop and keeps protection metadata',async()=>{
 const {captureVisiblePage}=await import('./kernel-browser-page-display.mjs');
 let live=true,policy={unknown:false},captured=0,change=false;
 const tab={tab_id:'t',document_id:'d'},stream={subscription_id:'crop',regionTarget:{kind:'visible_page'},observed_by:'terminal:a',tab_id:'t',generation:1,
  regionLease:{document_id:'d',isAlive:()=>live},regionSource:{valid:()=>live},regionWindow:{isCurrent:()=>live},regionCurrent:async()=>live,
  css_width:1264,css_height:712,device_scale_factor:1};
 const host={generation:1,protection:policy,displays:new Map([['crop',stream]]),displayScreenshot:async(t,clip)=>{
  captured++;assert.equal(t,tab);assert.deepEqual(clip,{x:0,y:0,width:1264,height:712,scale:1});
  if(change)host.protection={unknown:false};return {width:1264,height:712,[displayMaskRegions]:[{x:12,y:6,width:20,height:10}]};
 }};
 const frame={width:1264,height:712};
 assert.equal((await captureVisiblePage(host,tab,frame,'terminal:a')).protected_regions.length,1);assert.equal(captured,1);
 for(const args of [[frame,'terminal:b'],[{width:1264,height:800},'terminal:a']]){
  await assert.rejects(captureVisiblePage(host,tab,...args));assert.equal(captured,1);
 }
 live=false;await assert.rejects(captureVisiblePage(host,tab,frame,'terminal:a'));assert.equal(captured,1);
 live=true;change=true;await assert.rejects(captureVisiblePage(host,tab,frame,'terminal:a'));assert.equal(captured,2);
 host.protection=policy;change=false;stream.regionCurrent=async()=>{host.displays.clear();return true;};
 await assert.rejects(captureVisiblePage(host,tab,frame,'terminal:a'));assert.equal(captured,2);
});

// MP-08/MP-10/MP-11: after the video closes, the admitted mirror still names the same crop.
test('MP-08/MP-10/MP-11 mirrored visible-page capture survives video close with fresh kernel geometry',async()=>{
 const {captureVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const policy={unknown:false},tab={tab_id:'t',target_id:'target',document_id:'d'};let live=true,captured=0,change=false;
 const window={...binding(),isCurrent:()=>live,isPageCurrent:async target=>target==='target'&&live};
 const mirror={tab_id:'t',scope:'terminal:a',document_id:'d',policy,wire:2,navigationPreview:true,expires:Date.now()+60000};
 const host={generation:1,protection:policy,chromium:{policy:'managed'},displays:new Map(),mirror:{streams:new Map([['mirror',mirror]])},
  tabs:new Map([['t',tab]]),scales:new Map([['t',1]]),chromeWindows:{current:()=>({tabbed:window})},
  browser:{resolvePageTarget:async()=>({connection:{send:async method=>{assert.equal(method,'Page.getLayoutMetrics');return{cssLayoutViewport:{clientWidth:1265,clientHeight:800},cssVisualViewport:{scale:1,zoom:1}}}},sessionId:'s'})},
  displayScreenshot:async(t,clip)=>{captured++;assert.equal(t,tab);assert.deepEqual(clip,{x:0,y:0,width:1264,height:712,scale:1});if(change)live=false;return{width:1264,height:712,[displayMaskRegions]:[{x:1,y:2,width:3,height:4}]}}};
 const frame={width:1264,height:712};assert.equal((await captureVisiblePage(host,tab,frame,'terminal:a')).protected_regions.length,1);
 for(const [f,scope]of [[frame,'terminal:b'],[{width:1264,height:800},'terminal:a']])await assert.rejects(captureVisiblePage(host,tab,f,scope));
 assert.equal(captured,1);mirror.expires=0;await assert.rejects(captureVisiblePage(host,tab,frame,'terminal:a'));mirror.expires=Date.now()+60000;
 mirror.policy={unknown:false};await assert.rejects(captureVisiblePage(host,tab,frame,'terminal:a'));mirror.policy=policy;
 change=true;await assert.rejects(captureVisiblePage(host,tab,frame,'terminal:a'));assert.equal(captured,2);
});


test('MP-08/MP-10/MP-11 visible-page startup shares the mirror document owner while stored state catches up',async()=>{
 const {subscribeVisiblePage}=await import('./kernel-browser-page-display.mjs');const connection={send:async()=>({cssLayoutViewport:{clientWidth:1265,clientHeight:800},cssVisualViewport:{zoom:1,scale:1}})};let proof={connection,sessionId:'s',frame:{loaderId:'new'}};
 const stored={tab_id:'t',target_id:'target',document_id:'old'},tab={...stored,document_id:'new'};
 const window={...binding(),window_generation:2,browser_generation:1,pid:42,isPageCurrent:async()=>true};
 const host={generation:1,protection:{unknown:false},tabs:new Map([['t',stored]]),displays:new Map(),
  displayTarget:async()=>{throw Error('renderer document query is busy')},mirrorTarget:async()=>tab,
  chromium:{display:{},scale:1,browserPid:42},chromeWindows:{current:()=>({tabbed:window})},
  browser:{currentPageFrame:()=>proof,resolvePageTarget:async()=>{throw Error('cold target query is busy')}},timing:()=>{},armDisplayExpiry:()=>{}};
 class Capture {constructor(options){this.options=options;this.region=visiblePageGeometry(options.window,options.viewport)}async start(){return this}async close(){}}
 const result=await subscribeVisiblePage(host,{op:'display_subscribe',tab_id:'t',generation:1,codecs:['png','avc1.420033'],bitrate:8000000,device_scale_factor:1},'terminal:a',undefined,{Capture});
 assert.equal(result.document_id,'new');const stream=host.displays.get(result.subscription_id);assert.equal(await stream.regionCurrent(),true);
 proof={...proof};assert.equal(await stream.regionCurrent(),false,'a replaced navigation proof cannot re-admit the same loader');await stream.close();
});

// r47 quiet-host WHATWG (15.8 MB typed URL): the parsing renderer answered no Page.getLayoutMetrics within the 5 s CDP
// timeout, so each preview attach failed and the viewer kept the previous page for 40+ s.
test('MP-08/MP-10/MP-11 a navigation preview starts while the new document\'s renderer is too busy to report layout',async()=>{
 const {subscribeVisiblePage}=await import('./kernel-browser-page-display.mjs');
 let busy=false;const layout={cssLayoutViewport:{clientWidth:1265,clientHeight:800},cssVisualViewport:{zoom:1,scale:1}};
 const connection={send:async()=>busy?new Promise((_,reject)=>setTimeout(()=>reject(Error('browser_cdp_timeout')),1500)):layout};
 let proof={connection,sessionId:'s',frame:{loaderId:'old'}};const tab={tab_id:'t',target_id:'target',document_id:'old'};
 const window={...binding(),window_generation:2,browser_generation:1,pid:42,isPageCurrent:async()=>true};
 const host={generation:1,protection:{unknown:false},tabs:new Map([['t',tab]]),displays:new Map(),mirrorTarget:async()=>tab,
  chromium:{display:{},scale:1,browserPid:42},chromeWindows:{current:()=>({tabbed:window})},
  browser:{currentPageFrame:()=>proof,resolvePageTarget:async()=>{throw Error('cold target query is busy')}},timing:()=>{},armDisplayExpiry:()=>{}};
 const captures=[];class Capture {constructor(options){this.options=options;this.region=visiblePageGeometry(options.window,options.viewport);captures.push(this)}async start(){return this}async close(){}}
 const command={op:'display_subscribe',tab_id:'t',generation:1,codecs:['png','avc1.420033'],bitrate:8000000,device_scale_factor:1};
 await host.displays.get((await subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture})).subscription_id).close();
 // The typed URL commits; its renderer parses the document and answers nothing.
 busy=true;tab.document_id='new';proof={connection,sessionId:'s',frame:{loaderId:'new'}};
 const started=Date.now(),result=await subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture});
 assert(Date.now()-started<1000,`the preview waited ${Date.now()-started} ms for the busy renderer`);
 assert.equal(result.document_id,'new');const stream=host.displays.get(result.subscription_id);
 assert.equal(captures.at(-1).options.viewport.clientWidth,1265,'the crop is the last layout this target reported');
 assert.equal(await stream.regionCurrent(),true);await stream.close();
 // A changed window binding (resize, zoom, DPR) has no earlier read to stand in.
 window.window_generation=3;await assert.rejects(subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture}),/browser_cdp_timeout/);
});

// MP-08/MP-10/MP-11: full-page navigation previews share the page-region publication fence.
test('MP-08/MP-10/MP-11 visible-page preview holds its last page frame while Chrome UI overlaps it',async()=>{
 const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 const connection={send:async()=>assert.fail('unexpected CDP')};let covered=false;
 const source=new VisiblePageCapture({window:binding(),viewport:{clientWidth:1280,clientHeight:800,zoom:1,scale:1},scale:1,connection,sessionId:'s',tab:{target_id:'t',tab_id:'tab',document_id:'d'},policy:{},allowed:()=>true,validateCurrent:async()=>true,occluded:async()=>covered});
 source.valid=()=>!source.closed;source.attested=true;
 source.regions=new NativeRegionProtection(connection,'s',{policy:{targets:[{}],values:[]}});source.regions.measure=async()=>[];await source.regions.refresh();
 const published=[];source.subscribe(sample=>published.push(sample.serial));
 const readback=serial=>({width:64,height:32,length:64*32*4,pixels:Buffer.alloc(64*32*4,serial),serial,format:'bgr0',captured_ms:performance.timeOrigin+performance.now(),release(){}});
 try{
  source.pending=readback(1);await source.publish();covered=true;source.pending=readback(2);await source.publish();
  assert.deepEqual(published,[1]);assert.equal(source.sample().serial,1,'retain only the last unobscured page');
  covered=false;source.pending=readback(3);await source.publish();assert.deepEqual(published,[1,3]);
 }finally{await source.close();}
});

// r48 quiet-host WHATWG: the preview's own attestation screenshot hit the parsing renderer. The proof belongs to the
// window binding on this connection: a new document of the same binding reuses it, any changed binding attests again.
test('MP-08/MP-11 a navigation preview reuses the attestation of an unchanged window binding',async()=>{
 const {subscribeVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const layout={cssLayoutViewport:{clientWidth:1265,clientHeight:800},cssVisualViewport:{zoom:1,scale:1}};
 let connection={send:async()=>layout};let proof={connection,sessionId:'s',frame:{loaderId:'old'}};const tab={tab_id:'t',target_id:'target',document_id:'old'};
 const window={...binding(),window_generation:2,browser_generation:1,pid:42,isPageCurrent:async()=>true};
 const host={generation:1,protection:{unknown:false},tabs:new Map([['t',tab]]),displays:new Map(),mirrorTarget:async()=>tab,
  chromium:{display:{},scale:1,browserPid:42},chromeWindows:{current:()=>({tabbed:window})},
  browser:{currentPageFrame:()=>proof,resolvePageTarget:async()=>assert.fail()},timing:()=>{},armDisplayExpiry:()=>{}};
 let screenshots=0;class Capture {constructor(options){this.options=options;this.region=visiblePageGeometry(options.window,options.viewport)}async start(){if(!this.options.attestation?.proven()){screenshots++;this.options.attestation?.prove()}return this}async close(){}}
 const command={op:'display_subscribe',tab_id:'t',generation:1,codecs:['png','avc1.420033'],bitrate:8000000,device_scale_factor:1};
 const open=async()=>host.displays.get((await subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture})).subscription_id).close();
 await open();assert.equal(screenshots,1);
 tab.document_id='new';proof={connection,sessionId:'s',frame:{loaderId:'new'}};await open();assert.equal(screenshots,1,'same binding, new document: no screenshot');
 window.window_generation=3;await open();assert.equal(screenshots,2,'a resized/zoomed window attests again');
 connection={send:async()=>layout};proof={connection,sessionId:'s',frame:{loaderId:'new'}};await open();assert.equal(screenshots,3,'a new browser connection attests again');
});

// #1016 @3a955d73c review 2: a crop taken from the last layout while the renderer was busy is retired when the delayed
// read disagrees (a scrollbar page -> a page without one), also on the production path with a native-state guard.
test('MP-08/MP-11 a navigation preview retires its cached crop when the delayed layout read differs',async()=>{
 const {subscribeVisiblePage}=await import('./kernel-browser-page-display.mjs');
 for(const width of [1280,1265]){
  let busy=false;const layout=w=>({cssLayoutViewport:{clientWidth:w,clientHeight:800},cssVisualViewport:{zoom:1,scale:1}});
  const connection={send:async()=>busy?new Promise(resolve=>setTimeout(resolve,600,layout(width))):layout(1265)};
  let proof={connection,sessionId:'s',frame:{loaderId:'old'}};const tab={tab_id:'t',target_id:'target',document_id:'old'};
  const window={...binding(),window_generation:2,browser_generation:1,pid:42,isPageCurrent:async()=>true};
  const host={generation:1,protection:{unknown:false},tabs:new Map([['t',tab]]),displays:new Map(),mirrorTarget:async()=>tab,tabGroups:{nativeStateGuard:()=>()=>true},
   chromium:{display:{},scale:1,browserPid:42},chromeWindows:{current:()=>({tabbed:window})},
   browser:{currentPageFrame:()=>proof,viewportByTarget:new Map([['target','emulated']]),resolvePageTarget:async()=>assert.fail()},timing:()=>{},armDisplayExpiry:()=>{}};
  class Capture {constructor(options){this.options=options;this.region=visiblePageGeometry(options.window,options.viewport)}async start(){return this}async close(){}}
  const command={op:'display_subscribe',tab_id:'t',generation:1,codecs:['png','avc1.420033'],bitrate:8000000,device_scale_factor:1};
  await host.displays.get((await subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture})).subscription_id).close();
  busy=true;tab.document_id='new';proof={connection,sessionId:'s',frame:{loaderId:'new'}};
  const stream=host.displays.get((await subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture})).subscription_id);
  assert.equal(await stream.regionCurrent(),true,'the cached crop serves while the renderer is busy');
  await new Promise(resolve=>setTimeout(resolve,700));
  assert.equal(await stream.regionCurrent(),width===1265,width===1265?'an agreeing read keeps the display':'a differing read retires the display');
  await stream.close();
 }
});
