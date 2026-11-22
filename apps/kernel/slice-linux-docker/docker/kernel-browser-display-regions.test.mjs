import test from 'node:test';
import assert from 'node:assert/strict';
import {subscribeRegion, regionInput, regionInputTarget} from './kernel-browser-display-regions.mjs';
import {maxDisplays} from './kernel-browser-display-admission.mjs';

function fixture(kind='region') {
 const tab={tab_id:'t',target_id:'target',document_id:'doc'},calls=[];
 let alive=true,front='target';
 const lease={binding_id:'opaque',tab_id:'t',generation:2,document_id:'doc',sequence:3,input_epoch:3,
  rect:{x:10,y:20,width:100,height:50},element_offset:{x:0,y:0},device_scale_factor:2,
  isAlive:()=>alive,isCurrent:async()=>{calls.push('live');return alive;},close:async()=>{alive=false;},
  attachDisplay:s=>host.displays.set(s.subscription_id,s),resolveInput:async()=>({guard:async()=>{calls.push('hit');},input:{kind:'click',x:11,y:22}})};
 const binding={browser_generation:2,window_generation:7,role:'tabbed',xid:42,bounds:{width:2560,height:1774},content_top:174,page_rect:{left:0,top:174,width:2560,height:1600},scale:2,isCurrent:()=>alive,isPageCurrent:async target=>alive&&(target===undefined||target===front)};
 const host={generation:2,protection:{unknown:false},displays:new Map(),scales:new Map([['t',2]]),tabs:new Map([['t',tab]]),timing:()=>{},
  chromium:{display:{},child:{pid:42},scale:2,policy:'managed'},browser:{inputCapture:{hold:async()=>async()=>{}},resolvePageTarget:async()=>({connection:{},sessionId:'s'})},
  displayTarget:async()=>tab,displayScreenshot:async()=>{},armDisplayExpiry:()=>{},
  mirror:{v2:{regions:{resolve:async(target,scope)=>{calls.push([target,scope]);return lease;}}}},
  arbiter:{isActiveTab:async tab=>tab.target_id===front},chromeWindows:{current:()=>({tabbed:binding,auxiliary:[]}),resolve:async()=>{throw Error("MP-11 viewer must not mutate the window binding")},band:()=>({surface:'browser_chrome',window:{id:42,width:2560,height:1774},rect:{left:0,top:0,width:2560,height:174},contentHeight:1600,isCurrent:()=>alive})}};
 class Capture {constructor(options){this.options=options;this.region={css_width:options.rect.width/2,css_height:options.rect.height/2};calls.push(options);}async start(){return this;}async close(){calls.push('close');} valid(){return alive;}}
 const command={op:'display_subscribe',tab_id:'t',generation:2,codecs:['png','avc1.420033'],bitrate:2_000_000,device_scale_factor:1,
  target:kind==='region'?{kind,mirror_subscription_id:'mirror',document_id:'doc',sequence:3,region_id:'n1'}:{kind,window_generation:7}};
 return {host,tab,lease,binding,calls,command,Capture,activate:target=>{front=target;},retire:()=>{alive=false;}};
}
test('MP-08/MP-11 §2a lease supplies crop; viewer density never selects geometry',async()=>{
 const f=fixture();f.command.codecs.push('chariox-stripes-v1');const result=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 assert.deepEqual(f.calls[0],[{mirror_subscription_id:'mirror',generation:2,document_id:'doc',sequence:3,region_id:'n1'},'terminal:a']);
 assert.equal(result.source_kind,'region');assert.equal(result.binding_id,'opaque');assert.equal(result.css_width,100);assert.equal(result.css_height,50);assert.equal(result.device_scale_factor,2);
 const source=[...f.host.displays.values()][0].regionSource;
 assert.equal([...f.host.displays.values()][0].stripes,false,'even tiny crops retain whole-video encoding');
 assert.deepEqual(source.options.rect,{left:20,top:214,width:200,height:100});
 assert.equal(typeof source.options.validateCurrent,'function');
 await [...f.host.displays.values()][0].close();
});
test('MP-08/MP-11 chrome band uses owner constructor shape and window epoch',async()=>{
 const f=fixture('browser_chrome'),result=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 assert.equal(result.css_width,1280);assert.equal(result.css_height,87);assert.equal(result.sequence,7);assert.notEqual(result.document_id,'doc');
 const s=[...f.host.displays.values()][0];assert.equal(s.regionSource.options.surface,'browser_chrome');await s.close();
});
test('MP-11 a guessed Chrome window epoch cannot subscribe without managed lockdown',async()=>{
 for(const policy of ['unavailable',null,undefined]){
  const f=fixture('browser_chrome');f.host.chromium.policy=policy;
  await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture}));
  assert.equal(f.calls.length,0);assert.equal(f.host.displays.size,0);
 }
});
test('MP-11 Chrome capture and input retire if managed lockdown is lost',async()=>{
 const f=fixture('browser_chrome'),r=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const s=[...f.host.displays.values()][0];f.host.chromium.policy='unavailable';
 try{
  assert.equal(s.regionSource.options.isCurrent(),false);
  assert.equal(s.regionLease.isAlive(),false);assert.equal(await s.regionCurrent(),false);
  await assert.rejects(regionInputTarget(f.host,{op:'input',tab_id:'t',generation:2,document_id:r.document_id,
   binding_id:r.binding_id,sequence:r.sequence,input:{kind:'click',x:1,y:2}},'terminal:a'));
 }finally{await s.close();}
});
for(const extra of ['rect','xid','pid','selector'])test(`MP-11 rejects client ${extra} before resolving`,async()=>{
 const f=fixture();f.command.target[extra]=1;await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture}));assert.equal(f.calls.length,0);
});
test('MP-11 stale chrome generation and inactive page bindings refuse',async()=>{
 const f=fixture('browser_chrome');f.command.target.window_generation=6;await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture}));
 const p=fixture();p.command.tab_id='foreign';await assert.rejects(subscribeRegion(p.host,p.command,'terminal:a',undefined,{Capture:p.Capture}));
});
test('MP-11 region input checks caller, epoch, live geometry and final mirror hit',async()=>{
 const f=fixture();await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const stream=[...f.host.displays.values()][0];let dispatch=0;
 const command={op:'input',tab_id:'t',generation:2,document_id:'doc',binding_id:'opaque',sequence:3,input:{kind:'click',x:1,y:2}};
 f.host.dispatchRegionInput=async(_s,resolved)=>{await resolved.guard();dispatch++;};
 await regionInput(f.host,command,'terminal:a');assert.equal(dispatch,1);assert(f.calls.includes('hit'));
 for(const changed of [{sequence:4},{document_id:'new'},{_agent_input:true}])await assert.rejects(regionInput(f.host,{...command,...changed},'terminal:a'));
 await assert.rejects(regionInput(f.host,command,'terminal:b'));f.retire();await assert.rejects(regionInput(f.host,command,'terminal:a'));assert.equal(dispatch,1);await stream.close();
});
test('MP-11 chrome pre-ledger target comes from owner arbiter; missing seam refuses',async()=>{
 const f=fixture('browser_chrome'),r=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const command={op:'input',tab_id:'t',generation:2,document_id:r.document_id,binding_id:r.binding_id,sequence:r.sequence,input:{kind:'key',key:'l',modifiers:2}};
 await assert.rejects(regionInputTarget(f.host,command,'terminal:a'));
 f.host.arbiter={resolveDisplayInput:async(command)=>{assert.equal(command.window_generation,7);return {tab_id:'t',document_id:'current'};}};
 const target=await regionInputTarget(f.host,command,'terminal:a');assert.equal(target.document_id,'current');assert.equal(target.region_document_id,r.document_id);
 await [...f.host.displays.values()][0].close();
});
test('MP-08/MP-11 fractional clips round inward and input retains canonical offset',async()=>{
 const f=fixture();f.lease.rect={x:10.2,y:20.2,width:100.1,height:50.1};
 const r=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 assert.equal(r.css_width,99);assert.equal(r.css_height,49);
 const s=[...f.host.displays.values()][0];assert.deepEqual(s.regionSource.options.rect,{left:21,top:215,width:198,height:98});
 assert(Math.abs(s.regionOffset.x-.3)<1e-9);await s.close();
});
test('MP-11 capture rejected after async geometry retirement leaves no display',async()=>{
 const f=fixture();class Capture extends f.Capture {async start(){f.retire();return this;}}
 await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture}));assert.equal(f.host.displays.size,0);assert(f.calls.includes('close'));
});
test('MP-08/MP-11 actual credit and frame path uses crop/window lifetime; retires before delivery',async()=>{
 const {displayCredit}=await import('./kernel-browser-display-credit.mjs');
 const {DisplayStream}=await import('./kernel-browser-display.mjs');
 let current=true,admitted=true,checks=0;
 const tab={tab_id:'t',target_id:'target',document_id:'window-2-7'},policy={unknown:false};
 const sample={serial:1,tab_id:'t',document_id:tab.document_id,raw:{},width:200,height:100,motion:true,data_base64:'signature',encoded:{key:true,data_base64:'YWJj'}};
 const source={native:true,attested:true,regionRevision:0,policy,tab,closed:false,valid:()=>true,allowed:()=>true,sample:()=>sample};
 const stream=new DisplayStream({subscription_id:'s',tab_id:'t',codec:'avc1.420033',bitrate:2_000_000,device_scale_factor:2,css_width:100,css_height:50,dependencies:true},{encoder:{close:async()=>{}},wait:async()=>{}});
 const producer={source,frames:[],feedback(){},retireUnsent(){},invalidate(){},take:()=>sample,close:async()=>{}};
 Object.assign(stream,{regionSource:source,regionLease:{isAlive:()=>current},regionCurrent:async()=>{checks++;return current&&admitted;},regionWindow:{isCurrent:()=>current},compositorRegionRevision:0,producer,
  refinerDocument:tab.document_id,refiner:{request:()=>null,invalidate(){},close:async()=>{}}});
 const host={generation:2,protection:policy,compositors:new Map(),inputEpochs:new Map(),inputChangedAt:new Map(),timing:()=>{},compositorFor:async()=>source,
  displayTarget:()=>{throw Error('band must not acquire a page document');}};
 try {
  const result=await displayCredit(host,stream,{generation:2,after_sequence:0,push:{congested:false}});
  assert.equal(result.frame_sent,true);assert.equal(result.display_frame.document_id,tab.document_id);assert.equal(result.display_frame.width,200);assert.equal(result.display_frame.css_width,100);assert.equal(checks,1,'MP-08/MP-10/MP-11 one final fresh region query per delivered credit');
  sample.serial=2;host.compositorFor=async()=>{admitted=false;return source};
  const retired=await displayCredit(host,stream,{generation:2,after_sequence:1,push:{congested:false}});
  assert.equal(retired.frame_sent,false);assert.equal(stream.sequence,1);assert.equal(checks,2,'retirement during preparation is checked fresh before delivery');
  current=false;await assert.rejects(displayCredit(host,stream,{generation:2,after_sequence:1}));assert.equal(stream.sequence,1);
 }finally{await stream.close();}
});
test('MP-11 page region refuses a missing or changed owner native active-tab fence',async()=>{
 const f=fixture();delete f.host.arbiter;
 await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture}));assert.equal(f.host.displays.size,0);
 const g=fixture();await subscribeRegion(g.host,g.command,'terminal:a',undefined,{Capture:g.Capture});
 const s=[...g.host.displays.values()][0];g.activate('foreign');
 assert.equal(await s.regionCurrent(),false);await assert.rejects(regionInput(g.host,{op:'input',tab_id:'t',generation:2,document_id:'doc',binding_id:'opaque',sequence:3,input:{kind:'click',x:1,y:2}},'terminal:a'));await s.close();
});

