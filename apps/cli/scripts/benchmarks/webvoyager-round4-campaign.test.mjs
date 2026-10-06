// MP-08/MP-10/MP-11: execute round4 wiring; quota/auth stop admissions and retain in-flight outcomes.
import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { createHash } from 'node:crypto'
import os from 'node:os'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
const exec = promisify(execFile)
async function fixture(kind) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'wvanalysis-round4-'))
  try {
    const baseline = `${root}/baseline`, upstream = `${root}/upstream`, evidence = `${root}/new`, smokeEvidence = `${root}/smoke`
    await Promise.all([baseline, `${upstream}/data`, smokeEvidence].map(p => mkdir(p, { recursive: true })))
    const dataset = 'fixture\n', source = { image: 'fixture', runtimeSourceRevision: 'fixture', protocol: '423', clientIpcSha256: 'fixture', artifacts: [] }
    const selection = { taskRevision: '5a7896738c10bfb8b9edccce6bb0e0411f8ae569', datasetSha256: createHash('sha256').update(dataset).digest('hex'), tasks: Array.from({ length: 643 }, (_, i) => ({ id: `S--${i}`, ...(i >= 632 ? { excludedReason: 'fixture' } : {}) })) }
    const cleanup = { stateRemoved: true, workspaceRemoved: true, unresolvedRooms: [], unresolvedSlices: [], processes: [{ pid: 99999, stopped: true }] }
    await writeFile(`${upstream}/data/WebVoyager_data.jsonl`, dataset)
    await writeFile(`${baseline}/FULL_SELECTION.json`, JSON.stringify(selection))
    await writeFile(`${baseline}/CAMPAIGN.json`, JSON.stringify({ source }))
    await writeFile(`${baseline}/RESULTS.json`, JSON.stringify(selection.tasks.map(t => ({ taskId: t.id, ...(t.excludedReason ? { excluded: true, excludedReason: t.excludedReason } : { harnessValid: true, judgeValid: true, judgeVerdict: 'NOT SUCCESS' }) }))))
    const priorEvidence = `${root}/prior`
    let priorRows
    if (kind.startsWith('continuation')) {
      await mkdir(`${priorEvidence}/admissions`, { recursive: true })
      const taggedSelection = { ...selection, observationPolicy: 'vision-allowed', mpItems: ['MP-08', 'MP-10'] }
      priorRows = [...selection.tasks.filter(t => t.excludedReason).map(t => ({ taskId: t.id, excluded: true, excludedReason: t.excludedReason, observationPolicy: 'vision-allowed' })),
        { taskId: 'S--0', source, observationPolicy: 'vision-allowed', cleanupValid: true, harnessValid: false, judgeValid: false, firstFailingSeam: 'official_prompt_judge', providerUsageExhausted: kind === 'continuation-quota', judgeFailure: { abort: 'deadline', unauthorized: kind === 'continuation-auth' } }]
      await writeFile(`${priorEvidence}/FULL_SELECTION.json`, JSON.stringify(taggedSelection))
      await writeFile(`${priorEvidence}/RESULTS.json`, JSON.stringify(priorRows))
      await writeFile(`${priorEvidence}/CAMPAIGN.json`, JSON.stringify({ scope: 'round4-full', observationPolicy: kind === 'continuation-policy' ? 'text-only' : 'vision-allowed', promptVariant: 'recovery-vision-v1', finishedAt: 'fixture', completed: false, settled: 1, source, cleanup: { ...cleanup, stateRemoved: kind !== 'continuation-cleanup' }, harnessCommit: 'fixture-prior',
        model: 'gpt-6.1-sol', judgeModel: 'gpt-6.1-sol', effort: 'high', judgeEffort: 'low', maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000 }))
      await writeFile(`${priorEvidence}/runtime-resources.jsonl`, JSON.stringify({ memAvailable: 20 * 1024 ** 3, diskAvailable: 20 * 1024 ** 3 }) + '\n')
      await writeFile(`${priorEvidence}/admissions/round4-full-S--0.json`, JSON.stringify({ taskId: 'S--0', state: 'reserved' }))
      if (kind === 'continuation-ghost') await writeFile(`${priorEvidence}/admissions/round4-full-S--1.json`, JSON.stringify({ taskId: 'S--1', state: 'reserved' }))
    }
    await writeFile(`${smokeEvidence}/CAMPAIGN.json`, JSON.stringify({ completed: kind !== 'incomplete-smoke', settled: 10, source, observationPolicy: 'vision-allowed', cleanup }))
    const smoke = kind === 'smoke'
    const options = { runtime: { laneName: 'wvanalysis', sandboxCompatibility: false, evidence }, rooms: 2, promptVariant: 'recovery-vision-v1', scope: smoke ? 'round4-smoke' : 'round4-full', baseline, upstream, smokeEvidence, smokeTaskIds: selection.tasks.slice(0, 10).map(t => t.id), ...(kind.startsWith('continuation') ? { priorEvidence } : {}) }
    await writeFile(`${root}/options.json`, JSON.stringify(options))
    const calls = [], context = vm.createContext({ JSON, console: { log() {} }, process: { argv: ['node', 'fixture', `${root}/options.json`], on() {}, off() {}, exitCode: 0 } })
    const modules = new Map(), synthetic = (key, values) => {
      if (!modules.has(key)) modules.set(key, new vm.SyntheticModule(Object.keys(values), function () { for (const [name, value] of Object.entries(values)) this.setExport(name, value) }, { context }))
      return modules.get(key)
    }
    let closed = false
    const coordinator = new vm.SourceTextModule(await readFile(new URL('./webvoyager-round4.mjs', import.meta.url), 'utf8'), { context })
    await coordinator.link(async specifier => {
      if (specifier === './round2/runtime.mjs') return synthetic(specifier, { startOwnedRuntime: async () => ({ source, guard: async () => {}, close: async () => { closed = true; return cleanup } }) })
      if (specifier === './webvoyager-task.mjs') return synthetic(specifier, { runWebVoyagerTask: async ({ task }) => {
        calls.push(task.id)
        const good = { taskId: task.id, source, observationPolicy: 'vision-allowed', cleanupValid: true, turnLifecycle: 'completed', harnessValid: true, judgeValid: true, judgeVerdict: 'NOT SUCCESS' }
        if (['quota', 'auth', 'judge-quota', 'judge-auth'].includes(kind)) {
          if (calls.length === 1) return { ...good, harnessValid: false, providerUsageExhausted: kind === 'quota', providerUnauthorized: kind === 'auth', ...(kind.startsWith('judge-') ? { judgeFailure: { usageExhausted: kind === 'judge-quota', unauthorized: kind === 'judge-auth' } } : {}) }
          await new Promise(r => setTimeout(r, 10))
        }
        return good
      } })
      return synthetic(specifier, await import(specifier.startsWith('.') ? new URL(specifier, import.meta.url) : specifier))
    })
    if (['incomplete-smoke', 'continuation-quota', 'continuation-auth', 'continuation-policy', 'continuation-cleanup', 'continuation-ghost'].includes(kind)) { await assert.rejects(coordinator.evaluate()); assert.deepEqual(calls, []); return }
    await coordinator.evaluate()
    const campaign = JSON.parse(await readFile(`${evidence}/CAMPAIGN.json`, 'utf8'))
    const rows = JSON.parse(await readFile(`${evidence}/RESULTS.json`, 'utf8'))
    assert(closed); assert.equal(campaign.observationPolicy, 'vision-allowed')
    assert(rows.every(r => r.observationPolicy === 'vision-allowed'))
    if (['quota', 'auth', 'judge-quota', 'judge-auth'].includes(kind)) {
      assert.deepEqual(calls, ['S--0', 'S--1']); assert.equal(campaign.completed, false)
      assert.equal(campaign.settled, 2); assert.equal(campaign.invalid, 1); assert.equal(context.process.exitCode, 1)
      assert.equal(campaign.stopPoint.taskId, 'S--0')
      assert(campaign.stopPoint[kind.endsWith('quota') ? 'providerUsageExhausted' : 'providerUnauthorized'])
    } else {
      if (kind.startsWith('continuation')) {
        assert(!calls.includes('S--0')); assert.equal(calls.length, 631)
        assert.deepEqual(rows.slice(0, priorRows.length), priorRows)
        assert.equal(campaign.invalid, 1); assert.equal(campaign.continuation.priorSettled, 1)
        await writeFile(`${evidence}/runtime-resources.jsonl`, JSON.stringify({ memAvailable: 24 * 1024 ** 3, diskAvailable: 30 * 1024 ** 3 }) + '\n')
        await exec(process.execPath, [fileURLToPath(new URL('./webvoyager-round4-report.mjs', import.meta.url)), evidence])
        const comparison = JSON.parse(await readFile(`${evidence}/COMPARISON.json`, 'utf8'))
        assert.equal(comparison.round4VisionAllowed.tasks, 632); assert.equal(comparison.round4VisionAllowed.invalid, 1)
        assert.equal(comparison.harnessRuns.length, 2); assert.equal(comparison.harnessRuns[0].commit, 'fixture-prior')
        assert.equal(comparison.resources.minimumMemAvailableGiB, 20)
      }
      assert.equal(campaign.completed, true); assert.equal(campaign.settled, smoke ? 10 : 632)
      assert.equal(rows.length, smoke ? 10 : 643)
    }
  } finally { await rm(root, { recursive: true }) }
}
if (process.argv[2] === '--fixture') await fixture(process.argv[3])
else for (const kind of ['smoke', 'full', 'continuation', 'continuation-quota', 'continuation-auth', 'continuation-policy', 'continuation-cleanup', 'continuation-ghost', 'quota', 'auth', 'judge-quota', 'judge-auth', 'incomplete-smoke']) test(`MP-08/MP-10/MP-11 round4 vision-allowed campaign ${kind}`, async () => {
  await exec(process.execPath, ['--experimental-vm-modules', fileURLToPath(import.meta.url), '--fixture', kind], { maxBuffer: 65536 })
})
