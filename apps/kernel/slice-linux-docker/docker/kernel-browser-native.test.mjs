// MD-DISPLAY-02/04: native admission fails before opening any foreign display.
import test from 'node:test';
import assert from 'node:assert/strict';
import { OwnedDisplay, ownsDisplay } from './kernel-browser-owned-display.mjs';
import { LinuxCapture, selectNativeCapture, nativeRefusalReason } from './kernel-browser-native.mjs';
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
test('MP-11 field type changes retire native pixels even while attestation is pending',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 for(const phase of ['refreshing','attesting']){
  const source=new LinuxCapture({sessionId:'session'});source.regions=new NativeRegionProtection({},'session',{policy:{targets:[{}],values:[]}});source.regions.measure=async()=>[];
  if(phase!=='refreshing')await source.regions.refresh();source.attested=phase==='streaming';let released=0;
  source.latest={raw:{release:()=>released++}};source.pending={release:()=>released++};
  source.onCdp({sessionId:'foreign',method:'DOM.attributeModified',params:{name:'type'}});assert.equal(source.closed,false);
  source.onCdp({sessionId:'session',method:'DOM.attributeModified',params:{name:'type'}});
  // MP-08/MP-10: the source stays open; attestation needs a fresh, refenced readback.
  assert.equal(source.closed,false);assert.equal(source.attested,false);assert.equal(source.regions.guard,null);assert.equal(source.latest,null);assert.equal(source.pending,null);assert.equal(released,2);assert.equal(source.regionRevision,1);await source.close();
 }
});

test('MP-08/MP-10/MP-11 protection changes retire pixels without re-attesting the owned window',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 let maskX=0,refreshes=0,released=0;
 const connection={send:async method=>assert.fail('metadata refresh must not re-attest the window: '+method)};
 const source=new LinuxCapture({connection,sessionId:'s',tab:{target_id:'t',tab_id:'tab',document_id:'d'},policy:{},allowed:()=>true});
 source.valid=()=>!source.closed;source.attested=true;source.contextId=1;
 source.regions=new NativeRegionProtection(connection,'s',{policy:{targets:[{}],values:[]}});source.regions.measure=async()=>{refreshes++;return [[maskX,0,4,4]];};await source.regions.refresh();
 source.latest={raw:{release:()=>released++}};
 maskX=8;source.onCdp({sessionId:'s',method:'DOM.attributeModified',params:{name:'type'}});
 assert.equal(source.closed,false);assert.equal(source.sample(),null);assert.equal(released,1);assert.equal(source.regionRevision,1);
 source.pending={width:1280,height:800,length:1280*800*4,pixels:Buffer.alloc(1280*800*4,255),serial:2,format:'bgr0',captured_ms:performance.timeOrigin+performance.now()};
 await source.publish();assert.equal(refreshes,2,'old readback is refused after refresh; no surface attestation');
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
  let dispatched=0,finished=0;
  await inputHostTab(browser,{target_id:'t',document_id:'d'},{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120},{nativeWheel:(...args)=>source.wheel(...args),onDispatch:()=>{dispatched++;return()=>finished++}});
  assert.equal(source.closed,false);assert.deepEqual(writes,[]);assert.equal(dispatched,1);assert.equal(finished,1,"the admitted dispatch lease is released");
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
// window covers the point) or never sent reports false; never true.
test('MP-11 native pointer refusals are reported, not assumed dispatched',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const notSent=Object.assign(Error('MP-11: native control unavailable'),{dispatched:false});
 for(const [reply,expected] of [[{clicked:true},true],[{clicked:false},false],[notSent,false]]){
  const source=new LinuxCapture({scale:2});source.valid=()=>true;source.attested=true;source.child={stdin:{destroyed:false}};
  const requests=[];source.nativeWorker={request:async(operation,values)=>{requests.push({[operation]:values});if(reply instanceof Error)throw reply;return reply}};
  assert.equal(await source.click(10,20),expected);assert.deepEqual(requests,[{click:{point:[20,40]}}]);assert.equal(source.closed,false);
 }
});
// MP-11 (review #893 @8067044d1 P2): the worker dispatches before it
// replies; a lost reply leaves the action uncertain. It must not read as a
// refusal (the caller would replay it via CDP) and the source retires.
test('MP-11 a native pointer whose reply is lost is uncertain, never a refusal',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const {NativeWorkerControl}=await import('./kernel-browser-native-worker.mjs');
 for(const lose of ['timeout','closed']){
  const written=[],child={stdin:{destroyed:false,write:(bytes,callback)=>{written.push(bytes);callback?.();return true}}};
  const worker=new NativeWorkerControl(child);
  const source=new LinuxCapture({scale:2});source.valid=()=>!source.closed;source.attested=true;source.child=child;source.nativeWorker=worker;
  const click=source.click(10,20);
  assert.equal(written.length,1,'the click command reached the worker');
  if(lose==='closed')worker.close();else{const id=JSON.parse(written[0]).click.id;worker.pending.get(id).reject(Error('MP-10: native control timeout'));worker.pending.delete(id);}
  await assert.rejects(click,error=>error.code==='native_input_uncertain');
  assert.equal(source.closed,true,'an uncertain native source retires');
 }
 // Never written (closed before the request): a definite refusal.
 const closed=new NativeWorkerControl({stdin:{destroyed:true}});closed.closed=true;
 const source=new LinuxCapture({scale:2});source.valid=()=>true;source.attested=true;source.child={stdin:{destroyed:false}};source.nativeWorker=closed;
 assert.equal(await source.click(10,20),false);
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

// MP-08/MP-10: hosted, the first DPR1 viewer after a browser launch saw its
// bounds change dropped (Chromium still reported 1280x887 a second later), so
// the native start refused and the view stayed on CDP capture for seconds.
test('MP-10 native start re-sends window bounds that Chromium dropped',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const stop=Error('front'),sets=[];let applied=null;
 const connection={subscribe:()=>()=>{},send:async(method,params)=>{
  if(method==='Browser.getWindowForTarget')return {windowId:7};
  if(method==='Browser.setWindowBounds'){sets.push(params.bounds);if(sets.length===2)applied=params.bounds;return {};}
  if(method==='Browser.getWindowBounds')return {bounds:applied??{width:1280,height:887,windowState:'normal'}};
  if(method==='Page.bringToFront')throw stop;
  assert.fail('unexpected '+method);
 }};
 const source=new LinuxCapture({pid:42,connection,tab:{target_id:'own'},scale:1,hostScale:2});source.valid=()=>true;
 await assert.rejects(source.start(),stop);
 assert.equal(sets.length,2);assert.deepEqual(sets[1],{width:640,height:487});
});