for (const scale of [1,2]) test(`MP-08/MP-10/MP-11 clipped page placement includes inward rounding at DPR${scale}`,async()=>{
 const f=fixture();f.binding.scale=scale;f.lease.device_scale_factor=scale;
 f.lease.rect={x:10.2,y:20.2,width:100.1,height:50.1};f.lease.element_offset={x:12.25,y:30.5};
 const r=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const stream=[...f.host.displays.values()][0];
 try{
  assert.deepEqual(r.region_offset,{x:12.25+Math.ceil(10.2*scale)/scale-10.2,y:30.5+Math.ceil(20.2*scale)/scale-20.2});
  assert.deepEqual(stream.regionPlacement,r.region_offset);
  // Placement is captured at admission, never taken from later lease mutation.
  f.lease.element_offset.x=999;assert.equal(r.region_offset.x,stream.regionPlacement.x);assert(r.region_offset.x<14);
  let point;f.lease.resolveInput=async p=>{point=p;return {guard:async()=>{}}};
  f.host.dispatchRegionInput=async()=>{};
  await regionInput(f.host,{tab_id:'t',generation:2,document_id:'doc',binding_id:r.binding_id,sequence:3,input:{kind:'click',x:1,y:2}},'terminal:a');
  assert.equal(point.x,1+stream.regionOffset.x,'element clipping must not be added twice to input');
  assert.equal(point.y,2+stream.regionOffset.y);
 }finally{await stream.close();}
});
test('MP-08/MP-10/MP-11 unclipped page emits zero placement; Chrome band has none',async()=>{
 for(const kind of ['region','browser_chrome']){
  const f=fixture(kind),r=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
  try{if(kind==='region')assert.deepEqual(r.region_offset,{x:0,y:0});else assert(!Object.hasOwn(r,'region_offset'));}
  finally{await [...f.host.displays.values()][0].close();}
 }
});
for(const value of [undefined,{x:-1,y:0},{x:0,y:NaN},{x:Infinity,y:0}])test(`MP-11 invalid private element offset refuses before native capture ${JSON.stringify(value)}`,async()=>{
 const f=fixture();f.lease.element_offset=value;
 await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture}));
 assert.equal(f.host.displays.size,0);assert(!f.calls.some(x=>x?.surface));assert.equal(f.lease.isAlive(),false);
});

