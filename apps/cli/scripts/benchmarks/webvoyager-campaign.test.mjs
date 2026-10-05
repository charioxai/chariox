// MP-08/MP-10/MP-11: run the actual coordinator with isolated, synthetic kernel/provider seams.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { readFile, writeFile, mkdir, mkdtemp, rm } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import os from 'node:os'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
const exec = promisify(execFile)

async function fixture(kind) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'r2next-wv-campaign-'))
  try {
    const evidence = `${root}/new`, prior = `${root}/prior`, upstream = `${root}/upstream`
    await Promise.all([evidence, prior, `${prior}/admissions`, `${upstream}/data`].map(p => mkdir(p, { recursive: true })))
    const dataset = 'synthetic campaign data\n'
    await writeFile(`${upstream}/data/WebVoyager_data.jsonl`, dataset)
    const selection = { taskRevision: '5a7896738c10bfb8b9edccce6bb0e0411f8ae569',
      datasetSha256: createHash('sha256').update(dataset).digest('hex'),
      tasks: Array.from({ length: 643 }, (_, i) => ({ id: `fixture--${i}`, ...(i >= 632 ? { excludedReason: 'fixture exclusion' } : {}) })) }
    const source = { image: `sha256:${'1'.repeat(64)}`, runtimeSourceRevision: 'fixture-fingerprint',
      artifacts: [{ name: 'chariox-kernel', sha256: `sha256:${'2'.repeat(64)}` }], protocol: '423', clientIpcSha256: 'fixture-client' }
    const cleanup = { stateRemoved: true, workspaceRemoved: true, unresolvedRooms: [], processes: [{ pid: 99999, stopped: true }] }
    const excluded = selection.tasks.filter(t => t.excludedReason).map(t => ({ taskId: t.id, excluded: true, excludedReason: t.excludedReason }))
    const priorRows = [...excluded, { taskId: 'fixture--0', promptId: 'prior-prompt-0', cleanupValid: true },
      { taskId: 'fixture--1', promptId: 'prior-prompt-1', providerError: true, turnLifecycle: 'failed', cleanupValid: true }]
    const priorCampaign = { completed: false, finishedAt: '2026-10-05T00:00:00Z', settled: 2, source, cleanup,
      model: 'gpt-6.1-sol', judgeModel: 'gpt-6.1-sol', effort: 'high', judgeEffort: 'low', maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000 }
    await writeFile(`${root}/selection.json`, JSON.stringify(selection))
    await writeFile(`${prior}/FULL_SELECTION.json`, JSON.stringify(selection))
    await writeFile(`${prior}/CAMPAIGN.json`, JSON.stringify(priorCampaign))
    await writeFile(`${prior}/RESULTS.json`, JSON.stringify(priorRows))
    await writeFile(`${prior}/admissions/round3-full-fixture--0.json`, JSON.stringify({ taskId: 'fixture--0', state: 'completed' }))
    await writeFile(`${prior}/admissions/round3-full-fixture--1.json`, JSON.stringify({ taskId: 'fixture--1', state: 'reserved' }))
    if (kind === 'ghost-admission') await writeFile(`${prior}/admissions/round3-full-fixture--2.json`, JSON.stringify({ taskId: 'fixture--2', state: 'reserved' }))
    if (kind === 'unsettled-prior') {
      priorCampaign.cleanup = { ...cleanup, stateRemoved: false }
      await writeFile(`${prior}/CAMPAIGN.json`, JSON.stringify(priorCampaign))
    }
    const priorBefore = await readFile(`${prior}/RESULTS.json`, 'utf8')
    const options = { runtime: { evidence, image: source.image, sandboxCompatibility: false }, rooms: kind === 'parallel-pause' ? 2 : 1, upstream, selection: `${root}/selection.json`,
      ...(!['pause', 'parallel-pause'].includes(kind) ? { priorEvidence: prior } : {}) }
    await writeFile(`${root}/options.json`, JSON.stringify(options))
    const calls = [], context = vm.createContext({ JSON, console: { log() {} },
      process: { argv: ['node', 'fixture', `${root}/options.json`], on() {}, off() {}, exitCode: 0 } })
    const modules = new Map()
    const synthetic = (key, values) => {
      if (!modules.has(key)) modules.set(key, new vm.SyntheticModule(Object.keys(values), function () {
        for (const [name, value] of Object.entries(values)) this.setExport(name, value)
      }, { context }))
      return modules.get(key)
    }
    const coordinator = new vm.SourceTextModule(await readFile(new URL('./webvoyager-round3.mjs', import.meta.url), 'utf8'), { context })
    await coordinator.link(async specifier => {
      if (specifier === './round2/runtime.mjs') return synthetic(specifier, { startOwnedRuntime: async () => ({ source: kind === 'controller-change' ? { ...source, image: `sha256:${'3'.repeat(64)}` } : source, guard: async () => {}, close: async options => {
        if (['native-cleanup-failure', 'slice-cleanup-failure'].includes(kind)) {
          assert.equal(options?.preserveState, true, 'MP-11 native or slice ownership remains unsettled')
          return { ...cleanup, stateRemoved: false, workspaceRemoved: false }
        }
        return cleanup
      } }) })
      if (specifier === './webvoyager-task.mjs') return synthetic(specifier, { runWebVoyagerTask: async ({ task }) => {
        calls.push(task.id)
        if (kind === 'native-cleanup-failure') return { taskId: task.id, turnLifecycle: 'completed', cleanupValid: false, judgeFailure: { cleanupFailed: true } }
        if (kind === 'slice-cleanup-failure') return { taskId: task.id, turnLifecycle: 'completed', cleanupValid: false }
        if (kind === 'parallel-pause' && calls.length === 2) return { taskId: task.id, cleanupValid: true, harnessValid: true, judgeValid: true, judgeVerdict: 'NOT SUCCESS', turnLifecycle: 'completed' }
        if (calls.length > 1) throw Error('MP-10 fixture prevents repeated provider-failure storm')
        return { taskId: task.id, providerError: true, turnLifecycle: 'failed', cleanupValid: true, harnessValid: false, judgeValid: false }
      } })
      return synthetic(specifier, await import(specifier.startsWith('.') ? new URL(specifier, import.meta.url) : specifier))
    })
    if (['ghost-admission', 'unsettled-prior'].includes(kind)) {
      await assert.rejects(coordinator.evaluate(), kind === 'ghost-admission' ? /ambiguous admission/ : /cleanup incomplete/)
      assert.deepEqual(calls, [])
      assert.equal(await readFile(`${prior}/RESULTS.json`, 'utf8'), priorBefore)
      return
    }
    await coordinator.evaluate()
    assert.deepEqual(calls, kind === 'controller-change' ? [] : kind === 'parallel-pause' ? ['fixture--0', 'fixture--1'] : [kind !== 'pause' ? 'fixture--2' : 'fixture--0'])
    assert.equal(await readFile(`${prior}/RESULTS.json`, 'utf8'), priorBefore)
    const report = JSON.parse(await readFile(`${evidence}/CAMPAIGN.json`, 'utf8'))
    assert.equal(report.completed, false)
    assert.equal(report.cleanup.stateRemoved, !['native-cleanup-failure', 'slice-cleanup-failure'].includes(kind))
  } finally { await rm(root, { recursive: true }) }
}

if (process.argv[2] === '--fixture') await fixture(process.argv[3])
else for (const kind of ['pause', 'parallel-pause', 'continuation', 'ghost-admission', 'unsettled-prior', 'controller-change', 'native-cleanup-failure', 'slice-cleanup-failure']) test(`MP-08/MP-10/MP-11 actual campaign ${kind} prevents admitted-task replay`, async () => {
  await exec(process.execPath, ['--experimental-vm-modules', fileURLToPath(import.meta.url), '--fixture', kind], { maxBuffer: 65536 })
})
