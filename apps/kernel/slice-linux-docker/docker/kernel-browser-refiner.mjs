// MD-DISPLAY-02/04: bounded asynchronous exact verification; no input lock.
import {PixelWorker} from './kernel-browser-pixel-worker.mjs';
import {timestamp} from './kernel-browser-timing.mjs';
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
export class NativeRefiner {
 constructor(capture,{pixels=new PixelWorker(),now=()=>performance.now(),quietMs=300,quietNativeMs=50,verifyMs=250,prepareTiles=false,timing=()=>{}}={}){Object.assign(this,{capture,pixels,now,quietMs,quietNativeMs,verifyMs,prepareTiles,timing});this.latest=null;this.active=null;this.closed=false;this.verifiedAt=-Infinity;this.revision=0;this.prepared=null;}
 same(a,b){return a&&b&&a.source===b.source&&a.document===b.document&&a.policy===b.policy&&a.epoch===b.epoch&&a.serial===b.serial&&a.native===b.native&&a.nativeDelivered===b.nativeDelivered;}
 request(binding,changedAt,valid){
  if(this.closed)throw Error('MD-DISPLAY: refiner closed');
  if(this.failure)throw this.failure;
  if(!this.same(this.wanted,binding)){this.wanted=binding;this.latest=null;this.revision++}
  if(!this.active&&this.now()-changedAt>=(binding.native?this.quietNativeMs:this.quietMs)&&(!this.latest||(!binding.native&&this.now()-this.verifiedAt>=this.verifyMs))){
   const revision=this.revision;
   this.active=(async()=>{
    let at=timestamp();
    // MP-08/MP-10/MP-11: retain the admitted masked native snapshot before
    // yielding. Exact repair uses its bytes without a CDP/input sample lock.
    const raw=binding.native?binding.sample?.raw:null;raw?.retain?.();
    let source,pixels;
    try{
     source=await this.capture(binding);
     if(raw?.nativeExact){
      const exact=await raw.nativeExact({encoder:binding.encoder,regions:raw[displayMaskRegions]??[],patch:false,repair_only:true});
      source={...source,...exact,motion:false};pixels={width:raw.width,height:raw.height,pixels:null,signature:source.signature};
     }else if(raw){
      if(source.raw!==raw)throw Error('MP-11: native refinement binding changed');
      pixels=await this.pixels.run('native',{data:raw.pixels,width:raw.width,height:raw.height});
      source={...source,motion:false,[displayMaskRegions]:raw[displayMaskRegions]??[],data_base64:pixels.data_base64};
      delete pixels.data_base64;
     }
    }finally{raw?.release?.();}
    this.timing('exact_capture',at);at=timestamp();
    pixels??=await this.pixels.run('decode',{data:source.data_base64,scale:source.device_scale_factor??binding.scale});
    this.timing('exact_decode',at);at=timestamp();
    if(this.closed||revision!==this.revision||!this.same(this.wanted,binding)||!valid())return;
    let repair_tiles=source.repair_tiles;
    if(!source.native_exact&&this.prepareTiles&&source.data_base64.length+64>(binding.repairLimit??0)){
     // Expensive whole-raster PNG tile encoding never owns a capture credit.
     // Reuse only immutable identical verified RGB; every deadline still reads
     // a fresh protected PNG, including fine changes hidden by a lossy source.
     const previous=this.prepared;
     repair_tiles=previous&&previous.pixels.width===pixels.width&&previous.pixels.height===pixels.height&&previous.pixels.pixels.equals(pixels.pixels)
      ? previous.tiles : await this.pixels.run('tiles',{previous:null,current:pixels,all:true});
     this.timing('exact_tiles_prepare',at);
    }
    if(!this.closed&&revision===this.revision&&this.same(this.wanted,binding)&&valid()){
     this.latest={...source,pixels,repair_tiles,settled_verified:true,refinement_serial:binding.serial};this.verifiedAt=this.now();
     if(repair_tiles&&pixels?.pixels)this.prepared={pixels,tiles:repair_tiles};
    }
   })().catch(()=>{if(!this.closed&&revision===this.revision&&valid())this.failure=Error('MD-DISPLAY: exact verification failed')}).finally(()=>{this.active=null});
  }
  return this.same(this.wanted,binding)?this.latest:null;
 }
 invalidate(){this.revision++;this.latest=null;this.wanted=null;this.prepared=null;this.verifiedAt=-Infinity;}
 async close(){if(this.closed)return;this.closed=true;this.invalidate();await this.active;await this.pixels.close()}
}
