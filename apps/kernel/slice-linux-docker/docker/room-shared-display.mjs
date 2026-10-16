// MP-08/MP-10/MP-11: Room placement for the ordinary protected desktop producer.
// This adapter has no input seam. The home Room ledger remains its authority.
import {DesktopDisplay} from './kernel-desktop-display.mjs';
import {DisplaySessions} from './kernel-display-resumption.mjs';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
export class RoomSharedDisplay {
 constructor(desktop,{desktop:options,sessions,timing=()=>{}}={}) {
  this.desktop=desktop;this.generation=1;this.displays=new Map();this.timers=new Map();this.timing=timing;this.pending=Promise.resolve();
  this.chromium={desktop:{binding:()=>this.surface}};
  this.display=new DesktopDisplay(this,options);this.sessions=new DisplaySessions(this,sessions);
 }
 armDisplayExpiry(stream) {
  clearTimeout(this.timers.get(stream));
  const timer=setTimeout(()=>{this.timers.delete(stream);void this.removeDisplay(stream)},Math.max(1,stream.expires-Date.now()));timer.unref?.();this.timers.set(stream,timer);
 }
 async removeDisplay(stream){clearTimeout(this.timers.get(stream));this.timers.delete(stream);await this.display.remove(stream)}
 request(command,options) {
  const result=this.pending.then(()=>this.run(command,options));this.pending=result.catch(()=>{});return result;
 }
 async run(command,{signal}={}) {
  if(!['display_subscribe','display_attach','screenshot','unsubscribe','display_suspend','display_resume','display_resume_attach'].includes(command?.op)
   ||typeof command.observed_by!=='string'||!command.observed_by||command.observed_by.length>256
   ||command.policy?.unknown!==false||!Array.isArray(command.policy.targets)||!Array.isArray(command.policy.values))throw new UserDomainRefusal('not_granted');
  const surface=await this.desktop.binding(command.viewport),policy=JSON.stringify(command.policy);
  if(this.surface!==surface){await this.display.close();this.surface=surface;}
  if(this.policyKey!==policy){await this.display.retireSource();this.policyKey=policy;this.protection=command.policy;}
  if(command.op==='display_subscribe'){
   if(command.target&&(command.target.kind!=='desktop'||command.target.surface_id!==surface.surface_id||command.target.generation!==surface.generation))throw new UserDomainRefusal('stale_reference');
   const result=await this.display.subscribe({...command,surface_id:surface.surface_id,generation:surface.generation},command.observed_by);
   if(command.lease_id){
    if(!/^room-display-[a-f0-9]{32}$/.test(command.lease_id)||this.displays.has(command.lease_id)){await this.removeDisplay(this.displays.get(result.subscription_id));throw new UserDomainRefusal('not_granted')}
    const stream=this.displays.get(result.subscription_id);this.displays.delete(result.subscription_id);stream.subscription_id=command.lease_id;this.displays.set(stream.subscription_id,stream);result.subscription_id=stream.subscription_id;
   }
   return result;
  }
  const id=command.display_subscription_id??command.subscription_id;
  if(['display_suspend','display_resume','display_resume_attach'].includes(command.op)){const result=await this.sessions[command.op==='display_suspend'?'suspend':'resume'](id,command.observed_by,command.generation,async stream=>{
   if(stream.source_kind!=='desktop'||stream.tab_id!==surface.surface_id||stream.desktop_generation!==surface.generation)throw new UserDomainRefusal('stale_reference');
   return surface;
  });
   // A Room lease owns its observer scope; a resumed delivery receives a new one.
   if(command.op!=='display_suspend'&&command.observed_by===id)this.displays.get(result.subscription_id).observed_by=result.subscription_id;
   if(command.op==='display_resume_attach')await this.display.request({...command,op:'display_attach',subscription_id:result.subscription_id,generation:result.generation},this.displays.get(result.subscription_id).observed_by,{signal});
   return result;
  }
  if(command.op==='unsubscribe'){
   const stream=this.sessions.require(id,command.observed_by,command.generation);await this.removeDisplay(stream);return {generation:this.generation,unsubscribed:true};
  }
  return this.display.request(command,command.observed_by,{signal});
 }
 async close(){await this.pending;await this.display.close();for(const timer of this.timers.values())clearTimeout(timer);this.timers.clear()}
}
