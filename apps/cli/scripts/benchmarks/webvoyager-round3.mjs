// MP-08/MP-10/MP-11: same frozen selection, HIGH solver, LOW approved substitute judge.
import assert from 'node:assert/strict'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { startOwnedRuntime } from './round2/runtime.mjs'
import { runWebVoyagerTask } from './webvoyager-task.mjs'
import { requiresCampaignPause } from './round2/provider-availability.mjs'
import { loadContinuation, assertSameController } from './webvoyager-continuation.mjs'
const options = JSON.parse(await readFile(process.argv[2], 'utf8'))
const evidence = options.runtime.evidence
assert.equal(options.runtime.sandboxCompatibility, false)
assert(options.rooms === 1 || options.rooms === 2)
await mkdir(`${evidence}/attempts`, { recursive: true, mode: 0o700 })
const selection = JSON.parse(await readFile(options.selection, 'utf8'))
assert.equal(selection.tasks.length, 643)
assert.equal(selection.taskRevision, '5a7896738c10bfb8b9edccce6bb0e0411f8ae569')
const dataHash = createHash('sha256').update(await readFile(`${options.upstream}/data/WebVoyager_data.jsonl`)).digest('hex')
assert.equal(dataHash, selection.datasetSha256)
const included = selection.tasks.filter(task => !task.excludedReason)
assert.equal(included.length, 632)
const prior = options.priorEvidence ? await loadContinuation({ priorEvidence: options.priorEvidence, newEvidence: evidence, selection, cleanupSettlement: options.cleanupSettlement }) : null
const tasks = prior?.tasks ?? included
const currentTasks = new Set(tasks.map(task => task.id))
await writeFile(`${evidence}/FULL_SELECTION.json`, JSON.stringify(selection, null, 2) + '\n', { flag: 'wx' })
const rows = prior?.rows ?? selection.tasks.filter(task => task.excludedReason).map(task => ({ mpItems: ['MP-08', 'MP-10'],
  benchmark: 'WebVoyager', scope: 'round3-full', taskId: task.id, excluded: true, excludedReason: task.excludedReason }))
const report = { mpItems: ['MP-08', 'MP-10', 'MP-11'], source: null, startedAt: new Date().toISOString(),
  benchmark: 'WebVoyager', total: 643, included: 632, excluded: 11, effort: 'high', judgeEffort: 'low',
  model: 'gpt-6.1-sol', judgeModel: 'gpt-6.1-sol', maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000,
  judgeSubstitution: 'Same approved product-linked gpt-6.1-sol CLI and official prompts as round1; no OpenAI API key.',
  screenshotPolicy: 'Every5s passive capture; original transient errors retained without aborting provider; final capture strict.',
  completed: false, continuation: prior?.provenance ?? null }
let next = 0, interrupted = false, runtime, writes = Promise.resolve()
const interrupt = () => { interrupted = true }
process.on('SIGTERM', interrupt); process.on('SIGINT', interrupt)
const save = () => (writes = writes.then(async () => {
  const valid = rows.filter(row => !row.excluded && row.harnessValid && row.judgeValid)
  report.settled = rows.filter(row => !row.excluded).length; report.valid = valid.length
  report.invalid = report.settled - report.valid; report.wins = valid.filter(row => row.judgeVerdict === 'SUCCESS').length
  await writeFile(`${evidence}/RESULTS.json`, JSON.stringify(rows, null, 2) + '\n')
  await writeFile(`${evidence}/CAMPAIGN.json`, JSON.stringify(report, null, 2) + '\n')
}))
async function worker() {
  try {
    while (next < tasks.length && !interrupted) {
      const task = tasks[next++]; await runtime.guard()
      const row = await runWebVoyagerTask({ task, runtime, options })
      const pauseAdmission = requiresCampaignPause(row)
      if (pauseAdmission) interrupted = true
      rows.push(row); await save()
      console.log(JSON.stringify({ mpItems: report.mpItems, taskId: task.id, settled: report.settled,
        valid: report.valid, invalid: report.invalid, wins: report.wins, firstFailingSeam: row.firstFailingSeam }))
      if (pauseAdmission) throw Error('MP-08/MP-10 provider availability or owned cleanup requires attention')
    }
  } catch (error) { interrupted = true; throw error }
}
try {
  runtime = await startOwnedRuntime(options.runtime); report.source = runtime.source
  if (prior) assertSameController(prior.source, runtime.source)
  const results = await Promise.allSettled(Array.from({ length: options.rooms }, worker))
  const rejected = results.find(result => result.status === 'rejected')
  if (rejected) throw rejected.reason
  assert(!interrupted && rows.length === 643, 'MP-10 incomplete campaign')
  report.completed = true
} catch (error) {
  report.failureClass = error.name; process.exitCode = 1
  console.log('MP-08/MP-10 WebVoyager campaign RED; original attempt receipts retained')
} finally {
  if (runtime) report.cleanup = await runtime.close({ preserveState: rows.some(row => currentTasks.has(row.taskId) && (!row.cleanupValid || row.judgeFailure?.cleanupFailed)) })
  report.finishedAt = new Date().toISOString(); await save()
  process.off('SIGTERM', interrupt); process.off('SIGINT', interrupt)
}
