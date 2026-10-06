// MP-08/MP-10/MP-11: rejects ambiguous and admitted prompt replays.
import test from 'node:test'
import assert from 'node:assert/strict'
import { classifyTimeWarpContinuation } from './timewarp-continuation.mjs'
const row={taskId:2,era:1,admission:'reserved',firstFailingSeam:'prompt_submit',
  error:{errorClass:'LocalIpcError'},startedAt:new Date(1000).toISOString(),finishedAt:new Date(2000).toISOString()}
const failure={requestKind:'SubmitPrompt',startedAt:1500,error:{code:'attachment_not_found'}}
test('MP-10 explicit pre-dispatch rejection alone permits a retained recovery',()=>{
  const result=classifyTimeWarpContinuation([row],[failure]);assert.deepEqual(result.recover,['2:1'])
})
test('MP-10 every unknown response, mismatched interval, or post-submit seam stays non-replayable',()=>{
  for(const update of [{error:{code:'local_transport_error'}},{startedAt:3000}])
    assert.throws(()=>classifyTimeWarpContinuation([row],[{...failure,...update}]))
  assert.throws(()=>classifyTimeWarpContinuation([{...row,firstFailingSeam:'provider_settlement'}],[failure]))
  assert.throws(()=>classifyTimeWarpContinuation([row],[failure,failure]))
})
test('MP-10 acted failed tasks and judge exclusions are skipped, never resubmitted',()=>{
  const result=classifyTimeWarpContinuation([{...row,promptId:'prompt-1'},{taskId:32,era:1,excluded:true}],[])
  assert.deepEqual([...result.skip],['2:1','32:1']);assert.deepEqual(result.recover,[])
})
