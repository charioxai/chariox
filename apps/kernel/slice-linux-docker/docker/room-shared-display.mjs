// MP-08/MP-10/MP-11: Room placement for the ordinary protected desktop producer.
// This adapter has no input seam. The home Room ledger remains its authority.
import {createHash} from 'node:crypto';
import {timestamp} from './kernel-browser-timing.mjs';
import {DesktopDisplay} from './kernel-desktop-display.mjs';
import {DisplaySessions} from './kernel-display-resumption.mjs';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
// MP-10/MP-11: 52 ciphertext-style bits join an opaque public stream hint;
// never export the hint, lease, observer, caller, key or protected policy.
const OPS=['display_subscribe','display_attach','screenshot','unsubscribe','display_suspend','display_resume','display_resume_attach','display_park'];
const streamJoin=id=>Number(createHash('sha256').update(id).digest().readBigUInt64BE(0)>>12n);
export class RoomSharedDisplay {
 constructor(desktop,{desktop:options,sessions,timing=()=>{}}={}) {
  this.desktop=desktop;this.generation=1;this.displays=new Map();this.timers=new Map();this.timing=timing;this.pending=Promise.resolve();this.nextViewer=1;
  this.chromium={desktop:{binding:()=>this.surface}};
  this.display=new DesktopDisplay(this,options);this.sessions=new DisplaySessions(this,sessions);
 }
 armDisplayExpiry(stream) {
  clearTimeout(this.timers.get(stream));
  const timer=setTimeout(()=>{this.timers.delete(stream);void this.removeDisplay(stream)},Math.max(1,stream.expires-Date.now()));timer.unref?.();this.timers.set(stream,timer);
 }
 async removeDisplay(stream){clearTimeout(this.timers.get(stream));this.timers.delete(stream);await this.display.remove(stream)}
 // The Room queue orders binding and lifecycle work. A frame pump leaves it
 // before its idle producer wait, so one viewer never delays another's resume;
 // a stream's own pumps stay ordered on its pumpTail.
 request(command,options) {
  const queued=timestamp(),stream=this.displays.get(command?.display_subscription_id??command?.subscription_id);
  let release;const held=new Promise(resolve=>{release=resolve});
  const result=this.pending.then(()=>{
   this.timing.event?.('room_display_queue',{at:queued,viewer:stream?.timingViewer??0,delivery:stream?.timingDelivery??0,
    operation:OPS.indexOf(command?.op)});
   const started=timestamp();
   return this.run(command,options,release).finally(()=>{release();this.timing.event?.('room_display_operation',{at:started,viewer:stream?.timingViewer??0,delivery:stream?.timingDelivery??0,
    operation:OPS.indexOf(command?.op)})});
  });this.pending=held;return result;
 }
 async run(command,{signal}={},release=()=>{}) {
  if(!OPS.includes(command?.op)
   ||typeof command.observed_by!=='string'||!command.observed_by||command.observed_by.length>256
   ||command.policy?.unknown!==false||!Array.isArray(command.policy.targets)||!Array.isArray(command.policy.values))throw new UserDomainRefusal('not_granted');
  const started=timestamp(),surface=await this.desktop.binding(command.viewport),policy=JSON.stringify(command.policy);
  const traced=this.displays.get(command.display_subscription_id??command.subscription_id);
  if(traced&&command.op!=='screenshot'&&command.op!=='display_subscribe'){traced.producer?.wakeReady?.();await traced.pumpTail;}
  this.timing.event?.('room_display_binding',{at:started,viewer:traced?.timingViewer??0,delivery:traced?.timingDelivery??0});
  if(this.surface!==surface){await this.display.close();this.surface=surface;}
  if(this.policyKey!==policy){await this.display.retireSource();this.policyKey=policy;this.protection=command.policy;}
  if(command.op==='display_subscribe'){
   if(command.target&&(command.target.kind!=='desktop'||command.target.surface_id!==surface.surface_id||command.target.generation!==surface.generation))throw new UserDomainRefusal('stale_reference');
   const result=await this.display.subscribe({...command,surface_id:surface.surface_id,generation:surface.generation},command.observed_by);
   if(command.lease_id){
    if(!/^room-display-[a-f0-9]{32}$/.test(command.lease_id)||this.displays.has(command.lease_id)){await this.removeDisplay(this.displays.get(result.subscription_id));throw new UserDomainRefusal('not_granted')}
    const stream=this.displays.get(result.subscription_id);this.displays.delete(result.subscription_id);stream.subscription_id=command.lease_id;this.displays.set(stream.subscription_id,stream);result.subscription_id=stream.subscription_id;
   }
   const stream=this.displays.get(result.subscription_id);stream.timingViewer=this.nextViewer++;stream.timingDelivery=1;stream.timingDeliveryStarted=timestamp();
   this.timing.event?.('room_display_subscribed',{at:stream.timingDeliveryStarted,viewer:stream.timingViewer,delivery:stream.timingDelivery,stream_join:streamJoin(stream.subscription_id)});
   return result;
  }
  const id=command.display_subscription_id??command.subscription_id;
  if(['display_suspend','display_resume','display_resume_attach'].includes(command.op)){const result=await this.sessions[command.op==='display_suspend'?'suspend':'resume'](id,command.observed_by,command.generation,async stream=>{
   if(stream.source_kind!=='desktop'||stream.tab_id!==surface.surface_id||stream.desktop_generation!==surface.generation)throw new UserDomainRefusal('stale_reference');
   return surface;
  });
   // A Room lease owns its observer scope; a resumed delivery receives a new one.
   if(command.op!=='display_suspend'&&command.observed_by===id)this.displays.get(result.subscription_id).observed_by=result.subscription_id;
   if(command.op!=='display_suspend'){
    const stream=this.displays.get(result.subscription_id);stream.timingDelivery=(stream.timingDelivery??0)+1;stream.timingDeliveryStarted=timestamp();
    this.timing.event?.('room_display_resumed',{at:stream.timingDeliveryStarted,viewer:stream.timingViewer,delivery:stream.timingDelivery,stream_join:streamJoin(stream.subscription_id)});
   }
   if(command.op==='display_resume_attach')await this.display.request({...command,op:'display_attach',subscription_id:result.subscription_id,generation:result.generation},this.displays.get(result.subscription_id).observed_by,{signal});
   return result;
  }
  // An unwatched Room display: renew its binding (a keepalive), then stop the shared source once no viewer watches.
  // The next screenshot unparks it and starts the source again.
  if(command.op==='display_park'){
   const attached=await this.display.request({...command,op:'display_attach'},command.observed_by,{signal});
   await this.display.park(this.displays.get(id),true);
   return {...attached,parked:true};
  }
  if(command.op==='unsubscribe'){
   const stream=this.sessions.require(id,command.observed_by,command.generation);await this.removeDisplay(stream);return {generation:this.generation,unsubscribed:true};
  }
  const pump=(traced?.pumpTail??Promise.resolve()).then(()=>this.display.request(command,command.observed_by,{signal}));
  if(traced)traced.pumpTail=pump.catch(()=>{});
  release();return pump;
 }
 async close(){await this.pending;try{await this.display.close()}finally{for(const timer of this.timers.values())clearTimeout(timer);this.timers.clear();this.timing.flush?.()}}
}
