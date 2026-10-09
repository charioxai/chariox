// MD-DISPLAY-02/04: source-driven encoding, bounded latest work and recovery.
// Capture/encode never await a viewer credit. Dropped dependencies force a key.
import {displayMaskRegions,encodePng} from './kernel-browser-pixels.mjs';
const frameBytes=frame=>JSON.stringify(frame.encoded??{}).length+(frame.encoded?.packet?.length??0)+(frame.force_lossless?frame.data_base64.length:0);
// MP-08/MP-10: full viewport motion benefits from vertical reference reuse
// across stripe boundaries. Existing negotiated full-video packets provide
// that path; sparse rows still keep their independent bounded references.
// The private exact row span selects codec mode only, never cropped pixels.
const motionHeight=raw=>Number.isSafeInteger(raw.motion_height)&&raw.motion_height>=0&&raw.motion_height<=raw.height?raw.motion_height:raw.height;
const denseNative=raw=>raw?.nativeEncode&&!raw[displayMaskRegions]?.length&&Array.isArray(raw.damage)&&raw.damage.length===4&&
 (raw.damage[2]-raw.damage[0])*(raw.damage[3]-raw.damage[1])>raw.width*raw.height/2&&motionHeight(raw)>=raw.height*.75;
export class MotionEncoder {
 constructor(source,encoder,{bitrate,codec,independent=false,stripes=false,valid=()=>true,shouldEncode=()=>true,timing=()=>{},now=()=>performance.now()}={}){
  Object.assign(this,{source,encoder,bitrate,codec,independent,stripes,valid,shouldEncode,timing,now});this.frames=[];this.pending=null;this.active=null;this.key=true;this.resetRows=new Set();this.rate=new CreditBudget(bitrate,now);this.revision=0;this.lastSerial=-1;this.closed=false;this.readyWaiters=new Set();
  this.off=source.subscribe(sample=>this.offer(sample));this.offer(source.sample());
 }
 wakeReady(){for(const wake of [...this.readyWaiters])wake();}
 waitReady(ms=20,signal){
  if(this.frames.length||this.failure||this.closed||signal?.aborted)return Promise.resolve();
  return new Promise(resolve=>{
   const wake=()=>{clearTimeout(timer);signal?.removeEventListener('abort',wake);this.readyWaiters.delete(wake);resolve();};
   const timer=setTimeout(wake,ms);this.readyWaiters.add(wake);signal?.addEventListener('abort',wake,{once:true});
  });
 }
 // MP-08/MP-10: encode a sample previously skipped as a lossless candidate.
 // Nothing was encoded for it, so no reference chain is retired.
 retry(sample){if(sample?.serial===this.lastSerial&&!this.pending&&!this.closed){this.lastSerial=sample.serial-1;this.offer(sample)}}
 offer(sample){if(!sample||this.closed||!this.valid()||sample.serial<=this.lastSerial)return;this.lastSerial=sample.serial;if(!this.shouldEncode(sample)){this.wakeReady();return;}this.pending?.raw?.release?.();sample.raw?.retain?.();this.pending={...sample,offeredAt:this.now()};this.pump();}
 pump(){if(this.active||!this.pending||this.closed||this.frames.length>=2)return;this.active=this.run().catch(()=>{if(!this.closed)this.failure=Error('MD-DISPLAY: motion encoder failed')}).finally(()=>{this.active=null;if(this.failure)this.wakeReady();if(this.pending&&!this.closed&&!this.failure)this.pump()});}
 async run(){
  while(this.pending&&!this.closed&&!this.failure&&this.frames.length<2){
   const sample=this.pending;this.pending=null;const rowMode=Boolean(this.stripes&&sample.raw&&!denseNative(sample.raw));if(this.rowMode!==rowMode)this.key=true;this.rowMode=rowMode;const revision=this.revision,key=this.independent||this.key;this.key=false;const reset=key?true:[...this.resetRows];this.resetRows.clear();const at=performance.timeOrigin+this.now();
   try{
   let encoded;
   try{encoded=rowMode?await this.encoder.encodeStripes({...sample.raw,motion:true},this.rate.bitrate,reset,this.codec):await this.encoder.encode(sample.raw?{...sample.raw,motion:true}:sample.data_base64,this.rate.bitrate,key,this.codec,sample[displayMaskRegions]??[]);}
   // MP-11: an overlapping exact encode (busy) or one refused protected frame
   // skips this sample; the next one starts an independent reference chain.
   catch(error){if(!error?.busy&&!error?.refused)throw error;this.key=true;continue;}
   this.timing('motion_encode',at);
   if(encoded.dropped){
    // MP-11: discard every lossy byte, retire its references, and present only
    // this already-protected exact raster. Never freeze a newly masked field.
    this.key=true;
    if(this.closed||!this.valid()||revision!==this.revision)continue;
    let png=sample.data_base64,nativeExact;
    if(sample.raw?.nativeExact){
     const raw=sample.raw,exact=await raw.nativeExact({encoder:this.encoder.nativeSession,regions:raw[displayMaskRegions]??[],patch:false});
     png=exact.data_base64;
     // MP-08/MP-10/MP-11: the native exact contract includes indexed PNGs
     // and prepared repair tiles; never send it through the RGBA-only decoder.
     nativeExact={...exact,refinement_serial:sample.serial};
    }else if(sample.raw){
     const raw=sample.raw,pixels=Buffer.from(raw.pixels);
     for(let i=0;i<pixels.length;i+=4){const blue=pixels[i];pixels[i]=pixels[i+2];pixels[i+2]=blue;pixels[i+3]=255;}
     png=encodePng(raw.width,raw.height,pixels);
    }
    this.timing('protected_codec_drop',performance.timeOrigin+this.now());
    const {raw,...protectedSample}=sample;
    // MP-11: exact fallback uses the bounded PNG budget. The smaller video
    // packet budget must not prevent a safe Retina raster from settling.
    if(png.length+this.frames.reduce((n,f)=>n+frameBytes(f),0)>4*1024*1024){this.invalidate(false);throw Error('MP-11: protected fallback exceeds bounded queue');}
    this.frames.push({...protectedSample,...nativeExact,data_base64:png,motion:false,force_lossless:true});this.wakeReady();
    continue;
   }
   if(this.encoder.hardwareFallback)this.timing('motion_hardware_fallback',performance.timeOrigin+this.now());
   if(['vaapi','x264','openh264','vp8','vp9','webcodecs'].includes(this.encoder.backend))this.timing('motion_backend_'+this.encoder.backend,performance.timeOrigin+this.now());
   if([1,2,4].includes(this.encoder.workers))this.timing('motion_workers_'+this.encoder.workers,performance.timeOrigin+this.now());
   if(encoded.packet)this.timing('motion_packet_native',performance.timeOrigin+this.now());
   if(encoded.reduced)this.timing('motion_reduced_contention',performance.timeOrigin+this.now());
   if(['libyuv','swscale'].includes(this.encoder.converter))this.timing('motion_converter_'+this.encoder.converter,performance.timeOrigin+this.now());
   if(this.closed||!this.valid()){this.encoder.discard?.(encoded);return;}
   if(encoded.stripes?.length===0)continue;
   if(revision!==this.revision){this.encoder.discard?.(encoded);if(encoded.stripes&&!this.key)for(const row of encoded.stripes)this.resetRows.add(row.row);else this.key=true;continue;}
   if(this.frames.length>=2||this.frames.reduce((n,f)=>n+frameBytes(f),(JSON.stringify(encoded).length+(encoded.packet?.length??0)))>1024*1024){this.encoder.discard?.(encoded);this.invalidate();continue;}
   this.frames.push({...sample,encoded});this.wakeReady();
   }finally{sample.raw?.release?.()}
  }
 }
 feedback(lag){if(this.rate.feedback(lag))this.key=true;}
 take(){
  if(this.failure)throw this.failure;
  // Dropping an encoded delta also retires every reference after it. Recover
  // once from the latest source with an independent frame, never decode gaps.
  const oldest=this.frames[0];
  const age=oldest ? this.now()-oldest.offeredAt : 0;
  // An input burst can delay admitted capture credits beyond100ms. Repeatedly
  // discarding its recovery key then starves the decoder forever. Permit that
  // independently decodable base a bounded300ms; stale deltas still retire
  // their entire reference chain after100ms, and older keys also retire.
  if(oldest && age>((oldest.force_lossless||oldest.encoded?.key||oldest.encoded?.stripes?.every(r=>r.key))?300:100) && this.source.sample()?.serial>oldest.serial){
   this.rate.feedback(8);this.invalidateRows();this.pump();return null;
  }
  const frame=this.frames.shift()??null;this.pump();return frame;
 }
 retireUnsent(){if(this.frames.length||this.active||this.pending)this.invalidateRows(false)}
 invalidateRows(reoffer=true){if(!this.rowMode||this.frames.some(frame=>!frame.encoded?.stripes))return this.invalidate(reoffer);for(const frame of this.frames)for(const row of frame.encoded.stripes)this.resetRows.add(row.row);this.revision++;for(const frame of this.frames)this.encoder.discard?.(frame.encoded);this.frames=[];this.pending?.raw?.release?.();this.pending=null;if(reoffer){this.lastSerial=-1;if(!this.closed)this.offer(this.source.sample())}else this.lastSerial=Math.max(this.lastSerial,this.source.sample()?.serial??-1);}
 invalidate(reoffer=true){this.revision++;this.key=true;for(const frame of this.frames)this.encoder.discard?.(frame.encoded);this.frames=[];this.pending?.raw?.release?.();this.pending=null;if(reoffer){this.lastSerial=-1;if(!this.closed)this.offer(this.source.sample())}else this.lastSerial=Math.max(this.lastSerial,this.source.sample()?.serial??-1);}
 async close(){if(this.closed)return;this.closed=true;this.wakeReady();this.off?.();this.invalidate();await this.active;}
}

