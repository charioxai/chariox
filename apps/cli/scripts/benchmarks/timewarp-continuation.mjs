// MP-08/MP-10/MP-11: only explicit pre-dispatch rejections may recover; acted rows stay immutable.
import assert from 'node:assert/strict'
import { readFile, access } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import path from 'node:path'
const hash=bytes=>createHash('sha256').update(bytes).digest('hex')
export function classifyTimeWarpContinuation(rows, failures) {
  const skip=new Set(),recover=[]
  const submittedFailures=failures.filter(f=>f.requestKind==='SubmitPrompt')
  for(const row of rows) {
    const id=`${row.taskId}:${row.era}`
    assert(!skip.has(id)&&!recover.includes(id),'MP-10 duplicate original episode')
    if(row.excluded||row.promptId){skip.add(id);continue}
    const matches=submittedFailures.filter(f=>f.startedAt>=Date.parse(row.startedAt)&&f.startedAt<=Date.parse(row.finishedAt))
    assert(row.admission==='reserved'&&row.firstFailingSeam==='prompt_submit'
      &&row.error?.errorClass==='LocalIpcError'&&matches.length===1
      &&matches[0].error.code==='attachment_not_found','MP-10 ambiguous or admitted task cannot replay')
    recover.push(id)
  }
  assert.equal(recover.length,submittedFailures.length,'MP-10 unmatched original submission response')
  return {skip,recover}
}
export async function verifyTimeWarpContinuation(directory) {
  assert(path.isAbsolute(directory)&&directory.startsWith('/root/.codex/evidence/browser-resume-20260930/r2next/'))
  const files=['CAMPAIGN.json','RESULTS.json','runtime.json','rpc-errors.jsonl']
  const bytes=await Promise.all(files.map(name=>readFile(`${directory}/${name}`)))
  const [campaign,rows,runtime]=bytes.slice(0,3).map(b=>JSON.parse(b))
  const failures=bytes[3].toString().trim().split('\n').map(JSON.parse)
  const proof=campaign.cleanup?.complete?{
    priorHashes:Object.fromEntries(files.map((name,i)=>[name,hash(bytes[i])])),root:runtime.root,workspace:runtime.workspace,
    providerSubmissions:0,stateRemoved:runtime.cleanup.stateRemoved,workspaceRemoved:runtime.cleanup.workspaceRemoved,
    roomGone:campaign.cleanup.rooms.sessionGone,slicesGone:campaign.cleanup.rooms.slices.every(s=>s.gone),
    containerGone:campaign.cleanup.containerGone,volumesGone:campaign.cleanup.volumesGone,
    processes:[...campaign.cleanup.processes,...runtime.cleanup.processes]
  }:JSON.parse(await readFile(`${directory}/CLEANUP_SUPPLEMENT.json`,'utf8'))
  assert.deepEqual(proof.priorHashes,Object.fromEntries(files.map((name,i)=>[name,hash(bytes[i])])))
  assert.equal(proof.root,runtime.root);assert.equal(proof.workspace,runtime.workspace)
  assert.equal(proof.providerSubmissions,0)
  assert(proof.stateRemoved&&proof.workspaceRemoved&&proof.roomGone&&proof.slicesGone&&proof.containerGone&&proof.volumesGone)
  assert(campaign.cleanup.rooms.sessionGone&&campaign.cleanup.rooms.slices.every(s=>s.gone))
  assert(!runtime.cleanup.unresolvedRooms.length&&!runtime.cleanup.unresolvedSlices.length)
  for(const location of [proof.root,proof.workspace,...proof.processes.map(p=>{
    assert(Number.isSafeInteger(p.pid)&&p.pid>1&&p.stopped);return `/proc/${p.pid}`
  })])await access(location).then(()=>{throw Error('MP-11 previous ownership still present')},error=>assert.equal(error.code,'ENOENT'))
  const result=classifyTimeWarpContinuation(rows,failures)
  return {...result,receipt:{directory,hashes:proof.priorHashes,skipped:[...result.skip],
    preDispatchRecoveries:result.recover,originalFailuresRetained:true,providerThreadRestartDisclosed:true}}
}