test('MP-08/MP-10/MP-11 529 placement keeps native page origin out of CSS paint/input offsets',async()=>{
 const f=fixture();f.binding.page_rect={left:8,top:174,width:2544,height:1592};f.binding.isPageCurrent=async()=>true;
 const result=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const s=[...f.host.displays.values()][0];
 assert.deepEqual(s.regionSource.options.rect,{left:28,top:214,width:200,height:100});
 assert.deepEqual(s.regionSource.options.contentRect,f.binding.page_rect);
 assert.deepEqual(result.region_offset,{x:0,y:0});assert.deepEqual(s.regionOffset,{x:0,y:0});await s.close();
});
test('MP-08/MP-10/MP-11 page crop clips to the actual native viewport before inward rounding',async()=>{
 const f=fixture();f.binding.page_rect={left:8,top:174,width:2544,height:1592};f.binding.isPageCurrent=async()=>true;
 f.lease.rect={x:1270,y:790,width:10,height:20};
 const result=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const s=[...f.host.displays.values()][0];assert.deepEqual(s.regionSource.options.rect,{left:2548,top:1754,width:4,height:12});
 assert.equal(result.css_width,2);assert.equal(result.css_height,6);assert.deepEqual(result.region_offset,{x:0,y:0});await s.close();
});

