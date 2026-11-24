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
 const resize=(width,height)=>{window.current=false;for(const fn of listeners)fn({window_generation:window.window_generation,reason:'viewport_resize'});window=windowAt(window.window_generation+1,width,height);};
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
