// MP-08/MP-10/MP-11 (P1): a parked Tab display holds the push pump's credit without frames; a commit unparks it.
import test from 'node:test';
import assert from 'node:assert/strict';
import {DisplayStream} from './kernel-browser-display.mjs';
import {displayPushCredit} from './kernel-browser-display-credit.mjs';
function parkedStream({alive=()=>true}={}){
 const parks=[],stream=Object.assign(Object.create(DisplayStream.prototype),{invalidate(){},tab_id:'t',
  regionLease:{isAlive:alive},regionSource:{valid:()=>true,park:parked=>parks.push(parked)},regionWindow:{isCurrent:()=>true}});
 stream.park(true);return {stream,parks};
}
const host={generation:1,timing(){}};
test('MP-08/MP-10 a parked Tab display answers push credits with no frame and reads nothing',async()=>{
 const {stream,parks}=parkedStream(),at=performance.now();
 const reply=await displayPushCredit(host,stream,{generation:1,after_sequence:0,push:{}},undefined,{budgetMs:30});
 assert.deepEqual(reply,{generation:1,frame_sent:false,display_frame:null});
 assert.ok(performance.now()-at>=25,'the credit waits out its budget instead of spinning');assert.deepEqual(parks,[true]);
});
test('MP-08/MP-10 unparking wakes a held credit at once',async()=>{
 const {stream,parks}=parkedStream(),wait=stream.waitUnparked(5000),at=performance.now();
 setTimeout(()=>stream.park(false),10);await wait;
 assert.ok(performance.now()-at<1000);assert.deepEqual(parks,[true,false]);assert.equal(stream.parked,false);
});
test('MP-11 a parked Tab display whose binding retired ends the pump',async()=>{
 const {stream}=parkedStream({alive:()=>false});
 await assert.rejects(displayPushCredit(host,stream,{generation:1,after_sequence:0,push:{}},undefined,{budgetMs:5}),e=>e.code==='user_domain_stale_reference'||/stale_reference/.test(`${e.code} ${e.message}`));
});
test('MP-08/MP-11 only the observing viewer parks its own Tab display',async()=>{
 const {KernelBrowserHost}=await import('./kernel-browser-host.mjs');const {DisplaySessions}=await import('./kernel-display-resumption.mjs');
 const previous=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{
  const host=Object.create(KernelBrowserHost.prototype),{stream,parks}=parkedStream();stream.park(false);
  Object.assign(stream,{subscription_id:'d',observed_by:'viewer',tabDisplay:true,expires:Date.now()+1000});
  Object.assign(host,{generation:1,protection:{values:[]},streams:new Map(),displays:new Map([['d',stream]]),armDisplayExpiry(){},start:async()=>{}});
  host.displaySessions=new DisplaySessions(host);
  const park=(command)=>host.requestNow({op:'display_park',subscription_id:'d',generation:1,observed_by:'viewer',parked:true,...command});
  assert.deepEqual(await park({}),{generation:1,parked:true});assert.equal(stream.parked,true);
  assert.deepEqual(await park({parked:false}),{generation:1,parked:false});
  await assert.rejects(park({observed_by:'other'}),{code:'user_domain_not_granted'});
  await assert.rejects(park({focused_agent:true}),{code:'user_domain_not_granted'});
  stream.tabDisplay=false;assert.deepEqual(await park({}),{generation:1,parked:true},'a region display parks too: an unwatched view must not keep any source reading');
  await assert.rejects(park({parked:'yes'}),error=>error.race==='display_retired');
  assert.deepEqual(parks,[true,false,true,false,true]);
 }finally{if(previous===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=previous;}
});
test('MP-08/MP-11 an unwatched display parks every 20 s for minutes and keeps its host binding; silence ends it at 60 s',async t=>{
 const {KernelBrowserHost}=await import('./kernel-browser-host.mjs');const {DisplaySessions}=await import('./kernel-display-resumption.mjs');
 const previous=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 t.mock.timers.enable({apis:['setTimeout','Date']});
 try{
  const host=Object.create(KernelBrowserHost.prototype),{stream,parks}=parkedStream();stream.park(false);
  const closed=[];stream.close=async()=>{closed.push(stream.subscription_id)};
  Object.assign(stream,{subscription_id:'d',observed_by:'viewer',tabDisplay:true,expires:Date.now()+60_000});
  Object.assign(host,{generation:1,protection:{values:[]},streams:new Map(),displays:new Map([['d',stream]]),compositors:new Map(),displayTeardowns:new Set(),chromium:{},start:async()=>{}});
  host.displaySessions=new DisplaySessions(host);host.armDisplayExpiry(stream);
  const park=()=>host.requestNow({op:'display_park',subscription_id:'d',generation:1,observed_by:'viewer',parked:true,watch:true});
  for(let i=0;i<15;i++){t.mock.timers.tick(20_000);await park();await Promise.resolve();}
  assert.equal(host.displays.has('d'),true,'five minutes of parks keep the host display');assert.deepEqual(closed,[]);
  t.mock.timers.tick(61_000);await new Promise(resolve=>setImmediate(resolve));
  assert.equal(host.displays.has('d'),false,'silence alone still ends it');assert.deepEqual(closed,['d']);assert.deepEqual(parks.slice(-1),[true]);
 }finally{if(previous===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=previous;}
});

// Row 401 F1 (webperf-r19 A1): after a re-subscribe the push credit used to arrive before the restarted screencast's first frame
// and, with no producer yet, sleep a blind 100 ms. A screencast region's credit now wakes on the source's next frame.
test('row 401 F1: a screencast region credit with no producer wakes on the source\'s first frame, not a blind 100 ms',async()=>{
 const listeners=new Set(),passes=[],abort=new AbortController();let sample=null;
 const source={valid:()=>true,closed:false,allowed:()=>true,policy:{},tab:{tab_id:'t',document_id:'d'},regionRevision:0,changedAt:-Infinity,sessionId:'s',
  connection:{send:async()=>({cssVisualViewport:{pageX:0,pageY:0,scale:1}})},subscribe(fn){listeners.add(fn);return()=>listeners.delete(fn);},sample:()=>sample};
 const stream=Object.assign(Object.create(DisplayStream.prototype),{tab_id:'t',codec:'avc1.420033',regionScreencast:true,regionLease:{isAlive:()=>true},
  regionSource:source,regionWindow:{isCurrent:()=>true},compositorRegionRevision:0,device_scale_factor:1,css_width:10,css_height:10});
 const at=performance.now();
 // Each credit pass reads the region's compositor; the pass after the first frame ends the test (the stub has no encoder).
 const host={generation:1,timing(){},protection:source.policy,inputEpochs:new Map(),inputChangedAt:new Map(),compositors:new Map(),
  compositorFor:async()=>{passes.push(performance.now()-at);if(sample){sample=null;abort.abort();}return source;}};
 setTimeout(()=>{sample={serial:1};for(const listener of listeners)listener(sample);},6);
 await displayPushCredit(host,stream,{generation:1,after_sequence:0,push:{}},abort.signal,{budgetMs:100});
 assert.ok(passes.length>=2,`a second credit pass follows the first frame (passes at ${passes.map(t=>t.toFixed(1)).join(', ')} ms)`);
 assert.ok(passes[1]<40,`the credit woke on the frame, not after a blind sleep (second pass at ${passes[1]?.toFixed(1)} ms)`);
});
