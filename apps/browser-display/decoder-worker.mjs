// MP-08/MP-10/MP-11: one module fetch, nine independent bounded decoder chains.
const inWorker=typeof WorkerGlobalScope!=='undefined'&&globalThis instanceof WorkerGlobalScope;
if(inWorker){
 const channels=new Map();
 const retire=channel=>{const state=channels.get(channel);channels.delete(channel);if(state?.decoder?.state!=='closed')state?.decoder?.close();};
 self.onmessage=({data})=>{
  const {id,codec,key,sequence,width,height,bytes}=data,channel=data.channel??0;
  if(!Number.isSafeInteger(channel)||channel<0||channel>8){self.postMessage({id,error:'MD-DISPLAY: worker channel bound'});return}
  if(data.op==='reset'){retire(channel);return}
  let state=channels.get(channel);
  try{
   if(state?.pending!=null)throw Error('queue bound');
   if(!Number.isSafeInteger(id)||!bytes||bytes.byteLength>4*1024*1024||!['vp8','vp09.00.50.08','vp09.00.40.08','vp09.00.10.08','avc1.420033'].includes(codec))throw Error('codec bound');
   const format=`${codec}:${width}:${height}`;
   if(!state||state.format!==format){
    if(!key)throw Error('independent key required');
    retire(channel);state={pending:null,format};channels.set(channel,state);
    const current=state;
    const fail=()=>{if(channels.get(channel)!==current)return;const pending=current.pending;retire(channel);self.postMessage({id:pending,error:'MD-DISPLAY: worker decode failed'})};
    state.decoder=new VideoDecoder({output:frame=>{
     if(channels.get(channel)!==current||current.pending==null){frame.close();return}
     const pending=current.pending;current.pending=null;self.postMessage({id:pending,frame},[frame]);
    },error:fail});
    state.decoder.configure({codec,codedWidth:width,codedHeight:height,optimizeForLatency:true});
   }
   state.pending=id;
   state.decoder.decode(new EncodedVideoChunk({type:key?'key':'delta',timestamp:sequence*16667,data:bytes}));
  }catch{retire(channel);self.postMessage({id,error:'MD-DISPLAY: worker decode failed'})}
 };
}
export class WorkerVideoDecoder {
 constructor(WorkerClass=globalThis.Worker){
  this.jobs=new Map();this.channels=new Set();this.next=0;this.closed=false;
  this.worker=new WorkerClass(new URL('./decoder-worker.mjs',import.meta.url),{type:'module',name:'MD-DISPLAY decoder'});
  this.worker.onmessage=({data})=>{const p=this.jobs.get(data.id);if(!p){data.frame?.close();return}clearTimeout(p.timer);this.jobs.delete(data.id);this.channels.delete(p.channel);if(data.error)p.reject(Error(data.error));else p.resolve(data.frame)};
  this.worker.onerror=()=>this.close();
 }
 decode(frame,bytes,channel=0){
  if(this.closed||!Number.isSafeInteger(channel)||channel<0||channel>8||this.channels.has(channel)||this.jobs.size>=9)return Promise.reject(Error('MD-DISPLAY: worker decoder unavailable/busy'));
  return new Promise((resolve,reject)=>{
   const id=++this.next,timer=setTimeout(()=>{this.jobs.delete(id);this.channels.delete(channel);reject(Error('MD-DISPLAY: worker decode timeout'));this.close()},5000);
   this.jobs.set(id,{channel,resolve,reject,timer});this.channels.add(channel);
   try{this.worker.postMessage({id,channel,codec:frame.codec,key:frame.key,sequence:frame.sequence,width:frame.width,height:frame.height,bytes},[bytes.buffer])}catch(error){clearTimeout(timer);this.jobs.delete(id);this.channels.delete(channel);reject(error);this.close()}
  });
 }
 reset(channel){
  for(const [id,p]of this.jobs)if(p.channel===channel){clearTimeout(p.timer);this.jobs.delete(id);this.channels.delete(channel);p.reject(Error('MD-DISPLAY: worker decoder reset'))}
  if(!this.closed)this.worker.postMessage({op:'reset',channel});
 }
 close(){if(this.closed)return;this.closed=true;for(const p of this.jobs.values()){clearTimeout(p.timer);p.reject(Error('MD-DISPLAY: decoder worker closed'))}this.jobs.clear();this.channels.clear();this.worker.terminate()}
}
