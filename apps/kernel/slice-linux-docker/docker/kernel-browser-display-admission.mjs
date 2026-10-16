// MP-08/MP-10/MP-11: asynchronous first-frame readiness reserves capacity,
// without holding the controller input barrier. Reservations never admit pixels.
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
const pending=new WeakMap();
export async function reserveDisplay(host,start,reference){
 const entries=pending.get(host)??new Set();
 if(host.displays.size+entries.size>=8)throw new UserDomainRefusal('stale_reference');
 const entry={reference};entries.add(entry);pending.set(host,entries);
 try{return await start(entry);}
 finally{entries.delete(entry);if(!entries.size)pending.delete(host);}
}
export function hasPendingNavigationPreview(host,reference){
 return [...pending.get(host)??[]].some(({reference:r})=>r?.kind==='visible_page'&&r.tab_id===reference.tab_id&&r.generation===reference.generation&&r.scope===reference.scope&&
  (r.document_id===undefined||r.document_id===reference.document_id));
}
