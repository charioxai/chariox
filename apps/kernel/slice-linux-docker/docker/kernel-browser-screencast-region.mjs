// R1 (proposal R): region video for a browser without its own display (headless: always on macOS, test rigs on Linux). A region
// lease is a crop of the Tab's one shared screencast (Chrome's own frames, the source the whole-tab fallback uses), so a Tab's
// regions and its whole-tab video decode each frame once. The stream, encoder and viewer are the ones a native region uses.
import {randomUUID} from 'node:crypto';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
import {DisplayStream,PortableEncoder} from './kernel-browser-display.mjs';
const refuse=()=>{throw new UserDomainRefusal('stale_reference');};
export async function subscribeScreencastRegion(host,command,scope,signal,{Screencast}){
 const target=command.target,tab=await host.displayTarget(command),policy=host.protection,generation=host.generation;
 if(tab.tab_id!==command.tab_id||!host.mirror.v2?.regions||typeof host.screencastFor!=='function')refuse();
 const {kind,...reference}=target;
 const lease=await host.mirror.v2.regions.resolve({...reference,generation},scope,{signal});
 let source,stream;
 try{
  if(lease.tab_id!==tab.tab_id||lease.document_id!==tab.document_id||!await lease.isCurrent())refuse();
  const scale=lease.device_scale_factor,r=lease.rect;
  // Inside the canonical clip, even dimensions (as a native region); the page is the screencast's whole frame.
  const left=Math.ceil(r.x*scale),top=Math.ceil(r.y*scale),width=2*Math.floor((Math.floor((r.x+r.width)*scale)-left)/2),height=2*Math.floor((Math.floor((r.y+r.height)*scale)-top)/2);
  if(width<2||height<2)refuse();
  const offset={x:left/scale-r.x,y:top/scale-r.y};
  const shared=await host.screencastFor(tab,scale);
  assertNotCancelled(signal);
  source=new Screencast({shared,tab,scale,policy,rect:{left,top,width,height},
   // As the whole-tab screencast: no pixels while Vault-protected values or targets are on the page (R1c).
   allowed:p=>host.protection===p&&!p.unknown&&!p.values?.length&&!p.targets?.length&&host.generation===generation,validateCurrent:()=>lease.isCurrent(),timing:host.timing});
  await source.start();
  assertNotCancelled(signal);
  if(host.protection!==policy||!await lease.isCurrent())refuse();
  const result={source_kind:'region',binding_id:lease.binding_id,document_id:lease.document_id,sequence:lease.sequence,generation,
   subscription_id:`host-display-${randomUUID()}`,codec:'avc1.420033',bitrate:command.bitrate,device_scale_factor:scale,
   css_width:source.region.css_width,css_height:source.region.css_height};
  stream=new DisplayStream({...result,tab_id:tab.tab_id,observed_by:scope,relay_binary:command.codecs.includes('chariox-relay-binary-v96'),
   dependencies:command.codecs.includes('chariox-video-dependencies-v1'),stripes:false},{timing:host.timing,encoder:new PortableEncoder()});
  Object.assign(stream,{regionLease:lease,regionSource:source,regionTarget:target,regionOffset:offset,regionCurrent:lease.isCurrent});
  const close=stream.close.bind(stream);
  let closing;
  stream.close=()=>closing??=(async()=>{
   try{const errors=(await Promise.allSettled([close(),source.close()])).filter(r=>r.status==='rejected').map(r=>r.reason);if(errors.length)throw new AggregateError(errors,'MD-DISPLAY: region teardown failed');}
   finally{await lease.close();}
  })();
  lease.attachDisplay(stream);host.armDisplayExpiry(stream);
  return result;
 }catch(error){
  let current;
  try{current=await lease.isCurrent();}
  finally{if(stream)await stream.close();else{try{await source?.close();}finally{await lease.close();}}}
  if(!current)refuse();
  throw error;
 }
}
// One region's view of the Tab's shared screencast. Frames are the shared ones, named at the crop's size with the crop
// rectangle (device pixels) for the encoder; validity, protection fences and wakes are the shared source's. Closing a
// region only lets go of the shared source: it stops with its last user (host.screencastFor).
export class ScreencastRegionCapture {
 constructor({shared,tab,scale,policy,rect,allowed,validateCurrent,timing=()=>{}}){
  Object.assign(this,{shared,tab,scale,policy,rect,allowed,validateCurrent,timing});
  this.region={css_width:rect.width/scale,css_height:rect.height/scale};this.listeners=new Set();this.closed=false;
 }
 get source(){return this.shared.source;}
 get regionRevision(){return this.source.regionRevision;}
 get changedAt(){return this.source.changedAt;}
 crop(frame){return frame&&{...frame,width:this.rect.width,height:this.rect.height,crop:{...this.rect},motion:true};}
 async start(){
  if(this.closed)throw Error('MD-DISPLAY: region source closed');
  await this.shared.ready;
  if(!this.valid())throw Error('MD-DISPLAY: compositor policy fenced');
  this.off=this.source.subscribe(frame=>{if(this.valid())for(const listener of this.listeners)listener(this.crop(frame));});
  return this;
 }
 subscribe(fn){this.listeners.add(fn);return()=>this.listeners.delete(fn);}
 sample(after=-1){return this.valid()?this.crop(this.source.sample(after)):null;}
 valid(){return !this.closed&&!this.source.closed&&!this.source.fenced&&this.allowed(this.policy);}
 wake(){this.source.wake?.();}
 pause(){this.source.pause?.();}
 resume(){this.source.resume?.();}
 close(){
  if(this.closing)return this.closing;
  this.closed=true;this.off?.();this.off=null;this.listeners.clear();
  return this.closing=Promise.resolve(this.shared.release?.());
 }
}
// orch14 decision 11-22 04:4xZ under AGENTS.md "Secrets and agents" (Mac informed): a region crop may use Chrome's raw screencast
// pixels while the page's protection policy is known and has no Vault values or Vault-fill targets; the pixel-exact start check
// and the per-frame protected screenshot of the whole-tab fallback (CompositorSource, unchanged) protect nothing in that state.
// Every other MP-11 fence holds: no frame while a value or target is present; a navigation, a protection change or a policy
// replacement (a Vault fill registers its target first) fences at once, and a frame captured at or after a fence never reaches
// a lease (each frame carries its capture time and the fence generation it was taken under, checked again at publication).
import {regionProtectionChanged} from './kernel-browser-region-protection.mjs';
export class RegionScreencast {
 constructor({connection,sessionId,tab,width,height,policy,allowed,quality=80,timing=()=>{},now=()=>Date.now()}){
  Object.assign(this,{connection,sessionId,tab,width,height,policy,allowed,quality,timing,now});
  this.listeners=new Set();this.latest=null;this.serial=0;this.generation=0;this.fenceAt=0;this.fenced=false;this.closed=false;this.regionRevision=0;this.changedAt=0;
 }
 subscribe(fn){this.listeners.add(fn);return()=>this.listeners.delete(fn);}
 // Synchronous: called by the host before a policy replacement takes effect, and on every protection or navigation event.
 fence(reason,permanent=false){this.generation++;this.regionRevision++;this.fenceAt=this.now();this.latest=null;if(permanent)this.fenced=true;this.timing(`region_screencast_fenced ${reason}`,performance.timeOrigin+performance.now());}
 valid(){return !this.closed&&!this.fenced&&this.allowed(this.policy);}
 admits(frame){return !!frame&&this.valid()&&frame.generation===this.generation&&frame.capturedAt>=this.fenceAt&&frame.document_id===this.tab.document_id;}
 onMessage(message){
  if(message.sessionId!==this.sessionId||this.closed)return;
  if(message.method==='Page.frameNavigated'&&!message.params?.frame?.parentId){this.fence('navigation',true);return;}
  if(regionProtectionChanged(message,this.sessionId,{targets:true})){this.fence('protection');return;}
  if(message.method!=='Page.screencastFrame')return;
  void this.connection.send('Page.screencastFrameAck',{sessionId:message.params?.sessionId},this.sessionId).catch(()=>{});
  const data=message.params?.data,stamp=message.params?.metadata?.timestamp;
  if(typeof data!=='string'||data.length>4*1024*1024||!Number.isFinite(stamp))return;
  // CDP stamps the frame's capture in seconds since the epoch.
  const frame={data_base64:data,format:'jpeg',width:this.width,height:this.height,capturedAt:stamp*1000,generation:this.generation,document_id:this.tab.document_id,tab_id:this.tab.tab_id,motion:true};
  if(!this.admits(frame))return;
  frame.serial=++this.serial;frame.signature=`${this.generation}:${frame.serial}`;this.latest=frame;this.changedAt=performance.now();
  for(const listener of this.listeners)if(this.admits(frame))listener(frame);
 }
 async start(){
  if(!this.valid())throw Error('MD-DISPLAY: region screencast policy fenced');
  this.off=this.connection.subscribe(message=>this.onMessage(message));
  await this.connection.send('Page.startScreencast',{format:'jpeg',quality:this.quality,maxWidth:this.width,maxHeight:this.height,everyNthFrame:1},this.sessionId);
  return this;
 }
 sample(after=-1){const frame=this.latest;return this.admits(frame)&&frame.serial>after?frame:null;}
 wake(){}
 close(){
  if(this.closing)return this.closing;
  this.closed=true;this.latest=null;this.listeners.clear();this.off?.();this.off=null;
  return this.closing=this.connection.send('Page.stopScreencast',{},this.sessionId).catch(()=>{});
 }
}
