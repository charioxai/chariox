// MP-08/MP-10/MP-11: resume untouched sample tasks; preserve failed attempts and admission fences.
import assert from 'node:assert/strict'
import { readFile, readdir, access } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import path from 'node:path'
import { assertSameController, assertRuntimeClosed } from './webvoyager-continuation.mjs'
const exec = promisify(execFile)
const hash = bytes => createHash('sha256').update(bytes).digest('hex')
const absent = async location => access(location).then(() => { throw Error('MP-11 owned resource remains') }, error => assert.equal(error.code, 'ENOENT'))

export function validateSampleContinuation({ campaign, rows, priorInput, input, admissions, settlement, runtime, receipts }) {
  assert(campaign.finishedAt && !campaign.completed, 'MP-10 prior campaign must be settled and incomplete')
  assert.deepEqual(priorInput, input, 'MP-10 sample changed')
  assert.equal(rows.length, campaign.settled); assert(rows.length < 60)
  const selected = new Set(input.sample.map(s => s.taskId)), seen = new Set()
  for (const row of rows) {
    assert(selected.has(row.taskId) && !seen.has(row.taskId), 'MP-10 duplicate or foreign prior task')
    seen.add(row.taskId); assertSameController(input.source, row.source)
    for (const [key, value] of Object.entries({ model: 'gpt-6.1-sol', effort: 'high', judgeModel: 'gpt-6.1-sol', judgeEffort: 'low',
      maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000, promptVariant: 'recovery-v1' })) assert.equal(row[key], value)
  }
  assert.deepEqual(new Set(admissions.map(a => a.taskId)), seen, 'MP-10 ambiguous admission; never replay')
  assert.equal(admissions.length, seen.size)
  for (const admission of admissions) {
    assert.equal(admission.scope, 'wvanalysis-60')
    assert.equal(admission.runId, rows.find(r => r.taskId === admission.taskId).runId)
  }
  if (settlement) {
    assert.deepEqual(settlement.priorReceipts, receipts); assert.equal(settlement.root, runtime.root); assert.equal(settlement.workspace, runtime.workspace)
    assert.equal(settlement.providerSubmissions, 0); assert(settlement.originalFailuresRetained)
    assertSameController(input.source, settlement.source)
    assertRuntimeClosed({ ...settlement, unresolvedRooms: [], unresolvedSlices: [] })
    assert.deepEqual(settlement.processScanOwnedMatches, []); assert.equal(settlement.dockerContainersRemaining, 0); assert.equal(settlement.dockerVolumesRemaining, 0)
    assert.deepEqual([...settlement.settledTaskIds].sort(), rows.filter(r => !r.cleanupValid).map(r => r.taskId).sort())
    assert.deepEqual(runtime.cleanup.unresolvedRooms, []); assert.deepEqual(runtime.cleanup.unresolvedSlices, [])
    for (const row of rows) {
      assert(row.turnLifecycle === 'completed' && !row.providerError && !row.cancelFailed && !row.judgeFailure?.cleanupFailed, 'MP-11 provider ownership unsettled')
      assert(row.cleanup.sessionGone && row.cleanup.slices.every(s => s.gone) && row.containerGone && row.volumesGone)
      assert(row.cleanup.errors.every(e => e.seam === 'attachment_detach' && e.errorCode === 'attachment_not_found'), 'MP-11 unproved cleanup failure')
    }
  } else {
    assertRuntimeClosed(campaign.cleanup); assert(rows.every(r => r.cleanupValid))
  }
  return input.sample.filter(s => !seen.has(s.taskId))
}

export async function loadSampleContinuation({ priorEvidence, newEvidence, cleanupSettlement, input }) {
  const prefix = '/root/.codex/evidence/browser-resume-20260930/wvanalysis/'
  assert(path.resolve(priorEvidence).startsWith(prefix) && path.resolve(newEvidence).startsWith(prefix))
  assert.notEqual(path.resolve(priorEvidence), path.resolve(newEvidence))
  const names = ['CAMPAIGN.json', 'RESULTS.json', 'SAMPLE_INPUT.json', 'runtime.json']
  const bytes = await Promise.all(names.map(n => readFile(`${priorEvidence}/${n}`)))
  const [campaign, rows, priorInput, runtime] = bytes.map(b => JSON.parse(b))
  const receipts = names.map((name, i) => ({ name, sha256: hash(bytes[i]) }))
  const files = (await readdir(`${priorEvidence}/admissions`)).sort(), admissions = [], admissionHashes = []
  for (const file of files) {
    const data = await readFile(`${priorEvidence}/admissions/${file}`), admission = JSON.parse(data)
    assert.equal(file, `wvanalysis-60-${encodeURIComponent(admission.taskId)}.json`)
    admissions.push(admission); admissionHashes.push({ file, sha256: hash(data) })
  }
  const proofBytes = cleanupSettlement ? await readFile(cleanupSettlement) : null
  const settlement = proofBytes ? JSON.parse(proofBytes) : null
  if (settlement) assert.equal(path.resolve(settlement.priorEvidence), path.resolve(priorEvidence))
  const tasks = validateSampleContinuation({ campaign, rows, priorInput, input, admissions, settlement, runtime, receipts })
  for (const location of [runtime.root, runtime.workspace]) await absent(location)
  for (const pid of runtime.ownedPids) { assert(Number.isSafeInteger(pid) && pid > 1); await absent(`/proc/${pid}`) }
  for (const owner of new Set(rows.map(r => r.slice?.ownerKernelId).filter(Boolean))) {
    assert.match(owner, /^kernel-[a-f0-9]+$/)
    for (const args of [['ps', '-aq'], ['volume', 'ls', '-q']]) assert.equal((await exec('docker', [...args, '--filter', `label=io.chariox.slice.owner-kernel-id=${owner}`])).stdout.trim(), '')
  }
  const resources = await readFile(`${priorEvidence}/runtime-resources.jsonl`)
  return { rows, tasks, resources, provenance: { priorEvidence, priorReceipts: receipts, admissionHashes,
    resourceSha256: hash(resources), priorSettled: rows.length, unattempted: tasks.length, solverRetries: 0, judgeRetries: 0,
    cleanupSettlement: cleanupSettlement ? { file: cleanupSettlement, sha256: hash(proofBytes) } : null } }
}
