// MD-DISPLAY-02/04: native admission fails before opening any foreign display.
import test from 'node:test';
import assert from 'node:assert/strict';
import { OwnedDisplay, ownsDisplay } from './kernel-browser-owned-display.mjs';
import { selectNativeCapture, nativeRefusalReason } from './kernel-browser-native.mjs';
for(const display of [null,{}, {display:':0',pid:200}, {display:':99',owned:true}])
 test('MD-DISPLAY refuses unowned display '+JSON.stringify(display),async()=>{
  let launched=false;
  assert.equal(ownsDisplay(display),false);
  assert.equal(await selectNativeCapture({platform:'linux',display,create:()=>{launched=true}}),null);
  assert.equal(launched,false);
 });
test('MD-DISPLAY macOS unsupported falls back without opening display',async()=>{
 let called=false;assert.equal(await selectNativeCapture({platform:'darwin',create:()=>called=true}),null);assert.equal(called,false);
});
test('MD-DISPLAY owned display refuses unsafe IDs and environment adoption',()=>{
 const display=new OwnedDisplay('/tmp/not-started');
 for(const pid of [0,1,-1,undefined,NaN,2.1]){display.child={pid,exitCode:null,signalCode:null};assert.equal(ownsDisplay(display),false)}
 assert.equal(ownsDisplay(new OwnedDisplay('/tmp/other',{environment:{DISPLAY:':0'}})),false);
});
test('MD-DISPLAY selection admits only a live kernel-created server; retirement fences it',async()=>{
 const {mkdtemp,rm}=await import('node:fs/promises');const {tmpdir}=await import('node:os');
 const root=await mkdtemp(tmpdir()+'/chariox-md-native-test-'),display=new OwnedDisplay(root);
 try{
  await display.start();assert.equal(ownsDisplay(display),true);
  let called=false;const chosen=await selectNativeCapture({platform:'linux',display,create:()=>{called=true;return 'native'}});
  assert.equal(chosen,'native');assert.equal(called,true);
  assert.equal(await selectNativeCapture({platform:'linux',display,create:()=>{throw Error('XShm absent')}}),null);
  await display.close();assert.equal(ownsDisplay(display),false);
 }finally{await display.close();await rm(root,{recursive:true,force:true})}
});
test('MD-DISPLAY raw capture rejects a hidden or replaced source before publishing',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 for(const change of ['hidden','navigation','policy']){
  let allowed=true;
  const connection={send:async method=>method==='Page.getFrameTree'?{frameTree:{frame:{id:'frame',loaderId:change==='navigation'?'new':'document'}}}:{result:{value:'hidden'}}};
  const source=new LinuxCapture({connection,sessionId:'session',tab:{target_id:'target',tab_id:'tab',document_id:'document'},policy:{},allowed:()=>allowed});source.valid=()=>!source.closed&&allowed;source.attested=true;source.contextId=1;
  source.pending={serial:1,signature:'a'.repeat(64),width:1280,height:800};
  let emitted=false;source.subscribe(()=>emitted=true);
  if(change==='policy')allowed=false;
  await source.publish();assert.equal(emitted,false);assert.equal(source.sample(),null);await source.close();
 }
});

test('MP-10 input wake never writes to a fenced capture or destroyed pipe',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const writes=[],capture=new LinuxCapture({});let valid=true;capture.valid=()=>valid;
 capture.child={stdin:{destroyed:false,write:text=>writes.push(JSON.parse(text))}};
 capture.wake();valid=false;capture.wake();valid=true;capture.child.stdin.destroyed=true;capture.wake();
 assert.deepEqual(writes,[{wake:true}]);
});

// MP-11: an empty DOM admission is event-bound before/after native attestation.
test('MP-11 marker insertion retires native pixels even while attestation is pending',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 for(const phase of ['refreshing','attesting']){
  const source=new LinuxCapture({sessionId:'session'});source.regions=new NativeRegionProtection({send:async method=>method==='DOM.getDocument'?{root:{nodeId:1}}:{nodeIds:[]}},'session');
  if(phase!=='refreshing')await source.regions.refresh();source.attested=phase==='streaming';let released=0;
  source.latest={raw:{release:()=>released++}};source.pending={release:()=>released++};
  source.onCdp({sessionId:'foreign',method:'DOM.attributeModified',params:{name:'data-chariox-observation-protected'}});assert.equal(source.closed,false);
  source.onCdp({sessionId:'session',method:'DOM.attributeModified',params:{name:'data-chariox-observation-protected'}});
  // MP-08/MP-10: the source stays open; attestation needs a fresh, refenced readback.
  assert.equal(source.closed,false);assert.equal(source.attested,false);assert.equal(source.regions.guard,null);assert.equal(source.latest,null);assert.equal(source.pending,null);assert.equal(released,2);assert.equal(source.regionRevision,1);await source.close();
 }
});

