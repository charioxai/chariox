// MP-08/MP-10/MP-11 protocol 560: the owner's window display asks no page renderer and survives Tab changes.
import test from 'node:test';
import assert from 'node:assert/strict';
import {subscribeRegion,regionInput,regionInputTarget} from './kernel-browser-display-regions.mjs';
import {KernelBrowserHost} from './kernel-browser-host.mjs';
import {WindowPageCapture,windowCaptureArgs} from './kernel-browser-window-display.mjs';

function fixture({fills=0}={}){
 const calls=[];let alive=true,active='target-a';
 const tabs=new Map([['t',{tab_id:'t',target_id:'target-a',document_id:'doc-a'}]]);
 const connection={isOpen:()=>true,subscribe:()=>()=>{},send:async method=>{calls.push(['cdp',method]);return {};}};
 const window={browser_generation:2,window_generation:7,role:'tabbed',xid:42,bounds:{width:2560,height:1774},page_rect:{left:0,top:174,width:2560,height:1600},scale:2,pid:42,isCurrent:()=>alive};
 const host={generation:2,protection:{unknown:false,targets:[]},displays:new Map(),tabs,timing:()=>{},armDisplayExpiry:()=>{},
  chromium:{display:{},child:{pid:42},scale:2,policy:'managed'},
  browser:{fillTargets:new Map(Array.from({length:fills},(_,i)=>[i,{}])),ensureConnection:async()=>{calls.push('connection');return connection;},
   resolvePageTarget:async()=>{calls.push('page_session');return {connection,sessionId:'s'};},currentPageFrame:()=>{calls.push('page_frame');}},
  displayTarget:async()=>{calls.push('displayTarget');return tabs.get('t');},
  tabGroups:{active:async()=>active},
  chromeWindows:{current:()=>({tabbed:window,auxiliary:[]}),inWindowOverNow:async()=>false}};
 class Capture {constructor(options){this.options=options;this.region={css_width:options.rect.width/2,css_height:options.rect.height/2};this.tab=options.tab;calls.push(['capture',options]);}
  async start(){return this;}async close(){calls.push('close');}valid(){return alive;}}
 const command={op:'display_subscribe',tab_id:'t',generation:2,codecs:['png','avc1.420033'],bitrate:2_000_000,device_scale_factor:2,target:{kind:'browser_window',window_generation:7}};
 return {host,calls,command,Capture,window,tabs,retire:()=>{alive=false;},switchTo:t=>{active=t;}};
}
const subscribe=f=>subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});

