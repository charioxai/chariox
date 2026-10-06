// MP-08/MP-10/MP-11: round4 vision-allowed, frozen controller/budgets/LOW judge; no retries.
import assert from 'node:assert/strict'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { startOwnedRuntime } from './round2/runtime.mjs'
import { runWebVoyagerTask } from './webvoyager-task.mjs'
import { requiresCampaignPause } from './round2/provider-availability.mjs'
import { assertSameController } from './webvoyager-continuation.mjs'

const options = JSON.parse(await readFile(process.argv[2], 'utf8'))
assert.equal(options.promptVariant, 'recovery-vision-v1')
assert.equal(options.runtime.laneName, 'wvanalysis')
assert.equal(options.runtime.sandboxCompatibility, false)
assert(options.rooms === 1 || options.rooms === 2, 'MP-10 round3 concurrency ceiling')
assert(['round4-smoke', 'round4-full'].includes(options.scope))
const baseline = JSON.parse(await readFile(`${options.baseline}/CAMPAIGN.json`, 'utf8'))
const selectionBytes = await readFile(`${options.baseline}/FULL_SELECTION.json`)
const selection = JSON.parse(selectionBytes)
assert.equal(selection.tasks.length, 643)
assert.equal(selection.taskRevision, '5a7896738c10bfb8b9edccce6bb0e0411f8ae569')
assert.equal(createHash('sha256').update(await readFile(`${options.upstream}/data/WebVoyager_data.jsonl`)).digest('hex'), selection.datasetSha256)
const eligible = selection.tasks.filter(task => !task.excludedReason)
assert.equal(eligible.length, 632)
let tasks = eligible
if (options.scope === 'round4-smoke') {
  assert.equal(options.smokeTaskIds.length, 10)
  assert.equal(new Set(options.smokeTaskIds).size, 10)
  tasks = options.smokeTaskIds.map(id => { const task = eligible.find(t => t.id === id); assert(task); return task })
} else {
  const smoke = JSON.parse(await readFile(`${options.smokeEvidence}/CAMPAIGN.json`, 'utf8'))
  assert(smoke.completed && smoke.settled === 10, 'MP-10 complete smoke before full admission')
  assert.equal(smoke.observationPolicy, 'vision-allowed')
  assertSameController(baseline.source, smoke.source)
  assert(smoke.cleanup.stateRemoved && smoke.cleanup.workspaceRemoved && !smoke.cleanup.unresolvedRooms.length)
}
const evidence = options.runtime.evidence
await mkdir(evidence, { recursive: true, mode: 0o700 })
await mkdir(`${evidence}/attempts`, { recursive: false, mode: 0o700 })
await writeFile(`${evidence}/FULL_SELECTION.json`, JSON.stringify({ ...selection, observationPolicy: 'vision-allowed', mpItems: ['MP-08', 'MP-10'] }, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
const rows = options.scope === 'round4-full' ? selection.tasks.filter(t => t.excludedReason).map(t => ({
  mpItems: ['MP-08', 'MP-10'], observationPolicy: 'vision-allowed', round: 4, scope: options.scope,
  taskId: t.id, excluded: true, excludedReason: t.excludedReason })) : []
const report = { mpItems: ['MP-08', 'MP-10', 'MP-11'], observationPolicy: 'vision-allowed', round: 4,
  benchmark: 'WebVoyager', scope: options.scope, harnessCommit: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
  source: null, startedAt: new Date().toISOString(), total: tasks.length, eligible: 632,
  excluded: options.scope === 'round4-full' ? 11 : 0, rooms: options.rooms,
  effort: 'high', judgeEffort: 'low', model: 'gpt-6.1-sol', judgeModel: 'gpt-6.1-sol',
  maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000, promptVariant: options.promptVariant,
  baselineEvidence: options.baseline, baselineObservationPolicy: 'text-only',
  baselineResultsSha256: createHash('sha256').update(await readFile(`${options.baseline}/RESULTS.json`)).digest('hex'),
  selectionSha256: createHash('sha256').update(selectionBytes).digest('hex'),
  judgeSubstitution: 'Unchanged round3 official prompts and product-linked gpt-6.1-sol LOW native CLI judge.',
  completed: false }
let runtime, next = 0, interrupted = false, writes = Promise.resolve()
const stop = () => { interrupted = true }
process.on('SIGTERM', stop); process.on('SIGINT', stop)
const save = () => (writes = writes.then(async () => {
  const settled = rows.filter(r => !r.excluded), valid = settled.filter(r => r.harnessValid && r.judgeValid)
  Object.assign(report, { admitted: next, settled: settled.length, valid: valid.length, invalid: settled.length - valid.length,
    wins: valid.filter(r => r.judgeVerdict === 'SUCCESS').length })
  await writeFile(`${evidence}/RESULTS.json`, JSON.stringify(rows, null, 2) + '\n', { mode: 0o600 })
  await writeFile(`${evidence}/CAMPAIGN.json`, JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
}))
async function worker() {
  try {
    while (!interrupted && next < tasks.length) {
      await runtime.guard()
      if (interrupted || next >= tasks.length) break
      const task = tasks[next++]
      const row = await runWebVoyagerTask({ task, runtime, options })
      if (requiresCampaignPause(row)) {
        interrupted = true
        report.stopPoint ??= { taskId: row.taskId, admitted: next, firstFailingSeam: row.firstFailingSeam ?? null,
          providerUnauthorized: !!(row.providerUnauthorized || row.judgeFailure?.unauthorized), providerUsageExhausted: !!(row.providerUsageExhausted || row.judgeFailure?.usageExhausted),
          judgeFailure: !!row.judgeFailure, cleanupValid: row.cleanupValid }
      }
      rows.push(row); await save()
      console.log(JSON.stringify({ mpItems: report.mpItems, observationPolicy: 'vision-allowed', taskId: task.id,
        settled: report.settled, valid: report.valid, invalid: report.invalid, wins: report.wins,
        screenshotToolCalls: row.toolTrace?.filter(t => t.tool === 'slice_screenshot').length ?? 0,
        firstFailingSeam: row.firstFailingSeam ?? null, providerUnavailable: !!(row.providerUnauthorized || row.providerUsageExhausted || row.judgeFailure?.unauthorized || row.judgeFailure?.usageExhausted) }))
    }
  } catch (error) { interrupted = true; throw error }
}
try {
  runtime = await startOwnedRuntime({ ...options.runtime, observationPolicy: 'vision-allowed' })
  assertSameController(baseline.source, runtime.source); report.source = runtime.source; await save()
  const settled = await Promise.allSettled(Array.from({ length: options.rooms }, worker))
  const rejected = settled.find(r => r.status === 'rejected')
  if (rejected) throw rejected.reason
  assert(!interrupted && rows.filter(r => !r.excluded).length === tasks.length, 'MP-10 campaign incomplete; preserve stop point')
  report.completed = true
} catch (error) {
  report.failureClass = error.name; process.exitCode = 1
  console.log('MP-08/MP-10 round4 vision-allowed RED; original receipts retained')
} finally {
  if (runtime) report.cleanup = await runtime.close({ preserveState: rows.some(r => !r.excluded && (!r.cleanupValid || r.judgeFailure?.cleanupFailed)) })
  report.finishedAt = new Date().toISOString(); await save()
  process.off('SIGTERM', stop); process.off('SIGINT', stop)
}
