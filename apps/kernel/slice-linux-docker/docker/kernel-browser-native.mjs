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
export async function selectNativeCapture({platform=process.platform,display,create,refused=()=>{}}){
 if(platform!=='linux'||!ownsDisplay(display)){refused(platform!=='linux'?'platform':'display_not_owned');return null;}
 try{return await create()}catch{return null}
}
// MP-08/MP-10: fixed refusal labels for diagnostics; never error/page text.
const refusalReasons=[['Capture protection bounds unavailable','region_bounds'],['Capture protection tree limit exceeded','region_tree_limit'],['Capture protection limit exceeded','region_limit'],['Capture protection unavailable','region_query'],['native attestation RGB differed','attestation_mismatch'],['native attestation retired','attestation_retired'],['native surface not owned','surface_not_owned'],['native region fence retired','region_retired'],['stale_document_reference','document_changed']];
export function nativeRefusalReason(error,helperStage){
 const message=`${error?.message??''} ${error?.code??''}`;
 const known=refusalReasons.find(([text])=>message.includes(text));
 if(known)return known[1];
 if(message.includes('native readback unavailable'))return helperStage??'first_frame_timeout';
 return 'other';
}
// Pixels inside protected regions of the reference capture (null: none).
const STATUS_BAND_DIP=26;
// MP-08/MP-10: Chromium's link-status bubble (browser UI under a parked
// XTest pointer, bottom 26 DIP) is absent from CDP references; it is not page
// content and stays visible to the viewer, as on a native desktop. Browser UI
// renders at the window's density, not the page's emulated one.
export const statusBand=(raw,hostScale)=>{const band=STATUS_BAND_DIP*hostScale;return {x:0,y:raw.height-band,width:raw.width,height:band};};
export function attestationMask(regions,width,height){
 if(!regions.length)return null;
 const mask=new Uint8Array(width*height);
 for(const r of regions){
  const l=Math.max(0,Math.floor(r.x)-2),t=Math.max(0,Math.floor(r.y)-2),right=Math.min(width,Math.ceil(r.x+r.width)+2),bottom=Math.min(height,Math.ceil(r.y+r.height)+2);
  for(let y=t;y<bottom;y++)mask.fill(1,y*width+l,y*width+Math.max(l,right));
 }
 return mask;
}
export class LinuxCapture {
 constructor({display,pid,connection,sessionId,tab,scale,hostScale=1,policy,screenshot,allowed,frames,timing=()=>{}}){
  Object.assign(this,{display,pid,connection,sessionId,tab,scale,hostScale,policy,screenshot,allowed,frames,timing});this.listeners=new Set();this.closed=false;this.attested=false;this.regionRevision=0;this.changedAt=performance.now();this.motionStreak=0;this.ignoreIdleUntil=-Infinity;
 }
 subscribe(fn){this.listeners.add(fn);return()=>this.listeners.delete(fn)}
 valid(){return !this.closed&&ownsDisplay(this.display)&&this.allowed(this.policy)}
 onCdp(m){
  if(m.sessionId===this.sessionId&&m.method==='Page.frameNavigated'&&!m.params?.frame?.parentId)this.fence();
  if(m.method==='Target.targetCreated'&&m.params?.targetInfo?.type==='page')this.fence();
  // MP-11: attribute-only protection changes produce no XDamage. Retire even
  // before attestation, so an empty snapshot cannot outlive marker insertion.
  if(this.regions&&regionProtectionChanged(m,this.sessionId,this.regions.tracker())){
   // MP-08/MP-10/MP-11: the owned window/document binding stays intact,
   // also during attestation (pages that mutate continuously still attest).
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
   // MP-08/MP-10/MP-11: host-window DIPs are physical pixels (DPR1).
   // Page emulation owns negotiated DPR; keep the crop exact at both scales.
   // Window bounds are host DIPs: the negotiated raster divided by host scale.
   const bounds={width:geometry.width*this.scale/this.hostScale,height:geometry.height*this.scale/this.hostScale+87};
   // The X window resize is asynchronous; the worker needs its final size.
   // Chromium can drop a bounds change (the first DPR1 viewer after a browser
   // launch kept 1280x887 DIPs): re-send until it reports the requested size.
   for(let n=0;n<50;n++){if(n%10===0)await this.connection.send('Browser.setWindowBounds',{windowId,bounds});const {bounds:actual}=await this.connection.send('Browser.getWindowBounds',{windowId});if(actual?.width===bounds.width&&actual?.height===bounds.height)break;await delay(20);}
   await this.connection.send('Page.bringToFront',{},this.sessionId);
   await delay(100);
   // Force the emulated viewport to paint before establishing its native crop.
   await this.screenshot();
   // A DOM mutation may retire the first fence while it is measured (busy
   // real pages); retry a bounded number of times instead of refusing.
   this.regions=new NativeRegionProtection(this.connection,this.sessionId,{frames:this.frames,record:reason=>this.timing('region_frame_masked '+reason,performance.timeOrigin+performance.now())});
   for(let attempt=0;;attempt++){try{await this.regions.refresh();break}catch(error){if(attempt>=4||!/region fence retired/.test(error?.message??''))throw error;}}
   this.poolRoot=await mkdtemp(path.join(this.display.root,'raster-'));
   this.phase='readback';const executable=process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER;
   const child=spawn(executable||process.env.CHARIOX_BROWSER_DISPLAY_PYTHON||'python3',executable?['--display-native-worker']:['-u',fileURLToPath(new URL('./kernel-browser-xshm.py',import.meta.url))],{env:{...process.env,...this.display.environment},stdio:['pipe','pipe','pipe']});this.child=child;if(executable)this.nativeWorker=new NativeWorkerControl(child,this.timing);
   const fail=()=>this.fence();child.on('error',fail);child.on('exit',fail);child.stdin.on('error',fail);
   // MP-08/MP-10: fixed-label stage and owned-window sizes only.
   child.stderr.on('data',b=>{for(const line of b.toString().split('\n')){
    if(/^MD-DISPLAY: native stage [a-z_]+$/.test(line))this.helperStage=line.split(' ').at(-1);
    else if(/^MD-DISPLAY: native geometry requested \d+x\d+ owned (none|\d+x\d+(,\d+x\d+){0,7})$/.test(line))this.timing(line.replace('MD-DISPLAY: native geometry ','native_geometry '),performance.timeOrigin+performance.now());
   }});
   const raster=new NativeRaster();
   const pool=new SharedRasterPool(this.poolRoot,(slot,serial)=>this.control({release:slot,serial}));
   const pipe=new NativePipe(header=>{
    if(header.reply!==undefined){this.nativeWorker?.validate(header);if(!this.nativeWorker)throw Error('MP-11: unexpected native reply');return;}
    if(header.width!==geometry.width*this.scale||header.height!==geometry.height*this.scale||!Number.isSafeInteger(header.serial)||header.serial<1||!Number.isFinite(header.captured_ms)||!Number.isFinite(header.capture_ms)||!/^[a-f0-9]{16}$/.test(header.signature))throw Error('raw geometry');
   },(header,pixels)=>{
    if(header.reply!==undefined){this.nativeWorker.receive(header,pixels);return;}
    const received=performance.timeOrigin+performance.now();
    if(Array.isArray(header.native_cpu)&&header.native_cpu.length===3)for(const [index,stage] of ['native_cpu_xshm_fence_read','native_cpu_damage_compare','native_cpu_capture_copy'].entries()){const cpu=header.native_cpu[index];if(Number.isFinite(cpu)&&cpu>=0)this.timing(stage,header.captured_ms,header.captured_ms+cpu);}
    if(Number.isFinite(header.native_read_ms))this.timing('native_capture_compare_copy',header.captured_ms,header.native_read_ms);
    // MP-08/MP-10: proved scroll plans offered by this readback (telemetry only).
    if(header.shift_adjacent)this.timing('native_shift_adjacent',header.captured_ms,header.native_read_ms??header.captured_ms);
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
     raw.nativeCommit=(encoder,admit)=>this.nativeWorker.commit(encoder,raw.serial,admit);
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
   for(let attempt=0;attempt<6&&!matched;attempt++){
    const shot=await this.screenshot(),reference=decodePng(shot.data_base64,this.scale);
    await delay(50);
    // A protection event may have retired the readback meanwhile; its fresh
    // replacement follows the refreshed fence (bounded wait).
    let raw=this.latest?.raw;
    for(let n=0;n<50&&!raw&&this.valid();n++){await delay(10);raw=this.latest?.raw;}
    if(!raw||!this.valid()){this.timing('native_attestation_no_readback',performance.timeOrigin+performance.now());break;}
    matched=reference.width===raw.width&&reference.height===raw.height;
    const nativePixels=raw.pixels;
    // MP-08/MP-11: the reference is the protected capture; its masked
    // regions (+2 px for rounding) are excluded. The same trusted regions
    // mask every native frame before any encoder or client sees it.
    const masked=attestationMask([...(shot[displayMaskRegions]??[]),statusBand(raw,this.hostScale)],raw.width,raw.height);
    // MP-08/MP-10: on mismatch record only counts and bounds (no pixels).
    let differing=0,l=Infinity,t=Infinity,r=-1,b=-1;
    for(let n=0;matched&&n<nativePixels.length;n+=4)if(!masked?.[n/4]&&(reference.pixels[n]!==nativePixels[n+2]||reference.pixels[n+1]!==nativePixels[n+1]||reference.pixels[n+2]!==nativePixels[n])){const p=n/4,x=p%raw.width,y=(p-x)/raw.width;differing++;l=Math.min(l,x);t=Math.min(t,y);r=Math.max(r,x);b=Math.max(b,y);}
    // A mostly masked viewport cannot attest the surface; stay on CDP.
    if(matched&&masked&&masked.reduce((n,v)=>n+v,0)>raw.width*raw.height*.9){matched=false;this.timing('native_attestation_masked',performance.timeOrigin+performance.now());break;}
    if(differing){matched=false;this.timing(`native_attestation_differs ${attempt} ${differing} ${l},${t},${r},${b}`,performance.timeOrigin+performance.now());}
    else if(!matched)this.timing(`native_attestation_geometry ${reference.width}x${reference.height} ${raw.width}x${raw.height}`,performance.timeOrigin+performance.now());
   }
   if(!matched)throw Error('native attestation RGB differed');
   await assertCurrentDocument(this.connection,this.sessionId,this.tab.target_id,this.tab.document_id);
   if(!this.valid())throw Error('native attestation retired');
   this.attested=true;this.timing('native_admitted',performance.timeOrigin+performance.now());return this;
  }catch(error){this.timing('native_unavailable_'+this.phase+'_'+(this.helperStage??'host')+' '+nativeRefusalReason(error,this.helperStage),performance.timeOrigin+performance.now());await this.close();throw error}
 }
 async publish(){
  if(this.publishing)return;this.publishing=true;
  try{while(this.pending&&this.valid()){
   let raw=this.pending;this.pending=null;this.publishingRaw=raw;
   const revision=this.regionRevision;
   // MP-08/MP-10/MP-11: no per-readback CDP round trip. Navigation and new
   // page targets fence this source by event (onCdp); every delivered frame
   // still passes the credit's document check, which CDP orders after those
   // events. The owned single-tab window is never occluded or hidden.
   let at=performance.timeOrigin+performance.now();let regions;
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
   this.previousRegions=regions;
   this.motionStreak=performance.now()-this.changedAt<90?this.motionStreak+1:1;this.changedAt=performance.now();
   this.timing('native_xshm_capture',raw.captured_ms); // includes bounded pipe delivery and source fence.
   this.latest?.raw.release?.();
   this.latest={raw,signature:raw.signature,data_base64:raw.signature,width:raw.width,height:raw.height,serial:raw.serial,captured_ms:raw.captured_ms,published_ms:performance.timeOrigin+performance.now(),motion:true,tab_id:this.tab.tab_id,document_id:this.tab.document_id};
   this.publishingRaw=null;
   if(this.attested)for(const fn of this.listeners)fn(this.latest);
  }}catch{this.fence()}finally{this.publishingRaw?.release?.();this.publishingRaw=null;this.publishing=false}
 }
 // MP-08/MP-10: notch wheel input on the owned private X display (native
 // smooth scrolling). CSS coordinates; callers fence document and actor first.
 wheel(x,y,dx,dy){
  if(!this.nativeWorker||!this.valid()||!this.attested||!this.child||this.child.stdin.destroyed)return false;
  const px=Math.floor(x*this.scale),py=Math.floor(y*this.scale);
  if(![px,py,dx,dy].every(Number.isSafeInteger)||px<0||py<0||px>=geometry.width*this.scale||py>=geometry.height*this.scale||Math.abs(dx)>10||Math.abs(dy)>10||(!dx&&!dy))return false;
  return this.pointer('wheel',{point:[px,py],notches:[dx,dy]});
 }
 // MP-08/MP-10/MP-11: pointer input is a request. The worker reports whether
 // the owned window received it (it refuses when another top-level window,
 // e.g. a permission bubble outside the capture, covers the point), so a
 // refusal falls back to CDP instead of retiring native capture.
 async pointer(operation,values){
  try{const reply=await this.nativeWorker.request(operation,values);return reply?.[operation==='wheel'?'wheeled':'clicked']===true;}
  catch{return false;}
 }
 // MP-08/MP-10: primary click on the owned display (CSS coordinates).
 click(x,y){
  if(!this.nativeWorker||!this.valid()||!this.attested||!this.child||this.child.stdin.destroyed)return false;
  const px=Math.floor(x*this.scale),py=Math.floor(y*this.scale);
  if(![px,py].every(Number.isSafeInteger)||px<0||py<0||px>=geometry.width*this.scale||py>=geometry.height*this.scale)return false;
  return this.pointer('click',{point:[px,py]});
 }
 // MP-08/MP-10: one key press/release on the owned display (X keysym:
 // printable ASCII or a named editing key); callers fence the text target.
 key(keysym,shift=false){
  if(!this.nativeWorker||!this.valid()||!this.attested||!this.child||this.child.stdin.destroyed)return false;
  if(!Number.isSafeInteger(keysym)||!(keysym>=0x20&&keysym<=0x7e||keysym>=0xff08&&keysym<=0xffff))return false;
  return this.control({key:[keysym,shift?1:0]},true);
 }
 // MP-08/MP-10: scroll plans cost a full-frame compare per readback; request
 // them only while a viewer canvas is exact and unprotected.
 plans(enabled){
  enabled=Boolean(enabled);if(!this.nativeWorker||enabled===this.planning||!this.valid()||!this.child||this.child.stdin.destroyed)return;
  this.planning=enabled;this.control({plans:enabled});
 }
 // MP-08/MP-10: only admitted physical input reaches this owned helper.
 wake(refresh=false){if(this.valid()&&this.child&&!this.child.stdin.destroyed)this.control(refresh?{refresh:true}:{wake:true},true)}
 // MP-08/MP-10/MP-11: one ordered control stream, including lease release.
 control(command,immediate=false){
  if(this.nativeWorker)return this.nativeWorker.notify(command,immediate);
  if(!this.child||this.child.stdin.destroyed)return false;
  this.child.stdin.write(JSON.stringify(command)+'\n');return true;
 }
 sample(after=-1){return this.valid()&&this.attested&&this.latest?.serial>after?this.latest:null}
 fence(){this.closed=true;this.attested=false;this.regions?.retire();this.latest?.raw.release?.();this.pending?.release?.();this.latest=null;this.pending=null;this.listeners.clear();if(!this.closing)this.closing=this.cleanup();this.closing.catch(()=>{})}
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