test('MP-08/MP-10/MP-11 protection changes retire pixels without re-attesting the owned window',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 let maskX=0,refreshes=0,released=0;
 const connection={send:async method=>{
  if(method==='Page.getFrameTree')return {frameTree:{frame:{loaderId:'d'}}};
  if(method==='Runtime.evaluate')return {result:{value:'visible'}};
  if(method==='DOM.getDocument'){refreshes++;return {root:{nodeId:1}};}
  if(method==='DOM.querySelectorAll')return {nodeIds:[2]};
  if(method==='DOM.getBoxModel')return {model:{border:[maskX,0,maskX+4,0,maskX+4,4,maskX,4]}};
  assert.fail('protection refresh must not run window setup/screenshot attestation: '+method);
 }};
 const source=new LinuxCapture({connection,sessionId:'s',tab:{target_id:'t',tab_id:'tab',document_id:'d'},policy:{},allowed:()=>true});
 source.valid=()=>!source.closed;source.attested=true;source.contextId=1;
 source.regions=new NativeRegionProtection(connection,'s');await source.regions.refresh();
 source.latest={raw:{release:()=>released++}};
 maskX=8;source.onCdp({sessionId:'s',method:'DOM.attributeModified',params:{name:'data-chariox-observation-protected'}});
 assert.equal(source.closed,false);assert.equal(source.sample(),null);assert.equal(released,1);assert.equal(source.regionRevision,1);
 source.pending={width:1280,height:800,length:1280*800*4,pixels:Buffer.alloc(1280*800*4,255),serial:2,format:'bgr0',captured_ms:performance.timeOrigin+performance.now()};
 await source.publish();assert.equal(refreshes,2,'old readback gets full mask after refresh; no surface attestation');
 // This raw precedes the new metadata fence and must never be published.
 assert.equal(source.sample(),null);
 source.pending={width:1280,height:800,length:1280*800*4,pixels:Buffer.alloc(1280*800*4,255),serial:3,format:'bgr0',captured_ms:performance.timeOrigin+performance.now()};
 await source.publish();assert.equal(source.sample().raw.pixels[8*4],0);assert.equal(source.sample().raw.pixels[10000],255);
 source.onCdp({sessionId:'s',method:'Page.frameNavigated',params:{frame:{}}});assert.equal(source.closed,true);
 await source.close();
});

// MP-08/MP-10/MP-11: the Python fallback supports wake/refresh/release only.
test('MP-11 Python capture stays available across planning credits and uses fenced CDP wheel',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const {inputHostTab}=await import('./kernel-browser-input.mjs');
 const previous=process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER;
 delete process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER;
 try{
  const writes=[],source=new LinuxCapture({scale:1});source.valid=()=>!source.closed;source.attested=true;
  source.child={stdin:{destroyed:false,write:text=>{
   const command=JSON.parse(text);writes.push(command);
   if('plans'in command||'wheel'in command)source.closed=true;
  }}};
  for(const enabled of [false,true,false,true])source.plans(enabled);
  assert.equal(source.closed,false,'unsupported planning must never retire Python capture');
  assert.deepEqual(writes,[]);
  const sent=[],connection={send:async(method,params)=>{
   sent.push({method,params});return method==='Page.getFrameTree'?{frameTree:{frame:{loaderId:'d'}}}:{};
  }};
  const browser={resolvePageTarget:async()=>({connection,sessionId:'s'}),inputCapture:{run:async(_c,_s,fn)=>fn()}};
  let dispatched=0;
  await inputHostTab(browser,{target_id:'t',document_id:'d'},{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120},{nativeWheel:(...args)=>source.wheel(...args),onDispatch:()=>dispatched++});
  assert.equal(source.closed,false);assert.deepEqual(writes,[]);assert.equal(dispatched,1);
  assert.equal(sent.filter(call=>call.method==='Input.dispatchMouseEvent'&&call.params.type==='mouseWheel').length,1);
  source.wake();assert.deepEqual(writes,[{wake:true}]);
 }finally{if(previous===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER;else process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER=previous}
});

test('MP-08/MP-10 Rust capture retains supported planning and wheel controls',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const commands=[],requests=[],source=new LinuxCapture({scale:1});source.valid=()=>true;source.attested=true;
 source.child={stdin:{destroyed:false}};source.nativeWorker={notify:(command,immediate)=>(commands.push({command,immediate}),true),request:async(operation,values)=>(requests.push({[operation]:values}),{wheeled:true})};
 source.plans(false);source.plans(false);source.plans(true);
 assert.equal(await source.wheel(10,20,0,1),true);
 assert.deepEqual(commands.map(c=>c.command),[{plans:false},{plans:true}]);
 assert.deepEqual(requests,[{wheel:{point:[10,20],notches:[0,1]}}]);
});
// MP-11 (review #893 P2): a pointer refused by the worker (another top-level
// window covers the point) or a dead worker reports false; never true.
test('MP-11 native pointer refusals are reported, not assumed dispatched',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 for(const [reply,expected] of [[{clicked:true},true],[{clicked:false},false],[Error('MP-11: native worker closed'),false]]){
  const source=new LinuxCapture({scale:2});source.valid=()=>true;source.attested=true;source.child={stdin:{destroyed:false}};
  const requests=[];source.nativeWorker={request:async(operation,values)=>{requests.push({[operation]:values});if(reply instanceof Error)throw reply;return reply}};
  assert.equal(await source.click(10,20),expected);assert.deepEqual(requests,[{click:{point:[20,40]}}]);
 }
});

