// MP-08/MP-10/MP-11: paired diagnostic only, same controller/models/budgets/judge, fresh owned Rooms.
import assert from 'node:assert/strict'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { startOwnedRuntime } from './round2/runtime.mjs'
import { runWebVoyagerTask } from './webvoyager-task.mjs'
import { assertSameController } from './webvoyager-continuation.mjs'
import { requiresCampaignPause } from './round2/provider-availability.mjs'
import { loadSampleContinuation } from './webvoyager-sample-continuation.mjs'
const options = JSON.parse(await readFile(process.argv[2], 'utf8'))
const manifest = JSON.parse(await readFile(options.sample, 'utf8'))
assert.equal(options.runtime.laneName, 'wvanalysis'); assert.equal(options.runtime.sandboxCompatibility, false)
assert(options.rooms === 1 || options.rooms === 2); assert.equal(options.promptVariant, 'recovery-v1')
assert.equal(manifest.sample.length, 60); assert.equal(new Set(manifest.sample.map(s => s.taskId)).size, 60)
const selection = JSON.parse(await readFile(`${manifest.baseline}/FULL_SELECTION.json`, 'utf8'))
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')
assert.equal(sha256(await readFile(`${manifest.baseline}/RESULTS.json`)), manifest.baselineResultsSha256)
assert.equal(sha256(await readFile(`${manifest.baseline}/FULL_SELECTION.json`)), manifest.selectionSha256)
assert.equal(sha256(await readFile(`${options.upstream}/data/WebVoyager_data.jsonl`)), selection.datasetSha256)
for (const sample of manifest.sample) {
  assert.deepEqual(sample.task, selection.tasks.find(t => t.id === sample.taskId))
  assert(!sample.task.excludedReason)
}
const evidence = options.runtime.evidence
const continuation = options.priorEvidence ? await loadSampleContinuation({ priorEvidence: options.priorEvidence,
  newEvidence: evidence, cleanupSettlement: options.cleanupSettlement, input: manifest }) : null
const tasks = continuation?.tasks ?? manifest.sample
await mkdir(evidence, { recursive: true, mode: 0o700 })
await writeFile(`${evidence}/SAMPLE_INPUT.json`, JSON.stringify(manifest, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
await mkdir(`${evidence}/attempts`, { recursive: false, mode: 0o700 })
if (continuation) await writeFile(`${evidence}/runtime-resources.jsonl`, continuation.resources, { flag: 'wx', mode: 0o600 })
const report = { mpItems: ['MP-08', 'MP-10', 'MP-11'], benchmark: 'WebVoyager', scope: 'wvanalysis-60',
  harnessCommit: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(), total: 60,
  model: 'gpt-6.1-sol', effort: 'high', judgeModel: 'gpt-6.1-sol', judgeEffort: 'low',
  maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000, promptVariant: options.promptVariant,
  changes: ['Privacy-preserving consent rejection and overlay recovery prompt', 'Up to three fresh passive final screenshot attempts'],
  baselineResultsSha256: manifest.baselineResultsSha256, startedAt: new Date().toISOString(), completed: false }
if (continuation) report.continuation = continuation.provenance
let runtime, next = 0, interrupted = false, writes = Promise.resolve()
const rows = [...(continuation?.rows ?? [])], currentRows = []
const stop = () => { interrupted = true }
process.on('SIGTERM', stop); process.on('SIGINT', stop)
const save = () => (writes = writes.then(async () => {
  const valid = rows.filter(r => r.harnessValid && r.judgeValid)
  Object.assign(report, { settled: rows.length, valid: valid.length, invalid: rows.length - valid.length,
    wins: valid.filter(r => r.judgeVerdict === 'SUCCESS').length,
    baselineWins: manifest.sample.filter(s => s.baselineWin).length, baselineValid: manifest.sample.filter(s => s.baselineValid).length })
  await writeFile(`${evidence}/RESULTS.json`, JSON.stringify(rows, null, 2) + '\n', { mode: 0o600 })
  await writeFile(`${evidence}/CAMPAIGN.json`, JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
}))
async function worker() {
  try {
    while (!interrupted && next < tasks.length) {
      const sample = tasks[next++]; await runtime.guard()
      const row = await runWebVoyagerTask({ task: sample.task, runtime, options })
      Object.assign(row, { stratum: sample.stratum, baselineWin: sample.baselineWin, baselineValid: sample.baselineValid })
      rows.push(row); currentRows.push(row); await save()
      console.log(JSON.stringify({ mpItems: report.mpItems, taskId: row.taskId, settled: report.settled, wins: report.wins, valid: report.valid,
        firstFailingSeam: row.firstFailingSeam, providerUnavailable: !!(row.providerUnauthorized || row.providerUsageExhausted) }))
      if (requiresCampaignPause(row)) { interrupted = true; throw Error('MP-08/MP-10 sample stopped: provider availability/cleanup') }
    }
  } catch (error) { interrupted = true; throw error }
}
try {
  runtime = await startOwnedRuntime(options.runtime); assertSameController(manifest.source, runtime.source)
  report.source = runtime.source; await save()
  const settled = await Promise.allSettled(Array.from({ length: options.rooms }, worker))
  const rejected = settled.find(s => s.status === 'rejected')
  if (rejected) throw rejected.reason
  assert.equal(rows.length, 60); assert(!interrupted)
  report.completed = true
} catch (error) {
  report.failureClass = error.name; report.failure = 'MP-08/MP-10 sample unavailable or incomplete; inspect retained first failing seam'
  process.exitCode = 1
} finally {
  if (runtime) report.cleanup = await runtime.close({ preserveState: currentRows.some(r => !r.cleanupValid || r.judgeFailure?.cleanupFailed) })
  report.finishedAt = new Date().toISOString(); await save()
  process.off('SIGTERM', stop); process.off('SIGINT', stop)
}
