// MP-08/MP-10/MP-11: let the ordinary protected native producer initialize
// before large observer work competes for the renderer. This grants no pixels
// or input and skips hosts without the owned desktop. Missing video is bounded.
import {setTimeout as delay} from 'node:timers/promises';
import {assertNotCancelled} from './browser-controller-actions.mjs';
export const navigationPreviewBudgetMs=2500;
export async function waitForNavigationPreview(host,reference,signal,{budgetMs=navigationPreviewBudgetMs,now=Date.now,pause=ms=>delay(ms,undefined,{signal})}={}){
 if(!host.chromium?.desktop?.binding?.()||!host.displays)return false;
 const deadline=now()+budgetMs;
 for(;;){
  assertNotCancelled(signal);
  if([...host.displays.values()].some(s=>s.tab_id===reference.tab_id&&s.generation===reference.generation&&s.observed_by===reference.scope&&s.expires>now()&&
   s.regionTarget?.kind==='visible_page'&&s.regionLease?.document_id===reference.document_id&&s.regionSource?.attested&&s.regionSource.valid?.()&&s.regionSource.sample?.()))return true;
  if(now()>=deadline)return false;
  await pause(Math.min(25,deadline-now()));
 }
}
