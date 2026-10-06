// MP-08/MP-10: paired raw and prevalence-weighted diagnostic, never a leaderboard score.
import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { assertSameController, assertRuntimeClosed } from './webvoyager-continuation.mjs'
const [directory] = process.argv.slice(2)
const input = JSON.parse(await readFile(`${directory}/SAMPLE_INPUT.json`, 'utf8'))
const campaign = JSON.parse(await readFile(`${directory}/CAMPAIGN.json`, 'utf8'))
const rows = JSON.parse(await readFile(`${directory}/RESULTS.json`, 'utf8'))
assert(campaign.completed && campaign.finishedAt, 'MP-10 incomplete sample; no effect claim')
assertRuntimeClosed(campaign.cleanup)
const harnessRuns = [{ evidence: directory, commit: campaign.harnessCommit }]
if (campaign.continuation) {
  const prior = campaign.continuation
  for (const receipt of prior.priorReceipts) assert.equal(createHash('sha256').update(await readFile(`${prior.priorEvidence}/${receipt.name}`)).digest('hex'), receipt.sha256, 'MP-10 prior receipt mutated')
  assert.equal(createHash('sha256').update(await readFile(`${prior.priorEvidence}/runtime-resources.jsonl`)).digest('hex'), prior.resourceSha256)
  if (prior.cleanupSettlement) assert.equal(createHash('sha256').update(await readFile(prior.cleanupSettlement.file)).digest('hex'), prior.cleanupSettlement.sha256)
  const priorCampaign = JSON.parse(await readFile(`${prior.priorEvidence}/CAMPAIGN.json`, 'utf8'))
  const priorRows = JSON.parse(await readFile(`${prior.priorEvidence}/RESULTS.json`, 'utf8'))
  assert.equal(priorRows.length, prior.priorSettled)
  assert.deepEqual(rows.slice(0, prior.priorSettled), priorRows, 'MP-10 prior attempts changed or rescored')
  harnessRuns.unshift({ evidence: prior.priorEvidence, commit: priorCampaign.harnessCommit, settled: prior.priorSettled })
}
assert.equal(rows.length, 60); assert.equal(new Set(rows.map(row => row.taskId)).size, 60)
const original = await readFile(`${input.baseline}/RESULTS.json`)
assert.equal(createHash('sha256').update(original).digest('hex'), input.baselineResultsSha256, 'MP-10 baseline mutated')
const win = row => !!(row.harnessValid && row.judgeValid && row.judgeVerdict === 'SUCCESS')
const paired = input.sample.map(sample => {
  const row = rows.find(row => row.taskId === sample.taskId)
  assert(row, 'MP-10 missing selected task'); assertSameController(input.source, row.source)
  for (const [key, value] of Object.entries({ model: 'gpt-6.1-sol', effort: 'high', judgeModel: 'gpt-6.1-sol', judgeEffort: 'low',
    maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000, promptVariant: 'recovery-v1' })) assert.equal(row[key], value)
  const prior = campaign.continuation, root = prior && rows.indexOf(row) < prior.priorSettled ? prior.priorEvidence : directory
  return { taskId: row.taskId, stratum: sample.stratum, weight: sample.population / sample.selectedInStratum,
    attemptEvidence: `${root}/attempts/${row.taskId}`,
    baselineWin: sample.baselineWin, afterWin: win(row), baselineValid: sample.baselineValid, afterValid: !!(row.harnessValid && row.judgeValid),
    firstFailingSeam: row.firstFailingSeam ?? null }
})
const summarize = values => ({ tasks: values.length, beforeWins: values.filter(p => p.baselineWin).length,
  afterWins: values.filter(p => p.afterWin).length, beforeValid: values.filter(p => p.baselineValid).length,
  afterValid: values.filter(p => p.afterValid).length, gained: values.filter(p => !p.baselineWin && p.afterWin).length,
  lost: values.filter(p => p.baselineWin && !p.afterWin).length })
const baselineRows = JSON.parse(original)
const strata = [...new Set(paired.map(p => p.stratum))].map(stratum => ({ stratum, ...summarize(paired.filter(p => p.stratum === stratum)) }))
const resources = (await readFile(`${directory}/runtime-resources.jsonl`, 'utf8')).trim().split('\n').map(JSON.parse)
const weightedBefore = paired.reduce((sum, p) => sum + p.weight * Number(p.baselineWin), 0)
const weightedAfter = paired.reduce((sum, p) => sum + p.weight * Number(p.afterWin), 0)
const result = { mpItems: ['MP-08', 'MP-10', 'MP-11'], harnessRuns, continuation: campaign.continuation ?? null,
  baselineResultsSha256: input.baselineResultsSha256, raw: summarize(paired), strata, paired,
  weighted: { eligiblePopulation: 632, beforeEstimatedWins: weightedBefore, afterEstimatedWins: weightedAfter,
    beforeRate: weightedBefore / 632, afterRate: weightedAfter / 632, deltaPercentagePoints: (weightedAfter - weightedBefore) / 632 * 100,
    limitation: 'Stratified diagnostic point estimate; rare strata have one observation. No precise full-campaign inference or leaderboard claim.' },
  resourceSamples: resources.length, minimumMemAvailableGiB: Math.min(...resources.map(r => r.memAvailable)) / 1024 ** 3,
  minimumDiskAvailableGiB: Math.min(...resources.map(r => r.diskAvailable)) / 1024 ** 3,
  finalCaptureRetries: rows.filter(row => row.finalCapture?.attempts > 1).length, cleanup: campaign.cleanup }
result.solverWallSeconds = { before: input.sample.reduce((sum, sample) => sum + (baselineRows.find(row => row.taskId === sample.taskId).providerWallSeconds ?? 0), 0), after: rows.reduce((sum, row) => sum + (row.providerWallSeconds ?? 0), 0) }
assert(Math.abs(paired.reduce((sum, p) => sum + p.weight, 0) - 632) < 1e-6, 'MP-10 malformed prevalence weights')
assert(Math.abs(weightedBefore - 179) < 1e-6, 'MP-10 weighted baseline changed')
assert(result.minimumMemAvailableGiB >= 16 && result.minimumDiskAvailableGiB >= 10)
await writeFile(`${directory}/COMPARISON.json`, JSON.stringify(result, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
console.log(JSON.stringify({ mpItems: result.mpItems, raw: result.raw, weighted: result.weighted,
  minimumMemAvailableGiB: result.minimumMemAvailableGiB, minimumDiskAvailableGiB: result.minimumDiskAvailableGiB, cleanup: result.cleanup }, null, 2))
