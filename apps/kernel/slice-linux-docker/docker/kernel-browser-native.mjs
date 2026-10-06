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
import {safeChildPid} from './kernel-browser-display.mjs';
export async function selectNativeCapture({platform=process.platform,display,create}){
 if(platform!=='linux'||!ownsDisplay(display))return null;
 try{return await create()}catch{return null}
}
export class LinuxCapture {
 constructor({display,pid,connection,sessionId,tab,scale,policy,screenshot,allowed,timing=()=>{}}){
  Object.assign(this,{display,pid,connection,sessionId,tab,scale,policy,screenshot,allowed,timing});this.listeners=new Set();this.closed=false;this.attested=false;this.changedAt=performance.now();this.motionStreak=0;this.ignoreIdleUntil=-Infinity;
 }
 subscribe(fn){this.listeners.add(fn);return()=>this.listeners.delete(fn)}
 valid(){return !this.closed&&ownsDisplay(this.display)&&this.allowed(this.policy)}
 async start(){
  if(!this.valid()||!Number.isSafeInteger(this.pid)||this.pid<=1)throw Error('MD-DISPLAY: native surface not owned');
  this.off=this.connection.subscribe(m=>{
   if(m.sessionId===this.sessionId&&m.method==='Page.frameNavigated'&&!m.params?.frame?.parentId)this.fence();
   if(m.method==='Target.targetCreated'&&m.params?.targetInfo?.type==='page')this.fence();
   // MP-11: attribute-only protection changes produce no XDamage. Retire
   // cached pixels/dependencies before another credit can use their old mask.
   if(this.attested&&regionProtectionChanged(m,this.sessionId,this.regions?.guard?.hasRegions))this.fence();
  });
  try{
   this.phase='bounds';const {windowId}=await this.connection.send('Browser.getWindowForTarget',{targetId:this.tab.target_id});
   // Native DPR2: fit800 CSS pixels below Chromium's87px browser chrome.
   await this.connection.send('Browser.setWindowBounds',{windowId,bounds:{width:geometry.width,height:geometry.height+87}});
   await this.connection.send('Page.bringToFront',{},this.sessionId);
   const {frameTree}=await this.connection.send('Page.getFrameTree',{},this.sessionId);
   const {executionContextId}=await this.connection.send('Page.createIsolatedWorld',{frameId:frameTree.frame.id,worldName:'chariox-native-surface-fence',grantUniveralAccess:false},this.sessionId);
   this.contextId=executionContextId;
   await delay(100);
   // Force the emulated viewport to paint before establishing its native crop.
   await this.screenshot();
   this.regions=new NativeRegionProtection(this.connection,this.sessionId);await this.regions.refresh();
   this.poolRoot=await mkdtemp(path.join(this.display.root,'raster-'));
   this.phase='readback';const child=spawn(process.env.CHARIOX_BROWSER_DISPLAY_PYTHON||'python3',['-u',fileURLToPath(new URL('./kernel-browser-xshm.py',import.meta.url))],{env:{...process.env,...this.display.environment},stdio:['pipe','pipe','pipe']});this.child=child;
   const fail=()=>this.fence();child.on('error',fail);child.on('exit',fail);child.stdin.on('error',fail);
   child.stderr.on('data',b=>{if(/^MD-DISPLAY: native stage [a-z_]+\n$/.test(b.toString()))this.helperStage=b.toString().trim().split(' ').at(-1)});
   const raster=new NativeRaster();
   const pool=new SharedRasterPool(this.poolRoot,(slot,serial)=>{if(!child.stdin.destroyed)child.stdin.write(JSON.stringify({release:slot,serial})+'\n')});
   const pipe=new NativePipe(header=>{
    if(header.width!==geometry.width*this.scale||header.height!==geometry.height*this.scale||!Number.isSafeInteger(header.serial)||header.serial<1||!Number.isFinite(header.captured_ms)||!Number.isFinite(header.capture_ms)||!/^[a-f0-9]{16}$/.test(header.signature))throw Error('raw geometry');
   },(header,pixels)=>{
    const received=performance.timeOrigin+performance.now();
    for(const [name,start,end] of [['input_wake_to_capture',header.input_wake_ms,header.captured_ms],['native_damage_coalesce',header.damage_ready_ms,header.captured_ms],['native_window_fence',header.captured_ms,header.get_image_ms],['native_xshm_get_image',header.get_image_ms,header.image_ready_ms],['native_readback_copy',header.image_ready_ms,header.readback_ms],['native_readback',header.captured_ms,header.readback_ms],['native_fingerprint',header.readback_ms,header.fingerprint_ms],['native_damage_scan',header.fingerprint_ms,header.damage_ms]]){
     if(Number.isFinite(start)&&Number.isFinite(end)&&end>=start)this.timing(name,start,end);
    }
    if(Number.isFinite(header.damage_ms))this.timing('native_pipe',header.damage_ms,received);
    const at=performance.timeOrigin+performance.now(),raw=header.slot==null?raster.apply(header,pixels):pool.apply(header);this.timing('native_raster_copy',at);
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
    await delay(50);
    const raw=this.latest?.raw;if(!raw||!this.valid())break;
    matched=reference.width===raw.width&&reference.height===raw.height;
    const nativePixels=raw.pixels;
    for(let n=0;matched&&n<nativePixels.length;n+=4)matched=reference.pixels[n]===nativePixels[n+2]&&reference.pixels[n+1]===nativePixels[n+1]&&reference.pixels[n+2]===nativePixels[n];
   }
   if(!matched)throw Error('native attestation RGB differed');
   await assertCurrentDocument(this.connection,this.sessionId,this.tab.target_id,this.tab.document_id);
   if(!this.valid())throw Error('native attestation retired');
   this.attested=true;this.timing('native_admitted',performance.timeOrigin+performance.now());return this;
  }catch(error){this.timing('native_unavailable_'+this.phase+'_'+(this.helperStage??'host'),performance.timeOrigin+performance.now());await this.close();throw error}
 }
 async publish(){
  if(this.publishing)return;this.publishing=true;
  try{while(this.pending&&this.valid()){
   let raw=this.pending;this.pending=null;this.publishingRaw=raw;
   let at=performance.timeOrigin+performance.now();
   await assertCurrentDocument(this.connection,this.sessionId,this.tab.target_id,this.tab.document_id);this.timing('native_document_fence',at);
   at=performance.timeOrigin+performance.now();
   const visibility=await this.connection.send('Runtime.evaluate',{expression:'document.visibilityState',contextId:this.contextId,returnByValue:true},this.sessionId);
   this.timing('native_visibility_fence',at);
   if(visibility.result?.value!=='visible')throw Error('native source not visible');
   const masked=maskNativeRaster(raw,await this.regions.regions(raw));
   if(masked!==raw){raw.release?.();raw=masked;this.publishingRaw=raw;}
   if(!this.valid())break;
   this.motionStreak=performance.now()-this.changedAt<90?this.motionStreak+1:1;this.changedAt=performance.now();
   this.timing('native_xshm_capture',raw.captured_ms); // includes bounded pipe delivery and source fence.
   this.latest?.raw.release?.();
   this.latest={raw,signature:raw.signature,data_base64:raw.signature,width:raw.width,height:raw.height,serial:raw.serial,captured_ms:raw.captured_ms,motion:true,tab_id:this.tab.tab_id,document_id:this.tab.document_id};
   this.publishingRaw=null;
   if(this.attested)for(const fn of this.listeners)fn(this.latest);
  }}catch{this.fence()}finally{this.publishingRaw?.release?.();this.publishingRaw=null;this.publishing=false}
 }
 // MP-08/MP-10: only admitted physical input reaches this owned helper.
 wake(){if(this.valid()&&this.child&&!this.child.stdin.destroyed)this.child.stdin.write(JSON.stringify({wake:true})+'\n')}
 sample(after=-1){return this.valid()&&this.attested&&this.latest?.serial>after?this.latest:null}
 fence(){this.closed=true;this.attested=false;this.latest?.raw.release?.();this.pending?.release?.();this.latest=null;this.pending=null;this.listeners.clear();if(!this.closing)this.closing=this.cleanup();this.closing.catch(()=>{})}
 pause(){} resume(){} // CDP exact reads do not mutate the native display.
 async close(){this.fence();return this.closing}
 async cleanup(){
  this.off?.();const child=this.child;this.child=null;
  if(!child||child.pid===undefined)return;safeChildPid(child);child.stdin.end();
  for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await delay(20);
  if(child.exitCode===null&&child.signalCode===null){child.kill('SIGTERM');for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await delay(20)}
  if(child.exitCode===null&&child.signalCode===null){child.kill('SIGKILL');await new Promise(r=>child.once('exit',r))}
  if(this.poolRoot){await rm(this.poolRoot,{recursive:true,force:true});this.poolRoot=null}
 }
}
