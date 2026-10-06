// MD-DISPLAY-02/04: source-driven encoding, bounded latest work and recovery.
// Capture/encode never await a viewer credit. Dropped dependencies force a key.
export class MotionEncoder {
 constructor(source,encoder,{bitrate,codec,independent=false,stripes=false,valid=()=>true,shouldEncode=()=>true,timing=()=>{},now=()=>performance.now()}={}){
  Object.assign(this,{source,encoder,bitrate,codec,independent,stripes,valid,shouldEncode,timing,now});this.frames=[];this.pending=null;this.active=null;this.key=true;this.resetRows=new Set();this.rate=new CreditBudget(bitrate,now);this.revision=0;this.lastSerial=-1;this.closed=false;
  this.off=source.subscribe(sample=>this.offer(sample));this.offer(source.sample());
 }
 offer(sample){if(!sample||this.closed||!this.valid()||sample.serial<=this.lastSerial)return;this.lastSerial=sample.serial;if(!this.shouldEncode(sample))return;this.pending?.raw?.release?.();sample.raw?.retain?.();this.pending={...sample,offeredAt:this.now()};this.pump();}
 pump(){if(this.active||!this.pending||this.closed||this.frames.length>=2)return;this.active=this.run().catch(()=>{if(!this.closed)this.failure=Error('MD-DISPLAY: motion encoder failed')}).finally(()=>{this.active=null;if(this.pending&&!this.closed&&!this.failure)this.pump()});}
 async run(){
  while(this.pending&&!this.closed&&!this.failure&&this.frames.length<2){
   const sample=this.pending;this.pending=null;const rowMode=Boolean(this.stripes&&sample.raw);if(this.rowMode!==rowMode)this.key=true;this.rowMode=rowMode;const revision=this.revision,key=this.independent||this.key;this.key=false;const reset=key?true:[...this.resetRows];this.resetRows.clear();const at=performance.timeOrigin+this.now();
   let encoded;try{encoded=this.stripes&&sample.raw?await this.encoder.encodeStripes({...sample.raw,motion:true},this.rate.bitrate,reset,this.codec):await this.encoder.encode(sample.raw?{...sample.raw,motion:true}:sample.data_base64,this.rate.bitrate,key,this.codec)}finally{sample.raw?.release?.()}this.timing('motion_encode',at);
   if(['vaapi','x264','openh264','vp8','vp9','webcodecs'].includes(this.encoder.backend))this.timing('motion_backend_'+this.encoder.backend,performance.timeOrigin+this.now());
   if([1,2,4].includes(this.encoder.workers))this.timing('motion_workers_'+this.encoder.workers,performance.timeOrigin+this.now());
   if(encoded.packet)this.timing('motion_packet_native',performance.timeOrigin+this.now());
   if(['libyuv','swscale'].includes(this.encoder.converter))this.timing('motion_converter_'+this.encoder.converter,performance.timeOrigin+this.now());
   if(this.closed||!this.valid()){this.encoder.discard?.(encoded);return;}
   if(encoded.stripes?.length===0)continue;
   if(revision!==this.revision){this.encoder.discard?.(encoded);if(encoded.stripes&&!this.key)for(const row of encoded.stripes)this.resetRows.add(row.row);else this.key=true;continue;}
   if(this.frames.length>=2||this.frames.reduce((n,f)=>n+JSON.stringify(f.encoded).length+(f.encoded.packet?.length??0),(JSON.stringify(encoded).length+(encoded.packet?.length??0)))>1024*1024){this.encoder.discard?.(encoded);this.invalidateRows();continue;}
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
  if(oldest && age>((oldest.encoded.key||oldest.encoded.stripes?.every(r=>r.key))?300:100) && this.source.sample()?.serial>oldest.serial){
   this.rate.feedback(8);this.invalidateRows();this.pump();return null;
  }
  const frame=this.frames.shift()??null;this.pump();return frame;
 }
 retireUnsent(){if(this.frames.length||this.active||this.pending)this.invalidateRows(false)}
 invalidateRows(reoffer=true){if(!this.rowMode||this.frames.some(frame=>!frame.encoded.stripes))return this.invalidate(reoffer);for(const frame of this.frames)for(const row of frame.encoded.stripes)this.resetRows.add(row.row);this.revision++;for(const frame of this.frames)this.encoder.discard?.(frame.encoded);this.frames=[];this.pending?.raw?.release?.();this.pending=null;if(reoffer){this.lastSerial=-1;if(!this.closed)this.offer(this.source.sample())}else this.lastSerial=Math.max(this.lastSerial,this.source.sample()?.serial??-1);}
 invalidate(reoffer=true){this.revision++;this.key=true;for(const frame of this.frames)this.encoder.discard?.(frame.encoded);this.frames=[];this.pending?.raw?.release?.();this.pending=null;if(reoffer){this.lastSerial=-1;if(!this.closed)this.offer(this.source.sample())}else this.lastSerial=Math.max(this.lastSerial,this.source.sample()?.serial??-1);}
 async close(){if(this.closed)return;this.closed=true;this.off?.();this.invalidate();await this.active;}
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