// MP-08/MP-10/MP-11: renderer image scale must match the physical DPR1 host
// window; deviceScaleFactor alone produces a different native DPR2 surface.
for(const scale of [1,2])test(`MP-10 negotiated DPR${scale} preserves the native view image scale`,async()=>{
 const geometry=await import('./kernel-browser-geometry.mjs');
 assert.deepEqual(geometry.displayDeviceMetrics(1280,800,scale),{width:1280,height:800,deviceScaleFactor:scale,scale:1,mobile:false});
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

// MP-08/MP-10/MP-11: each readback fences live fill metadata; navigation retires it.
test('MP-08/MP-10/MP-11 native readbacks refence metadata without re-attesting the window',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 const connection={send:async method=>assert.fail('readback must not re-attest the window: '+method)};let measurements=0;
 const source=new LinuxCapture({connection,sessionId:'s',tab:{target_id:'t',tab_id:'tab',document_id:'d'},policy:{},allowed:()=>true});
 source.valid=()=>!source.closed;source.attested=true;
 source.regions=new NativeRegionProtection(connection,'s',{policy:{targets:[{}],values:[]}});source.regions.measure=async()=>{measurements++;return [];};await source.regions.refresh();
 const published=[];source.subscribe(sample=>published.push(sample.serial));
 for(let serial=1;serial<=5;serial++){source.pending={width:1280,height:800,length:1280*800*4,pixels:Buffer.alloc(1280*800*4,255),serial,format:'bgr0',captured_ms:performance.timeOrigin+performance.now()};await source.publish();}
 assert.deepEqual(published,[1,2,3,4,5]);assert.equal(measurements,6,'every readback rechecks the live fill metadata');
 source.onCdp({sessionId:'s',method:'Page.frameNavigated',params:{frame:{}}});
 assert.equal(source.closed,true);assert.equal(source.sample(),null,'a navigated source never offers its pixels');await source.close();
});
// MP-08/MP-10: a DPR1 page on the DSF-2 window: the link-status bubble is
// 52 physical rows (hosted Wikipedia attestation differed at y 752..771).
test('MP-10 attestation excludes the status bubble at the window density',async()=>{
 const {statusBand}=await import('./kernel-browser-native.mjs');
 assert.deepEqual(statusBand({width:1280,height:800},2),{x:0,y:748,width:1280,height:52});
 assert.deepEqual(statusBand({width:2560,height:1600},2),{x:0,y:1548,width:2560,height:52});
 assert.deepEqual(statusBand({width:1920,height:1080},1),{x:0,y:1054,width:1920,height:26});
});

test('MP-11 CDP disconnect event retires native capture before source reuse',()=>{
 const source=new LinuxCapture({sessionId:'s',tab:{target_id:'t'}});let fenced=0;source.fence=()=>{source.closed=true;fenced++};
 source.onCdp({method:'Chariox.browserDisconnected',params:{code:'browser_cdp_disconnected'}});
 assert.equal(fenced,1);assert.equal(source.closed,true);
});

test('MP-11 a failed native spawn reclaims its private raster directory',async()=>{
 const {mkdtemp,access}=await import('node:fs/promises');
 const {tmpdir}=await import('node:os');const path=await import('node:path');
 const source=new LinuxCapture({});source.poolRoot=await mkdtemp(path.join(tmpdir(),'chariox-region-failed-'));
 const root=source.poolRoot;
 try{await source.close();await assert.rejects(access(root),{code:'ENOENT'});}
 finally{const {rm}=await import('node:fs/promises');await rm(root,{recursive:true,force:true})}
});

for(const retire of ['metadata','document'])test(`MP-08/MP-10/MP-11: ${retire} retires pending pixels during live validation without a publish exception`,async()=>{
 const timings=[],source=new LinuxCapture({connection:{presentationFence:{stable:()=>true}},timing:stage=>timings.push(stage)});
 source.valid=()=>!source.closed;source.attested=true;let released=0;
 source.regions={guard:{},beforeAt:0,regions:async()=>[],retire(){}};
 source.pending={captured_ms:1,release(){released++}};
 source.validateCurrent=async()=>{
  if(retire==='document')source.fence();
  else source.retireFrames();
  return true;
 };
 try{
  await source.publish();assert.equal(released,1);assert.equal(source.closed,retire==='document');
  assert(!timings.some(s=>s.startsWith('native_publish_failed')),'normal retirement must not become a capture failure');
  assert.equal(source.sample(),null);assert.equal(source.publishing,false);
 }finally{await source.close()}
});
// r10 (mirror r38 white video regions): Chrome's omnibox dropdown is drawn inside the tabbed window over the page.
// A page region under it holds its last frame (never Chrome's pixels); a retire while a readback is validated
// leaves the source running (it threw on the vanished readback and fenced the whole region).
test('MP-08/MP-10/MP-11 a page region holds its frame under in-window Chrome UI and survives a retire mid-publish',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 const connection={send:async method=>assert.fail('no CDP: '+method)};
 const source=new LinuxCapture({connection,sessionId:'s',tab:{target_id:'t',tab_id:'tab',document_id:'d'},policy:{},allowed:()=>true,timing:()=>{}});
 source.valid=()=>!source.closed;source.attested=true;
 source.regions=new NativeRegionProtection(connection,'s',{policy:{targets:[{}],values:[]}});source.regions.measure=async()=>[];await source.regions.refresh();
 const published=[];source.subscribe(sample=>published.push(sample.serial));
 const readback=serial=>({width:64,height:32,length:64*32*4,pixels:Buffer.alloc(64*32*4,serial),serial,format:'bgr0',captured_ms:performance.timeOrigin+performance.now(),release(){}});
 let covered=false;source.occluded=()=>covered;
 source.pending=readback(1);await source.publish();
 covered=true;source.pending=readback(2);await source.publish();
 assert.deepEqual(published,[1],'the dropdown\'s pixels never leave');assert.equal(source.sample().serial,1,'the last page frame stays');
 covered=false;source.pending=readback(3);await source.publish();assert.deepEqual(published,[1,3]);
 source.validateCurrent=async()=>{source.retireFrames();return true;};// the real protection retirement while validation awaits
 source.pending=readback(4);await source.publish();
 assert.equal(source.closed,false,'the region keeps running');assert.equal(source.sample(),null,'retirement clears its old mask base and never publishes readback4');
 await source.close();
});

