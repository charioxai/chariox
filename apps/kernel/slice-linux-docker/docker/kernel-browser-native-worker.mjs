// MP-08/MP-10/MP-11: control-only bridge to the kernel-owned native worker.
// Frame pixels and compressed packets never pass through the motion bridge.
const notSent=(message='MP-11: native control unavailable')=>Object.assign(Error(message),{dispatched:false});
export class NativeWorkerControl {
 constructor(child,timing){this.child=child;this.timing=timing;this.pending=new Map();this.sequence=0;this.notifications=[];this.flushScheduled=null;}
 request(operation,values){
  if(this.pending.size>=16||this.closed)throw notSent();
  const id=++this.sequence;
  return new Promise((resolve,reject)=>{
   const timer=setTimeout(()=>{this.pending.delete(id);reject(Error('MP-10: native control timeout'));},10000);
   this.pending.set(id,{resolve:value=>{clearTimeout(timer);resolve(value)},reject:error=>{clearTimeout(timer);reject(error)}});
   // A command that was never written is a definite refusal; any later loss
   // (timeout, closed worker, bad reply) leaves its dispatch uncertain.
   if(!this.notify({[operation]:{id,...values}},true)){this.pending.delete(id);clearTimeout(timer);reject(notSent());this.close();}
  });
 }
 validate(header){if(!Number.isSafeInteger(header.reply)||!this.pending.has(header.reply)||header.length>8*1024*1024)throw Error('MP-11: native reply binding');}
 receive(header,bytes){
  const waiter=this.pending.get(header.reply);this.pending.delete(header.reply);
  try{
   const value=JSON.parse(bytes);
   // MP-10 (#933 review 7): a full worker queue refuses one request softly.
   if(value.busy===true&&Object.keys(value).length===1){waiter.reject(Object.assign(Error('MD-DISPLAY: native worker busy'),{busy:true}));return;}
   if(value.error){waiter.reject(notSent('MP-11: native worker refused'));return;}
   if(Array.isArray(value.timings)&&value.timings.length<=64){
    const stages=new Set(['native_codec','codec_packetize','native_exact_prepare','native_shift_prepare','native_cpu_mask_guard','native_cpu_compare','native_cpu_convert','native_cpu_encode','native_cpu_output_guard','native_cpu_reference_copy']);
    const spans=value.timings.filter(s=>Array.isArray(s)&&s.length===3&&stages.has(s[0])&&Number.isFinite(s[1])&&Number.isFinite(s[2])&&s[2]>=s[1]);
    if(this.timing?.batch)this.timing.batch(spans);else for(const span of spans)this.timing?.(...span);
   }
   delete value.timings;waiter.resolve(value);
  }catch(error){waiter.reject(error);this.close()}
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
 delivered(encoder,revision){this.notify({delivered:encoder,revision});}
 commit(encoder,serial,admit=true){this.notify({commit:encoder,serial,...(admit?{}:{admit:false})});}
 close(){if(this.closed)return;this.closed=true;if(this.flushScheduled!==null)clearImmediate(this.flushScheduled);this.flushScheduled=null;this.notifications=[];for(const p of this.pending.values())p.reject(Error('MP-11: native worker closed'));this.pending.clear();}
}
