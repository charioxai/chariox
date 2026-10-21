// MP-08/MP-10/MP-11: control-only bridge to the kernel-owned native worker.
// Frame pixels and compressed packets never pass through the motion bridge.
import path from 'node:path';
import {unlinkSync} from 'node:fs';
import {randomBytes} from 'node:crypto';
// MP-08/MP-10/MP-11: browser and desktop workers share the kernel reader's
// private packet directory; the worker derives it from its raster pool parent.
export function nativeWorkerResource(environment=process.env){
 const executable=environment.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER,root=environment.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT;
 return path.isAbsolute(executable??'')&&path.isAbsolute(root??'')?{executable,root}:null;
}
// L3: an auto-encoded packet nobody published (the kernel never names it).
export function discardPacket(packet,root=process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT){
 if(!root||!/^[a-f0-9]{32}\.json$/.test(packet?.name??''))return;
 try{unlinkSync(path.join(root,packet.name))}catch(error){if(error.code!=='ENOENT')throw error}
}
// L3: live workers; an event that can change any leased frame's verdict
// retires every record at once (a record only feeds auto encodes and leases).
const live=new Set();
export function retireNativeStates(){for(const worker of live)worker.retireState();}
const notSent=(message='MP-11: native control unavailable')=>Object.assign(Error(message),{dispatched:false});
// MP-08/MP-10/MP-11: two full native RGB rasters (PNG + WebP repair),
// base64 and metadata at the admitted 2560x1600 geometry fit within32 MiB.
// Only an outstanding exact request may use this private IPC ceiling.
export const nativeExactReplyLimit=32*1024*1024;
export class NativeWorkerControl {
 // A record armed right after a retire is retired again by the same storm
 // (a page that mutates every frame): auto-encodes and grants would only be
 // wasted, so the next arm waits until retires have been quiet this long.
 static retireQuietMs=250;
 retiredAt=-Infinity;
 constructor(child,timing){this.child=child;this.timing=timing;this.pending=new Map();this.sequence=0;this.notifications=[];this.flushScheduled=null;this.autos=new Map();this.generation=0;this.auto=null;
  // L3 stage 2b: fd 3 is the synchronous retire pipe; the token names this worker's frame channel.
  live.add(this);this.retires=child.stdio?.[3]??null;
  // A peer that closed its end reports EPIPE as an event, after the write: fence the capture, never let Node throw.
  this.retires?.on?.('error',()=>this.fence());this.token=this.retires&&process.env.CHARIOX_BROWSER_DISPLAY_FRAMES?randomBytes(16).toString('hex'):null;this.lease=null;}
 request(operation,values){
  if(this.pending.size>=16||this.closed)throw notSent();
  const id=++this.sequence;
  return new Promise((resolve,reject)=>{
   const timer=setTimeout(()=>{this.pending.delete(id);reject(Error('MP-10: native control timeout'));},10000);
   this.pending.set(id,{limit:operation==='exact'?nativeExactReplyLimit:8*1024*1024,resolve:value=>{clearTimeout(timer);resolve(value)},reject:error=>{clearTimeout(timer);reject(error)}});
   // A command that was never written is a definite refusal; any later loss
   // (timeout, closed worker, bad reply) leaves its dispatch uncertain.
   if(!this.notify({[operation]:{id,...values}},true)){this.pending.delete(id);clearTimeout(timer);reject(notSent());this.close();}
  });
 }
 validate(header){const pending=this.pending.get(header.reply);if(!Number.isSafeInteger(header.reply)||!pending||!Number.isSafeInteger(header.length)||header.length<1||header.length>pending.limit)throw Error('MP-11: native reply binding');}
 receive(header,bytes){
  const waiter=this.pending.get(header.reply);this.pending.delete(header.reply);
  try{
   const value=JSON.parse(bytes);
   // MP-10 (#933 review 7): a full worker queue refuses one request softly.
   if(value.busy===true&&Object.keys(value).length===1){waiter.reject(Object.assign(Error('MD-DISPLAY: native worker busy'),{busy:true}));return;}
   if(value.error){waiter.reject(notSent('MP-11: native worker refused'));return;}
   this.spans(value);waiter.resolve(value);
  }catch(error){waiter.reject(error);this.close()}
 }
 spans(value){
  if(Array.isArray(value.timings)&&value.timings.length<=64){
   const stages=new Set(['native_codec_queue','native_codec','codec_packetize','native_exact_prepare','native_shift_prepare','native_cpu_mask_guard','native_cpu_compare','native_cpu_convert','native_cpu_encode','native_cpu_output_guard','native_cpu_reference_copy']);
   const spans=value.timings.filter(s=>Array.isArray(s)&&s.length===3&&stages.has(s[0])&&Number.isFinite(s[1])&&Number.isFinite(s[2])&&s[2]>=s[1]);
   if(this.timing?.batch)this.timing.batch(spans);else for(const span of spans)this.timing?.(...span);
  }
  delete value.timings;
 }
 // L3 (AUDIT2 §4): one state record per encoder session. Its generation
 // changes only on retire, so renewals and rate changes keep the chain.
 arm(record){
  if(this.closed||performance.now()-this.retiredAt<NativeWorkerControl.retireQuietMs)return null;
  if(this.auto?.encoder!==record.encoder)this.auto={generation:++this.generation};
  Object.assign(this.auto,record);
  this.notify({state:{...this.auto}},true);return this.auto;
 }
 // Synchronous from every retiring event, before any other work on it.
 // MP-11: the retire pipe is read by the codec thread before every leased
 // send, so no frame read back after this write can leave the worker.
 retireState(encoder){
  if(this.auto&&encoder!==undefined&&this.auto.encoder!==encoder)return;
  this.retiredAt=performance.now();
  if(!this.auto)return;
  const generation=this.auto.generation;this.auto=null;this.lease=null;
  const pipe=this.retires;
  try{if(!pipe||pipe.destroyed)throw Error();pipe.write(`${generation}\n`);if(pipe.writableLength)throw Error();}
  catch{this.fence();}
 }
 // A retirement that cannot be delivered leaves nothing to trust: close and kill the worker.
 fence(){this.close();if(this.fenced)return;this.fenced=true;this.child.kill('SIGKILL');}
 // L3: grant the worker the frames after the one Node is publishing now;
 // the kernel binds the same lease from this frame's reply.
 grant(lease){
  if(this.closed||!this.token||this.lease||this.auto?.generation!==lease.generation)return null;
  this.lease={generation:lease.generation};this.notify({lease},true);
  return {token:this.token,generation:lease.generation};
 }
 // The worker ended a lease (or refused a grant): that generation is spent.
 ended(generation){
  this.retiredAt=performance.now();
  if(this.auto?.generation===generation)this.auto=null;
  if(this.lease?.generation===generation)this.lease=null;
 }
 // The frame header or its auto reply may arrive first; each entry is
 // taken by its encoder or dropped with its raster, never both.
 autoEntry(serial){
  let entry=this.autos.get(serial);
  if(!entry){
   if(this.closed||this.autos.size>=32)throw Error('MP-11: native auto bound');
   entry={};entry.promise=new Promise((resolve,reject)=>Object.assign(entry,{resolve,reject}));entry.promise.catch(()=>{});this.autos.set(serial,entry);
  }
  return entry;
 }
 autoReceive(header,bytes){
  const value=JSON.parse(bytes);
  if(this.closed){discardPacket(value.packet);return;}
  let entry;try{entry=this.autoEntry(header.auto)}catch(error){discardPacket(value.packet);throw error}
  this.spans(value);entry.done=value;
  if(entry.dropped){this.autos.delete(header.auto);discardPacket(value.packet);return;}
  entry.resolve({generation:header.generation,value});if(entry.taken)this.autos.delete(header.auto);
 }
 takeAuto(serial){
  const entry=this.autos.get(serial);if(!entry||entry.dropped||entry.taken)return null;
  entry.taken=true;if(entry.done)this.autos.delete(serial);return entry.promise;
 }
 dropAuto(serial){
  const entry=this.autos.get(serial);if(!entry||entry.taken)return;
  if(entry.done){this.autos.delete(serial);discardPacket(entry.done.packet);}else entry.dropped=true;
 }
 // MP-08/MP-10/MP-11: same ordered JSON-lines contract, one bounded write
 // for a frame's notifications. Requests and physical input flush immediately.
 notify(command,immediate=false){
  if(this.closed||this.child.stdin.destroyed)return false;
  this.notifications.push(JSON.stringify(command)+'\n');
  if(immediate||this.notifications.length>=64)return this.flush();
  if(this.flushScheduled===null)this.flushScheduled=setImmediate(()=>this.flush());
  return true;
 }
 flush(){
  if(this.flushScheduled!==null){clearImmediate(this.flushScheduled);this.flushScheduled=null;}
  if(this.closed||this.child.stdin.destroyed){this.close();return false;}
  if(!this.notifications.length)return true;
  const bytes=this.notifications.join('');this.notifications=[];
  try{this.child.stdin.write(bytes,error=>{if(error)this.close()});return true;}catch{this.close();return false;}
 }
 retire(encoder){this.notify({retire:encoder});}
 // MP-08/MP-10/MP-11: diagnostic-only, ordered with native controls.
 rejected(reason){
  if(!['stale_document','epoch','region_revision','serial_skipped','shift_hold'].includes(reason))throw Error('MP-11: rejection reason');
  // A document fence immediately closes this bridge: flush its diagnostic
  // before cleanup discards the otherwise batched control notifications.
  this.notify({reject:reason},true);
 }
 delivered(encoder,revision){this.notify({delivered:encoder,revision});}
 commit(encoder,serial,admit=true){this.notify({commit:encoder,serial,...(admit?{}:{admit:false})});}
 close(){if(this.closed)return;this.closed=true;live.delete(this);if(this.flushScheduled!==null)clearImmediate(this.flushScheduled);this.flushScheduled=null;this.notifications=[];for(const p of this.pending.values())p.reject(Error('MP-11: native worker closed'));this.pending.clear();for(const entry of this.autos.values()){if(entry.done&&!entry.taken)discardPacket(entry.done.packet);entry.reject(Error('MP-11: native worker closed'));}this.autos.clear();this.auto=null;}
}
