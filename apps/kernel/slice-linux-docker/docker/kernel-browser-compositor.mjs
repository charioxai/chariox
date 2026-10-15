import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
// MD-DISPLAY-02/04: private latest-only compositor source, empty-policy only.
// A document-bound CSS screenshot attests the first compositor image. Main-frame
// navigation fences the source synchronously; no raw pixels after Vault use.
import {setTimeout as delay} from 'node:timers/promises';
import {PortableEncoder} from './kernel-browser-display.mjs';
import {decodePng,displayMaskRegions} from './kernel-browser-pixels.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';
import {protectionDeclared,regionProtectionChanged} from './kernel-browser-region-protection.mjs';
export function jpegDimensions(bytes) {
  if(bytes[0]!==255||bytes[1]!==216)return null;
  for(let offset=2;offset+4<bytes.length;){
    if(bytes[offset++]!==255)return null;
    while(bytes[offset]===255)offset++;
    const marker=bytes[offset++];
    if(marker===217||marker===218)return null;
    if(marker===1||marker>=208&&marker<=215)continue;
    if(offset+2>bytes.length)return null;
    const length=bytes.readUInt16BE(offset);
    if(length<2||offset+length>bytes.length)return null;
    if([192,193,194,195,197,198,199,201,202,203,205,206,207].includes(marker))return length>=8?{width:bytes.readUInt16BE(offset+5),height:bytes.readUInt16BE(offset+3)}:null;
    offset+=length;
  }
  return null;
}
export class CompositorSource {
  constructor({connection,sessionId,tab,scale,policy,screenshot,protect,allowed,width=1280,height=800,format='png',hasher=new PortableEncoder(),acquire=async()=>async()=>{},timing=()=>{},now=()=>performance.now()}) {
    Object.assign(this,{connection,sessionId,tab,scale,policy,screenshot,protect,allowed,width,height,format,hasher,acquire,timing,now});
    this.listeners=new Set();this.latest=null;this.serial=0;this.regionRevision=0;this.attested=false;this.closed=false;this.changedAt=now();this.fenced=false;this.sampling=0;this.ignoreUntil=-Infinity;this.motionStreak=0;this.ignoreIdleUntil=-Infinity;this.pendingImage=null;this.hashing=false;
  }
  subscribe(listener){this.listeners.add(listener);return()=>this.listeners.delete(listener)}
  async start() {
    if(this.fenced||!this.allowed(this.policy))throw Error('MD-DISPLAY: compositor policy fenced');
    this.off=this.connection.subscribe(message=>{
      if(message.sessionId!==this.sessionId||this.closed)return;
      if(this.attested&&this.protect&&regionProtectionChanged(message,this.sessionId,this.masks!=='[]')){
        // MP-11: retire old pixels without stopping a hidden renderer between
        // mouse press/release. A fresh masked capture wakes even without paint;
        // tree churn retires frames only once that capture changes the masks.
        if(protectionDeclared(message)){this.regionRevision++;this.latest=null;}
        this.pendingImage={receivedAt:performance.timeOrigin+this.now(),format:'png'};
        void this.processLatest();return;
      }
      if(message.method==='Page.frameNavigated'&&!message.params?.frame?.parentId){this.attested=false;this.latest=null;this.fenced=true;return;}
      if(message.method!=='Page.screencastFrame')return;
      // ACK the one shared source immediately; never let viewer credit delay CDP.
      void this.connection.send('Page.screencastFrameAck',{sessionId:message.params?.sessionId},this.sessionId).catch(()=>{});
      if(this.fenced||!this.allowed(this.policy)){this.latest=null;this.attested=false;return;}
      if(this.sampling||this.now()<this.ignoreUntil)return;
      const data=message.params?.data;
      if(typeof data!=='string'||data.length>4*1024*1024){this.latest=null;this.attested=false;return;}
      const header=Buffer.from(data.slice(0,40),'base64');
      const png=header.length>=24&&header.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10]));
      const geometry=png?{width:header.readUInt32BE(16),height:header.readUInt32BE(20)}:jpegDimensions(Buffer.from(data,'base64'));
      if(!geometry||geometry.width!==this.width||geometry.height!==this.height){this.latest=null;return;}
      this.pendingImage={data,receivedAt:performance.timeOrigin+this.now(),format:png?'png':'jpeg'};
      void this.processLatest();
    });
    try {
      this.release=await this.acquire();
      if(this.closed||this.fenced||!this.allowed(this.policy)){await this.release();this.release=null;throw Error('MD-DISPLAY: source admission retired')}
      await this.connection.send('Page.startScreencast',{format:'png',maxWidth:2560,maxHeight:1600,everyNthFrame:1},this.sessionId);
      const deadline=this.now()+2000;
      while(!this.latest&&!this.closed&&this.now()<deadline)await delay(10);
      const native=this.width===geometry.width*this.scale&&this.height===geometry.height*this.scale;
      const reference=await this.screenshot(native?null:{x:0,y:0,width:this.width,height:this.height,scale:1/this.scale});
      await assertCurrentDocument(this.connection,this.sessionId,this.tab.target_id,this.tab.document_id);
      if(this.closed||this.fenced||!this.latest||!this.allowed(this.policy))throw Error('MD-DISPLAY: compositor source changed');
      const expected=decodePng(reference.data_base64,this.scale),actual=decodePng(this.latest.data_base64,this.scale);
      if(expected.width!==actual.width||expected.height!==actual.height||!expected.pixels.equals(actual.pixels))throw Error('MD-DISPLAY: compositor attestation differed');
      this.attested=true;
      if(this.format==='jpeg'){await this.connection.send('Page.stopScreencast',{},this.sessionId);await this.connection.send('Page.startScreencast',{format:'jpeg',quality:95,maxWidth:this.width,maxHeight:this.height,everyNthFrame:1},this.sessionId)}
      return this;
    } catch(error){await this.close();throw error;}
  }
  async processLatest(){
    if(this.hashing||this.closed||this.fenced)return;
    this.hashing=true;
    try{
      while(this.pendingImage&&!this.closed&&!this.fenced){
        let {data,receivedAt,format}=this.pendingImage;this.pendingImage=null;const revision=this.regionRevision;
        // MP-11: screencast pixels have no protected-layout binding. Use them
        // only as a wake; capture/mask afresh before hashing or video encoding.
        let protectedRegions=[];
        if(this.protect){const source=await this.protect();data=source.data_base64;protectedRegions=source[displayMaskRegions]??[];format='png';}
        const fingerprint=await this.hasher.hash(data);this.timing('source_'+format+'_fingerprint',receivedAt);
        if(this.closed||this.fenced||!this.allowed(this.policy))break;
        if(revision!==this.regionRevision)continue;
        if(this.sampling||this.now()<this.ignoreUntil)continue;
        if(fingerprint.width!==this.width||fingerprint.height!==this.height)throw Error('source geometry');
        const masks=JSON.stringify(protectedRegions);if(this.masks!==undefined&&masks!==this.masks)this.regionRevision++;this.masks=masks;
        if(this.latest?.signature!==fingerprint.signature){this.motionStreak=this.now()-this.changedAt<90?this.motionStreak+1:1;this.serial++;this.changedAt=this.now();}
        this.latest={data_base64:data,[displayMaskRegions]:protectedRegions,signature:fingerprint.signature,width:this.width,height:this.height,motion:true,tab_id:this.tab.tab_id,document_id:this.tab.document_id,serial:this.serial};
        if(this.attested)for(const listener of this.listeners)listener(this.latest);
      }
    }catch{this.fenced=true;this.attested=false;this.latest=null;await this.close().catch(error=>{this.failure=error});}
    finally{this.hashing=false;}
  }
  pause(){this.sampling++;}
  resume(){this.sampling=Math.max(0,this.sampling-1);this.ignoreUntil=this.now()+80;this.ignoreIdleUntil=this.now()+200;}
  sample(after=-1) {
    if(this.closed||this.fenced||!this.attested||!this.allowed(this.policy))return null;
    return this.latest?.serial>after?this.latest:null;
  }
  close(){
    if(this.closing)return this.closing;
    this.closed=true;this.attested=false;this.latest=null;this.pendingImage=null;this.off?.();this.off=null;this.listeners.clear();
    this.closing=(async()=>{await this.connection.send('Page.stopScreencast',{},this.sessionId).catch(()=>{});try{await this.hasher.close()}finally{await this.release?.()}})();
    return this.closing;
  }
}
