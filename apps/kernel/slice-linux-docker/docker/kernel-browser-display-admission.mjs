// MP-08/MP-10/MP-11: asynchronous first-frame readiness reserves capacity,
// without holding the controller input barrier. Reservations never admit pixels.
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
// One display per watching viewer: a shared room is watched by as many viewers as the transport admits sessions
// (32 local browser sessions, 32 WebRTC peers). A lower bound refuses every viewer past it, and the refusal reads as a
// retired document, so those viewers resubscribe for ever without video (TR-19).
export const maxDisplays=32;
const pending=new WeakMap();
export async function reserveDisplay(host,start){
 const count=pending.get(host)??0;
 if(host.displays.size+count>=maxDisplays)throw new UserDomainRefusal('stale_reference');
 pending.set(host,count+1);
 try{return await start();}
 finally{const left=pending.get(host)-1;if(left)pending.set(host,left);else pending.delete(host);}
}