// MP-11 review #1016 P1: the final asynchronous lease check must not resurrect a retired mask projection.
test('MP-11 retiring masks during final lease validation rejects the old raster until a fresh readback',async()=>{
 const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 const connection={send:async()=>assert.fail('unexpected CDP')};
 const source=new LinuxCapture({connection,sessionId:'s',tab:{target_id:'t',tab_id:'tab',document_id:'d'},policy:{},allowed:()=>true,timing:()=>{}});
 source.valid=()=>!source.closed;source.attested=true;
 source.regions=new NativeRegionProtection(connection,'s',{policy:{targets:[{}],values:[]}});source.regions.measure=async()=>[];await source.regions.refresh();
 const published=[];source.subscribe(sample=>published.push(sample.serial));
 const readback=serial=>({width:64,height:32,length:64*32*4,pixels:Buffer.alloc(64*32*4,serial),serial,format:'bgr0',captured_ms:performance.timeOrigin+performance.now(),release(){}});
 let release,checks=0;source.validateCurrent=async()=>{if(++checks===1)await new Promise(resolve=>{release=resolve});return true};
 try{
  source.pending=readback(1);const publishing=source.publish();
  for(let n=0;n<100&&!release;n++)await new Promise(resolve=>setTimeout(resolve,1));
  assert(release,'publication reached the final lease check');
  source.retireFrames();await source.regions.refresh();release();await publishing;
  assert.deepEqual(published,[],'the retired projection never returns to delivery');
  assert.equal(source.sample(),null,'startup must still wait for a current projection');
  source.validateCurrent=async()=>true;source.pending=readback(2);await source.publish();
  assert.deepEqual(published,[2]);assert.equal(source.sample().serial,2,'only a fresh protected readback can satisfy startup');
 }finally{release?.();await source.close();}
});