// MD-DISPLAY-02/04: credit pressure, not a bandwidth estimator. Never exceed
// the negotiated ceiling; sustained backlog lowers motion bitrate only.
export class CreditBudget {
 constructor(ceiling,now=()=>performance.now()){this.ceiling=ceiling;this.bitrate=ceiling;this.floor=Math.min(ceiling,500000);this.now=now;this.changedAt=now();this.pressureAt=null;this.clearAt=null;}
 feedback(lag){
  const now=this.now();if(!Number.isSafeInteger(lag)||lag<0||lag>8)return false;
  // MP-08/MP-10: up to four pipelined credits are ordinary scheduling, not
  // evidence of congestion. Reserve adaptation for a nearly full8-frame window.
  if(lag>=6){this.clearAt=null;this.pressureAt??=now;if(now-this.pressureAt<500||now-this.changedAt<500)return false;this.pressureAt=now;return this.set(Math.max(this.floor,Math.round(this.bitrate*.8)),now)}
  this.pressureAt=null;if(lag<=1){this.clearAt??=now;if(now-this.clearAt>=2000&&now-this.changedAt>=2000){this.clearAt=now;return this.set(Math.min(this.ceiling,Math.round(this.bitrate*1.1)),now)}}else this.clearAt=null;return false;
 }
 set(value,now){if(value===this.bitrate)return false;this.bitrate=value;this.changedAt=now;return true;}
}
