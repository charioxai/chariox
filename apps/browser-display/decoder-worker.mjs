// MP-08/MP-10/MP-11: one worker, eight repair chains and a <=3 video queue.
const inWorker=typeof WorkerGlobalScope!=='undefined'&&globalThis instanceof WorkerGlobalScope;
if(inWorker){
 const channels=new Map();let writer=null,writing=Promise.resolve(),lastTimestamp=0;
 const now=()=>globalThis.performance?performance.timeOrigin+performance.now():Date.now();
 const retire=channel=>{const state=channels.get(channel);channels.delete(channel);if(state?.decoder?.state!=='closed')state?.decoder?.close();return state;};
 const fail=channel=>{const state=retire(channel);for(const job of state?.pending?.values()??[])self.postMessage({id:job.id,error:'MD-DISPLAY: worker decode failed'});};
 self.onmessage=({data})=>{
  if(data.op==='sink'){writer=data.writable.getWriter();return;}
  const {id,codec,key,sequence,width,height,bytes}=data,channel=data.channel??0;
  if(!Number.isSafeInteger(channel)||channel<0||channel>8){self.postMessage({id,error:'MD-DISPLAY: worker channel bound'});return;}
  if(data.op==='reset'){fail(channel);return;}
  let state=channels.get(channel);
  try{
   if((state?.pending.size??0)>=(channel===8?3:1))throw Error('queue bound');
   if(!Number.isSafeInteger(id)||!Number.isSafeInteger(sequence)||!bytes||bytes.byteLength<1||bytes.byteLength>4*1024*1024||!['vp8','vp09.00.50.08','vp09.00.40.08','vp09.00.10.08','avc1.420033'].includes(codec))throw Error('codec bound');
   const format=`${codec}:${width}:${height}`;
   if(!state||state.format!==format){
    if(!key)throw Error('independent key required');
    fail(channel);state={pending:new Map(),format};channels.set(channel,state);
    const current=state;
    state.decoder=new VideoDecoder({output:frame=>{
     const timestamp=frame.timestamp,job=current.pending.get(timestamp);
     if(channels.get(channel)!==current||!job){frame.close();return;}
     const decodedAt=now();
     // Geometry is checked before a frame can enter the native track. Repair
     // outputs are validated by StripePresenter before its atomic canvas commit.
     if(job.sink&&!((frame.displayWidth===job.width&&frame.displayHeight===job.height)||(frame.displayWidth===job.cssWidth&&frame.displayHeight===job.cssHeight)||(job.cssWidth===1920&&job.cssHeight===1080&&frame.displayWidth===1280&&frame.displayHeight===720))){frame.close();fail(channel);return;}
     const deliver=async()=>{
      if(channels.get(channel)!==current){frame.close();return;}
      let image=frame,writtenAt;
      try{
       if(job.sink&&writer){image=frame.clone();await writer.ready;await writer.write(frame);writtenAt=now();}
       if(channels.get(channel)!==current){image.close();return;}
       current.pending.delete(timestamp);
       self.postMessage({id:job.id,frame:image,decodedAt,writtenAt},[image]);
      }catch{image.close();fail(channel);}
      finally{if(image!==frame)frame.close();}
     };
     // Track writes remain source-ordered even when writable backpressure waits.
     if(job.sink&&writer){writing=writing.then(deliver,deliver);}else void deliver();
    },error:()=>{if(channels.get(channel)===current){if(current.pending.size===0)self.postMessage({id:null,error:'MD-DISPLAY: worker decode failed'});fail(channel);}}});
    state.decoder.configure({codec,codedWidth:width,codedHeight:height,optimizeForLatency:true});
   }
   // MP-08/MP-10/MP-11: sparse damage/repairs make sequence a dependency
   // cursor, never a60Hz media clock. Native live tracks must see the actual
   // arrival cadence, including idle gaps. Break same-tick ties by1us.
   const timestamp=Math.max(Math.round((globalThis.performance?performance.now():Date.now())*1000),lastTimestamp+1);lastTimestamp=timestamp;
   state.pending.set(timestamp,data);
   state.decoder.decode(new EncodedVideoChunk({type:key?'key':'delta',timestamp,data:bytes}));
  }catch{fail(channel);self.postMessage({id,error:'MD-DISPLAY: worker decode failed'});}
 };
}
export class WorkerVideoDecoder {
 constructor(WorkerClass=globalThis.Worker){
  this.jobs=new Map();this.channels=new Map();this.next=0;this.closed=false;
  this.worker=new WorkerClass(new URL('./decoder-worker.mjs',import.meta.url),{type:'module',name:'MD-DISPLAY decoder'});
  this.worker.onmessage=({data})=>{const p=this.jobs.get(data.id);if(!p){data.frame?.close();return}clearTimeout(p.timer);this.jobs.delete(data.id);this.channels.set(p.channel,(this.channels.get(p.channel)??1)-1);if(data.error)p.reject(Error(data.error));else p.resolve(p.sink?{image:data.frame,decodedAt:data.decodedAt,writtenAt:data.writtenAt}:data.frame)};
  this.worker.onerror=()=>this.close();
 }
 setSink(writable){this.worker.postMessage({op:'sink',writable},[writable]);}
 decode(frame,bytes,channel=0,sink=false){
  if(this.closed||!Number.isSafeInteger(channel)||channel<0||channel>8||(this.channels.get(channel)??0)>=(channel===8?3:1)||this.jobs.size>=11)return Promise.reject(Error('MD-DISPLAY: worker decoder unavailable/busy'));
  return new Promise((resolve,reject)=>{
   const id=++this.next,timer=setTimeout(()=>{this.jobs.delete(id);this.channels.set(channel,(this.channels.get(channel)??1)-1);reject(Error('MD-DISPLAY: worker decode timeout'));this.close()},5000);
   this.jobs.set(id,{channel,sink,resolve,reject,timer});this.channels.set(channel,(this.channels.get(channel)??0)+1);
   try{this.worker.postMessage({id,channel,codec:frame.codec,key:frame.key,sequence:frame.sequence,width:frame.width,height:frame.height,cssWidth:frame.css_width,cssHeight:frame.css_height,sink,bytes},[bytes.buffer])}catch(error){clearTimeout(timer);this.jobs.delete(id);this.channels.set(channel,(this.channels.get(channel)??1)-1);reject(error);this.close()}
  });
 }
 reset(channel){
  for(const [id,p]of this.jobs)if(p.channel===channel){clearTimeout(p.timer);this.jobs.delete(id);this.channels.delete(channel);p.reject(Error('MD-DISPLAY: worker decoder reset'))}
  if(!this.closed)this.worker.postMessage({op:'reset',channel});
 }
 close(){if(this.closed)return;this.closed=true;for(const p of this.jobs.values()){clearTimeout(p.timer);p.reject(Error('MD-DISPLAY: decoder worker closed'))}this.jobs.clear();this.channels.clear();this.worker.terminate()}
}