// MP-08/MP-10/MP-11: metadata awaits precede the final live admission;
// no pixel masking/consumer can run before it, and no duplicate query follows.
test('MP-08/MP-10/MP-11 native pixel admission follows metadata with one live binding query',async()=>{
 const events=[],source=new LinuxCapture({connection:{},tab:{target_id:"t",tab_id:"t",document_id:"d"},timing:()=>{}});source.valid=()=>!source.closed;source.attested=true;
 source.regions={guard:{},beforeAt:0,regions:async()=>{events.push('metadata');return[{x:0,y:0,width:1,height:1}];},retire(){}};
 source.validateCurrent=async()=>{events.push('binding');return true};
 const pixels=Buffer.alloc(16*16*4,255);
 source.pending={width:16,height:16,length:pixels.length,format:'bgr0',serial:1,captured_ms:performance.timeOrigin+performance.now(),get pixels(){events.push('pixels');return pixels;},release(){}};
 source.subscribe(frame=>{void frame.raw.pixels});
 try{await source.publish();assert.equal(source.sample()?.serial,1);assert.equal(source.sample().raw.pixels[0],0,'the admitted frame still masks its protected pixel');assert(events.indexOf('metadata')<events.indexOf('binding'));assert(events.indexOf('binding')<events.indexOf('pixels'));assert.equal(events.filter(e=>e==='binding').length,1);}
 finally{await source.close()}
});

for(const phase of ['occlusion','metadata','binding'])for(const retirement of ['metadata','document'])test(`MP-08/MP-10/MP-11 ${retirement} retirement during ${phase} releases held pixels before any read`,async()=>{
 const timings=[],source=new LinuxCapture({connection:{},timing:stage=>timings.push(stage)});source.valid=()=>!source.closed;source.attested=true;
 let released=0,reads=0,delivered=0;
 const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 source.regions=new NativeRegionProtection({},'s');source.regions.measure=async()=>[];await source.regions.refresh();
 const retire=()=>retirement==='document'?source.fence():source.retireFrames();
 source.occluded=async()=>{if(phase==='occlusion')retire();return false};
 source.regions.measure=async()=>{if(phase==='metadata')retire();return[]};
 source.validateCurrent=async()=>{if(phase==='binding')retire();return true};
 source.pending={captured_ms:performance.timeOrigin+performance.now(),get pixels(){reads++;throw Error('retired pixels read');},release(){released++}};
 source.subscribe(()=>delivered++);
 try{await source.publish();assert.equal(released,1);assert.equal(reads,0);assert.equal(delivered,0);assert.equal(source.sample(),null);assert(!timings.some(s=>s.startsWith('native_publish_failed')));}
 finally{await source.close()}
});


