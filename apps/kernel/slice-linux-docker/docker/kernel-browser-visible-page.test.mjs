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

test('MP-08/MP-10/MP-11 visible-page input is admitted without a live renderer read; the final guard still reads live',async()=>{
 const {regionInput}=await import('./kernel-browser-display-regions.mjs');
 let live=0,request;
 const busy=Object.assign(Error('browser CDP method Page.getFrameTree timed out after 5000ms'),{code:'browser_cdp_timeout'});
 const stream={subscription_id:'crop',binding_id:'crop',tab_id:'t',generation:1,observed_by:'terminal:a',expires:Date.now()+60000,
  regionTarget:{kind:'visible_page'},regionLease:{document_id:'d',sequence:2,isAlive:()=>true},regionWindow:{isCurrent:()=>true},
  regionSource:{attested:true,valid:()=>true},css_width:1280,css_height:712,regionCurrent:async()=>{live++;throw busy},regionFrameCurrent:async()=>true};
 const host={generation:1,displays:new Map([['crop',stream]]),tabs:new Map([['t',{tab_id:'t',document_id:'d'}]]),request:async(command,options)=>{request={command,options};}};
 const command={op:'input',tab_id:'t',generation:1,document_id:'d',binding_id:'crop',sequence:2,input:{kind:'text',text:'a'}};
 await regionInput(host,command,'terminal:a');
 assert.equal(live,0,'a parsing page answers renderer reads late: nothing before the parkable fences may wait for it');
 assert(request,'the input reached the fenced request (where a busy renderer parks it)');
 await assert.rejects(request.options.inputGuard(),error=>error.code==='browser_cdp_timeout');assert.equal(live,1,'the dispatch fence reads live');
});

test('MP-08/MP-10/MP-11 visible page crops browser scrollbars out of the source, without an RGB exemption',()=>{
 const r=visiblePageGeometry(binding(),{clientWidth:1265,clientHeight:800,zoom:1,scale:1});
 assert.equal(r.width,1264);assert.equal(r.height,712);assert.equal(r.window_region.left,0);assert.equal(r.window_region.top,87);
 for(const viewport of [{clientWidth:0,clientHeight:800,zoom:1,scale:1},{clientWidth:1280,clientHeight:800,zoom:2,scale:1},{clientWidth:1280,clientHeight:800,zoom:1,scale:2}])assert.throws(()=>visiblePageGeometry(binding(),viewport));
});


// A-L13: the owner's selection needs no live stream or mirror, so it works right after a navigation closed them.
test('A-L13 visible-page capture is served without a live stream or mirror and keeps protection metadata',async()=>{
 const {captureVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const tab={tab_id:'t',target_id:'target',document_id:'d'};let captured=0,live=true,metrics={clientWidth:1265,clientHeight:800};
 const window={...binding(),isCurrent:()=>live};
 const host={generation:1,protection:{unknown:false},displays:new Map(),mirror:{streams:new Map()},tabs:new Map([['t',tab]]),chromeWindows:{current:()=>({tabbed:window})},
  browser:{resolvePageTarget:async()=>({connection:{send:async method=>{assert.equal(method,'Page.getLayoutMetrics');return{cssLayoutViewport:metrics,cssVisualViewport:{scale:1,zoom:1}};}},sessionId:'s'})},
  displayScreenshot:async(t,clip)=>{captured++;assert.equal(t,tab);assert.deepEqual(clip,{x:0,y:0,width:1264,height:712,scale:1});return{width:1264,height:712,[displayMaskRegions]:[{x:12,y:6,width:20,height:10}]};}};
 const frame={width:1264,height:712};
 assert.equal((await captureVisiblePage(host,tab,frame)).protected_regions.length,1);assert.equal(captured,1);
 host.protection={unknown:false};assert.equal((await captureVisiblePage(host,tab,frame)).protected_regions.length,1,'a replaced policy object does not stale the capture');
 await assert.rejects(captureVisiblePage(host,tab,{width:1264,height:800}),{code:'capture_view_resized',message:/select the region again/});
 live=false;await assert.rejects(captureVisiblePage(host,tab,frame),{code:'capture_page_unavailable'});
 live=true;metrics={clientWidth:1265,clientHeight:700};await assert.rejects(captureVisiblePage(host,tab,frame),{code:'capture_view_resized'});
 assert.equal(captured,2,'a refused capture never takes the shot');
});
