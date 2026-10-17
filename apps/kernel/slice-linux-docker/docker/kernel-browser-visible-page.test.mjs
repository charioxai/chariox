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
 let active=true,closed=0;
 const tab={tab_id:'t',target_id:'target',document_id:'d'},window={...binding(),window_generation:2,browser_generation:1,pid:42,isPageCurrent:async target=>target==='target'&&active};
 const host={generation:1,protection:{unknown:false},tabs:new Map([['t',tab]]),displays:new Map(),displayTarget:async()=>tab,
  chromium:{display:{},scale:1,browserPid:42},chromeWindows:{current:()=>({tabbed:window})},arbiter:{isActiveTab:async()=>active},
  browser:{resolvePageTarget:async()=>({connection:{send:async()=>({cssLayoutViewport:{clientWidth:1280,clientHeight:800},cssVisualViewport:{zoom:1,scale:1}})},sessionId:'s'})},timing:()=>{},armDisplayExpiry:()=>{}};
 class Capture {constructor(options){this.options=options;this.region=visiblePageGeometry(options.window,options.viewport);}async start(){return this;}async close(){closed++;}}
 const command={op:'display_subscribe',tab_id:'t',generation:1,codecs:['png','avc1.420033'],bitrate:8_000_000,device_scale_factor:2};
 active=false;await assert.rejects(subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture}));assert.equal(host.displays.size,0);
 active=true;const result=await subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture});
 assert.equal(result.css_height,712);assert.equal(result.device_scale_factor,1);assert.equal(result.source_kind,'region');assert.equal(result.document_id,'d');
 const stream=[...host.displays.values()][0];assert.equal(await stream.regionCurrent(),true);
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

// vidlat r22: publication borrows the native owner verdict, never the layout read; input and admission stay fresh.
test('MP-08/MP-10/MP-11 visible-page publication reuses the owner verdict but reads the layout viewport every time',async()=>{
 const {subscribeVisiblePage}=await import('./kernel-browser-page-display.mjs');
 let fresh=0,cached=0,layoutReads=0,clientWidth=1280,document='d';
 const tab={tab_id:'t',target_id:'target',document_id:'d'},window={...binding(),window_generation:2,browser_generation:1,pid:42,
  isPageCurrent:async()=>{fresh++;return true},isPagePublishable:async()=>{cached++;return true}};
 const host={generation:1,protection:{unknown:false},tabs:new Map([['t',tab]]),displays:new Map(),displayTarget:async()=>tab,
  chromium:{display:{},scale:1,browserPid:42},chromeWindows:{current:()=>({tabbed:window})},
  browser:{resolvePageTarget:async()=>({connection:{send:async()=>{layoutReads++;return {cssLayoutViewport:{clientWidth,clientHeight:800},cssVisualViewport:{zoom:1,scale:1}}}},sessionId:'s'})},timing:()=>{},armDisplayExpiry:()=>{}};
 class Capture {constructor(options){this.options=options;this.region=visiblePageGeometry(options.window,options.viewport);}async start(){return this;}async close(){}}
 const command={op:'display_subscribe',tab_id:'t',generation:1,codecs:['png','avc1.420033'],bitrate:8_000_000,device_scale_factor:2};
 await subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture});
 const stream=[...host.displays.values()][0],{publishCurrent}=stream.regionSource.options;
 fresh=cached=layoutReads=0;
 assert.equal(await publishCurrent(),true);assert.equal(await publishCurrent(),true);
 assert.deepEqual([fresh,cached,layoutReads],[0,2,2],'two publications: owner verdict reused, layout read each time');
 assert.equal(await stream.regionCurrent(),true);assert.deepEqual([fresh,cached,layoutReads],[1,2,3],'input guard stays fresh');
 assert.equal(await stream.regionSource.options.validateCurrent(),true);assert.equal(fresh,2,'attestation stays fresh');
 clientWidth=1265;assert.equal(await publishCurrent(),false,'a scrollbar appearing (no CDP event) retires the next publication');
 clientWidth=1280;tab.document_id='d2';assert.equal(await publishCurrent(),false,'a new document retires it');
 tab.document_id='d';window.isCurrent=()=>false;assert.equal(await publishCurrent(),false,'a retired window retires it');
});

test('MP-11 visible-page publication reads the layout once, then only after a page report, a mismatch or a lost observer',async()=>{
 const {subscribeVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const tab={tab_id:'t',target_id:'target',document_id:'d'},window={...binding(),window_generation:2,browser_generation:1,pid:42,isPageCurrent:async()=>true};
 const listeners=new Set(),reads=[];let metrics={clientWidth:1280,clientHeight:800},binding_;
 const connection={subscribe:fn=>{listeners.add(fn);return()=>listeners.delete(fn);},async send(method,params){
  if(method==='Page.getLayoutMetrics'){reads.push(1);return {cssLayoutViewport:metrics,cssVisualViewport:{zoom:1,scale:1}};}
  if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'f'}}};
  if(method==='Page.createIsolatedWorld')return {executionContextId:7};
  if(method==='Runtime.addBinding')binding_=params.name;
  return {};
 }};
 const host={generation:1,protection:{unknown:false},tabs:new Map([['t',tab]]),displays:new Map(),displayTarget:async()=>tab,
  chromium:{display:{},scale:1,browserPid:42},chromeWindows:{current:()=>({tabbed:window})},arbiter:{isActiveTab:async()=>true},
  browser:{resolvePageTarget:async()=>({connection,sessionId:'s'})},timing:()=>{},armDisplayExpiry:()=>{}};
 let options;class Capture {constructor(o){options=o;this.region=visiblePageGeometry(o.window,o.viewport);}async start(){return this;}async close(){}}
 const command={op:'display_subscribe',tab_id:'t',generation:1,codecs:['png','avc1.420033'],bitrate:8_000_000,device_scale_factor:2};
 const stream=await subscribeVisiblePage(host,command,'terminal:a',undefined,{Capture});
 const report=()=>[...listeners].forEach(fn=>fn({sessionId:'s',method:'Runtime.bindingCalled',params:{name:binding_,executionContextId:7}}));
 const publishReads=async()=>{reads.length=0;const ok=await options.publishCurrent();return [ok,reads.length];};
 assert.deepEqual(await publishReads(),[true,1],'the first publication has no verdict and reads');
 for(let i=0;i<5;i++)assert.deepEqual(await publishReads(),[true,0],'a proven verdict is reused');
 reads.length=0;assert.equal(await options.validateCurrent(),true);assert.equal(reads.length,1,'admission and input always read fresh');
 report();assert.deepEqual(await publishReads(),[true,1],'a page report forces one read');
 assert.deepEqual(await publishReads(),[true,0]);
 metrics={clientWidth:1265,clientHeight:800};report();
 assert.deepEqual(await publishReads(),[false,1],'a changed layout is refused');
 assert.deepEqual(await publishReads(),[false,1],'and never becomes the verdict');
 metrics={clientWidth:1280,clientHeight:800};assert.deepEqual(await publishReads(),[true,1]);assert.deepEqual(await publishReads(),[true,0]);
 [...listeners].forEach(fn=>fn({sessionId:'s',method:'Page.frameNavigated',params:{frame:{id:'f'}}}));
 assert.deepEqual(await publishReads(),[true,1]);assert.deepEqual(await publishReads(),[true,1],'a lost observer reads every publication');
 await [...host.displays.values()][0].close();assert.equal(listeners.size,0);
});