// Protocol 530: a popup of the band's window (menu, omnibox dropdown, extension popup) by its kernel id.
test('MP-08/MP-11 a band popup subscribes by kernel id, retires with the popup and takes input through the arbiter',async()=>{
 const f=fixture('browser_auxiliary');let listed=true,watched=0;
 const popup={auxiliary_id:4,role:'auxiliary',xid:77,parent_xid:42,bounds:{x:2000,y:140,width:496,height:800},isCurrent:()=>listed};
 Object.assign(f.host.chromeWindows,{auxiliary:id=>{if(id!==4)throw Object.assign(Error('x'),{code:'user_domain_stale_reference'});return popup;},
  auxiliaryRegion:()=>({surface:'browser_auxiliary',window:{id:77,width:496,height:800},rect:{left:0,top:0,width:496,height:800},isCurrent:()=>listed}),
  watch:()=>(watched++,()=>{})});
 f.command.target.auxiliary_id=4;
 const r=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 assert.equal(r.css_width,248);assert.equal(r.css_height,400);assert.equal(watched,0,'only a band watches its window');
 const s=[...f.host.displays.values()][0];assert.equal(s.regionSource.options.surface,'browser_auxiliary');assert.equal(s.regionSource.options.contentHeight,undefined);
 const inputs=[];f.host.arbiter={input:async({stream,command})=>(inputs.push([stream.regionSource.options.surface,command.input.kind]),{dispatched:true})};
 const command={op:'input',tab_id:'t',generation:2,document_id:r.document_id,binding_id:r.binding_id,sequence:r.sequence,input:{kind:'click',x:3,y:4}};
 assert.deepEqual(await regionInput(f.host,command,'terminal:a'),{dispatched:true});assert.deepEqual(inputs,[['browser_auxiliary','click']]);
 listed=false;assert.equal(s.regionLease.isAlive(),false,'a closed popup retires its stream');
 await assert.rejects(regionInput(f.host,command,'terminal:a'));await s.close();
 for(const target of [{kind:'browser_auxiliary',window_generation:7,auxiliary_id:5},{kind:'browser_auxiliary',window_generation:6,auxiliary_id:4},{kind:'browser_auxiliary',window_generation:7}]){
  const g=fixture('browser_auxiliary');Object.assign(g.host.chromeWindows,f.host.chromeWindows);listed=true;g.command.target=target;
  await assert.rejects(subscribeRegion(g.host,g.command,'terminal:a',undefined,{Capture:g.Capture}));assert.equal(g.host.displays.size,0);
 }
 const band=fixture('browser_chrome');band.host.chromeWindows.watch=()=>(watched++,()=>{});
 class Watched extends band.Capture{windows(){return [];}}
 const b=await subscribeRegion(band.host,band.command,'terminal:a',undefined,{Capture:Watched});assert.equal(watched,1,'a live band follows its popups');
 await [...band.host.displays.values()][0].close();assert(b.binding_id);
});
test('MP-11 page regions never take middle/right buttons or drags',async()=>{
 const f=fixture();await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 let dispatch=0;f.host.dispatchRegionInput=async()=>{dispatch++;};
 const command={op:'input',tab_id:'t',generation:2,document_id:'doc',binding_id:'opaque',sequence:3};
 for(const input of [{kind:'click',x:1,y:2,button:'right'},{kind:'click',x:1,y:2,button:'middle'},{kind:'drag',x:1,y:2,to_x:3,to_y:4}])await assert.rejects(regionInput(f.host,{...command,input},'terminal:a'));
 assert.equal(dispatch,0);await [...f.host.displays.values()][0].close();
});

// Coordinator 12:22Z ask 1 (r8 live drill): a browser-modal dialog's page overlay shows Chrome's ACTIVE Tab.
// Its frames belong to the window epoch like every Chrome UI stream (the client binds them to it); the
// active Tab's own document and session fence its page masks and attestation.
test('MP-08/MP-11 a dialog page overlay binds the active Tab for masks and keeps the window epoch for frames',async()=>{
 const f=fixture('browser_auxiliary'),active={tab_id:'a',target_id:'target-a',document_id:'doc-a'};
 const overlay={auxiliary_id:9,role:'auxiliary',xid:42,parent_xid:42,in_window:true,page_overlay:true,bounds:{x:0,y:174,width:2560,height:1600},isCurrent:()=>true};
 const shots=[];f.host.displayScreenshot=async tab=>{shots.push(tab.tab_id);};
 const sessions=[];f.host.browser.resolvePageTarget=async target=>(sessions.push(target),{connection:{},sessionId:'s-'+target});
 Object.assign(f.host.chromeWindows,{auxiliary:()=>overlay,auxiliaryRegion:()=>({surface:'browser_auxiliary',inWindow:true,pageOverlay:true,window:{id:42,width:2560,height:1774},rect:{left:0,top:174,width:2560,height:1600},contentHeight:1600,isCurrent:()=>true})});
 let front=true;f.host.arbiter={activeTab:async()=>active,isActiveTab:async(tab)=>front&&tab===active};
 f.command.target.auxiliary_id=9;
 const r=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const s=[...f.host.displays.values()][0],o=s.regionSource.options;
 assert.deepEqual(sessions,['target-a'],'the active Tab\'s page session');assert.equal(o.sessionId,'s-target-a');
 assert.equal(o.tab.target_id,'target-a');assert.match(o.tab.document_id,/^chrome-window-/,'frames carry the window epoch');assert.equal(r.document_id,o.tab.document_id);
 assert.equal(o.pageDocument,'doc-a','the page document fences its masks');
 await o.screenshot();assert.deepEqual(shots,['a']);
 assert.equal(await o.validateCurrent(),true);front=false;assert.equal(await o.validateCurrent(),false,'another Tab in front: no readback');
 await s.close();
 const g=fixture('browser_auxiliary');Object.assign(g.host.chromeWindows,f.host.chromeWindows);g.host.arbiter={activeTab:async()=>null,isActiveTab:async()=>false};g.command.target.auxiliary_id=9;
 await assert.rejects(subscribeRegion(g.host,g.command,'terminal:a',undefined,{Capture:g.Capture}),'no confirmed active Tab: refused');
});

