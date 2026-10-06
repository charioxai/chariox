// MP-08/MP-10: invalid attempts cannot become wins through a successful judge alone.
import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { createHash } from 'node:crypto'
const exec = promisify(execFile)
const evidence = '/root/.codex/evidence/browser-resume-20260930/wvanalysis'
test('MP-10 sample comparison retains invalid rows, counts gains/losses, and rejects source drift or incomplete measurement', async () => {
  await mkdir(evidence, { recursive: true })
  const root = await mkdtemp(`${evidence}/report-test-`), baseline = `${root}/baseline`
  const write = (name, value) => writeFile(`${root}/${name}`, JSON.stringify(value))
  const source = { image: 'image', runtimeSourceRevision: 'revision', protocol: '423', clientIpcSha256: 'client', artifacts: [] }
  const rows = Array.from({ length: 60 }, (_, i) => ({ taskId: `task-${i}`, source,
    harnessValid: i !== 16, judgeValid: true, judgeVerdict: i === 0 ? 'NOT SUCCESS' : i < 18 ? 'SUCCESS' : 'NOT SUCCESS',
    model: 'gpt-6.1-sol', effort: 'high', judgeModel: 'gpt-6.1-sol', judgeEffort: 'low',
    maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000, promptVariant: 'recovery-v1' }))
  const sample = rows.map((row, i) => ({ taskId: row.taskId, stratum: i < 16 ? 'win' : 'failure',
    baselineWin: i < 16, baselineValid: i < 54, population: i < 16 ? 179 : 453, selectedInStratum: i < 16 ? 16 : 44 }))
  const campaign = { completed: true, finishedAt: 'done', cleanup: { stateRemoved: true, workspaceRemoved: true,
    unresolvedRooms: [], unresolvedSlices: [], processes: [{ stopped: true }] } }
  try {
    await mkdir(baseline)
    const original = JSON.stringify(rows)
    await writeFile(`${baseline}/RESULTS.json`, original)
    await write('SAMPLE_INPUT.json', { baseline, baselineResultsSha256: createHash('sha256').update(original).digest('hex'), source, sample })
    await write('CAMPAIGN.json', campaign); await write('RESULTS.json', rows)
    await writeFile(`${root}/runtime-resources.jsonl`, JSON.stringify({ memAvailable: 32 * 1024 ** 3, diskAvailable: 100 * 1024 ** 3 }) + '\n')
    const report = new URL('./webvoyager-sample-report.mjs', import.meta.url).pathname
    await exec('node', [report, root])
    const result = JSON.parse(await readFile(`${root}/COMPARISON.json`, 'utf8'))
    assert.equal(result.raw.afterWins, 16); assert.equal(result.raw.gained, 1); assert.equal(result.raw.lost, 1)
    assert.equal(result.raw.afterValid, 59); assert.equal(result.paired[16].afterWin, false)
    await write('CAMPAIGN.json', { ...campaign, completed: false })
    await assert.rejects(exec('node', [report, root]))
    await write('CAMPAIGN.json', campaign)
    await write('RESULTS.json', rows.map((row, i) => i === 0 ? { ...row, source: { ...source, protocol: '999' } } : row))
    await assert.rejects(exec('node', [report, root]))
    const prior = `${root}/prior`; await mkdir(prior)
    const priorRows = rows.slice(0, 2), priorFiles = {
      'CAMPAIGN.json': { harnessCommit: 'first-harness' }, 'RESULTS.json': priorRows,
      'SAMPLE_INPUT.json': sample, 'runtime.json': { source } }
    const priorReceipts = []
    for (const [name, value] of Object.entries(priorFiles)) {
      const bytes = JSON.stringify(value); await writeFile(`${prior}/${name}`, bytes)
      priorReceipts.push({ name, sha256: createHash('sha256').update(bytes).digest('hex') })
    }
    await writeFile(`${prior}/runtime-resources.jsonl`, 'prior resource receipt')
    await write('CAMPAIGN.json', { ...campaign, harnessCommit: 'continuation-harness', continuation: {
      priorEvidence: prior, priorSettled: 2, priorReceipts, resourceSha256: createHash('sha256').update('prior resource receipt').digest('hex') } })
    await write('RESULTS.json', rows); await rm(`${root}/COMPARISON.json`)
    await exec('node', [report, root])
    const continued = JSON.parse(await readFile(`${root}/COMPARISON.json`, 'utf8'))
    assert.equal(continued.harnessRuns[0].commit, 'first-harness')
    assert.equal(continued.harnessRuns[1].commit, 'continuation-harness')
    assert.equal(continued.paired[0].attemptEvidence, `${prior}/attempts/task-0`)
    await write('RESULTS.json', rows.map((row, i) => i === 0 ? { ...row, judgeVerdict: 'SUCCESS' } : row))
    await assert.rejects(exec('node', [report, root]), /prior attempts changed or rescored/)
  } finally {
    assert(root.startsWith(`${evidence}/report-test-`)); await rm(root, { recursive: true })
  }
})
