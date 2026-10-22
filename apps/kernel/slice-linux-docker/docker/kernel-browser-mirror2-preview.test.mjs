// MP-08/MP-10/MP-11: scheduling only; the native path still admits every frame.
import test from 'node:test';
import assert from 'node:assert/strict';
import {beginNavigationPreview,waitForNavigationPreview} from './kernel-browser-mirror2-preview.mjs';
import {subscribeVisiblePage} from './kernel-browser-page-display.mjs';
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

test('MP-08/MP-10: a refused native preview does not hold the mirror for its startup budget',async()=>{
 const h=host();h.generation=4;h.browser={};
 h.mirrorTarget=async()=>({tab_id:'t',document_id:'new',target_id:'T'});
 h.chromeWindows={current(){throw Error('native window unavailable');}};
 await assert.rejects(subscribeVisiblePage(h,{tab_id:'t',generation:4,codecs:['png','avc1.420033'],bitrate:4_000_000,device_scale_factor:1},'owner'),/native window unavailable/);
 let now=0;
 assert.equal(await waitForNavigationPreview(h,reference,undefined,{budgetMs:100,now:()=>now,pause:async ms=>{now+=ms}}),false);
 assert.equal(now,0,'a known refusal must not add startup latency to a healthy mirror');
 for(const changed of [{scope:'other'},{document_id:'later'},{generation:5}]){
  now=0;
  await waitForNavigationPreview(h,{...reference,...changed},undefined,{budgetMs:60,now:()=>now,pause:async ms=>{now+=ms}});
  assert.equal(now,60,'another viewer, document or generation keeps its own startup wait');
 }
});

test('MP-08/MP-10: a retired preview attempt cannot end a newer startup wait',async()=>{
 const h=host(),old=beginNavigationPreview(h,reference),current=beginNavigationPreview(h,reference);
 old(false);let now=0;
 await waitForNavigationPreview(h,reference,undefined,{budgetMs:60,now:()=>now,pause:async ms=>{now+=ms}});
 assert.equal(now,60);
 current(false);now=0;
 await waitForNavigationPreview(h,reference,undefined,{budgetMs:60,now:()=>now,pause:async ms=>{now+=ms}});
 assert.equal(now,0);
});
