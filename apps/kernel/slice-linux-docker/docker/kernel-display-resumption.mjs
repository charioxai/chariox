// MP-08/MP-10/MP-11: one producer lifetime for Browser, Desktop and regions.
// Resume IDs are hints. The Rust terminal admission and the current protected
// source binding must both pass again; no old delivery grant is replayed.
import { randomUUID } from 'node:crypto';
import { UserDomainRefusal } from './kernel-browser-refusal.mjs';

export const displayResumeGraceMs = 5000;
export class DisplaySessions {
  constructor(host,{now=()=>Date.now(),clock=()=>performance.now(),id=()=>`host-display-${randomUUID()}`}={}) {
    Object.assign(this,{host,now,clock,id});
  }
  require(id,owner,generation) {
    const stream=this.host.displays.get(id);
    if(stream&&stream.observed_by!==owner)throw new UserDomainRefusal('not_granted');
    if(!stream||generation!==this.host.generation)throw new UserDomainRefusal('stale_reference','display_retired');
    return stream;
  }
  async retire(stream) {
    if(this.host.removeDisplay)return this.host.removeDisplay(stream);
    this.host.displays.delete(stream.subscription_id);await stream.close();
  }
  async suspend(id,owner,generation,binding) {
    const stream=this.require(id,owner,generation),now=this.now();
    if(stream.expires<=now){await this.retire(stream);throw new UserDomainRefusal('stale_reference');}
    if(stream.resumeBinding)return {generation,suspended:true}; // never extend grace
    let source;
    try{source=await binding(stream);}catch(error){await this.retire(stream);throw error;}
    if(this.host.displays.get(id)!==stream)throw new UserDomainRefusal('stale_reference');
    stream.resumeBinding={source,policy:this.host.protection};
    stream.resumeUntil=this.clock()+displayResumeGraceMs;
    stream.deliveryEpoch=(stream.deliveryEpoch??0)+1;
    stream.suspended=true;
    stream.expires=Math.min(stream.expires,now+displayResumeGraceMs);
    // Keeps the producer/encoder. Its bounded queue holds a newly independent
    // protected frame through the grace; obsolete reference packets retire.
    stream.invalidate();this.host.armDisplayExpiry(stream);
    return {generation,suspended:true};
  }
  async resume(id,owner,generation,binding) {
    const stream=this.require(id,owner,generation),now=this.now();
    if(!stream.resumeBinding){
      if(!Number.isFinite(stream.lastCreditAt)||now<stream.lastCreditAt||now-stream.lastCreditAt>displayResumeGraceMs){await this.retire(stream);throw new UserDomainRefusal('stale_reference');}
      await this.suspend(id,owner,generation,binding);
    }
    let source;
    try{source=await binding(stream);}catch(error){await this.retire(stream);throw error;}
    if(stream.expires<=this.now()||this.clock()>=stream.resumeUntil||stream.resumeBinding.source!==source||stream.resumeBinding.policy!==this.host.protection){await this.retire(stream);throw new UserDomainRefusal('stale_reference');}
    if(this.host.displays.get(id)!==stream)throw new UserDomainRefusal('stale_reference');
    const next=this.id();
    this.host.displays.delete(id);stream.subscription_id=next;
    stream.deliveryEpoch=(stream.deliveryEpoch??0)+1;
    stream.sequence=0;stream.suspended=false;stream.resumeBinding=null;stream.resumeUntil=null;
    stream.resumed=true;stream.lastCreditAt=this.now();stream.expires=this.now()+60000;
    this.host.displays.set(next,stream);this.host.armDisplayExpiry(stream);
    return {generation,subscription_id:next,codec:stream.codec,bitrate:stream.bitrate,
      device_scale_factor:stream.device_scale_factor,resumed:true,
      ...(stream.source_kind==='desktop'?{source:{kind:'desktop',surface_id:source.surface_id,
        generation:source.generation,width:source.width,height:source.height}}:{}),
      ...(stream.source_kind==='region'?{source_kind:'region',binding_id:stream.binding_id,
        document_id:stream.regionLease.document_id,sequence:stream.regionLease.sequence,
        css_width:stream.css_width,css_height:stream.css_height,
        ...(stream.regionPlacement?{region_offset:stream.regionPlacement}:{})}:{})};
  }
}
