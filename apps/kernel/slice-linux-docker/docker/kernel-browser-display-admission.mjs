// MP-08/MP-10/MP-11: asynchronous first-frame readiness reserves capacity,
// without holding the controller input barrier. Reservations never admit pixels.
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
const pending=new WeakMap();
export async function reserveDisplay(host,start){
 const count=pending.get(host)??0;
 if(host.displays.size+count>=8)throw new UserDomainRefusal('stale_reference');
 pending.set(host,count+1);
 try{return await start();}
 finally{const left=pending.get(host)-1;if(left)pending.set(host,left);else pending.delete(host);}
}