// MP-08/MP-10/MP-11: WHATWG live readback arrived at2747ms, after the old2s wait.
test('MP-08/MP-10/MP-11 startup awaits a late current projection instead of accepting a retired first sample',async()=>{
 const source=new LinuxCapture({});source.valid=()=>!source.closed;source.latest={revision:-1};let at=0;
 const ready=await source.waitForCurrentSample({now:()=>at,pause:async ms=>{at+=ms;if(at>=2750)source.latest={revision:source.regionRevision}}});
 assert.equal(ready,true);assert.equal(at,2750);assert.equal(source.currentSample(),true);
});
test('MP-08/MP-10/MP-11 startup sample wait is bounded and exits immediately when the lease retires',async()=>{
 for(const retire of [false,true]){
  const source=new LinuxCapture({});source.valid=()=>!source.closed;let at=0;
  assert.equal(await source.waitForCurrentSample({now:()=>at,pause:async ms=>{at+=ms;if(retire&&at>=40)source.closed=true}}),false);
  assert.equal(at,retire?40:5000);assert.equal(source.latest,undefined);
 }
});


test('MP-08/MP-10/MP-11 screenshot restoration coalesces native retries without admitting transient pixels',async()=>{
 const {CapturePresentationFence}=await import('./browser-capture-presentation.mjs');
 let clock=100,finish,wakes=0,released=0;
 const fence=new CapturePresentationFence(()=>clock),held=new Promise(resolve=>finish=resolve),restored=fence.run(()=>held,async()=>{});
 const source=new LinuxCapture({connection:{presentationFence:fence}});source.valid=()=>!source.closed;source.wake=()=>{wakes++};
 try{
  for(let n=0;n<20;n++){source.pending={captured_ms:101,release:()=>{released++}};await source.publish()}
  assert.equal(wakes,0,'transient rejections must not force another readback');assert.equal(released,20);assert(!source.latest);
  clock=200;finish();await restored;clock=202;await new Promise(resolve=>setTimeout(resolve,10));assert.equal(wakes,1,'one fresh readback after restoration');assert(!fence.stable(101),'old transient pixels remain rejected');
 }finally{await source.close()}
});
test('MP-08/MP-10/MP-11 closing native capture cancels its pending restoration wake',async()=>{
 const {CapturePresentationFence}=await import('./browser-capture-presentation.mjs');let clock=100,finish,wakes=0;
 const fence=new CapturePresentationFence(()=>clock),restored=fence.run(()=>new Promise(resolve=>finish=resolve),async()=>{});
 const source=new LinuxCapture({connection:{presentationFence:fence}});source.valid=()=>!source.closed;source.wake=()=>{wakes++};source.pending={captured_ms:101,release(){}};
 await source.publish();await source.close();clock=200;finish();await restored;clock=202;await new Promise(resolve=>setTimeout(resolve,10));assert.equal(wakes,0);
});

// r48 quiet-host WHATWG typed URL: each new preview re-attested with a screenshot of the parsing renderer
// (5 s CDP timeouts, then a failed fence restore held every native frame for ~30 s).
test('MP-08/MP-11 a proven window binding admits a new document without a renderer screenshot',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 let proven=false,proofs=0,attests=0;const attestation={proven:()=>proven,prove:()=>{proofs++;proven=true}};
 const protectedPolicy={targets:[{}],values:[]};
 const source=new LinuxCapture({tab:{target_id:'t'},attestation,policy:protectedPolicy});
 source.screenshot=()=>assert.fail('a proven binding takes no screenshot');
 source.attest=async()=>{attests++;if(attests===1)throw Error('native attestation RGB differed');};
 await assert.rejects(source.admitSurface(),/RGB differed/);assert.equal(proofs,0,'a failed attestation proves nothing');
 await source.admitSurface();assert.deepEqual([attests,proofs],[2,1]);
 await source.admitSurface();assert.deepEqual([attests,proofs],[2,1],'the proven binding is reused');
 const unbound=new LinuxCapture({tab:{target_id:'t'},policy:protectedPolicy});unbound.attest=async()=>{attests++};await unbound.admitSurface();assert.equal(attests,3,'no binding, fresh attestation');
});

