// MP-08/MP-10/MP-11: independent cleanup proof permits untouched tasks only.
import assert from 'node:assert/strict'
import { readFile, access } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import path from 'node:path'
const exec = promisify(execFile)
const hash = bytes => createHash('sha256').update(bytes).digest('hex')
const sorted = values => [...values].sort()

export function assertCleanupSettlement({ proof, priorEvidence, runtime, rows, hashes }) {
  assert.equal(path.resolve(proof.priorEvidence), path.resolve(priorEvidence))
  assert.deepEqual(proof.priorReceipts, hashes, 'MP-10 original receipt changed')
  assert.equal(proof.root, runtime.root); assert.equal(proof.workspace, runtime.workspace)
  assert.equal(proof.providerSubmissions, 0, 'MP-10 cleanup cannot execute a solver')
  assert(proof.stateRemoved && proof.workspaceRemoved && proof.processes?.length
    && proof.processes.every(p => Number.isSafeInteger(p.pid) && p.pid > 1 && p.stopped), 'MP-11 incomplete cleanup settlement')
  assert.deepEqual(sorted(proof.settledRooms), sorted(runtime.cleanup.unresolvedRooms))
  assert.deepEqual(sorted(proof.settledSlices), sorted(runtime.cleanup.unresolvedSlices))
  const failed = rows.filter(row => !row.excluded && !row.cleanupValid)
  assert(failed.length, 'MP-11 cleanup settlement has no failed ownership')
  assert.deepEqual(sorted(proof.settledTaskIds), sorted(failed.map(row => row.taskId)))
  for (const row of failed) {
    assert(row.turnLifecycle === 'completed' && !row.providerError && !row.cancelFailed
      && !row.judgeFailure?.cleanupFailed, 'MP-11 external provider ownership remains unsettled')
    assert(row.cleanup?.sessionGone === true || proof.settledRooms.includes(row.owned?.sessionId))
    assert(row.owned?.slices.length && row.owned.slices.every(s => proof.settledSlices.includes(s.id)
      || row.cleanup?.slices?.some(closed => closed.sliceId === s.id && closed.gone === true)))
  }
  return new Set(proof.settledTaskIds)
}

export async function verifyCleanupSettlement({ file, priorEvidence, bytes, rows }) {
  assert(path.isAbsolute(file), 'MP-11 absolute cleanup evidence required')
  const proofBytes = await readFile(file), proof = JSON.parse(proofBytes)
  const runtimeBytes = await readFile(`${priorEvidence}/runtime.json`), runtime = JSON.parse(runtimeBytes)
  const hashes = ['CAMPAIGN.json', 'RESULTS.json', 'FULL_SELECTION.json'].map((name, i) => ({ name, sha256: hash(bytes[i]) }))
  hashes.push({ name: 'runtime.json', sha256: hash(runtimeBytes) })
  const settled = assertCleanupSettlement({ proof, priorEvidence, runtime, rows, hashes })
  for (const location of [runtime.root, runtime.workspace]) {
    assert(path.isAbsolute(location))
    await access(location).then(() => { throw Error('MP-11 retained state remains') }, error => assert.equal(error.code, 'ENOENT'))
  }
  for (const pid of [...runtime.ownedPids, ...proof.processes.map(p => p.pid)]) {
    assert(Number.isSafeInteger(pid) && pid > 1)
    await access(`/proc/${pid}`).then(() => { throw Error('MP-11 owned process remains') }, error => assert.equal(error.code, 'ENOENT'))
  }
  for (const owner of new Set(rows.map(row => row.slice?.ownerKernelId).filter(Boolean))) {
    assert.match(owner, /^kernel-[a-f0-9]+$/)
    for (const args of [['ps', '-aq'], ['volume', 'ls', '-q']]) {
      assert.equal((await exec('docker', [...args, '--filter', `label=io.chariox.slice.owner-kernel-id=${owner}`], { timeout: 60000 })).stdout.trim(), '', 'MP-11 physical resources remain')
    }
  }
  return { settled, source: proof.source, provenance: { file, sha256: hash(proofBytes), settledTaskIds: proof.settledTaskIds,
    originalFailuresRetained: true, solverSubmissions: 0 } }
}
