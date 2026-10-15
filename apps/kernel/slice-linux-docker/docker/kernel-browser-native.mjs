import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
// MD-DISPLAY-02/04: OS-neutral capture interface: start/subscribe/sample/close.
// Linux XDamage/XShm implementation; macOS unsupported until ScreenCaptureKit.
import {mkdtemp,rm} from 'node:fs/promises';
import path from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
import {ownsDisplay} from './kernel-browser-owned-display.mjs';
import {startNativeReadback} from './kernel-browser-native-readback.mjs';
import {decodePng,maskNativeRaster} from './kernel-browser-pixels.mjs';
import {NativeRegionProtection,regionProtectionChanged} from './kernel-browser-region-protection.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';
import {NativeWorkerControl,nativeWorkerResource} from './kernel-browser-native-worker.mjs';
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
import {safeChildPid} from './kernel-browser-display.mjs';
import {fitNativeViewport} from './kernel-browser-native-viewport.mjs';
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
 constructor({display,pid,connection,sessionId,tab,scale,hostScale=1,policy,browser,screenshot,allowed,frames,timing=()=>{}}){
  Object.assign(this,{display,pid,connection,sessionId,tab,scale,hostScale,policy,browser,screenshot,allowed,frames,timing});this.native=true;this.listeners=new Set();this.closed=false;this.attested=false;this.regionRevision=0;this.changedAt=performance.now();this.motionStreak=0;this.ignoreIdleUntil=-Infinity;
 }
 subscribe(fn){this.listeners.add(fn);return()=>this.listeners.delete(fn)}
 async createRasterPool(){return mkdtemp(path.join(nativeWorkerResource()?.root??this.display.root,'raster-'));}
 valid(){return !this.closed&&this.connection?.isOpen?.()!==false&&ownsDisplay(this.display)&&this.allowed(this.policy)}
 rejected(reason){this.nativeWorker?.rejected?.(reason);}
 onCdp(m){
  if(m.method==='Chariox.browserDisconnected'||m.method==='Target.detachedFromTarget'&&m.params?.sessionId===this.sessionId||m.method==='Target.targetDestroyed'&&m.params?.targetId===this.tab?.target_id||m.sessionId===this.sessionId&&m.method==='Inspector.detached'){this.rejected('stale_document');this.fence();return;}
  if(m.sessionId===this.sessionId&&m.method==='Page.frameNavigated'&&!m.params?.frame?.parentId){if(this.latest||this.pending||this.publishingRaw)this.rejected('stale_document');this.fence();}
  if(m.method==='Target.targetCreated'&&m.params?.targetInfo?.type==='page')this.fence();
  // MP-11: attribute-only protection changes produce no XDamage. Retire even
  // before attestation, so an empty snapshot cannot outlive marker insertion.
  if(this.regions&&regionProtectionChanged(m,this.sessionId,this.regions.tracker())){
   // MP-08/MP-10/MP-11: the owned window/document binding stays intact,
   // also during attestation (pages that mutate continuously still attest).
   // Retire all pixels and queued consumers by revision, refresh only trusted
   // region metadata, and mask any readback older than that new fence in full.
   if(this.latest||this.pending||this.publishingRaw)this.rejected('region_revision');
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
   await fitNativeViewport({connection:this.connection,windowId,sessionId:this.sessionId,width:geometry.width*this.scale,height:geometry.height*this.scale,hostScale:this.hostScale,
    measure:async()=>decodePng((await this.screenshot()).data_base64,this.scale)});
   // A DOM mutation may retire the first fence while it is measured (busy
   // real pages); retry a bounded number of times instead of refusing.
   this.regions=new NativeRegionProtection(this.connection,this.sessionId,{policy:this.policy,browser:this.browser,tab:this.tab,scale:this.scale,record:reason=>this.timing('region_frame_masked '+reason,performance.timeOrigin+performance.now())});
   for(let attempt=0;;attempt++){try{await this.regions.refresh();break}catch(error){if(attempt>=4||!/region fence retired/.test(error?.message??''))throw error;}}
   this.poolRoot=await this.createRasterPool();
   startNativeReadback(this,{width:geometry.width*this.scale,height:geometry.height*this.scale});
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
   // MP-11: mirror geometry is live authority, not a cached synchronous epoch.
   // Refuse a readback before any mask, encoder offer or delivery can consume it.
   if(this.validateCurrent&&!await this.validateCurrent()){this.fence();break;}
   let raw=this.pending;this.pending=null;this.publishingRaw=raw;
   if(this.connection.presentationFence&&!this.connection.presentationFence.stable(raw.captured_ms,raw.captured_ms+(raw.capture_ms??0))){
    this.timing('native_capture_transient_dropped',raw.captured_ms);raw.release?.();this.publishingRaw=null;this.wake(true);continue;
   }
   const revision=this.regionRevision;
   // MP-08/MP-10/MP-11: every readback checks live Vault-filled field type
   // and geometry. Document changes also retire the source through onCdp.
   let at=performance.timeOrigin+performance.now();let regions;
   try{
    if(!this.regions.guard)await this.regions.refresh();
    if(raw.captured_ms>=this.regions.beforeAt)regions=await this.regions.regions(raw);
   }catch(error){
    if(error.code==='fill_capture_retry'){this.regions.retire();this.wake(true);raw.release?.();this.publishingRaw=null;continue;}
    if(revision===this.regionRevision)throw error;
   }
   this.timing('native_region_fence',at);
   if(revision!==this.regionRevision){this.rejected('region_revision');raw.release?.();this.publishingRaw=null;continue;}
   // Attribute-only changes need a new readback even if XDamage/pixels did
   // not change. The first readback may precede the refreshed metadata.
   if(raw.captured_ms<this.regions.beforeAt){this.wake(true);raw.release?.();this.publishingRaw=null;continue;}
   at=performance.timeOrigin+performance.now();const masked=maskNativeRaster(raw,regions,this.previousRegions,this.latest?.raw);this.timing('native_mask_copy_hash',at);
   if(masked!==raw){raw.release?.();raw=masked;this.publishingRaw=raw;}
   if(!this.valid()||this.validateCurrent&&!await this.validateCurrent())break;
   this.previousRegions=regions;
   this.motionStreak=performance.now()-this.changedAt<90?this.motionStreak+1:1;this.changedAt=performance.now();
   this.timing('native_xshm_capture',raw.captured_ms); // includes bounded pipe delivery and source fence.
   this.latest?.raw.release?.();
   this.latest={raw,signature:raw.signature,data_base64:raw.signature,width:raw.width,height:raw.height,serial:raw.serial,captured_ms:raw.captured_ms,published_ms:performance.timeOrigin+performance.now(),motion:true,tab_id:this.tab.tab_id,document_id:this.tab.document_id};
   this.publishingRaw=null;
   if(this.attested)for(const fn of this.listeners)fn(this.latest);
  }}catch(error){this.timing(`native_publish_failed ${String(error?.message).replace(/[^a-z_ -]/gi,'').slice(0,60)}`,performance.timeOrigin+performance.now());this.fence()}finally{this.publishingRaw?.release?.();this.publishingRaw=null;this.publishing=false}
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
 // refusal falls back to CDP instead of retiring native capture. A command
 // sent without a reply may have been dispatched: never replay it; retire
 // this source and report the uncertain action (code native_input_uncertain).
 async pointer(operation,values){
  let reply;
  try{reply=await this.nativeWorker.request(operation,values);}
  catch(error){
   if(error?.dispatched===false)return false;
   this.timing?.('native_input_uncertain',performance.timeOrigin+performance.now());this.fence();
   throw Object.assign(Error('MP-11: native input outcome uncertain'),{code:'native_input_uncertain'});
  }
  return reply?.[{wheel:'wheeled',drag:'dragged'}[operation]??'clicked']===true;
 }
 // MP-08/MP-10: click on the owned display (CSS coordinates); protocol 530
 // buttons 2 (middle) and 3 (right) only where the caller admitted them.
 devicePoint(x,y){
  const px=Math.floor(x*this.scale),py=Math.floor(y*this.scale);
  return [px,py].every(Number.isSafeInteger)&&px>=0&&py>=0&&px<geometry.width*this.scale&&py<geometry.height*this.scale?[px,py]:null;
 }
 click(x,y,button=1){
  if(!this.nativeWorker||!this.valid()||!this.attested||!this.child||this.child.stdin.destroyed||![1,2,3].includes(button))return false;
  const point=this.devicePoint(x,y);
  return point?this.pointer('click',{point,...(button===1?{}:{button})}):false;
 }
 // Protocol 530: primary-button drag between two owned points.
 drag(x,y,toX,toY){
  if(!this.nativeWorker||!this.valid()||!this.attested||!this.child||this.child.stdin.destroyed)return false;
  const point=this.devicePoint(x,y),to=this.devicePoint(toX,toY);
  return point&&to?this.pointer('drag',{point,to}):false;
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
  if(child&&child.pid!==undefined){
   safeChildPid(child);child.stdin.end();
   for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await delay(20);
   if(child.exitCode===null&&child.signalCode===null){child.kill('SIGTERM');for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await delay(20)}
   if(child.exitCode===null&&child.signalCode===null){child.kill('SIGKILL');await new Promise(r=>child.once('exit',r))}
  }
  if(this.poolRoot){await rm(this.poolRoot,{recursive:true,force:true});this.poolRoot=null}
 }
}
