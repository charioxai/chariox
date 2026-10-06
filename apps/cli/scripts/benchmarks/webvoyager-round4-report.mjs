// MP-08/MP-10: separate vision-allowed measured rows from frozen round3 text-only; no rescoring.
import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { pathToFileURL } from 'node:url'
import { classifyFailure, stratum } from './webvoyager-analysis.mjs'
import { assertSameController } from './webvoyager-continuation.mjs'
const win = row => !!(row.harnessValid && row.judgeValid && row.judgeVerdict === 'SUCCESS')
const valid = row => !!(row.harnessValid && row.judgeValid)
const count = rows => ({ tasks: rows.length, wins: rows.filter(win).length, valid: rows.filter(valid).length,
  invalid: rows.filter(row => !valid(row)).length, rate: rows.length ? rows.filter(win).length / rows.length : null })
const site = row => row.taskId.split('--')[0]
const fixes = {
  'reading/extraction error': 'Use fresh protected screenshots for labels/units and targeted text; verify every requested constraint before answering.',
  'navigation failure': 'Inspect the visible overlay, reject optional tracking, refresh field IDs and verify the resulting filter state.',
  'site blocked / anti-bot / captcha': 'Record the exact access blocker; stop at challenges or mandatory login without bypass.',
  'live-site drift (answer changed)': 'Keep frozen-task losses separate; request a separately versioned fresh-task campaign if dates/listings must change.',
  'timeout / step budget': 'Reduce repeated observation/action loops and prioritize requested filters within the unchanged budget.',
  'harness/runtime error': 'Repair the first failing evidence/provider/cleanup seam before a separately disclosed new attempt.',
  'other': 'Review original answer and trajectory to distinguish forbidden state changes from unmet requirements.',
}
export function compareRound4(baseline, rows, eligible = 632) {
  const before = baseline.filter(r => !r.excluded), after = rows.filter(r => !r.excluded)
  assert.equal(new Set(before.map(r => r.taskId)).size, before.length)
  assert.equal(new Set(after.map(r => r.taskId)).size, after.length)
  const byId = new Map(before.map(r => [r.taskId, r]))
  assert(after.every(r => byId.has(r.taskId)), 'MP-10 foreign task')
  const paired = after.map(r => ({ taskId: r.taskId, baselineClass: stratum(byId.get(r.taskId)),
    baselineWin: win(byId.get(r.taskId)), afterWin: win(r), baselineValid: valid(byId.get(r.taskId)), afterValid: valid(r) }))
  const summarize = subset => ({ tasks: subset.length, beforeWins: subset.filter(p => p.baselineWin).length,
    afterWins: subset.filter(p => p.afterWin).length, beforeValid: subset.filter(p => p.baselineValid).length,
    afterValid: subset.filter(p => p.afterValid).length, gained: subset.filter(p => !p.baselineWin && p.afterWin).length,
    lost: subset.filter(p => p.baselineWin && !p.afterWin).length,
    deltaPercentagePoints: subset.length ? (subset.filter(p => p.afterWin).length - subset.filter(p => p.baselineWin).length) / subset.length * 100 : null })
  const losses = after.map(r => ({ taskId: r.taskId, ...classifyFailure(r) })).filter(r => r.category)
  const classes = [...new Set(losses.map(r => r.category))].map(category => ({ category,
    count: losses.filter(r => r.category === category).length, nextFix: fixes[category] }))
    .sort((a, b) => b.count - a.count || a.category.localeCompare(b.category))
  const sites = [...new Set(before.map(site))].sort().map(name => {
    const a = before.filter(r => site(r) === name), b = after.filter(r => site(r) === name)
    return { site: name, round3TextOnly: count(a), round4VisionAllowed: count(b),
      unattempted: a.length - b.length, ...summarize(paired.filter(p => site(p) === name)) }
  })
  return { mpItems: ['MP-08', 'MP-10', 'MP-11'], observationPolicy: 'vision-allowed',
    round3TextOnly: { ...count(before), eligibleRate: before.filter(win).length / eligible },
    round4VisionAllowed: { ...count(after), eligible: eligible, unattempted: eligible - after.length,
      eligibleRate: after.length === eligible ? after.filter(win).length / eligible : null },
    paired: summarize(paired), byBaselineClass: [...new Set(paired.map(p => p.baselineClass))].sort()
      .map(category => ({ category, ...summarize(paired.filter(p => p.baselineClass === category)) })),
    perSite: sites, remainingLossClasses: classes, top3RemainingLossClasses: classes.slice(0, 3),
    limitation: 'Local reproduction with approved substitute LOW native CLI judge; joint prompt/vision/capture changes are not causally isolated. No leaderboard submission. Partial campaigns do not establish an eligible rate.' }
}

