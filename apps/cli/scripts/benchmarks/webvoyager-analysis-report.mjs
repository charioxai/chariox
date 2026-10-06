// MP-08/MP-10: immutable per-task taxonomy and deterministic paired sample manifest.
import assert from 'node:assert/strict'
import { readFile, writeFile, mkdir, stat } from 'node:fs/promises'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { classifyFailure, categories, stratifiedSample } from './webvoyager-analysis.mjs'
const [baseline, evidence] = process.argv.slice(2)
assert(path.isAbsolute(baseline) && path.isAbsolute(evidence) && path.resolve(baseline) !== path.resolve(evidence))
const bytes = await readFile(`${baseline}/RESULTS.json`), rows = JSON.parse(bytes)
const selectionBytes = await readFile(`${baseline}/FULL_SELECTION.json`), selection = JSON.parse(selectionBytes)
assert.equal(rows.length, 643); assert.equal(rows.filter(r => !r.excluded).length, 632)
const campaign = JSON.parse(await readFile(`${baseline}/CAMPAIGN.json`, 'utf8'))
assert.equal(campaign.wins, 179); assert.equal(campaign.valid, 615); assert.equal(campaign.invalid, 17)
const roots = []
let current = baseline
for (;;) {
  roots.push(current)
  const report = JSON.parse(await readFile(`${current}/CAMPAIGN.json`, 'utf8'))
  if (!report.continuation?.priorEvidence) break
  current = report.continuation.priorEvidence
  assert(!roots.includes(current), 'MP-10 circular continuation')
}
const failures = []
for (const row of rows) {
  const classification = classifyFailure(row)
  if (!classification) continue
  let directory
  for (const root of roots) {
    const candidate = `${root}/attempts/${row.taskId}`
    if (await stat(`${candidate}/row.json`).then(() => true, () => false)) { directory = candidate; break }
  }
  assert(directory, `MP-10 missing original receipt for ${row.taskId}`)
  const original = JSON.parse(await readFile(`${directory}/row.json`, 'utf8'))
  assert.equal(original.runId, row.runId)
  failures.push({ taskId: row.taskId, ...classification, valid: !!(row.harnessValid && row.judgeValid),
    originalDirectory: directory, rowSha256: createHash('sha256').update(await readFile(`${directory}/row.json`)).digest('hex'),
    answer: row.answer ?? null, judgeAnswer: row.judgeAnswer ?? null, firstFailingSeam: row.firstFailingSeam ?? null,
    toolTrace: row.toolTrace ?? [], actions: row.actions ?? [],
    screenshots: (row.screenshots ?? []).map(name => `${directory}/${name}`) })
}
assert.equal(failures.length, 453)
await mkdir(evidence, { recursive: true, mode: 0o700 })
const sample = stratifiedSample(rows)
const manifest = { mpItems: ['MP-08', 'MP-10', 'MP-11'], seed: 'MP-08/MP-10-wvanalysis-v1',
  baseline, baselineResultsSha256: createHash('sha256').update(bytes).digest('hex'),
  selectionSha256: createHash('sha256').update(selectionBytes).digest('hex'), source: campaign.source,
  taxonomy: categories.map(category => ({ category, count: failures.filter(f => f.category === category).length })),
  subclasses: [...new Set(failures.map(f => f.subclass))].map(subclass => ({ subclass, count: failures.filter(f => f.subclass === subclass).length })),
  sample: sample.map(s => { const row = rows.find(r => r.taskId === s.taskId); return { ...s, baselineValid: !!(row.harnessValid && row.judgeValid), baselineWin: !!(row.harnessValid && row.judgeValid && row.judgeVerdict === 'SUCCESS'), task: selection.tasks.find(t => t.id === s.taskId) } }),
  interpretation: 'Reported primary seam; screenshots and trajectories sample-reviewed by class. Zero independently established judge disagreements. No rescoring.' }
await writeFile(`${evidence}/FAILURE_TAXONOMY.json`, JSON.stringify(failures, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
await writeFile(`${evidence}/SAMPLE.json`, JSON.stringify(manifest, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
console.log(JSON.stringify({ mpItems: manifest.mpItems, taxonomy: manifest.taxonomy, subclasses: manifest.subclasses,
  sample: sample.length, baselineWins: manifest.sample.filter(s => s.baselineWin).length, baselineValid: manifest.sample.filter(s => s.baselineValid).length,
  sampleStrata: [...new Set(sample.map(s => s.stratum))].map(stratum => ({ stratum, count: sample.filter(s => s.stratum === stratum).length })) }, null, 2))
