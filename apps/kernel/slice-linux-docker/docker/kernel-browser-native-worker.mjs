// MP-08/MP-10/MP-11: control-only bridge to the kernel-owned native worker.
// Frame pixels and compressed packets never pass through the motion bridge.
export class NativeWorkerControl {
 constructor(child,timing){this.child=child;this.timing=timing;this.pending=new Map();this.sequence=0;}
 request(operation,values){
  if(this.pending.size>=16||this.closed)throw Error('MP-11: native control unavailable');
  const id=++this.sequence;
  return new Promise((resolve,reject)=>{
   const timer=setTimeout(()=>{this.pending.delete(id);reject(Error('MP-10: native control timeout'));},10000);
   this.pending.set(id,{resolve:value=>{clearTimeout(timer);resolve(value)},reject:error=>{clearTimeout(timer);reject(error)}});
   this.child.stdin.write(JSON.stringify({[operation]:{id,...values}})+'\n',error=>{if(error)this.close()});
  });
 }
 validate(header){if(!Number.isSafeInteger(header.reply)||!this.pending.has(header.reply)||header.length>8*1024*1024)throw Error('MP-11: native reply binding');}
 receive(header,bytes){
  const waiter=this.pending.get(header.reply);this.pending.delete(header.reply);
  try{
   const value=JSON.parse(bytes);
   if(value.error)throw Error('MP-11: native worker refused');
   if(Array.isArray(value.timings)&&value.timings.length<=64){
    const stages=new Set(['native_codec','codec_packetize','native_exact_prepare','native_shift_prepare','native_cpu_mask_guard','native_cpu_compare','native_cpu_convert','native_cpu_encode','native_cpu_output_guard','native_cpu_reference_copy']);
    const spans=value.timings.filter(s=>Array.isArray(s)&&s.length===3&&stages.has(s[0])&&Number.isFinite(s[1])&&Number.isFinite(s[2])&&s[2]>=s[1]);
    if(this.timing?.batch)this.timing.batch(spans);else for(const span of spans)this.timing?.(...span);
   }
   delete value.timings;waiter.resolve(value);
  }catch(error){waiter.reject(error);this.close()}
 }
 retire(encoder){if(!this.closed&&!this.child.stdin.destroyed)this.child.stdin.write(JSON.stringify({retire:encoder})+'\n');}
 delivered(encoder,revision){if(!this.closed&&!this.child.stdin.destroyed)this.child.stdin.write(JSON.stringify({delivered:encoder,revision})+'\n');}
 // MP-08/MP-10: lossless scroll commits skip capture admission (overlay only).
 commit(encoder,serial,admit=true){if(!this.closed&&!this.child.stdin.destroyed)this.child.stdin.write(JSON.stringify({commit:encoder,serial,...(admit?{}:{admit:false})})+'\n');}
 close(){if(this.closed)return;this.closed=true;for(const p of this.pending.values())p.reject(Error('MP-11: native worker closed'));this.pending.clear();}
}
