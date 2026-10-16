// MP-08/MP-10/MP-11: do not start heavy observation over this viewer's admitted native initialization.
import test from 'node:test';
import assert from 'node:assert/strict';
import {waitForNavigationPreview} from './kernel-browser-mirror2-preview.mjs';
import {reserveDisplay} from './kernel-browser-display-admission.mjs';
const reference={kind:'visible_page',tab_id:'t',document_id:'doc',generation:1,scope:'viewer'};
const host=()=>({chromium:{desktop:{binding:()=>({})}},displays:new Map()});
const candidate=()=>({tab_id:'t',generation:1,observed_by:'viewer',expires:1000,regionTarget:{kind:'visible_page'},regionLease:{document_id:'doc'},regionSource:{attested:true,valid:()=>true,sample:()=>({})}});
test('MP-08/MP-10/MP-11 matching native initialization settles before heavy observer work starts',async()=>{
 const h=host(),ready=Promise.withResolvers();let now=0;
 const pending=reserveDisplay(h,async()=>{await ready.promise;h.displays.set('native',candidate());},reference);
 try{const result=await waitForNavigationPreview(h,reference,undefined,{budgetMs:60,admissionBudgetMs:120,now:()=>now,pause:async ms=>{now+=ms;if(now>=90){ready.resolve();await pending;}}});assert.equal(result,true);assert(now>=90&&now<=120);}
 finally{ready.resolve();await pending;}
});
for(const field of ['scope','document_id','generation','kind'])test('MP-11 unrelated '+field+' reservation never delays this preview',async()=>{
 const h=host(),ready=Promise.withResolvers();let now=0;
 const pending=reserveDisplay(h,()=>ready.promise,{...reference,[field]:'other'});
 try{assert.equal(await waitForNavigationPreview(h,reference,undefined,{budgetMs:60,admissionBudgetMs:120,now:()=>now,pause:async ms=>{now+=ms}}),false);assert.equal(now,60);}
 finally{ready.resolve();await pending;}
});
test('MP-08/MP-10/MP-11 stalled native initialization is bounded and never admits pixels',async()=>{
 const h=host(),ready=Promise.withResolvers();let now=0;const pending=reserveDisplay(h,()=>ready.promise,reference);
 try{assert.equal(await waitForNavigationPreview(h,reference,undefined,{budgetMs:60,admissionBudgetMs:120,now:()=>now,pause:async ms=>{now+=ms}}),false);assert.equal(now,120);assert.equal(h.displays.size,0);}
 finally{ready.resolve();await pending;}
});

test('MP-11 cancellation stays prompt with a matching native admission',async()=>{
 const h=host(),ready=Promise.withResolvers(),abort=new AbortController(),pending=reserveDisplay(h,()=>ready.promise,reference);
 try{await assert.rejects(waitForNavigationPreview(h,reference,abort.signal,{pause:async()=>abort.abort()}));}
 finally{ready.resolve();await pending;}
});