// r41: pixel attestation exists so Vault masks line up on the native stream. Without a
// kernel-known protected fill target the window binding is admitted by identity and
// geometry alone; an animating page must not be refused for lack of a pixel match.
const unprotected=()=>new LinuxCapture({connection:{},tab:{target_id:'t',tab_id:'tab',document_id:'d'},scale:2,policy:{targets:[],values:[]},browser:{fillTargets:new Map()},timing:()=>{}});
test('MP-11 a page without a protected fill target is admitted by geometry, no pixel attestation',async()=>{
 const {displayGeometry:geometry}=await import('./kernel-browser-geometry.mjs');
 const source=unprotected(),events=[];source.timing=label=>events.push(label);
 source.attest=async()=>assert.fail('no pixel attestation without a protected target');
 source.latest={raw:{width:geometry.width*2,height:geometry.height*2}};
 await source.admitSurface();
 assert.equal(source.pixelProven,false);assert(events.includes('native_admitted_geometry'));
});
test('MP-11 geometry admission refuses a sample of another size',async()=>{
 const source=unprotected();source.attest=async()=>assert.fail('no pixel attestation');
 source.latest={raw:{width:10,height:10}};
 await assert.rejects(source.admitSurface(),/native attestation RGB differed/);
});
for(const [label,policy,browser] of [['policy target',{targets:[{}],values:[]},{fillTargets:new Map()}],['recorded fill',{targets:[],values:[]},{fillTargets:new Map([['t:1',{}]])}]])
 test(`MP-11 a ${label} keeps the exact pixel attestation`,async()=>{
  const source=unprotected();source.policy=policy;source.browser=browser;let attested=0;
  source.attest=async()=>{attested++};await source.admitSurface();
  assert.equal(attested,1);assert.equal(source.pixelProven,true);
 });
test('MP-11 a Vault fill that appears on a geometry-admitted source retires it before publishing',async()=>{
 const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 const source=unprotected(),published=[];source.valid=()=>!source.closed;source.attested=true;source.pixelProven=false;source.allowed=()=>true;
 source.regions=new NativeRegionProtection({},'s',{policy:source.policy,browser:source.browser});source.regions.measure=async()=>[];await source.regions.refresh();
 source.validateCurrent=async()=>true;source.subscribe(sample=>published.push(sample.serial));
 const readback=serial=>({width:64,height:32,length:64*32*4,pixels:Buffer.alloc(64*32*4,serial),serial,format:'bgr0',captured_ms:performance.timeOrigin+performance.now(),release(){}});
 source.pending=readback(1);await source.publish();assert.deepEqual(published,[1],'no target: frames flow');
 source.browser.fillTargets.set('t:1',{});
 source.pending=readback(2);await source.publish();
 assert.deepEqual(published,[1],'the frame after the fill never publishes');assert.equal(source.closed,true,'the unproven source retires so the next start attests exactly');
});
test('MP-11 a pixel-proven source keeps publishing when a Vault fill appears',async()=>{
 const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 const source=unprotected(),published=[];source.valid=()=>!source.closed;source.attested=true;source.pixelProven=true;source.allowed=()=>true;
 source.regions=new NativeRegionProtection({},'s',{policy:source.policy,browser:source.browser});source.regions.measure=async()=>[];await source.regions.refresh();
 source.validateCurrent=async()=>true;source.subscribe(sample=>published.push(sample.serial));
 source.browser.fillTargets.set('t:1',{});
 source.pending={width:64,height:32,length:64*32*4,pixels:Buffer.alloc(64*32*4,1),serial:1,format:'bgr0',captured_ms:performance.timeOrigin+performance.now(),release(){}};
 await source.publish();assert.deepEqual(published,[1]);assert.equal(source.closed,false);
});

// r61 UX-09: a source fenced by a navigation must not spend a screenshot (700 ms on a loading page) attesting a surface nobody will admit.
test('MP-11 a fenced native source skips the attestation screenshot and records why',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');
 const marks=[];let shots=0;
 const source=new LinuxCapture({sessionId:'session',timing:name=>marks.push(name),screenshot:async()=>{shots++;throw Error('no screenshot expected');}});
 source.closed=true;
 await assert.rejects(source.attest(),/native attestation RGB differed/);
 assert.equal(shots,0);assert.deepEqual(marks,['native_attestation_retired']);
});

