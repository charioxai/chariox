// MD-DISPLAY-02/04: private isolated-world encoder; source scripts cannot see it.
import {PortableEncoder} from './kernel-browser-display.mjs';
import {randomUUID} from 'node:crypto';
function codecRuntime() {
 let encoder,pending,configuration,selected,busy=false;
 const finish=(error,value)=>{const p=pending;pending=null;if(!p)return;clearTimeout(p.timer);error?p.reject(error):p.resolve(value)};
 const base64=bytes=>{let text='';for(let i=0;i<bytes.length;i+=8192)text+=String.fromCharCode(...bytes.subarray(i,i+8192));return btoa(text)};
 const codec={
  close(){encoder?.close();encoder=null;finish(Error('closed'));configuration=null;selected=null},
  async encode({image,bitrate,codec,reset}){
   if(busy)throw Error('encoder busy');busy=true;
   const text=atob(image),bytes=new Uint8Array(text.length);for(let i=0;i<text.length;i++)bytes[i]=text.charCodeAt(i);
   let bitmap;
   try{
    bitmap=await createImageBitmap(new Blob([bytes]));
    const requestSignature=JSON.stringify([codec,bitmap.width,bitmap.height,bitrate]);
    let config=selected?.signature===requestSignature?selected.config:null;
    if(!config){config={codec,width:bitmap.width,height:bitmap.height,framerate:60,bitrate:Math.floor(bitrate*.45),bitrateMode:'variable',latencyMode:'realtime',hardwareAcceleration:'prefer-hardware',...(codec.startsWith('avc')?{avc:{format:'annexb'}}:{})};
    if(!(await VideoEncoder.isConfigSupported(config)).supported){config.hardwareAcceleration='prefer-software';if(!(await VideoEncoder.isConfigSupported(config)).supported)throw Error('codec unavailable');}
     selected={signature:requestSignature,config};
    }
    const signature=JSON.stringify(config);
    if(!encoder||configuration!==signature){
     encoder?.close();encoder=new VideoEncoder({output:chunk=>{try{if(chunk.byteLength>3*1024*1024)throw Error('output bound');const value=new Uint8Array(chunk.byteLength);chunk.copyTo(value);finish(null,{data_base64:base64(value),key:chunk.type==='key'})}catch(error){finish(error)}},error:()=>finish(Error('encode failed'))});
     encoder.configure(config);configuration=signature;
    }
    return await new Promise((resolve,reject)=>{
     pending={resolve,reject,timer:setTimeout(()=>finish(Error('encode timeout')),5000)};const frame=new VideoFrame(bitmap,{timestamp:Math.round(performance.now()*1000)});
     try{encoder.encode(frame,{keyFrame:reset})}catch(error){finish(error)}finally{frame.close()}
    });
   }finally{bitmap?.close();busy=false}
  }
 };return codec;
}
const install=`(() => {
 const createCodec=${codecRuntime.toString()};
 if(typeof Worker==='undefined'){globalThis.charioxDisplayCodec=createCodec();return;}
 const code='const codec=('+createCodec.toString()+')();self.onmessage=async({data})=>{try{self.postMessage({id:data.id,result:await codec.encode(data.value)})}catch{self.postMessage({id:data.id,error:true})}};';
 let worker,pending,id=0;
 const url=URL.createObjectURL(new Blob([code],{type:'text/javascript'}));
 try{worker=new Worker(url)}finally{URL.revokeObjectURL(url)}
 const fail=()=>{const p=pending;pending=null;if(p){clearTimeout(p.timer);p.reject(Error('isolated worker failed'))}};
 worker.onerror=fail;
 worker.onmessage=({data})=>{const p=pending;if(!p||p.id!==data.id)return;pending=null;clearTimeout(p.timer);data.error?p.reject(Error('isolated worker failed')):p.resolve(data.result)};
 globalThis.charioxDisplayCodec={
  encode(value){if(pending)return Promise.reject(Error('encoder busy'));return new Promise((resolve,reject)=>{const job=++id,timer=setTimeout(fail,5000);pending={id:job,timer,resolve,reject};try{worker.postMessage({id:job,value})}catch{fail()}})},
  close(){fail();worker.terminate()}
 };
})()`;
export class BrowserEncoder {
 constructor(browser,targetId,fallback=new PortableEncoder()){this.worldName='chariox-kernel-display-codec-'+randomUUID();this.browser=browser;this.targetId=targetId;this.context=null;this.fallback=fallback;this.fallbackOnly=false;this.closed=false;this.busy=false;}
 get nativeRevision(){return this.fallback.nativeRevision;}
 get nativeDeliveredRevision(){return this.fallback.nativeDeliveredRevision;}
 get nativeSession(){return this.fallback.nativeSession;}
 get backend(){return this.fallback.backend ?? 'webcodecs';}
 get hardwareFallback(){return this.fallback.hardwareFallback;}
 get converter(){return this.fallback.converter;}
 get workers(){return this.fallback.workers;}
 set timing(value){this.fallback.timing=value;}
 discard(encoded){this.fallback.discard?.(encoded)}
 adoptPacket(packet){this.fallback.adoptPacket(packet)}
 handedOff(encoded){this.fallback.handedOff?.(encoded)}
 retire(){this.fallback.retire?.()}
 async encodeStripes(...args){return this.fallback.encodeStripes(...args)}
 async contextFor(){
  const {connection,sessionId}=await this.browser.resolvePageTarget(this.targetId);
  const {frameTree}=await connection.send('Page.getFrameTree',{},sessionId);
  const frame=frameTree?.frame;
  if(!frame?.loaderId)throw Error('MD-DISPLAY: encoder source unavailable');
  if(this.context?.document===frame.loaderId)return this.context;
  await this.closeContext();
  const {executionContextId}=await connection.send('Page.createIsolatedWorld',{frameId:frame.id,worldName:this.worldName,grantUniveralAccess:false},sessionId);
  this.context={connection,sessionId,id:executionContextId,document:frame.loaderId};
  const result=await connection.send('Runtime.evaluate',{expression:install,contextId:executionContextId,returnByValue:true},sessionId);
  if(result.exceptionDetails)throw Error('MD-DISPLAY: browser encoder unavailable');
  return this.context;
 }
 async encode(image,bitrate,reset=false,codec='vp09.00.10.08',regions=[]){
  if(this.closed||this.busy)throw Error('MD-DISPLAY: browser encoder closed/busy');
  this.busy=true;
  try{
  if(typeof image==='object'||regions.length||process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE==='1')return await this.fallback.encode(image,bitrate,reset,codec,regions);
  if(this.fallbackOnly)return await this.fallback.encode(image,bitrate,reset,codec);
  try{
   const c=await this.contextFor();
   if(this.closed)throw Error('MD-DISPLAY: browser encoder closed');
   const reply=await c.connection.send('Runtime.callFunctionOn',{executionContextId:c.id,functionDeclaration:'function(value){return globalThis.charioxDisplayCodec.encode(value)}',arguments:[{value:{image,bitrate,codec,reset}}],awaitPromise:true,returnByValue:true},c.sessionId);
   const result=reply.result?.value;
   if(this.closed||reply.exceptionDetails||typeof result?.data_base64!=='string'||result.data_base64.length>4*1024*1024||typeof result.key!=='boolean')throw Error('MD-DISPLAY: browser codec failed');
   return result;
  }catch{
   // Switching backend must start an independent frame, never reuse its delta base.
   await this.closeContext();this.fallbackOnly=true;
   if(this.closed)throw Error('MD-DISPLAY: browser encoder closed');
   return await this.fallback.encode(image,bitrate,true,codec);
  }
  }finally{this.busy=false}
 }
 async closeContext(){const c=this.context;this.context=null;if(c)await c.connection.send('Runtime.callFunctionOn',{executionContextId:c.id,functionDeclaration:'function(){globalThis.charioxDisplayCodec?.close()}',returnByValue:true},c.sessionId).catch(()=>{});}
 async close(){this.closed=true;await this.closeContext();await this.fallback.close();}
}
