// MD-DISPLAY-02/04: source-driven encoding, bounded latest work and recovery.
// Capture/encode never await a viewer credit. Dropped dependencies force a key.
export class MotionEncoder {
 constructor(source,encoder,{bitrate,codec,independent=false,valid=()=>true,shouldEncode=()=>true,timing=()=>{},now=()=>performance.now()}={}){
  Object.assign(this,{source,encoder,bitrate,codec,independent,valid,shouldEncode,timing,now});this.frames=[];this.pending=null;this.active=null;this.key=true;this.rate=new CreditBudget(bitrate,now);this.revision=0;this.lastSerial=-1;this.closed=false;
  this.off=source.subscribe(sample=>this.offer(sample));this.offer(source.sample());
 }
 offer(sample){if(!sample||this.closed||!this.valid()||sample.serial<=this.lastSerial)return;this.lastSerial=sample.serial;if(!this.shouldEncode(sample))return;this.pending={...sample,offeredAt:this.now()};this.pump();}
 pump(){if(this.active||!this.pending||this.closed||this.frames.length>=2)return;this.active=this.run().catch(()=>{if(!this.closed)this.failure=Error('MD-DISPLAY: motion encoder failed')}).finally(()=>{this.active=null;if(this.pending&&!this.closed&&!this.failure)this.pump()});}
 async run(){
  while(this.pending&&!this.closed&&!this.failure&&this.frames.length<2){
   const sample=this.pending;this.pending=null;const revision=this.revision,key=this.independent||this.key;this.key=false;const at=performance.timeOrigin+this.now();
   const encoded=await this.encoder.encode(sample.raw?{...sample.raw,motion:true}:sample.data_base64,this.rate.bitrate,key,this.codec);this.timing('motion_encode',at);
   if(['vaapi','x264','vp9','webcodecs'].includes(this.encoder.backend))this.timing('motion_backend_'+this.encoder.backend,performance.timeOrigin+this.now());
   if(this.closed||!this.valid())return;
   if(revision!==this.revision){this.key=true;continue;}
   if(this.frames.length>=2||this.frames.reduce((n,f)=>n+f.encoded.data_base64.length,encoded.data_base64.length)>1024*1024){this.frames=[];this.key=true;continue;}
   this.frames.push({...sample,encoded});
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
  if(oldest && age>(oldest.encoded.key?300:100) && this.source.sample()?.serial>oldest.serial){
   this.rate.feedback(4);this.invalidate();this.pump();return null;
  }
  const frame=this.frames.shift()??null;this.pump();return frame;
 }
 retireUnsent(){if(this.frames.length||this.active||this.pending)this.invalidate(false)}
 invalidate(reoffer=true){this.revision++;this.key=true;this.frames=[];this.pending=null;if(reoffer){this.lastSerial=-1;if(!this.closed)this.offer(this.source.sample())}else this.lastSerial=Math.max(this.lastSerial,this.source.sample()?.serial??-1);}
 async close(){if(this.closed)return;this.closed=true;this.off?.();this.invalidate();await this.active;}
}

// MD-DISPLAY-02/04: credit pressure, not a bandwidth estimator. Never exceed
// the negotiated ceiling; sustained backlog lowers motion bitrate only.
export class CreditBudget {
 constructor(ceiling,now=()=>performance.now()){this.ceiling=ceiling;this.bitrate=ceiling;this.floor=Math.min(ceiling,500000);this.now=now;this.changedAt=now();this.pressureAt=null;this.clearAt=null;}
 feedback(lag){
  const now=this.now();if(!Number.isSafeInteger(lag)||lag<0||lag>8)return false;
  if(lag>=3){this.clearAt=null;this.pressureAt??=now;if(now-this.pressureAt<500||now-this.changedAt<500)return false;this.pressureAt=now;return this.set(Math.max(this.floor,Math.round(this.bitrate*.8)),now)}
  this.pressureAt=null;if(lag<=1){this.clearAt??=now;if(now-this.clearAt>=2000&&now-this.changedAt>=2000){this.clearAt=now;return this.set(Math.min(this.ceiling,Math.round(this.bitrate*1.1)),now)}}else this.clearAt=null;return false;
 }
 set(value,now){if(value===this.bitrate)return false;this.bitrate=value;this.changedAt=now;return true;}
}
