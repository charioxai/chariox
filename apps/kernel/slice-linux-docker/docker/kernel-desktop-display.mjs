// MP-08 / MP-10 / MP-11: desktop viewer leases use shared codecs and frame transport.
import { randomUUID } from 'node:crypto';
import { DesktopSource, nativeDesktopWorker } from './kernel-desktop-source.mjs';
import { DisplayStream, exactPatchLimit } from './kernel-browser-display.mjs';
import { NativeRefiner } from './kernel-browser-refiner.mjs';
import { MotionEncoder } from './kernel-browser-motion.mjs';
import { UserDomainRefusal } from './kernel-browser-refusal.mjs';
export class DesktopDisplay {
  constructor(host,{createSource=async(binding,policy)=>new DesktopSource(binding,policy,{timing:host.timing}).start(),createProducer=(...args)=>new MotionEncoder(...args)}={}) {
    this.host=host;this.createSource=createSource;this.createProducer=createProducer;this.source=null;this.sourcePending=null;this.sourceEpoch=0;
  }
  streams(){return [...this.host.displays.values()].filter(stream=>stream.source_kind==='desktop');}
  binding(command) {
    const binding=this.host.chromium.desktop?.binding();
    if(!binding||command.surface_id!==binding.surface_id||command.generation!==binding.generation)throw new UserDomainRefusal('stale_reference');
    return binding;
  }
  async ensureSource() {
    if(this.source&&!this.source.closed)return this.source;
    if(!this.sourcePending){
      const epoch=this.sourceEpoch,binding=this.host.chromium.desktop.binding(),policy=this.host.protection;
      this.sourcePending=(async()=>{
        await this.source?.close();
        const source=await this.createSource(binding,policy);
        if(epoch!==this.sourceEpoch||this.host.protection!==policy||this.host.chromium.desktop?.binding()!==binding){await source.close();throw new UserDomainRefusal('stale_reference');}
        this.source=source;return source;
      })().finally(()=>{this.sourcePending=null;});
    }
    return this.sourcePending;
  }
  async subscribe(command,scope) {
    const binding=this.binding(command);
    // MP-08/MP-10: the kernel's native x264 encoder serves H.264 offers first.
    const codec=nativeDesktopWorker()&&command.codecs?.includes('avc1.420033')?'avc1.420033':command.codecs?.find(value=>['avc1.420033','vp8','vp09.00.10.08'].includes(value));
    if(command._agent_input||!codec||![1,2].includes(command.device_scale_factor)||!Number.isSafeInteger(command.bitrate)||command.bitrate<500000||command.bitrate>20000000||this.host.displays.size>=8)throw new UserDomainRefusal('not_granted');
    await this.ensureSource();
    if(this.host.chromium.desktop?.binding()!==binding)throw new UserDomainRefusal('stale_reference');
    const id='host-display-'+randomUUID();
    const stream=new DisplayStream({subscription_id:id,tab_id:binding.surface_id,desktop_generation:binding.generation,
      source_kind:'desktop',observed_by:scope,codec,bitrate:command.bitrate,device_scale_factor:command.device_scale_factor,
      css_width:binding.width/command.device_scale_factor,css_height:binding.height/command.device_scale_factor,
      dependencies:command.codecs.includes('chariox-video-dependencies-v1'),
      stripes:command.codecs.includes('chariox-stripes-v1')&&['avc1.420033','vp8'].includes(codec),relay_binary:command.codecs.includes('chariox-relay-binary-v96')},{timing:this.host.timing});
    this.host.displays.set(id,stream);this.host.armDisplayExpiry(stream);
    return {generation:this.host.generation,subscription_id:id,codec,bitrate:command.bitrate,device_scale_factor:command.device_scale_factor,source:{kind:'desktop',surface_id:binding.surface_id,generation:binding.generation,width:binding.width,height:binding.height}};
  }
  async request(command,scope,{signal}={}) {
    const id=command.display_subscription_id??command.subscription_id,stream=this.host.displays.get(id),binding=this.host.chromium.desktop?.binding();
    if(!stream||stream.source_kind!=='desktop'||stream.observed_by!==scope||command.generation!==this.host.generation||binding?.surface_id!==stream.tab_id||binding.generation!==stream.desktop_generation||stream.expires<Date.now())throw new UserDomainRefusal('not_granted');
    if(command.op==='unsubscribe'){await this.remove(stream);return {generation:this.host.generation,unsubscribed:true};}
    stream.expires=Date.now()+60000;this.host.armDisplayExpiry(stream);
    if(command.op==='display_attach')return {generation:this.host.generation,attached:true};
    if(command.op!=='screenshot'||!stream.acceptsCredit(command.after_sequence))throw new UserDomainRefusal('stale_reference');
    const policy=this.host.protection,source=await this.ensureSource();
    const lifetimeValid=()=>this.host.protection===policy&&source.valid()&&this.host.displays.get(id)===stream&&this.host.chromium.desktop?.binding()===binding;
    if(!stream.producer){stream.producer=this.createProducer(source,stream.encoder,{codec:stream.codec,bitrate:stream.bitrate,independent:!stream.dependencies,stripes:stream.stripes,valid:lifetimeValid,timing:this.host.timing});}
    const valid=()=>!signal?.aborted&&lifetimeValid();
    // MP-08/MP-10: a protocol 475 push credit may ask for a key and waits
    // briefly for the next frame (the kernel pump paces the stream).
    if(command.push?.reset)stream.invalidate();
    await stream.producer.waitReady(command.push?100:20,signal);
    if(!valid())throw new UserDomainRefusal('not_granted');
    const encoded=stream.producer.take(),sample=encoded??this.refine(stream,source,policy,lifetimeValid);
    if(!sample)return {generation:this.host.generation,frame_sent:false,display_frame:null};
    // A newer published sample abandons a link-paced repair batch (motion first).
    const published=source.sample(),current=encoded?valid:()=>valid()&&source.sample()===published;
    const frame=await stream.frame({...sample,generation:this.host.generation},binding.generation,command.after_sequence,async()=>current(),current);
    if(frame)this.host.timing.event?.('desktop_frame_out',{at:sample.captured_ms??sample.raw?.captured_ms,sequence:frame.sequence,serial:sample.serial,captured_ms:sample.captured_ms??sample.raw?.captured_ms,published_ms:sample.published_ms,encoded_ms:sample.encoded_ms});
    return {generation:this.host.generation,frame_sent:frame!==null,display_frame:frame};
  }
  // MP-08/MP-10: a quiet lossy desktop settles to exact repair tiles of the
  // same published (already protected) sample; motion frames stay video.
  refine(stream,source,policy,valid) {
    const sample=source.sample();
    if(!sample?.raw?.nativeExact||!stream.previous||(stream.exact&&!stream.repair))return null;
    // MP-08/MP-10: keep repairs out of the first second after input, but
    // allow a quiet field to settle between caret changes. Every repair
    // remains bound to the current protected sample; motion abandons it.
    const quietNativeMs=150;
    stream.refiner??=new NativeRefiner(binding=>binding.sample,{quietNativeMs,timing:this.host.timing});
    const binding={source,document:stream.desktop_generation,policy,epoch:0,serial:sample.serial,native:true,sample,
      repairLimit:exactPatchLimit(stream.bitrate),encoder:stream.encoder.nativeSession,nativeDelivered:stream.encoder.nativeDeliveredRevision};
    const changedAt=Math.max(source.changedAt??-Infinity,(this.inputAt??-Infinity)+1000-quietNativeMs);
    const exact=stream.refiner.request(binding,changedAt,()=>valid()&&source.sample()===sample);
    return exact&&{...exact,generation:this.host.generation};
  }
  async remove(stream) {
    this.host.displays.delete(stream.subscription_id);await stream.close();
    if(!this.streams().length){const source=this.source;this.source=null;await source?.close();}
  }
  async retireSource() {
    this.sourceEpoch++;
    await this.sourcePending?.catch(()=>{});
    for(const stream of this.streams()){await stream.producer?.close();stream.producer=null;stream.invalidate();}
    const source=this.source;this.source=null;await source?.close();
  }
  async retire(observer){for(const stream of this.streams())if(stream.observed_by===observer)await this.remove(stream);}
  async close(){for(const stream of this.streams())await this.remove(stream);await this.retireSource();}
  wake(){this.inputAt=performance.now();this.source?.wake();}
}