test('MP-11 the omnibox dropdown over the page binds the active Tab for masks and its readback fence',async()=>{
 const f=fixture('browser_auxiliary'),active={tab_id:'a',target_id:'target-a',document_id:'doc-a'};
 const popup={auxiliary_id:9,role:'auxiliary',xid:42,parent_xid:42,in_window:true,bounds:{x:238,y:174,width:2156,height:176},isCurrent:()=>true};
 const sessions=[];f.host.browser.resolvePageTarget=async target=>(sessions.push(target),{connection:{},sessionId:'s-'+target});
 Object.assign(f.host.chromeWindows,{auxiliary:()=>popup,auxiliaryRegion:()=>({surface:'browser_auxiliary',inWindow:true,window:{id:42,width:2560,height:1774},rect:{left:238,top:174,width:2156,height:176},contentHeight:1600,pageRect:{left:0,top:174,width:2560,height:1600},isCurrent:()=>true})});
 let front=true;f.host.arbiter={activeTab:async()=>active,isActiveTab:async(tab)=>front&&tab===active};
 f.command.target.auxiliary_id=9;
 await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const s=[...f.host.displays.values()][0],o=s.regionSource.options;
 assert.deepEqual(sessions,['target-a'],'the active Tab\'s page session measures the masks');assert.equal(o.tab.target_id,'target-a');
 assert.match(o.tab.document_id,/^chrome-window-/,'frames carry the window epoch');
 assert.equal(await o.validateCurrent(),true);front=false;assert.equal(await o.validateCurrent(),false,'another Tab in front: no readback');
 await s.close();
 const g=fixture('browser_auxiliary');Object.assign(g.host.chromeWindows,f.host.chromeWindows);g.host.arbiter={activeTab:async()=>null,isActiveTab:async()=>false};g.command.target.auxiliary_id=9;
 await assert.rejects(subscribeRegion(g.host,g.command,'terminal:a',undefined,{Capture:g.Capture}),'no confirmed active Tab: refused');
});

test('MP-08/MP-10/MP-11 concurrent native starts reserve the bounded display capacity',async()=>{
 const f=fixture('browser_chrome'),release=Promise.withResolvers(),entered=Promise.withResolvers();let starting=0;
 class Capture extends f.Capture {async start(){if(++starting===maxDisplays)entered.resolve();await release.promise;return this;}}
 const pending=Array.from({length:maxDisplays},()=>subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture}));
 try{
  await entered.promise;
  const over=subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture}).then(()=>null,e=>e);
  const result=await Promise.race([over,new Promise(r=>setTimeout(()=>r(null),100))]);
  assert(result instanceof Error,'pending native starts must count against the display limit');
 }finally{
  release.resolve();await Promise.allSettled(pending);await new Promise(r=>setImmediate(r));
  for(const s of [...f.host.displays.values()])await s.close();
 }
 const next=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 assert(next.subscription_id);for(const s of [...f.host.displays.values()])await s.close();
});

for(const change of ['cancel','policy','active-tab'])test(`MP-11 pending native readiness retires on ${change} before registration`,async()=>{
 const f=fixture(),entered=Promise.withResolvers(),release=Promise.withResolvers(),abort=new AbortController();
 class Capture extends f.Capture {async start(){entered.resolve();await release.promise;return this;}}
 const ready=subscribeRegion(f.host,f.command,'terminal:a',abort.signal,{Capture});
 await entered.promise;
 if(change==='cancel')abort.abort();
 if(change==='policy')f.host.protection={unknown:false};
 if(change==='active-tab')f.activate('foreign');
 release.resolve();await assert.rejects(ready);assert.equal(f.host.displays.size,0);assert(f.calls.includes('close'));
});

test('MP-08/MP-10/MP-11 one native viewport query proves both page geometry and active target',async()=>{
 const f=fixture();await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const stream=[...f.host.displays.values()][0],query=f.binding.isPageCurrent,active=f.host.arbiter.isActiveTab;
 let reads=0,activeReads=0,requested;
 f.binding.isPageCurrent=async target=>{reads++;requested=target;return query(target)};
 f.host.arbiter.isActiveTab=async(...args)=>{activeReads++;return active(...args)};
 try{
  assert.equal(await stream.regionCurrent(),true);assert.equal(reads,1);assert.equal(requested,'target');assert.equal(activeReads,0);
  f.activate('foreign');assert.equal(await stream.regionCurrent(),false,'the same fresh native query refuses a different active Tab');
 }finally{await stream.close()}
});

test('MP-08/MP-10/MP-11 frame delivery reuses one fresh source lease while input stays fresh',async()=>{
 const f=fixture();let cached=true,reads=0;const fresh=f.binding.isPageCurrent;
 f.binding.isPageCurrentCached=target=>cached&&target==='target'&&f.lease.isAlive();
 f.binding.isPageCurrent=async target=>{reads++;return fresh(target)};
 await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const stream=[...f.host.displays.values()][0];reads=0;
 try{
  assert.equal(await stream.regionSource.options.validateCurrent(),true);
  assert.equal(await (stream.regionFrameCurrent??stream.regionCurrent)(),true);
  assert.equal(reads,1,'one fresh native owner query for source publication, none for delivery');
  cached=false;f.activate('foreign');assert.equal(await stream.regionFrameCurrent(),false);assert.equal(reads,1);
  await assert.rejects(regionInputTarget(f.host,{op:'input',tab_id:'t',generation:2,document_id:'doc',binding_id:'opaque',sequence:3,input:{kind:'click',x:1,y:2}},'terminal:a'));
  assert.equal(reads,2,'input never borrows the frame cache');
  assert.equal('cachedCurrent' in stream.regionSource.options,false,'the delivery cache is separate from fresh source admission');
 }finally{await stream.close()}
});

