// MP-08/MP-10/MP-11: let the ordinary protected native producer initialize
// before large observer work competes for the renderer. This grants no pixels
// or input and skips hosts without the owned desktop. Missing video is bounded.
import {setTimeout as delay} from 'node:timers/promises';
import {hasPendingNavigationPreview} from './kernel-browser-display-admission.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
export const navigationPreviewBudgetMs=2500;
// The ordinary native first-sample path already permits up to five seconds.
export const navigationPreviewAdmissionBudgetMs=5000;
export async function waitForNavigationPreview(host,reference,signal,{budgetMs=navigationPreviewBudgetMs,admissionBudgetMs=navigationPreviewAdmissionBudgetMs,now=Date.now,pause=ms=>delay(ms,undefined,{signal})}={}){
 if(!host.chromium?.desktop?.binding?.()||!host.displays)return false;
 const started=now(),discoveryDeadline=started+budgetMs,admissionDeadline=started+Math.max(budgetMs,admissionBudgetMs);let reported=false;
 for(;;){
  assertNotCancelled(signal);
  if([...host.displays.values()].some(s=>s.tab_id===reference.tab_id&&s.generation===reference.generation&&s.observed_by===reference.scope&&s.expires>now()&&
   s.regionTarget?.kind==='visible_page'&&s.regionLease?.document_id===reference.document_id&&s.regionSource?.attested&&s.regionSource.valid?.()&&s.regionSource.sample?.()))return true;
  const pending=hasPendingNavigationPreview(host,reference),deadline=pending?admissionDeadline:discoveryDeadline;
  if(pending&&now()>=discoveryDeadline&&!reported){reported=true;host.timing?.event?.('mirror2_preview_admission_wait',{budget_ms:Math.max(budgetMs,admissionBudgetMs)});}
  if(now()>=deadline)return false;
  await pause(Math.min(25,deadline-now()));
 }
}
