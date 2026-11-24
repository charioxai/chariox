// Row 487 (provisional local 1016): a visible page that asked for resize_rebind keeps its subscription across a resize of
// the Browser window; the kernel admits the same Tab on the new window and input must name the new window epoch.
import test from 'node:test';
import assert from 'node:assert/strict';
import {visiblePageGeometry} from './kernel-browser-visible-page.mjs';
import {subscribeVisiblePage} from './kernel-browser-page-display.mjs';
import {rebindResizedDisplays} from './kernel-browser-resize.mjs';
import {regionInputTarget} from './kernel-browser-display-regions.mjs';

const windowAt=(generation,width,height)=>({role:'tabbed',xid:42,scale:1,window_generation:generation,browser_generation:1,pid:42,current:true,
 bounds:{width,height:height+87},page_rect:{left:0,top:87,width,height},isCurrent(){return this.current;},isPageCurrent:async target=>target==='target'});
function rig(){
 const tab={tab_id:'t',target_id:'target',document_id:'d'};let window=windowAt(2,1280,712);const listeners=new Set(),removed=[],closed=[];
 const host={generation:1,protection:{unknown:false},tabs:new Map([['t',tab]]),displays:new Map(),displayTarget:async()=>tab,
  chromium:{display:{},scale:1,browserPid:42},timing:Object.assign(()=>{},{event:()=>{}}),armDisplayExpiry:()=>{},
  chromeWindows:{current:()=>({tabbed:window}),inWindowOverNow:async()=>false,onRetire:fn=>{listeners.add(fn);return()=>listeners.delete(fn);}},
  browser:{inputCapture:{hold:async()=>async()=>{}},resolvePageTarget:async()=>({connection:{send:async()=>({cssLayoutViewport:{clientWidth:window.page_rect.width,clientHeight:window.page_rect.height},cssVisualViewport:{zoom:1,scale:1}})},sessionId:'s'})},
  removeDisplay:async id=>{removed.push(id);const s=host.displays.get(id);if(s)await s.close();}};
 class Capture {constructor(options){this.options=options;this.region=visiblePageGeometry(options.window,options.viewport);this.tab=options.tab;}async start(){return this;}async close(){closed.push(this.region.css_width);}valid(){return true;}}
 // The resize: the old window retires (viewport_resize), the window re-binds at the new size.
 // A real resize retires the window twice: viewport_resize, then window_changed when the window re-binds (bench timeline).
 const resize=(width,height)=>{window.current=false;for(const fn of listeners)fn({window_generation:window.window_generation,reason:'viewport_resize'});
  for(const fn of listeners)fn({window_generation:window.window_generation,reason:'window_changed'});window=windowAt(window.window_generation+1,width,height);};
 return {host,tab,Capture,resize,removed,closed,get window(){return window;}};
}
const subscribe=(r,extra={})=>subscribeVisiblePage(r.host,{op:'display_subscribe',tab_id:'t',generation:1,codecs:['png','avc1.420033'],bitrate:8_000_000,device_scale_factor:1,...extra},'terminal:a',undefined,{Capture:r.Capture});

test('Row 487 resize_rebind: the same subscription follows a resize at the new size and window epoch',async()=>{
 const r=rig();const result=await subscribe(r,{resize_rebind:true});
 assert.equal(result.resize_rebind,true,'the kernel echoes the capability');
 const stream=r.host.displays.get(result.subscription_id),firstSource=stream.regionSource;
 assert.deepEqual([stream.css_width,stream.css_height,stream.regionLease.sequence],[1280,712,2]);
 r.resize(1672,846);
 assert.equal(r.host.displays.get(result.subscription_id),stream,'the resize does not remove the display');assert.ok(stream.rebinding,'its delivery waits');
 await rebindResizedDisplays(r.host);
 assert.equal(r.host.displays.get(result.subscription_id),stream,'same subscription');assert.equal(stream.binding_id,result.binding_id,'same binding');
 assert.deepEqual([stream.css_width,stream.css_height,stream.regionLease.sequence],[1672,846,3],'new size and window epoch');
 assert.notEqual(stream.regionSource,firstSource);assert.deepEqual(r.closed,[1280],'the old source closed');
 assert.equal(stream.rebinding,null);assert.deepEqual(r.removed,[]);
 // Frames name the new window epoch.
 assert.equal(stream.resizeRebind,true);assert.equal(stream.regionLease.sequence,3);
 await stream.close();
});

test('Row 487 resize_rebind (orch15 cell): after a rebind, input with the OLD binding sequence is refused and the NEW one lands',async()=>{
 const r=rig();const result=await subscribe(r,{resize_rebind:true});const stream=r.host.displays.get(result.subscription_id);
 const input=sequence=>regionInputTarget(r.host,{op:'display_input_target',tab_id:'t',generation:1,document_id:'d',binding_id:result.binding_id,sequence,input:{kind:'click',x:10,y:10}},'terminal:a');
 assert.deepEqual(await input(2),{tab_id:'t',document_id:'d'});
 r.resize(1672,846);
 await assert.rejects(input(2),/refused|stale/,'while the rebind is pending, nothing is admitted');
 await rebindResizedDisplays(r.host);
 await assert.rejects(input(2),/refused|stale/,'the old window epoch is refused');
 assert.deepEqual(await input(3),{tab_id:'t',document_id:'d'},'the new one is admitted');
 await stream.close();
});

