// MD-DISPLAY-04: one bounded decoder job; dependency order stays in presenter.
const inWorker=typeof WorkerGlobalScope!=='undefined'&&globalThis instanceof WorkerGlobalScope;
if(inWorker){
 let decoder,pending;
 const fail=()=>{const id=pending;pending=null;self.postMessage({id,error:'MD-DISPLAY: worker decode failed'})};
 self.onmessage=({data})=>{
  const {id,codec,key,sequence,width,height,bytes}=data;
  try{
   if(pending!==undefined&&pending!==null)throw Error('queue bound');
   if(!Number.isSafeInteger(id)||bytes.byteLength>4*1024*1024||!['vp8','vp09.00.50.08','vp09.00.40.08','vp09.00.10.08','avc1.420033'].includes(codec))throw Error('codec bound');
   pending=id;
   if(!decoder||key){
    if(decoder?.state!=='closed')decoder?.close();decoder=new VideoDecoder({output:frame=>{if(pending===null||pending===undefined){frame.close();return}const id=pending;pending=null;self.postMessage({id,frame},[frame])},error:fail});
    decoder.configure({codec,codedWidth:width,codedHeight:height,optimizeForLatency:true});
   }
   decoder.decode(new EncodedVideoChunk({type:key?'key':'delta',timestamp:sequence*16667,data:bytes}));
  }catch{fail()}
 };
}
export class WorkerVideoDecoder {
 constructor(WorkerClass=globalThis.Worker){
  this.pending=null;this.next=0;this.closed=false;
  this.worker=new WorkerClass(new URL('./decoder-worker.mjs',import.meta.url),{type:'module',name:'MD-DISPLAY decoder'});
  this.worker.onmessage=({data})=>{const p=this.pending;if(!p||data.id!==p.id){data.frame?.close();return}clearTimeout(p.timer);this.pending=null;if(data.error)p.reject(Error(data.error));else p.resolve(data.frame)};
  this.worker.onerror=()=>{const p=this.pending;this.pending=null;if(p){clearTimeout(p.timer);p.reject(Error('MD-DISPLAY: decoder worker unavailable'))}this.close()};
 }
 decode(frame,bytes){
  if(this.closed||this.pending)return Promise.reject(Error('MD-DISPLAY: worker decoder unavailable/busy'));
  return new Promise((resolve,reject)=>{
   const id=++this.next,timer=setTimeout(()=>{this.pending=null;reject(Error('MD-DISPLAY: worker decode timeout'));this.close()},5000);
   this.pending={id,resolve,reject,timer};
   try{this.worker.postMessage({id,codec:frame.codec,key:frame.key,sequence:frame.sequence,width:frame.width,height:frame.height,bytes},[bytes.buffer])}catch(error){clearTimeout(timer);this.pending=null;reject(error);this.close()}
  });
 }
 close(){if(this.closed)return;this.closed=true;const p=this.pending;this.pending=null;if(p){clearTimeout(p.timer);p.reject(Error('MD-DISPLAY: decoder worker closed'))}this.worker.terminate()}
}