test('MP-08/MP-10/MP-11 resized page capture keeps the exact owned window without fitting it again',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');const capture=new LinuxCapture({size:{width:1150,height:592},scale:1,pageWindow:async()=>({xid:42,bounds:{width:1150,height:680},page_rect:{left:0,top:88,width:1150,height:592}}),connection:{send:()=>assert.fail('a resized owned window must not be fitted again')}});
 assert.deepEqual(await capture.prepareReadback(),{width:1150,height:592,window_region:{id:42,width:1150,height:680,left:0,top:88}});
});
// MP-08/MP-10/MP-11 (P1): a Tab display's source keeps its worker across the Tab's documents; old pixels retire.
test('MP-08/MP-10/MP-11 a Tab display source follows a main-frame commit instead of fencing',async()=>{
 const {LinuxCapture}=await import('./kernel-browser-native.mjs');const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 const connection={send:async method=>assert.fail('a followed document must not re-attest the window: '+method)};
 const source=new LinuxCapture({connection,sessionId:'s',tab:{target_id:'t',tab_id:'tab',document_id:'d1'},policy:{},allowed:()=>true,followsDocument:true});
 const controls=[];source.valid=()=>!source.closed;source.attested=true;source.child={stdin:{destroyed:false,write:text=>controls.push(JSON.parse(text))}};
 source.regions=new NativeRegionProtection(connection,'s',{policy:{targets:[],values:[]}});source.regions.measure=async()=>[];await source.regions.refresh();
 let released=0;const followed=[];source.onDocument(id=>followed.push(id));
 source.latest={raw:{release:()=>released++},serial:4};
 source.onCdp({sessionId:'s',method:'Page.frameNavigated',params:{frame:{id:'f',loaderId:'d2'}}});
 assert.equal(source.closed,false,'the worker and its window binding stay');
 assert.equal(source.tab.document_id,'d2');assert.deepEqual(followed,['d2']);
 assert.equal(released,1);assert.equal(source.sample(),null,'no pixel of the old document is offered as the new one');
 assert.ok(controls.some(c=>c.refresh===true),'the new document is read back at once');
 await source.regions.refresh(); // the new document's Vault projection, then its first readback
 const readback=serial=>({width:1280,height:800,length:1280*800*4,pixels:Buffer.alloc(1280*800*4,255),serial,format:'bgr0',captured_ms:performance.timeOrigin+performance.now()});
 // Chrome shows white or old tiles until the new document paints: nothing is published before that paint.
 source.pending=readback(5);await source.publish();assert.equal(source.sample(),null);
 source.onCdp({sessionId:'s',method:'Page.lifecycleEvent',params:{name:'firstPaint',loaderId:'old',frameId:'f'}});assert.ok(source.paintHold,'another document\'s paint does not end the hold');
 const refreshes=controls.filter(c=>c.refresh===true).length;
 source.onCdp({sessionId:'s',method:'Page.lifecycleEvent',params:{name:'firstContentfulPaint',loaderId:'d2',frameId:'f'}});
 assert.equal(source.paintHold,null);assert.equal(controls.filter(c=>c.refresh===true).length,refreshes+1,'the paint is read back at once');
 await new Promise(resolve=>setTimeout(resolve,2));
 source.pending=readback(6);await source.publish();assert.equal(source.sample().document_id,'d2');
 // A child frame commit is not the Tab's document; a source without the Tab display binding still fences.
 source.onCdp({sessionId:'s',method:'Page.frameNavigated',params:{frame:{id:'c',parentId:'f',loaderId:'x'}}});assert.equal(source.tab.document_id,'d2');
 const plain=new LinuxCapture({connection,sessionId:'s',tab:{target_id:'t',tab_id:'tab',document_id:'d1'},policy:{},allowed:()=>true});
 plain.valid=()=>!plain.closed;plain.attested=true;
 plain.onCdp({sessionId:'s',method:'Page.frameNavigated',params:{frame:{id:'f',loaderId:'d2'}}});assert.equal(plain.closed,true);
 await source.close();await plain.close();
});
test('MP-08/MP-10 parking a Tab display source pauses its worker readback and unparking resumes it',()=>{
 return import('./kernel-browser-native.mjs').then(({LinuxCapture})=>{
  const controls=[],source=new LinuxCapture({tab:{target_id:'t',tab_id:'tab',document_id:'d'},policy:{},allowed:()=>true});
  source.valid=()=>!source.closed;source.child={stdin:{destroyed:false,write:text=>controls.push(JSON.parse(text))}};
  source.park(true);source.park(true);source.park(false);
  assert.deepEqual(controls,[{pause:true},{pause:false}]);
 });
});