test('Row 487 resize_rebind: a viewer that did not ask is removed on the resize, as before; a refused rebind removes the display',async()=>{
 {const r=rig();const result=await subscribe(r);assert.equal(result.resize_rebind,undefined);
  r.resize(1672,846);await new Promise(resolve=>setTimeout(resolve,0));assert.deepEqual(r.removed,[result.subscription_id]);}
 {const r=rig();const result=await subscribe(r,{resize_rebind:true});const stream=r.host.displays.get(result.subscription_id);
  r.resize(1672,846);r.tab.document_id='d2';r.host.tabs.set('t',{...r.tab});
  const waiting=stream.rebinding.done;await rebindResizedDisplays(r.host);
  assert.equal(await waiting,false,'the waiting credits learn it');assert.deepEqual(r.removed,[result.subscription_id],'another document: removed, the viewer re-subscribes');}
});

test('Row 487 #1723 review 1: a resumed resize_rebind display tells the viewer it still follows rebinds',async()=>{
 const {DisplaySessions}=await import('./kernel-display-resumption.mjs');
 let now=1000;const lease={document_id:'d',sequence:3};
 const stream={subscription_id:'old',source_kind:'region',observed_by:'owner',sequence:9,expires:61000,lastCreditAt:1000,document_id:'d',binding_id:'b',
  regionLease:lease,css_width:1672,css_height:846,resizeRebind:true,invalidate(){},async close(){}};
 const host={generation:7,protection:{},displays:new Map([['old',stream]]),armDisplayExpiry(){}};
 const sessions=new DisplaySessions(host,{now:()=>now,clock:()=>now,id:()=>'new'});
 const source={};await sessions.suspend('old','owner',7,()=>source);now+=500;
 const resumed=await sessions.resume('old','owner',7,()=>source);
 assert.equal(resumed.resize_rebind,true);assert.deepEqual([resumed.sequence,resumed.css_width,resumed.css_height],[3,1672,846],'at the epoch and size it is at now');
});

test('Row 487 #1723 review 2: a credit under way when the resize retires the window waits for the rebind instead of failing',async()=>{
 const {displayCredit}=await import('./kernel-browser-display-credit.mjs');
 let settle;const done=new Promise(resolve=>{settle=resolve;});let checks=0;
 const stream={tab_id:'t',regionTarget:{kind:'visible_page'},regionLease:{isAlive:()=>true},regionWindow:{isCurrent:()=>true},adoptLease:()=>0,
  // The first liveness check of this credit sees the window the resize just retired (and the hold it left).
  regionSource:{valid:()=>{checks++;if(checks===1){stream.rebinding={done,settled:false};return false;}return false;},closed:false,allowed:()=>true}};
 const host={generation:1,timing:Object.assign(()=>{},{event:()=>{}}),pageSize:()=>({width:1672,height:846}),inputEpochs:new Map(),inputChangedAt:new Map()};
 let failed=null;const credit=displayCredit(host,stream,{generation:1,after_sequence:0},undefined).catch(error=>{failed=error;});
 await new Promise(resolve=>setTimeout(resolve,30));
 assert.equal(failed,null,'it waits for the rebind');
 stream.rebinding=null;settle(false);await credit;
 assert.equal(failed?.code,'user_domain_stale_reference','a refused rebind ends it as a retired region');
});

test('Row 487 #1723 review 3: a rebind whose last check ends after the hold timed out installs nothing and closes what it opened',async()=>{
 const r=rig();const result=await subscribe(r,{resize_rebind:true});const stream=r.host.displays.get(result.subscription_id);const first=stream.regionSource;
 r.resize(1672,846);
 // The new window's page check is slow; the hold times out (and the display is removed) while it is pending.
 let release;const slow=new Promise(resolve=>{release=resolve;});const pageCurrent=r.window.isPageCurrent;let calls=0;
 r.window.isPageCurrent=async target=>{if(++calls>=2)await slow;return pageCurrent(target);};
 const rebinding=rebindResizedDisplays(r.host);
 await new Promise(resolve=>setTimeout(resolve,20));
 stream.rebinding.settle(false);await r.host.removeDisplay(result.subscription_id);
 release();await rebinding;
 assert.equal(stream.regionSource,first,'nothing was swapped into the closed stream');
 assert.deepEqual(r.closed.sort(),[1280,1672],'the old source closed with the stream, the new one at the refusal');
});

test('Row 487 #1723 review 2b: a credit that waited through a whole completed rebind starts again on the new lease',async()=>{
 const {displayCredit}=await import('./kernel-browser-display-credit.mjs');
 const oldLease={isAlive:()=>true},newLease={isAlive:()=>true};let checks=0;
 const stream={tab_id:'t',regionTarget:{kind:'visible_page'},regionLease:oldLease,regionWindow:{isCurrent:()=>true},adoptLease:()=>0,
  regionSource:{valid:()=>{checks++;
   // While the first attempt waited, the rebind completed: the hold is gone and the lease was replaced.
   if(checks===1){stream.regionLease=newLease;stream.rebinding=null;return false;}
   return false;},closed:false,allowed:()=>true}};
 const host={generation:1,timing:Object.assign(()=>{},{event:()=>{}}),pageSize:()=>({width:1672,height:846}),inputEpochs:new Map(),inputChangedAt:new Map()};
 await assert.rejects(displayCredit(host,stream,{generation:1,after_sequence:0},undefined),/refused|stale/);
 assert.equal(checks,2,'it started again against the new lease (the second attempt is the one that saw it retired)');
});
