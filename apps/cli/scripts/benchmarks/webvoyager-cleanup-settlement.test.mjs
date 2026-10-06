// MP-08/MP-10/MP-11: cleanup settlement closes ownership only; it cannot replay or requalify a solver.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { assertCleanupSettlement } from './webvoyager-cleanup-settlement.mjs'
const rows=[{taskId:'fixture--0',runId:'fixture-run',owned:{sessionId:'room-0',slices:[{id:'slice-0'}]},cleanupValid:false,turnLifecycle:'completed',providerError:false}]
const runtime={root:'/owned/runtime-0',workspace:'/owned/workspace-0',ownedPids:[10,11],cleanup:{unresolvedRooms:['room-0'],unresolvedSlices:['slice-0']}}
const hashes=[{name:'RESULTS.json',sha256:'original-results'},{name:'CAMPAIGN.json',sha256:'original-campaign'},{name:'FULL_SELECTION.json',sha256:'original-selection'},{name:'runtime.json',sha256:'original-runtime'}]
const proof={priorEvidence:'/evidence/prior',root:runtime.root,workspace:runtime.workspace,providerSubmissions:0,stateRemoved:true,workspaceRemoved:true,processes:[{pid:12,stopped:true},{pid:13,stopped:true}],settledRooms:['room-0'],settledSlices:['slice-0'],settledTaskIds:['fixture--0'],priorReceipts:hashes}
test('MP-11 receipt-bound ownership settlement keeps the original failed row immutable',()=>{
 const before=JSON.stringify(rows);assert.deepEqual([...assertCleanupSettlement({proof,priorEvidence:'/evidence/prior',runtime,rows,hashes})],['fixture--0']);assert.equal(JSON.stringify(rows),before);assert.equal(rows[0].cleanupValid,false)
})
test('MP-11 rejects foreign, incomplete, unbound or provider-executing cleanup proof',()=>{
 for(const patch of [{priorEvidence:'/elsewhere'},{providerSubmissions:1},{stateRemoved:false},{workspaceRemoved:false},{root:'/foreign'},{workspace:'/foreign'},{processes:[{pid:1,stopped:true}]},{processes:[{pid:12,stopped:false}]},{settledTaskIds:['foreign-task']},{settledRooms:['foreign-room']},{settledSlices:['foreign-slice']},{priorReceipts:[]}])assert.throws(()=>assertCleanupSettlement({proof:{...proof,...patch},priorEvidence:'/evidence/prior',runtime,rows,hashes}))
 for(const patch of [{judgeFailure:{cleanupFailed:true}},{cancelFailed:true},{providerError:true},{turnLifecycle:'failed'}])assert.throws(()=>assertCleanupSettlement({proof,priorEvidence:'/evidence/prior',runtime,rows:[{...rows[0],...patch}],hashes}))
})
test('MP-08/MP-10/MP-11 settles attachment-not-found after the original Room and slice were deleted',()=>{
 const closedRuntime={...runtime,cleanup:{unresolvedRooms:[],unresolvedSlices:[]}}
 const closedProof={...proof,settledRooms:[],settledSlices:[]}
 const closedRow={...rows[0],cleanup:{sessionGone:true,attachmentDetached:false,slices:[{sliceId:'slice-0',gone:true}],errors:[{seam:'attachment_detach',errorCode:'attachment_not_found'}],complete:false}}
 const original=JSON.stringify(closedRow)
 const check=row=>assertCleanupSettlement({proof:closedProof,priorEvidence:'/evidence/prior',runtime:closedRuntime,rows:[row],hashes})
 assert.deepEqual([...check(closedRow)],['fixture--0'])
 assert.equal(JSON.stringify(closedRow),original);assert.equal(closedRow.cleanupValid,false)
 for(const cleanup of [{...closedRow.cleanup,sessionGone:false},{...closedRow.cleanup,sessionGone:1},{...closedRow.cleanup,slices:[]},{...closedRow.cleanup,slices:[{sliceId:'foreign-slice',gone:true}]},{...closedRow.cleanup,slices:[{sliceId:'slice-0',gone:false}]}])assert.throws(()=>check({...closedRow,cleanup}))
})
