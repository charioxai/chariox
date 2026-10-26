// MP-08 / MP-10 / MP-11: exclusive retention before scorer admission.
import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { retainFinalHar } from './har-retention.mjs'
const snapshot = () => ({log:{entries:[{request:{url:'https://fixture.test/'}}],_capture:{flushAcknowledged:true,activeRequests:0,missingExtraInfo:0,unmatchedExtraInfo:0,errors:[]}}})
test('MP-08 / MP-10 / MP-11 H13 copies/hash-verifies final HAR once without rewriting frozen file',async()=>{
 const dir=await mkdtemp(path.join(os.tmpdir(),'b218-har-'))
 try {const target=path.join(dir,'fixture.har'),receipt=await retainFinalHar(snapshot(),target)
  assert.equal(receipt.flushAcknowledged,true);assert.match(receipt.sha256,/^[a-f0-9]{64}$/)
  const original=await readFile(target);await assert.rejects(retainFinalHar(snapshot(),target),{code:'EEXIST'})
  assert.deepEqual(await readFile(target),original)
 } finally {await rm(dir,{recursive:true})}
})
test('MP-08 / MP-10 / MP-11 H13 missing flush/active requests/empty capture fail admission',async()=>{
 for(const [key,value] of [['flushAcknowledged',false],['activeRequests',1],['missingExtraInfo',1]]){
  const s=snapshot();s.log._capture[key]=value;await assert.rejects(retainFinalHar(s,'/unused'),/incomplete/)
 }
 const s=snapshot();s.log.entries=[];await assert.rejects(retainFinalHar(s,'/unused'),/incomplete/)
})
