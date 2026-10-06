// MP-08/MP-10/MP-11: rejects ambiguous and admitted prompt replays.
import test from 'node:test'
import assert from 'node:assert/strict'
import { classifyTimeWarpContinuation, verifyTimeWarpContinuation } from './timewarp-continuation.mjs'
import { mkdtemp, writeFile, rm } from 'node:fs/promises'
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
for (const errors of [null, '']) test(`MP-08/MP-10/MP-11 clean continuation retains ${errors===null?'absent':'empty'} RPC log`,async()=>{
  const directory=await mkdtemp('/root/.codex/evidence/browser-resume-20260930/r2next/continuation-test-')
  try {
    const rows=[{taskId:1,era:1,promptId:'acted'}]
    const cleanup={stateRemoved:true,workspaceRemoved:true,unresolvedRooms:[],unresolvedSlices:[],processes:[]}
    await writeFile(`${directory}/runtime.json`,JSON.stringify({root:`${directory}/removed-state`,workspace:`${directory}/removed-workspace`,cleanup}))
    await writeFile(`${directory}/CAMPAIGN.json`,JSON.stringify({cleanup:{complete:true,rooms:{sessionGone:true,slices:[]},containerGone:true,volumesGone:true,processes:[]}}))
    await writeFile(`${directory}/RESULTS.json`,JSON.stringify(rows))
    if(errors!==null)await writeFile(`${directory}/rpc-errors.jsonl`,errors)
    const result=await verifyTimeWarpContinuation(directory)
    assert.deepEqual([...result.skip],['1:1']);assert.deepEqual(result.recover,[])
    assert.equal(result.receipt.hashes['rpc-errors.jsonl'],errors===null?null:'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855')
    await writeFile(`${directory}/rpc-errors.jsonl`,'malformed')
    await assert.rejects(verifyTimeWarpContinuation(directory),SyntaxError)
    await rm(`${directory}/RESULTS.json`)
    await assert.rejects(verifyTimeWarpContinuation(directory),{code:'ENOENT'})
  }finally{await rm(directory,{recursive:true})}
})