async function report(directory) {
  const campaign = JSON.parse(await readFile(`${directory}/CAMPAIGN.json`, 'utf8'))
  assert(campaign.finishedAt, 'MP-10 unsettled campaign')
  assert.equal(campaign.observationPolicy, 'vision-allowed')
  const original = await readFile(`${campaign.baselineEvidence}/RESULTS.json`)
  assert.equal(createHash('sha256').update(original).digest('hex'), campaign.baselineResultsSha256)
  const baselineCampaign = JSON.parse(await readFile(`${campaign.baselineEvidence}/CAMPAIGN.json`, 'utf8'))
  if (campaign.source) assertSameController(baselineCampaign.source, campaign.source)
  const rows = JSON.parse(await readFile(`${directory}/RESULTS.json`, 'utf8'))
  assert(rows.every(row => row.observationPolicy === 'vision-allowed'))
  for (const row of rows.filter(r => !r.excluded)) assertSameController(baselineCampaign.source, row.source)
  const result = { ...compareRound4(JSON.parse(original), rows), scope: campaign.scope,
    completed: campaign.completed, harnessCommit: campaign.harnessCommit, source: campaign.source,
    stopPoint: campaign.stopPoint ?? null, cleanup: campaign.cleanup ?? null,
    providerAvailabilityFailures: rows.filter(r => r.providerUnauthorized || r.providerUsageExhausted || r.judgeFailure?.unauthorized || r.judgeFailure?.usageExhausted).map(r => ({ taskId: r.taskId, firstFailingSeam: r.firstFailingSeam, unauthorized: !!(r.providerUnauthorized || r.judgeFailure?.unauthorized), usageExhausted: !!(r.providerUsageExhausted || r.judgeFailure?.usageExhausted) })) }
  const roots = [directory], harnessRuns = [{ evidence: directory, commit: campaign.harnessCommit }]
  let previous = campaign.continuation
  while (previous) {
    assert(!roots.includes(previous.priorEvidence), 'MP-10 circular continuation')
    for (const receipt of previous.priorReceipts) assert.equal(createHash('sha256').update(await readFile(`${previous.priorEvidence}/${receipt.name}`)).digest('hex'), receipt.sha256, 'MP-10 prior result mutation')
    const priorRows = JSON.parse(await readFile(`${previous.priorEvidence}/RESULTS.json`, 'utf8'))
    assert.deepEqual(rows.slice(0, priorRows.length), priorRows, 'MP-10 prior attempts changed or rescored')
    assert.equal(createHash('sha256').update(await readFile(`${previous.priorEvidence}/runtime-resources.jsonl`)).digest('hex'), previous.resourceSha256)
    if (previous.cleanupSettlement) assert.equal(createHash('sha256').update(await readFile(previous.cleanupSettlement.file)).digest('hex'), previous.cleanupSettlement.sha256)
    const priorCampaign = JSON.parse(await readFile(`${previous.priorEvidence}/CAMPAIGN.json`, 'utf8'))
    assertSameController(baselineCampaign.source, priorCampaign.source)
    roots.push(previous.priorEvidence); harnessRuns.unshift({ evidence: previous.priorEvidence, commit: priorCampaign.harnessCommit, settled: priorCampaign.settled })
    previous = priorCampaign.continuation
  }
  result.harnessRuns = harnessRuns
  const samples = (await Promise.all(roots.map(async root => (await readFile(`${root}/runtime-resources.jsonl`, 'utf8')).trim().split('\n').map(JSON.parse)))).flat()
  result.resources = { samples: samples.length,
    minimumMemAvailableGiB: Math.min(...samples.map(s => s.memAvailable)) / 1024 ** 3,
    minimumDiskAvailableGiB: Math.min(...samples.map(s => s.diskAvailable)) / 1024 ** 3 }
  result.screenshotToolCalls = rows.reduce((sum, r) => sum + (r.toolTrace ?? []).filter(t => t.tool === 'slice_screenshot').length, 0)
  result.finalCaptureRetries = rows.filter(r => r.finalCapture?.attempts > 1).length
  await writeFile(`${directory}/COMPARISON.json`, JSON.stringify(result, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
  console.log(JSON.stringify({ mpItems: result.mpItems, observationPolicy: result.observationPolicy,
    round3TextOnly: result.round3TextOnly, round4VisionAllowed: result.round4VisionAllowed, stopPoint: result.stopPoint }))
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await report(process.argv[2])
