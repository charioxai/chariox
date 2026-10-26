// MP-08/MP-10/MP-11: scheduling only; the native path still admits every frame.
import test from 'node:test';
import assert from 'node:assert/strict';
import {waitForNavigationPreview} from './kernel-browser-mirror2-preview.mjs';
const reference={tab_id:'t',document_id:'new',generation:4,scope:'owner'};
const candidate=()=>({tab_id:'t',generation:4,observed_by:'owner',expires:1000,regionTarget:{kind:'visible_page'},regionLease:{document_id:'new'},regionSource:{attested:true,valid:()=>true,sample:()=>({})}});
const host=()=>({chromium:{desktop:{binding:()=>({})}},displays:new Map()});
test('MP-08/MP-10/MP-11 only this viewer and document native producer ends the bounded wait',async()=>{
 const h=host();h.displays.set('other',{...candidate(),observed_by:'other'});let now=0;
 assert.equal(await waitForNavigationPreview(h,reference,undefined,{budgetMs:100,now:()=>now,pause:async ms=>{now+=ms;if(now===50)h.displays.set('ready',candidate())}}),true);
 assert.equal(now,50);
});
test('MP-08/MP-10/MP-11 unavailable or unprotected native startup is bounded and cancellation stays prompt',async()=>{
 const h=host(),source=candidate();source.regionSource.attested=false;h.displays.set('starting',source);let now=0;
 assert.equal(await waitForNavigationPreview(h,reference,undefined,{budgetMs:60,now:()=>now,pause:async ms=>{now+=ms}}),false);assert.equal(now,60);
 const abort=new AbortController();await assert.rejects(waitForNavigationPreview(h,reference,abort.signal,{pause:async()=>abort.abort()}));
 assert.equal(await waitForNavigationPreview({displays:new Map()},reference),false,'no owned desktop never waits');
});