// vidlat 07:52Z (C10): band input bumps its stream's input epoch/time and emits a numeric input_dispatch.
test('MP-08/MP-10 band input marks the band stream input epoch at dispatch',async()=>{
 const f=fixture('browser_chrome'),events=[];f.host.timing=Object.assign(()=>{},{event:(name,fields)=>events.push([name,fields.kind])});
 f.host.inputEpochs=new Map();f.host.inputChangedAt=new Map();
 const r=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 f.host.arbiter={input:async({onDispatch})=>{onDispatch();return {dispatched:true};}};
 const command={op:'input',tab_id:'t',generation:2,document_id:r.document_id,binding_id:r.binding_id,sequence:r.sequence,input:{kind:'key',key:'a'}};
 assert.deepEqual(await regionInput(f.host,command,'terminal:a'),{dispatched:true});
 assert.equal(f.host.inputEpochs.get('t'),1);assert(f.host.inputChangedAt.get('t')>0);assert.deepEqual(events,[['input_dispatch',3]]);
 await [...f.host.displays.values()][0].close();
});

test('MP-08/MP-10/MP-11 naming a visible page input target needs no renderer read; a stale lease still refuses',async()=>{
 const stream=(frame)=>({binding_id:'b',observed_by:'terminal:a',tab_id:'t',expires:Date.now()+1000,regionLease:{document_id:'d',sequence:3,isAlive:()=>true},regionWindow:{isCurrent:()=>true},regionTarget:{kind:'visible_page'},
  regionCurrent:()=>new Promise(()=>{}),regionFrameCurrent:frame});
 const host=s=>({generation:2,displays:new Map([['s',s]]),tabs:new Map()}),command={op:'input',tab_id:'t',generation:2,document_id:'d',binding_id:'b',sequence:3,input:{kind:'click',x:1,y:2}};
 assert.deepEqual(await regionInputTarget(host(stream(()=>true)),command,'terminal:a'),{tab_id:'t',document_id:'d'});
 await assert.rejects(regionInputTarget(host(stream(()=>false)),command,'terminal:a'));
});

test('MP-08/MP-10/MP-11 region capture and encoder drain concurrently before releasing the lease',async()=>{
 const f=fixture(),result=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture});
 const stream=f.host.displays.get(result.subscription_id),release=Promise.withResolvers(),sourceStarted=Promise.withResolvers();
 let leaseClosed=false;
 stream.encoder.close=()=>release.promise;
 stream.regionSource.close=async()=>{sourceStarted.resolve(true);await release.promise;};
 stream.regionLease.close=async()=>{leaseClosed=true;};
 const closing=stream.close();
 let timer;
 try {
  const concurrent=await Promise.race([sourceStarted.promise,new Promise(resolve=>{timer=setTimeout(()=>resolve(false),100);})]);
  assert.equal(concurrent,true,'source shutdown must start while encoder shutdown is waiting');
  assert.equal(leaseClosed,false,'the source lease remains owned until both helpers settle');
 } finally {clearTimeout(timer);release.resolve();await closing;}
 assert.equal(leaseClosed,true);
});

// A resize can retire the admitted region while native startup awaits a mask or codec.
test('MP-08/MP-10 retired region startup reports the existing stale refusal and releases resources',async()=>{
 const f=fixture();
 class Capture extends f.Capture {async start(){await Promise.resolve();f.retire();throw Error('MP-11: native readback source not owned');}}
 await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture}),{code:'user_domain_stale_reference'});
 assert.equal(f.host.displays.size,0);assert.equal(f.lease.isAlive(),false);assert.equal(f.calls.filter(c=>c==='close').length,1);
});

test('MP-11 active-region startup failures retain the original ownership error',async()=>{
 const f=fixture(),failure=Error('MP-11: native readback source not owned');
 class Capture extends f.Capture {async start(){throw failure;}}
 await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture}),error=>error===failure);
 assert.equal(f.host.displays.size,0);assert.equal(f.lease.isAlive(),false);assert.equal(f.calls.filter(c=>c==='close').length,1);
});

test('MP-08/MP-10 a retired native window invalidates startup even while mirror geometry is unchanged',async()=>{
 const f=fixture();
 class Capture extends f.Capture {async start(){f.binding.isCurrent=()=>false;throw Error('MP-11: native readback source not owned');}}
 await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture}),{code:'user_domain_stale_reference'});
 assert.equal(f.host.displays.size,0);assert.equal(f.calls.filter(c=>c==='close').length,1);
});

for(const reason of ['region_epoch','region_target','region_identity','region_retired'])test(`MP-08/MP-10 private ${reason} during subscription is a stale public reference`,async()=>{
 for(const stage of ['resolve','start']){
  const f=fixture(),failure=Object.assign(Error(`MP-11: ${reason}`),{mirrorReason:reason});
  class Capture extends f.Capture {async start(){throw failure;}}
  if(stage==='resolve')f.host.mirror.v2.regions.resolve=async()=>{throw failure;};
  await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:stage==='start'?Capture:f.Capture}),{code:'user_domain_stale_reference'});
  assert.equal(f.host.displays.size,0);
  assert.equal(f.calls.filter(c=>c==='close').length,stage==='start'?1:0);
 }
});

test('MP-11 subscription keeps malformed, capacity, geometry and foreign-owner failures loud',async()=>{
 for(const reason of ['region_request','region_limit','region_geometry','region_input','region_display',undefined]){
  const f=fixture(),failure=Object.assign(Error('MP-11: rejected'),reason?{mirrorReason:reason}:{});
  f.host.mirror.v2.regions.resolve=async()=>{throw failure;};
  await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture:f.Capture}),error=>error===failure);
 }
});

