import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
// MD-DISPLAY-02/04: OS-neutral capture interface: start/subscribe/sample/close.
// Linux XDamage/XShm implementation; macOS unsupported until ScreenCaptureKit.
import {mkdtemp,rm} from 'node:fs/promises';
import path from 'node:path';
import {SharedRasterPool} from './kernel-browser-shared-raster.mjs';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {setTimeout as delay} from 'node:timers/promises';
import {NativePipe,NativeRaster} from './kernel-browser-native-pipe.mjs';
import {ownsDisplay} from './kernel-browser-owned-display.mjs';
import {decodePng,maskNativeRaster} from './kernel-browser-pixels.mjs';
import {NativeRegionProtection,regionProtectionChanged} from './kernel-browser-region-protection.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';
import {NativeWorkerControl} from './kernel-browser-native-worker.mjs';
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
import {safeChildPid} from './kernel-browser-display.mjs';
export async function selectNativeCapture({platform=process.platform,display,create}){
 if(platform!=='linux'||!ownsDisplay(display))return null;
 try{return await create()}catch{return null}
}
export function nativeReferenceMatches(reference,raw){
 if(reference.width!==raw.width||reference.height!==raw.height||reference.pixels.length!==raw.pixels.length||raw.pixels.length!==raw.width*raw.height*4)return false;
 for(let n=0;n<raw.pixels.length;n+=4)if(reference.pixels[n]!==raw.pixels[n+2]||reference.pixels[n+1]!==raw.pixels[n+1]||reference.pixels[n+2]!==raw.pixels[n])return false;
 return true;
}
export class LinuxCapture {
 constructor({display,pid,connection,sessionId,tab,scale,policy,screenshot,allowed,timing=()=>{}}){
  Object.assign(this,{display,pid,connection,sessionId,tab,scale,policy,screenshot,allowed,timing});this.listeners=new Set();this.closed=false;this.attested=false;this.attestationRasters=[];this.regionRevision=0;this.changedAt=performance.now();this.motionStreak=0;this.ignoreIdleUntil=-Infinity;
 }
 subscribe(fn){this.listeners.add(fn);return()=>this.listeners.delete(fn)}
 valid(){return !this.closed&&ownsDisplay(this.display)&&this.allowed(this.policy)}
 onCdp(m){
  if(m.sessionId===this.sessionId&&m.method==='Page.frameNavigated'&&!m.params?.frame?.parentId)this.fence();
  if(m.method==='Target.targetCreated'&&m.params?.targetInfo?.type==='page')this.fence();
  // MP-11: attribute-only protection changes produce no XDamage. Fence even
  // before attestation, so an empty snapshot cannot outlive marker insertion.
  if(this.regions&&regionProtectionChanged(m,this.sessionId,this.regions.guard?.hasRegions)){
   if(!this.attested){this.fence();return;}
   // MP-08/MP-10/MP-11: the owned window/document binding stays intact.
   // Retire all pixels and queued consumers by revision, refresh only trusted
   // region metadata, and mask any readback older than that new fence in full.
   this.regionRevision++;this.regions.retire();this.latest?.raw.release?.();this.pending?.release?.();
   this.latest=null;this.pending=null;this.changedAt=performance.now();this.wake(true);
  }
 }
 async start(){
  if(!this.valid()||!Number.isSafeInteger(this.pid)||this.pid<=1)throw Error('MD-DISPLAY: native surface not owned');
  this.off=this.connection.subscribe(m=>this.onCdp(m));
  try{
   this.phase='bounds';const {windowId}=await this.connection.send('Browser.getWindowForTarget',{targetId:this.tab.target_id});
   // Window bounds are native DIPs; subscriber geometry is in device pixels.
   await this.connection.send('Browser.setWindowBounds',{windowId,bounds:{width:geometry.width*this.scale/geometry.dpr,height:geometry.height*this.scale/geometry.dpr+87}});
   await this.connection.send('Page.bringToFront',{},this.sessionId);
   const {frameTree}=await this.connection.send('Page.getFrameTree',{},this.sessionId);
   const {executionContextId}=await this.connection.send('Page.createIsolatedWorld',{frameId:frameTree.frame.id,worldName:'chariox-native-surface-fence',grantUniveralAccess:false},this.sessionId);
   this.contextId=executionContextId;
   await delay(100);
   // Force the emulated viewport to paint before establishing its native crop.
   await this.screenshot();
   this.regions=new NativeRegionProtection(this.connection,this.sessionId);await this.regions.refresh();
   this.poolRoot=await mkdtemp(path.join(this.display.root,'raster-'));
   this.phase='readback';const executable=process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER;
   const child=spawn(executable||process.env.CHARIOX_BROWSER_DISPLAY_PYTHON||'python3',executable?['--display-native-worker']:['-u',fileURLToPath(new URL('./kernel-browser-xshm.py',import.meta.url))],{env:{...process.env,...this.display.environment},stdio:['pipe','pipe','pipe']});this.child=child;if(executable)this.nativeWorker=new NativeWorkerControl(child,this.timing);
   const fail=()=>this.fence();child.on('error',fail);child.on('exit',fail);child.stdin.on('error',fail);
   child.stderr.on('data',b=>{if(/^MD-DISPLAY: native stage [a-z_]+\n$/.test(b.toString()))this.helperStage=b.toString().trim().split(' ').at(-1)});
   const raster=new NativeRaster();
   const pool=new SharedRasterPool(this.poolRoot,(slot,serial)=>{if(!child.stdin.destroyed)child.stdin.write(JSON.stringify({release:slot,serial})+'\n')});
   const pipe=new NativePipe(header=>{
    if(header.reply!==undefined){this.nativeWorker?.validate(header);if(!this.nativeWorker)throw Error('MP-11: unexpected native reply');return;}
    if(header.width!==geometry.width*this.scale||header.height!==geometry.height*this.scale||!Number.isSafeInteger(header.serial)||header.serial<1||!Number.isFinite(header.captured_ms)||!Number.isFinite(header.capture_ms)||!/^[a-f0-9]{16}$/.test(header.signature))throw Error('raw geometry');
   },(header,pixels)=>{
    if(header.reply!==undefined){this.nativeWorker.receive(header,pixels);return;}
    const received=performance.timeOrigin+performance.now();
    if(Array.isArray(header.native_cpu)&&header.native_cpu.length===3)for(const [index,stage] of ['native_cpu_xshm_fence_read','native_cpu_damage_compare','native_cpu_capture_copy'].entries()){const cpu=header.native_cpu[index];if(Number.isFinite(cpu)&&cpu>=0)this.timing(stage,header.captured_ms,header.captured_ms+cpu);}
    if(Number.isFinite(header.native_read_ms))this.timing('native_capture_compare_copy',header.captured_ms,header.native_read_ms);
    for(const [name,start,end] of [['input_wake_to_capture',header.input_wake_ms,header.captured_ms],['native_damage_coalesce',header.damage_ready_ms,header.captured_ms],['native_window_fence',header.captured_ms,header.get_image_ms],['native_xshm_get_image',header.get_image_ms,header.image_ready_ms],['native_readback_copy',header.image_ready_ms,header.readback_ms],['native_readback',header.captured_ms,header.readback_ms],['native_fingerprint',header.readback_ms,header.fingerprint_ms],['native_damage_scan',header.fingerprint_ms,header.damage_ms]]){
     if(Number.isFinite(start)&&Number.isFinite(end)&&end>=start)this.timing(name,start,end);
    }
    if(Number.isFinite(header.damage_ms))this.timing('native_pipe',header.damage_ms,received);
    const at=performance.timeOrigin+performance.now(),raw=header.slot==null?raster.apply(header,pixels):pool.apply(header);this.timing('native_raster_copy',at);
    if(this.nativeWorker){
     raw.nativeEncode=values=>this.nativeWorker.request('encode',{...values,serial:raw.serial});
     raw.nativeExact=values=>this.nativeWorker.request('exact',{...values,serial:raw.serial});
     raw.nativeRetire=encoder=>this.nativeWorker.retire(encoder);
     raw.nativeDelivered=(encoder,revision)=>this.nativeWorker.delivered(encoder,revision);
     raw.nativeCommit=encoder=>this.nativeWorker.commit(encoder,raw.serial);
    }
    if(this.valid()){raw.format='bgr0';this.pending?.release?.();this.pending=raw;void this.publish()}else raw.release?.()
   });
   child.stdout.on('data',b=>{try{pipe.push(b)}catch{this.fence()}});
   child.stdin.write(JSON.stringify({pid:this.pid,width:geometry.width*this.scale,height:geometry.height*this.scale,pool:this.poolRoot})+'\n');
   for(let n=0;n<200&&!this.latest&&!this.closed;n++)await delay(10);
   if(!this.latest||!this.valid())throw Error('MD-DISPLAY: native readback unavailable');
   this.phase='attestation';let matched=false;
   // Chromium may finish a viewport paint after the first shared readback.
   // Retry only this observation; never admit an approximate pixel binding.
   for(let attempt=0;attempt<3&&!matched;attempt++){
    const reference=decodePng((await this.screenshot()).data_base64,this.scale);
    // A screenshot may temporarily resize the emulated viewport. Await an
    // exact post-capture readback instead of racing a fixed50ms delay.
    for(let observation=0;observation<20&&!matched&&this.valid();observation++){
     await delay(10);
     // The screenshot can refer to a paint preceding the newest XShm readback.
     // Four immutable, already-masked rasters bound startup memory to64MiB;
     // each still requires exact device-pixel RGB, document and policy fences.
     matched=this.attestationRasters.some(raw=>nativeReferenceMatches(reference,raw));
    }
   }
   if(!matched)throw Error('native attestation RGB differed');
   await assertCurrentDocument(this.connection,this.sessionId,this.tab.target_id,this.tab.document_id);
   if(!this.valid())throw Error('native attestation retired');
   this.attested=true;this.attestationRasters=[];this.timing('native_admitted',performance.timeOrigin+performance.now());return this;
  }catch(error){this.timing('native_unavailable_'+this.phase+'_'+(this.helperStage??'host'),performance.timeOrigin+performance.now());await this.close();throw error}
 }
 async publish(){
  if(this.publishing)return;this.publishing=true;
  try{while(this.pending&&this.valid()){
   let raw=this.pending;this.pending=null;this.publishingRaw=raw;
   const revision=this.regionRevision;
   let at=performance.timeOrigin+performance.now();const fenced=at;
   // Both fences follow this readback; each must pass. Concurrent, not skipped.
   const [,visibility]=await Promise.all([
    assertCurrentDocument(this.connection,this.sessionId,this.tab.target_id,this.tab.document_id).then(()=>this.timing('native_document_fence',fenced)),
    this.connection.send('Runtime.evaluate',{expression:'document.visibilityState',contextId:this.contextId,returnByValue:true},this.sessionId).then(value=>{this.timing('native_visibility_fence',fenced);return value})]);
   if(visibility.result?.value!=='visible')throw Error('native source not visible');
   at=performance.timeOrigin+performance.now();let regions;
   try{
    if(!this.regions.guard)await this.regions.refresh();
    if(raw.captured_ms>=this.regions.beforeAt)regions=await this.regions.regions(raw);
   }catch(error){if(revision===this.regionRevision)throw error;}
   this.timing('native_region_fence',at);
   if(revision!==this.regionRevision){raw.release?.();this.publishingRaw=null;continue;}
   // Attribute-only changes need a new readback even if XDamage/pixels did
   // not change. The first readback may precede the refreshed metadata.
   if(raw.captured_ms<this.regions.beforeAt){this.wake(true);raw.release?.();this.publishingRaw=null;continue;}
   at=performance.timeOrigin+performance.now();const masked=maskNativeRaster(raw,regions,this.previousRegions,this.latest?.raw);this.timing('native_mask_copy_hash',at);
   if(masked!==raw){raw.release?.();raw=masked;this.publishingRaw=raw;}
   if(!this.valid())break;
   if(!this.attested){this.attestationRasters.push({width:raw.width,height:raw.height,pixels:Buffer.from(raw.pixels)});if(this.attestationRasters.length>4)this.attestationRasters.shift();}
   this.previousRegions=regions;
   this.motionStreak=performance.now()-this.changedAt<90?this.motionStreak+1:1;this.changedAt=performance.now();
   this.timing('native_xshm_capture',raw.captured_ms); // includes bounded pipe delivery and source fence.
   this.latest?.raw.release?.();
   this.latest={raw,signature:raw.signature,data_base64:raw.signature,width:raw.width,height:raw.height,serial:raw.serial,captured_ms:raw.captured_ms,motion:true,tab_id:this.tab.tab_id,document_id:this.tab.document_id};
   this.publishingRaw=null;
   if(this.attested)for(const fn of this.listeners)fn(this.latest);
  }}catch{this.fence()}finally{this.publishingRaw?.release?.();this.publishingRaw=null;this.publishing=false}
 }
 // MP-08/MP-10: only admitted physical input reaches this owned helper.
 wake(refresh=false){if(this.valid()&&this.child&&!this.child.stdin.destroyed)this.child.stdin.write(JSON.stringify(refresh?{refresh:true}:{wake:true})+'\n')}
 sample(after=-1){return this.valid()&&this.attested&&this.latest?.serial>after?this.latest:null}
 fence(){this.closed=true;this.attested=false;this.attestationRasters=[];this.regions?.retire();this.latest?.raw.release?.();this.pending?.release?.();this.latest=null;this.pending=null;this.listeners.clear();if(!this.closing)this.closing=this.cleanup();this.closing.catch(()=>{})}
 pause(){} resume(){} // CDP exact reads do not mutate the native display.
 async close(){this.fence();return this.closing}
 async cleanup(){
  this.off?.();this.nativeWorker?.close();const child=this.child;this.child=null;
  if(!child||child.pid===undefined)return;safeChildPid(child);child.stdin.end();
  for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await delay(20);
  if(child.exitCode===null&&child.signalCode===null){child.kill('SIGTERM');for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await delay(20)}
  if(child.exitCode===null&&child.signalCode===null){child.kill('SIGKILL');await new Promise(r=>child.once('exit',r))}
  if(this.poolRoot){await rm(this.poolRoot,{recursive:true,force:true});this.poolRoot=null}
 }
}
