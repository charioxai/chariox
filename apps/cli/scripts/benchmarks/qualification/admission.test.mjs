// MP-08 / MP-10 / MP-11: synthetic public-policy fixtures, never gold answers.
import assert from 'node:assert/strict'
import test from 'node:test'
import { admitSites, checkSiteAdmission, AttemptLedger } from './admission.mjs'
const sites = () => ({ generation: 'fixture-generation', sourceDigest: 'a'.repeat(64), resetAtMs: 100, nowMs: 400,
  shops: Array.from({ length: 4 }, (_, i) => ({ id: `shop-${i}`, generation: 'fixture-generation', sourceDigest: 'a'.repeat(64),
    serviceHealthy: true, jvmCompatible: true, searchHealthy: true, indexedProducts: 20, expectedProducts: 20, indexedAtMs: 200, observedAtMs: 300 })) })
test('MP-08 / MP-10 / MP-11 H10 HTML health alone fails; four fresh indexed shops admit', () => {
 const input=sites(); input.shops[0].searchHealthy=false
 assert.throws(()=>admitSites(input),/RED/)
 input.shops[0].searchHealthy=true; const receipt=admitSites(input)
 assert.equal(checkSiteAdmission(receipt,{nowMs:500,generation:input.generation,sourceDigest:input.sourceDigest}).status,'admitted')
 assert.throws(()=>checkSiteAdmission(receipt,{nowMs:500,generation:'next-generation',sourceDigest:input.sourceDigest}),/stale/)
 assert.throws(()=>checkSiteAdmission(receipt,{nowMs:40000,generation:input.generation,sourceDigest:input.sourceDigest}),/stale/)
})
for (const [field,value] of [['indexedProducts',1],['indexedAtMs',50],['observedAtMs',999],['jvmCompatible',false],['sourceDigest','b'.repeat(64)]])
 test(`MP-08 / MP-10 / MP-11 H10 rejects ${field} drift`,()=>{const input=sites(); input.shops[3][field]=value; assert.throws(()=>admitSites(input),/RED/)})
const ledger = (maxCalls=2,maxMutations=1) => new AttemptLedger({attemptId:'attempt-1',taskId:'synthetic/👋',promptId:'prompt-1',maxCalls,maxMutations,tools:{slice_browser_status:'read',slice_browser_click:'mutation'}})
test('MP-08 / MP-10 / MP-11 H12 qualified first-party identity and duplicate charging',()=>{
 const l=ledger(); const read={callId:'c1',tool:'mcp__chariox__slice_browser_status',mutationUnits:0}
 assert.equal(l.admit(read).admitted,true); assert.equal(l.admit(read).duplicate,true)
 assert.equal(l.admit({callId:'c2',tool:'chariox.slice_browser_click',mutationUnits:1}).admitted,true)
 const row=l.settle({lifecycle:'completed',finalText:'Résumé 👩‍💻 é\n'})
 assert.equal(row.valid,true); assert.equal(row.calls.length,2); assert.equal(row.mutationUnits,1)
 assert.throws(()=>l.admit(read),/settlement/); assert.throws(()=>l.settle({}),/retry/)
})
test('MP-08 / MP-10 / MP-11 H12 call cap blocks dispatch but retains invalid final and denominator',()=>{
 const l=ledger(1); l.admit({callId:'c1',tool:'slice_browser_status',mutationUnits:0})
 assert.equal(l.admit({callId:'c2',tool:'slice_browser_click',mutationUnits:1}).admitted,false)
 const row=l.settle({lifecycle:'completed',finalText:'Synthetic terminal response.'})
 assert.equal(row.valid,false); assert.equal(row.denominatorIncluded,true); assert.equal(row.finalText,'Synthetic terminal response.')
 assert.deepEqual(row.violations,['call_cap'])
})
test('MP-08 / MP-10 / MP-11 H12 separate action budget charges all sequence units',()=>{
 const l=ledger(4,2); assert.equal(l.admit({callId:'c1',tool:'slice_browser_click',mutationUnits:3}).cause,'mutation_cap')
 assert.equal(l.settle({lifecycle:'cancelled',finalText:'partial'}).mutationUnits,3)
})
for (const tool of ['mcp__foreign__slice_browser_click','exec_command','chariox_evil',''])
 test(`MP-08 / MP-10 / MP-11 H12 rejects forbidden ${tool}`,()=>{assert.equal(ledger().admit({callId:'c1',tool,mutationUnits:1}).cause,'forbidden_tool')})
test('MP-08 / MP-10 / MP-11 H12 conflicting duplicate is invalid',()=>{
 const l=ledger(); l.admit({callId:'c1',tool:'slice_browser_status',mutationUnits:0})
 assert.throws(()=>l.admit({callId:'c1',tool:'slice_browser_click',mutationUnits:1}),/conflicting/)
 assert.equal(l.settle({lifecycle:'completed',finalText:'fixture'}).valid,false)
})
test('MP-08 / MP-10 / MP-11 H10 rejects private/non-contract readiness fields',()=>{
 const input=sites();input.shops[0].authorization='fixture-private';assert.throws(()=>admitSites(input),/RED/)
})
test('MP-08 / MP-10 / MP-11 H12 inherited object keys do not admit tools',()=>{
 assert.equal(ledger().admit({callId:'c1',tool:'constructor',mutationUnits:1}).cause,'forbidden_tool')
})
test('MP-08 / MP-10 / MP-11 H12 secret-like terminal response cannot be scored',()=>{
 const row=ledger().settle({lifecycle:'completed',finalText:'Bearer '+ 'A'.repeat(24)})
 assert.equal(row.valid,false);assert.equal(row.finalSha256,null);assert.equal(row.finalText.includes('A'.repeat(24)),false)
})
