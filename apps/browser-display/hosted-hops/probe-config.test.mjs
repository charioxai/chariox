// MP-08/MP-10/MP-11: probes must stay in the lane and separate A/B runtime roots.
import test from 'node:test';import assert from 'node:assert/strict';
import {probePaths,typingMetric} from './probe-config.mjs';
test('MP-10 owner A/B paths select the product runtime and separate source CPU trees',()=>{
 const root='/root/.chariox/dev/vidlat/owner-ab-r12-fixed',e='/root/.codex/evidence/browser-resume-20260930/vidlat/r12/cell';
 const p=probePaths(root,e);assert.equal(p.runtime,root+'/V/runtime');assert.equal(p.V,root+'/V');assert.equal(p.S,root+'/S');
 assert.equal(probePaths('/root/.chariox/dev/vidlat/r6/drills/control',e).runtime,'/root/.chariox/dev/vidlat/r6/drills/control/runtime');
});
test('MP-11 malformed and shared roots fail before any state read or process accounting',()=>{
 const e='/root/.codex/evidence/browser-resume-20260930/vidlat/r12/cell';
 for(const root of [undefined,'/root/.chariox','/root/.chariox/dev/mirror/owner-ab-r12-fixed','/root/.chariox/dev/vidlat/owner-ab-r12-fixed/../other','/root/.chariox/dev/vidlat/r6/drills/'])assert.throws(()=>probePaths(root,e),/owned/);
 assert.throws(()=>probePaths('/root/.chariox/dev/vidlat/owner-ab-r12-fixed','/root/work/evidence'),/owned/);
});
test('MP-10 typing sample count requires an actual source echo and physical endpoint',()=>{
 const good={source_echo_matches:true,physical:{latency_ms:125}};
 assert.deepEqual(typingMetric([good,{source_echo_matches:false,physical:{latency_ms:1}},{source_echo_matches:true,physical:{latency_ms:null}}]),{attempts:3,samples:[125]});
 assert.deepEqual(typingMetric([{source_equals_public_c:true,physical:{latency_ms:126}}]).samples,[126]);
});

test('MP-10 source read-only oracle follows the visible tab rather than CDP list order',async()=>{
 const {visibleSourcePage}=await import('./probe-config.mjs');
 const pages=[{type:'page',id:'new-background'},{type:'page',id:'active'},{type:'worker',id:'worker'}];
 assert.equal((await visibleSourcePage(pages,async p=>p.id==='active')).id,'active');
 await assert.rejects(()=>visibleSourcePage(pages,async()=>true),/ambiguous/);
 await assert.rejects(()=>visibleSourcePage(pages,async()=>false),/missing/);
});
