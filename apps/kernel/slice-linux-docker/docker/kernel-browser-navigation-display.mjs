// MP-08/MP-10/MP-11: the native page display a viewer asks for when its mirror reports a navigation, started by the
// kernel when it sends that report instead of one round trip later (the source start then overlaps the report's way to
// the viewer). Only the same observer's next ordinary display_subscribe for the Tab, with the negotiation it used last
// time, adopts it; anything else retires it. Admission, protection and document fencing are subscribeVisiblePage's own.
import {subscribeVisiblePage} from './kernel-browser-page-display.mjs';
const UNCLAIMED_MS=3000,OFFERS=16;
const key=(scope,tabId)=>JSON.stringify([scope,tabId]);
const negotiation=command=>JSON.stringify([command.codecs,command.bitrate,command.device_scale_factor]);
export class NavigationDisplays{
 constructor(host,{subscribe=subscribeVisiblePage,unclaimedMs=UNCLAIMED_MS}={}){
  Object.assign(this,{host,subscribe,unclaimedMs});this.offers=new Map();this.started=new Map();
 }
 // The negotiation of the observer's last page display of this Tab.
 remember(command,scope){
  const k=key(scope,command.tab_id);this.offers.delete(k);
  this.offers.set(k,{codecs:[...command.codecs],bitrate:command.bitrate,device_scale_factor:command.device_scale_factor});
  if(this.offers.size>OFFERS)this.offers.delete(this.offers.keys().next().value);
 }
 start(scope,tabId,documentId){
  const k=key(scope,tabId),offer=this.offers.get(k);
  if(!offer)return;
  this.discard(k);
  const generation=this.host.generation,command={op:'display_subscribe',tab_id:tabId,generation,...offer};
  const entry={documentId,generation,negotiation:negotiation(command),run:this.subscribe(this.host,command,scope).catch(error=>{
   this.host.timing?.(`navigation_display_unavailable ${error?.code??'error'}`,performance.timeOrigin+performance.now());return null;
  })};
  entry.timer=setTimeout(()=>this.discard(k,entry),this.unclaimedMs);entry.timer.unref?.();
  this.started.set(k,entry);
 }
 // The started display when it is still the one this command would open, else null (and it is retired).
 async adopt(command,scope){
  const k=key(scope,command.tab_id),entry=this.started.get(k);
  if(!entry)return null;
  this.started.delete(k);clearTimeout(entry.timer);
  const result=await entry.run,stream=result&&this.host.displays.get(result.subscription_id);
  if(!stream)return null;
  const tab=entry.negotiation===negotiation(command)&&entry.generation===command.generation&&command.generation===this.host.generation
   ?await (this.host.mirrorTarget??this.host.displayTarget).call(this.host,command).catch(()=>null):null;
  if(tab?.document_id===entry.documentId&&stream.regionLease?.document_id===entry.documentId&&stream.regionLease.isAlive()){
   this.host.timing?.('navigation_display_adopted',performance.timeOrigin+performance.now());return result;
  }
  await this.host.removeDisplay(stream).catch(()=>{});
  return null;
 }
 discard(k,entry=this.started.get(k)){
  if(!entry||this.started.get(k)!==entry)return;
  this.started.delete(k);clearTimeout(entry.timer);
  void entry.run.then(result=>{const stream=result&&this.host.displays.get(result.subscription_id);return stream&&this.host.removeDisplay(stream);}).catch(()=>{});
 }
 clear(){for(const k of [...this.started.keys()])this.discard(k);this.offers.clear();}
}