test('MP-08/MP-10 the window display asks neither a Tab nor a page renderer before its first frame',async()=>{
 const f=fixture(),result=await subscribe(f);
 assert.deepEqual(f.calls.filter(c=>typeof c==='string'),['connection'],'only the browser-level connection is used');
 assert.equal(f.calls.some(c=>c[0]==='cdp'),false);
 assert.equal(result.source_kind,'region');assert.equal(result.sequence,7);assert.equal(result.document_id,'chrome-window-2-7-page');
 assert.equal(result.css_width,1280);assert.equal(result.css_height,800);assert.equal(result.device_scale_factor,2);
 const options=f.calls.find(c=>c[0]==='capture')[1];
 assert.deepEqual(options.rect,{left:0,top:174,width:2560,height:1600});assert.equal(options.sessionId,undefined);
 await [...f.host.displays.values()][0].close();
});
test('MP-11 only the current window epoch of a managed-or-not tabbed window is admitted',async()=>{
 const stale=fixture();stale.command.target.window_generation=6;await assert.rejects(subscribe(stale));
 const gone=fixture();gone.retire();await assert.rejects(subscribe(gone));
 const extra=fixture();extra.command.target.xid=42;await assert.rejects(subscribe(extra));
 const agent=fixture();agent.command._agent_input=true;await assert.rejects(subscribe(agent));
 for(const f of [stale,gone,extra,agent])assert.equal(f.host.displays.size,0);
});
test('MP-08/MP-10 the display survives Tab create, switch and navigation and retires only with its window',async()=>{
 const f=fixture();await subscribe(f);const stream=[...f.host.displays.values()][0];
 f.tabs.set('t2',{tab_id:'t2',target_id:'target-b',document_id:'doc-b'});f.switchTo('target-b');
 f.tabs.get('t').document_id='doc-a2';
 assert.equal(stream.regionLease.isAlive(),true);assert.equal(await stream.regionCurrent(),true);assert.equal(stream.regionSource.valid(),true);
 assert.equal(stream.regionLease.document_id,'chrome-window-2-7-page');
 f.retire();assert.equal(stream.regionLease.isAlive(),false);await stream.close();
});
test('MP-08/MP-10 opening a Tab and following Chrome\'s foreground keep a window display',async()=>{
 const host=new KernelBrowserHost('/nonexistent',{chromium:{},browserFactory:()=>null});
 let closed=0;const stream={subscription_id:'d',tab_id:'t',regionTarget:{kind:'browser_window'},close:async()=>{closed++;}};
 host.displays.set('d',stream);
 await host.closeCompositors();host.foreground.reset();await host.retireNativeForeground();
 assert.equal(host.displays.has('d'),true);assert.equal(closed,0);
});
test('MP-11 the window display is view-only: input is refused',async()=>{
 const f=fixture(),r=await subscribe(f),input={op:'input',tab_id:'t',generation:2,document_id:r.document_id,binding_id:r.binding_id,sequence:r.sequence,input:{kind:'click',x:1,y:2}};
 await assert.rejects(regionInput(f.host,input,'terminal:a'));await assert.rejects(regionInputTarget(f.host,input,'terminal:a'));
 await [...f.host.displays.values()][0].close();
});
// The protection is the only part that may touch a page, and only while a Vault fill target exists.
const frame=captured_ms=>({captured_ms});
function protection({fills,policyTargets=[]}){
 const f=fixture({fills});f.host.protection.targets=policyTargets;
 const capture=Object.create(WindowPageCapture.prototype);Object.assign(capture,{host:f.host,scale:2,region:{pageLeft:0,pageTop:0}});
 return {f,regions:capture.protection()};
}
test('MP-11 without a fill target no page is asked for masks',async()=>{
 const {f,regions}=protection({fills:0});await regions.refresh();
 assert.deepEqual(await regions.regions(frame(performance.timeOrigin+performance.now())),[]);
 assert.equal(f.calls.length,0);
});
test('MP-11 a readback older than the fence is retried, and a fill target needs the front Tab\'s page',async()=>{
 const {f,regions}=protection({fills:1});await regions.refresh();
 await assert.rejects(regions.regions(frame(0)),{code:'fill_capture_retry'});
 assert.equal(regions.tracker().targets,true);
 f.tabs.clear();
 await assert.rejects(regions.regions(frame(performance.timeOrigin+performance.now()+1)),{code:'fill_capture_retry'},'no front Tab: withhold, never publish unmasked');
 assert.equal(f.calls.includes('page_session'),false);
});
test('MP-11 the real capture class derives the page crop from the window epoch alone',()=>{
 const f=fixture(),capture=new WindowPageCapture({host:f.host,window:{id:42,width:2560,height:1774},rect:{left:0,top:174,width:2560,height:1600},contentRect:{left:0,top:174,width:2560,height:1600},contentHeight:1600,
  isCurrent:()=>true,validateCurrent:async()=>true,occluded:async()=>false,tab:{tab_id:'t',document_id:'d'},scale:2,connection:{isOpen:()=>true,subscribe:()=>()=>{}},
  display:{},pid:42,policy:f.host.protection,browser:f.host.browser,allowed:()=>true,timing:()=>{}});
 assert.equal(capture.region.css_width,1280);assert.equal(capture.region.css_height,800);assert.equal(capture.region.pageTop,0);
 assert.equal(capture.surface,'page');assert.equal(typeof capture.occluded,'function');
 capture.onCdp({method:'Target.targetCreated',params:{targetInfo:{type:'page'}}});
 assert.equal(capture.closed,false,'a new Tab does not fence the window display');
});

test('MP-11 an odd page rect still builds the real capture (the kernel window need not be even)',()=>{
 const f=fixture(),window={xid:42,bounds:{width:2560,height:1775},page_rect:{left:0,top:175,width:2559,height:1599}};
 const capture=new WindowPageCapture({host:f.host,...windowCaptureArgs(window),
  isCurrent:()=>true,validateCurrent:async()=>true,occluded:async()=>false,tab:{tab_id:'t',document_id:'d'},scale:2,connection:{isOpen:()=>true,subscribe:()=>()=>{}},
  display:{},pid:42,policy:f.host.protection,browser:f.host.browser,allowed:()=>true,timing:()=>{}});
 assert.equal(capture.region.css_width,1279);assert.equal(capture.region.css_height,799);assert.equal(capture.surface,'page');
});