for(const kind of ['browser_chrome','browser_auxiliary'])test(`MP-08/MP-10 retired ${kind} startup is a stale reference`,async()=>{
 const f=fixture('browser_chrome');
 if(kind==='browser_auxiliary'){
  f.command.target={kind,window_generation:7,auxiliary_id:'popup'};
  f.host.chromeWindows.auxiliary=()=>({isCurrent:()=>f.lease.isAlive()});
  f.host.chromeWindows.auxiliaryRegion=()=>f.host.chromeWindows.band();
 }
 class Capture extends f.Capture {async start(){f.retire();throw Error('MP-08: native region readback unavailable');}}
 await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Capture}),{code:'user_domain_stale_reference'});
 assert.equal(f.calls.filter(c=>c==='close').length,1);assert.equal(f.host.displays.size,0);
});

// R1 (proposal R, row 343): a browser without its own display (headless: macOS always, test rigs on Linux) gets region video
// from Chrome's own frames: the lease is granted, its source is a crop of the Tab's one shared screencast (lease rect in device
// pixels, no window offset), and two regions of one Tab share that screencast.
test('R1a/R1b: a headless browser grants a region lease as a crop of the Tab\'s one shared screencast',async()=>{
 const f=fixture();delete f.host.chromium.display;f.host.chromeWindows={current:()=>null,resolve:async()=>{throw Error('no window on a headless browser')}};
 const shared=[];f.host.screencastFor=async(tab,scale)=>{let entry=shared.find(s=>s.tab===tab);if(!entry){entry={tab,scale,starts:1};shared.push(entry);}return entry;};
 class Screencast {constructor(options){this.options=options;this.region={css_width:options.rect.width/options.scale,css_height:options.rect.height/options.scale};f.calls.push(['screencast',options]);}async start(){return this;}async close(){f.calls.push('close');}valid(){return true;}}
 const first=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Screencast});
 assert.equal(first.source_kind,'region');assert.equal(first.css_width,100);assert.equal(first.css_height,50);assert.equal(first.device_scale_factor,2);
 const source=[...f.host.displays.values()][0].regionSource;
 assert(source instanceof Screencast,'the region source is the screencast crop');
 assert.deepEqual(source.options.rect,{left:10,top:20,width:100,height:50},'the lease rect relative to the page, at DPR1 geometry (R2)');
 await subscribeRegion(f.host,{...f.command},'terminal:a',undefined,{Screencast});
 assert.equal(shared.length,1,'two regions of one Tab share one screencast');
 for(const stream of [...f.host.displays.values()])await stream.close();
});

// R1: a region's view of the shared screencast: frames at the crop's size with the crop for the encoder; a fenced or closed shared
// source fences the region; closing the region only releases the shared source.
test('R1: a screencast region crops the shared frames and follows the shared fences',async()=>{
 const {ScreencastRegionCapture}=await import('./kernel-browser-screencast-region.mjs');
 const listeners=new Set(),source={closed:false,fenced:false,regionRevision:4,changedAt:1,subscribe(fn){listeners.add(fn);return()=>listeners.delete(fn);},sample:()=>({data_base64:'j',width:2560,height:1600,serial:7,signature:'s'})};
 let released=0;const shared={source,ready:Promise.resolve(),release:()=>{released++;}};
 const policy={},rect={left:20,top:40,width:200,height:100};
 const region=new ScreencastRegionCapture({shared,tab:{tab_id:'t'},scale:2,policy,rect,allowed:p=>p===policy});
 await region.start();
 assert.deepEqual(region.region,{css_width:100,css_height:50});
 assert.deepEqual(region.sample(),{data_base64:'j',width:200,height:100,serial:7,signature:'s',crop:{x:20,y:40,width:200,height:100},motion:true},'the shared frame named at the crop');
 const seen=[];region.subscribe(f=>seen.push(f));for(const l of listeners)l({data_base64:'k',width:2560,height:1600,serial:8});
 assert.deepEqual(seen.map(f=>[f.serial,f.width,f.crop.x]),[[8,200,20]]);
 source.fenced=true;assert.equal(region.valid(),false,'a fenced shared source fences the region');assert.equal(region.sample(),null);
 source.fenced=false;await region.close();assert.equal(released,1,'closing releases the shared source');assert.equal(listeners.size,0);
});

// R1c: a page with Vault-protected values or targets on it never gets region video from the screencast (as the whole-tab
// fallback): the lease is refused, and a region already streaming stops being valid once protection appears.
test('R1c: a protected page gets no screencast region video',async()=>{
 const f=fixture();delete f.host.chromium.display;f.host.chromeWindows={current:()=>null};
 f.host.screencastFor=async()=>({source:{closed:false,fenced:false,subscribe:()=>()=>{},sample:()=>null},ready:Promise.resolve(),release:()=>{}});
 const {ScreencastRegionCapture}=await import('./kernel-browser-screencast-region.mjs');
 f.host.protection={unknown:false,values:['v'],targets:[]};
 await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Screencast:ScreencastRegionCapture}),/refused|fenced/);
 assert.equal(f.host.displays.size,0,'no region stream is left');
 const g=fixture();delete g.host.chromium.display;g.host.chromeWindows={current:()=>null};g.host.screencastFor=f.host.screencastFor;
 g.host.protection={unknown:false,values:[],targets:[]};
 await subscribeRegion(g.host,g.command,'terminal:a',undefined,{Screencast:ScreencastRegionCapture});
 const source=[...g.host.displays.values()][0].regionSource;assert.equal(source.valid(),true);
 g.host.protection={unknown:false,values:[],targets:['t']};assert.equal(source.valid(),false,'protection appearing stops the region');
 for(const stream of [...g.host.displays.values()])await stream.close();
});

