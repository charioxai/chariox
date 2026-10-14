// MP-08 / MP-10 / MP-11: slice placement adapter; authority stays in the home.
import {readFile, readdir, stat, open} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import {constants} from 'node:fs';
import {NativeAccessibility} from './native-accessibility.mjs';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';

async function sliceProcesses() {
  const uid=process.getuid();
  return (await Promise.all((await readdir('/proc')).filter(name=>/^\d+$/.test(name) && Number(name)>1).map(async name=>{
    try {
      if((await stat(`/proc/${name}`)).uid!==uid)return null;
      const text=await readFile(`/proc/${name}/stat`,'utf8');
      const fields=text.slice(text.lastIndexOf(')')+2).split(' ');
      return {pid:Number(name),started:fields[19]};
    } catch(error) {if(['ENOENT','ESRCH','EACCES'].includes(error.code))return null;throw error;}
  }))).filter(Boolean);
}
export class RoomNativeAccessibility {
  constructor({binding,execute}={}) {
    if(!binding) {
      if(!process.env.CHARIOX_SLICE_ID || process.getuid()===0)throw new Error('MP-11: native Room slice binding unavailable');
      this.runtime=process.env.CHARIOX_SLICE_PRIVATE_ROOT ? `${process.env.CHARIOX_SLICE_PRIVATE_ROOT}/runtime` : `${process.env.CHARIOX_SLICE_ROOT??'/opt/chariox-slice'}/private`;
      this.surface={surface_id:`slice:${process.env.CHARIOX_SLICE_ID}`,generation:randomUUID(),environment:{...process.env,DISPLAY:process.env.CHARIOX_SLICE_DISPLAY??':99'},ownedProcesses:sliceProcesses};
      if(!/^:[0-9]{1,4}$/.test(this.surface.environment.DISPLAY))throw new Error('MP-11: invalid Room display');
      binding=()=>this.surface;
    }
    this.native=new NativeAccessibility({binding,...(execute?{execute}:{})});
  }
  async request(params,{signal}={}) {
    if(this.runtime) {
      const file=await open(`${this.runtime}/desktop-session-address`,constants.O_RDONLY|constants.O_NOFOLLOW);
      let address;
      try {
        const metadata=await file.stat();
        if(!metadata.isFile() || metadata.uid!==process.getuid() || metadata.mode&0o077 || metadata.size>512)throw new Error('MP-11: Room desktop bus binding unavailable');
        address=await file.readFile('utf8');
        if(!/^unix:(?:path|abstract)=[^\x00-\x20]{1,480}$/.test(address))throw new Error('MP-11: invalid Room desktop bus binding');
      } finally {await file.close();}
      if(this.address && this.address!==address) {this.native.clear();this.surface={...this.surface,generation:randomUUID(),environment:{...this.surface.environment}};}
      this.address=address;this.surface.environment.DBUS_SESSION_BUS_ADDRESS=address;
    }
    if(!params || typeof params.observer!=='string' || !params.observer || params.observer.length>256 || !params.policy || params.policy.unknown)throw new UserDomainRefusal('not_granted');
    if(params.op==='snapshot')return this.native.snapshot(params.observer,params.policy,{signal});
    if(params.op==='target_action')return this.native.action(params.observer,params,params.policy,{signal});
    throw new Error('MP-08: unsupported Room native operation');
  }
}
