// MD-DISPLAY-02/04: native admission fails before opening any foreign display.
import test from 'node:test';
import {nativeReferenceMatches} from './kernel-browser-native.mjs';
test('MP-11 native attestation matches an exact retained paint and refuses device-pixel differences',()=>{
 const reference={width:1,height:1,pixels:Buffer.from([10,20,30,255])};
 const captured={width:1,height:1,pixels:Buffer.from([30,20,10,255])},newer={width:1,height:1,pixels:Buffer.from([31,20,10,255])};
 assert.equal(nativeReferenceMatches(reference,newer),false);
 assert([captured,newer].some(raw=>nativeReferenceMatches(reference,raw)));
 assert.equal(nativeReferenceMatches({...reference,width:2},captured),false);
 assert.equal(nativeReferenceMatches(reference,{...captured,pixels:Buffer.alloc(8)}),false);
});
import assert from 'node:assert/strict';
import { OwnedDisplay, ownsDisplay } from './kernel-browser-owned-display.mjs';
import { selectNativeCapture } from './kernel-browser-native.mjs';
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
  assert.equal(source.closed,true);assert.equal(source.attested,false);assert.equal(source.regions.guard,null);assert.equal(source.latest,null);assert.equal(released,2);await source.close();
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

// MD-454: park only the kernel-owned pointer before publishing browser readiness.
test('owned pointer parking refuses foreign/retired displays and stays outside the browser window',async()=>{
 const {mkdtemp,rm}=await import('node:fs/promises'),{tmpdir}=await import('node:os'),{execFile,spawn}=await import('node:child_process'),{promisify}=await import('node:util');
 const root=await mkdtemp(tmpdir()+'/chariox-md-pointer-test-'),display=new OwnedDisplay(root);let keeper;
 try{
  await assert.rejects(display.parkPointer(),/owned display/);await display.start();
  // Xvfb resets the pointer when its last client leaves. Chromium holds this
  // connection in production; retain one here for the cross-process check.
  keeper=spawn('python3',['-u','-c',`import ctypes as c,ctypes.util,time
x=c.CDLL(ctypes.util.find_library('X11'));x.XOpenDisplay.restype=c.c_void_p;x.XOpenDisplay.argtypes=[c.c_char_p];d=x.XOpenDisplay(None)
assert d;print('ready');time.sleep(10)`],{env:{...process.env,...display.environment},stdio:['ignore','pipe','ignore']});
  await new Promise((resolve,reject)=>{keeper.stdout.once('data',resolve);keeper.once('error',reject);keeper.once('exit',()=>reject(Error('pointer test connection exited'))) });await display.parkPointer();
  const probe=`import ctypes as c,ctypes.util
x=c.CDLL(ctypes.util.find_library('X11'));x.XOpenDisplay.restype=c.c_void_p;x.XOpenDisplay.argtypes=[c.c_char_p];d=x.XOpenDisplay(None)
x.XDefaultRootWindow.restype=c.c_ulong;x.XDefaultRootWindow.argtypes=[c.c_void_p]
r=c.c_ulong();w=c.c_ulong();a=c.c_int();b=c.c_int();u=c.c_int();v=c.c_int();m=c.c_uint()
x.XQueryPointer.argtypes=[c.c_void_p,c.c_ulong,c.POINTER(c.c_ulong),c.POINTER(c.c_ulong),c.POINTER(c.c_int),c.POINTER(c.c_int),c.POINTER(c.c_int),c.POINTER(c.c_int),c.POINTER(c.c_uint)]
x.XQueryPointer(d,x.XDefaultRootWindow(d),c.byref(r),c.byref(w),c.byref(a),c.byref(b),c.byref(u),c.byref(v),c.byref(m));print(a.value,b.value)
x.XCloseDisplay.argtypes=[c.c_void_p];x.XCloseDisplay(d)
`;
  const result=await promisify(execFile)('python3',['-c',probe],{env:{...process.env,...display.environment},timeout:2000});assert.equal(result.stdout.trim(),'2559 1799');
  await display.close();await assert.rejects(display.parkPointer(),/owned display/);
 }finally{if(keeper&&keeper.exitCode===null){keeper.kill('SIGTERM');await new Promise(r=>keeper.once('exit',r))}await display.close();await rm(root,{recursive:true,force:true})}
});