// R1 S2/S3 (orch14 decision 11-22): a navigation fences the region screencast for good (no frame of the new document goes out on
// it); a protection event (DOM rebuilt, a target attribute) fences at once and no frame captured before that fence goes out later.
test('R1 S2/S3: navigation and protection events fence the region screencast; older frames never go out after a fence',async()=>{
 const {RegionScreencast}=await import('./kernel-browser-screencast-region.mjs');
 let clock=1_000_000;const sent=[],listeners=new Set();
 const connection={subscribe(fn){listeners.add(fn);return()=>listeners.delete(fn);},send:async(method)=>{sent.push(method);return {};}};
 const policy={unknown:false,values:[],targets:[]},tab={tab_id:'t',document_id:'doc'};
 const cast=new RegionScreencast({connection,sessionId:'s',tab,width:100,height:50,policy,allowed:p=>p===policy,now:()=>clock});
 await cast.start();
 const out=[];cast.subscribe(f=>out.push(f.capturedAt));
 const frame=at=>{for(const l of listeners)l({sessionId:'s',method:'Page.screencastFrame',params:{sessionId:1,data:'j',metadata:{timestamp:at/1000}}});};
 frame(clock-10);assert.equal(out.length,1,'a frame goes out');
 clock+=100;for(const l of listeners)l({sessionId:'s',method:'DOM.documentUpdated',params:{}});
 assert.equal(cast.sample(),null,'S3: the protection event drops the latest frame');
 frame(clock-50);assert.equal(out.length,1,'S3: a frame captured before the fence never goes out after it');
 frame(clock+5);assert.equal(out.length,2,'S3: frames after the fence go out while the policy still allows them');
 for(const l of listeners)l({sessionId:'s',method:'Page.frameNavigated',params:{frame:{id:'main',loaderId:'new'}}});
 frame(clock+50);assert.equal(out.length,2,'S2: nothing goes out after a navigation');assert.equal(cast.valid(),false);assert.equal(cast.sample(),null);
 await cast.close();assert(sent.includes('Page.stopScreencast'));
});

// R2 (P5 for screencast regions): a DPR2 viewer's region gets motion at DPR1 geometry (R0: full DPR2 frames cost 3x headed CPU
// and +43 ms P95; DPR1 geometry 1.8x and +25 ms). The shared screencast runs at the page's CSS size and the crop is the lease
// rect in CSS pixels; the exact still at rest stays the lossless DPR2 picture (unchanged).
test('R2: a DPR2 region of a display-less browser crops a DPR1-geometry screencast',async()=>{
 const f=fixture();delete f.host.chromium.display;f.host.chromeWindows={current:()=>null};
 const asked=[];f.host.screencastFor=async(tab,scale)=>{asked.push(scale);return {source:{closed:false,fenced:false,subscribe:()=>()=>{},sample:()=>null},ready:Promise.resolve(),release:()=>{}};};
 class Screencast {constructor(options){this.options=options;this.region={css_width:options.rect.width/options.scale,css_height:options.rect.height/options.scale};}async start(){return this;}async close(){}valid(){return true;}}
 const result=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Screencast});
 assert.deepEqual(asked,[1],'the shared screencast is asked at DPR1 geometry');
 const source=[...f.host.displays.values()][0].regionSource;
 assert.deepEqual(source.options.rect,{left:10,top:20,width:100,height:50},'the crop is the lease rect in CSS pixels');
 assert.equal(result.css_width,100);assert.equal(result.css_height,50);
 for(const stream of [...f.host.displays.values()])await stream.close();
});

// R1 review 1 (P2): a screencast region returns the same validated placement as a native one: the opaque element's clip origin
// plus the inward rounding (motion runs at DPR1 geometry, R2), captured at admission; input keeps only the rounding.
const headless=f=>{delete f.host.chromium.display;f.host.chromeWindows={current:()=>null};
 f.host.screencastFor=async()=>({source:{closed:false,fenced:false,subscribe:()=>()=>{},sample:()=>null},ready:Promise.resolve(),release:()=>{}});
 return class Screencast {constructor(options){this.options=options;this.region={css_width:options.rect.width,css_height:options.rect.height};}async start(){return this;}async close(){}valid(){return true;}};};
for(const scale of [1,2])test(`R1: a clipped screencast region carries its placement with inward rounding at DPR${scale}`,async()=>{
 const f=fixture();f.lease.device_scale_factor=scale;const Screencast=headless(f);
 f.lease.rect={x:10.2,y:20.2,width:100.1,height:50.1};f.lease.element_offset={x:12.25,y:30.5};
 const r=await subscribeRegion(f.host,f.command,'terminal:a',undefined,{Screencast});
 const stream=[...f.host.displays.values()][0];
 try{
  assert.deepEqual(r.region_offset,{x:12.25+Math.ceil(10.2)-10.2,y:30.5+Math.ceil(20.2)-20.2});
  assert.deepEqual(stream.regionPlacement,r.region_offset);
  f.lease.element_offset.x=999;assert.equal(stream.regionPlacement.x,r.region_offset.x,'captured at admission');
  assert.deepEqual(stream.regionOffset,{x:Math.ceil(10.2)-10.2,y:Math.ceil(20.2)-20.2},'input uses only the rounding');
 }finally{await stream.close();}
});
for(const value of [undefined,{x:-1,y:0},{x:0,y:NaN}])test(`R1: a screencast region refuses an invalid element offset ${JSON.stringify(value)}`,async()=>{
 const f=fixture();const Screencast=headless(f);f.lease.element_offset=value;
 await assert.rejects(subscribeRegion(f.host,f.command,'terminal:a',undefined,{Screencast}));
 assert.equal(f.host.displays.size,0);
});
