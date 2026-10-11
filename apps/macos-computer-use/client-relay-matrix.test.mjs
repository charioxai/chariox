import test from 'node:test';
import assert from 'node:assert/strict';
import {matrixCases, runMacClientMatrix} from './client-relay-matrix.mjs';
const pins={helper_sha256:'a'.repeat(64),kernel_sha256:'b'.repeat(64)};
const proof=c=>({...c,...pins,live:true,route:c.client==='local-tui'?'kernel':'hosted-wss',backend:c.codec==='png'?'png':'videotoolbox-hardware',
 source_pixels_sha256:'c'.repeat(64),presented_pixels_sha256:'c'.repeat(64),protection_canary_absent:true,independent_native_effect:true,
 stale_epoch_refused:true,no_input_replay:true,scenario_verified:true,owned_cleanup:true,evidence:'/Users/miguel/.codex/evidence/public/receipt.json',
 metrics:{samples:30,dpr:2,rtt_ms:40,input_visual_p50_ms:30,input_visual_p95_ms:80,capture_publish_p95_ms:30,presented_fps:30,settle_ms:200,egress_bps:1e6,budget_bps:2e6,peak_owned_rss_bytes:1e8}});
test('missing real callbacks block every exact matrix id',async()=>{const r=await runMacClientMatrix();assert.equal(r.functional,'BLOCKED');assert.equal(r.rows.length,54);assert.equal(new Set(r.rows.map(r=>r.id)).size,54);});
test('all proofs and separate performance gates are required',async()=>{
 const r=await runMacClientMatrix({pins,runCase:async c=>proof(c),cleanup:async()=>true});assert.equal(r.functional,'PASS');assert.equal(r.performance,'PASS');
 const slow=await runMacClientMatrix({pins,runCase:async c=>({...proof(c),metrics:{...proof(c).metrics,presented_fps:10}}),cleanup:async()=>true});assert.equal(slow.functional,'PASS');assert.equal(slow.performance,'FAIL');
});
test('route, canary, fallback substitution and exact pixels cannot pass; failure stops mutation',async()=>{
 for(const changed of [{route:'local-replay'},{protection_canary_absent:false},{codec:'png'},{presented_pixels_sha256:'d'.repeat(64)},{live:false},{backend:'unproven'}]){
  let runs=0,cleanups=0;const r=await runMacClientMatrix({pins,runCase:async c=>{runs++;return {...proof(c),...changed}},cleanup:async()=>{cleanups++;return true}});
  assert.equal(r.functional,'FAIL');assert.equal(runs,1);assert.equal(cleanups,1);assert.equal(r.rows[1].functional,'BLOCKED');
 }
});
test('timeout and failed cleanup fail, blocked dependencies never become acceptance',async()=>{
 const r=await runMacClientMatrix({pins,runCase:async()=>{throw Error('timeout')},cleanup:async()=>true});assert.equal(r.functional,'FAIL');
 const blocked=await runMacClientMatrix({pins,runCase:async()=>({blocked:true}),cleanup:async()=>true});assert.equal(blocked.functional,'BLOCKED');
 const dirty=await runMacClientMatrix({pins,runCase:async c=>proof(c),cleanup:async()=>false});assert.equal(dirty.functional,'FAIL');
});

test('a stuck real callback aborts at the deadline and prevents later input',async()=>{
 let signal, calls=0;
 const r=await runMacClientMatrix({pins,caseTimeoutMs:1,runCase:async(c,options)=>{calls++;signal=options.signal;return new Promise(()=>{});},cleanup:async()=>true});
 assert.equal(signal.aborted,true);assert.equal(calls,1);assert.equal(r.functional,'FAIL');assert.equal(r.rows[1].functional,'BLOCKED');
});