// MP-08/MP-10/MP-11: The host window uses physical DPR1; negotiated page DPR
// sets physical capture bounds before exact RGB attestation.
for(const scale of [1,2])test(`MP-10 native window fits negotiated DPR${scale} before exact attestation`,async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const {displayGeometry:geometry}=await import('./kernel-browser-geometry.mjs');
 const calls=[],stop=Error('bounds observed');
 const connection={subscribe:()=>()=>{},send:async(method,params)=>{
  calls.push({method,params});if(method==='Browser.getWindowForTarget')return {windowId:7};
  if(method==='Browser.setWindowBounds')throw stop;
  assert.fail('readback cannot precede bounded window setup');
 }};
 const source=new LinuxCapture({pid:42,connection,tab:{target_id:'own'},scale});source.valid=()=>true;
 await assert.rejects(source.start(),stop);
 assert.deepEqual(calls[1],{method:'Browser.setWindowBounds',params:{windowId:7,bounds:{width:geometry.width*scale,height:geometry.height*scale+87}}});
 assert.equal(source.attested,false,'bounds do not admit a surface');assert.equal(source.latest,null);
});

// MP-08/MP-10/MP-11: renderer image scale must match the physical DPR1 host
// window; deviceScaleFactor alone produces a different native DPR2 surface.
for(const scale of [1,2])test(`MP-10 negotiated DPR${scale} also selects native view image scale`,async()=>{
 const geometry=await import('./kernel-browser-geometry.mjs');
 assert.deepEqual(geometry.displayDeviceMetrics(1280,800,scale),{width:1280,height:800,deviceScaleFactor:scale,scale,mobile:false});
});

// MP-08/MP-10: refusal diagnostics are fixed labels, never error/page text.
test('MP-10 native refusals carry fixed reasons',async()=>{
 assert.equal(nativeRefusalReason(Error('Capture protection bounds unavailable')),'region_bounds');
 assert.equal(nativeRefusalReason(Error('Capture protection tree limit exceeded')),'region_tree_limit');
 assert.equal(nativeRefusalReason(Error('native attestation RGB differed')),'attestation_mismatch');
 assert.equal(nativeRefusalReason(Error('MD-DISPLAY: native readback unavailable'),'window_not_found'),'window_not_found');
 assert.equal(nativeRefusalReason(Error('MD-DISPLAY: native readback unavailable')),'first_frame_timeout');
 assert.equal(nativeRefusalReason(Error('secret https://example.test/page')),'other');
 const reasons=[];assert.equal(await selectNativeCapture({platform:'linux',display:null,refused:r=>reasons.push(r),create:()=>assert.fail()}),null);
 assert.deepEqual(reasons,['display_not_owned']);
});

// MP-08/MP-10/MP-11 phase 1.3: readbacks publish without a CDP round trip;
// navigation fences the source by event before any later delivery.
test('MP-08/MP-10 native readbacks publish without per-frame CDP fences; navigation still fences by event',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 const sent=[];const connection={send:async method=>{sent.push(method);if(method==='DOM.getDocument')return {root:{nodeId:1}};if(method==='DOM.querySelectorAll')return {nodeIds:[]};assert.fail('MP-10: per-readback CDP call '+method);}};
 const source=new LinuxCapture({connection,sessionId:'s',tab:{target_id:'t',tab_id:'tab',document_id:'d'},policy:{},allowed:()=>true});
 source.valid=()=>!source.closed;source.attested=true;source.regions=new NativeRegionProtection(connection,'s');await source.regions.refresh();
 const setup=sent.length,published=[];source.subscribe(sample=>published.push(sample.serial));
 for(let serial=1;serial<=5;serial++){source.pending={width:1280,height:800,length:1280*800*4,pixels:Buffer.alloc(1280*800*4,255),serial,format:'bgr0',captured_ms:performance.timeOrigin+performance.now()};await source.publish();}
 assert.deepEqual(published,[1,2,3,4,5]);assert.equal(sent.length,setup,'no CDP call per readback');
 source.onCdp({sessionId:'s',method:'Page.frameNavigated',params:{frame:{}}});
 assert.equal(source.closed,true);assert.equal(source.sample(),null,'a navigated source never offers its pixels');await source.close();
});
