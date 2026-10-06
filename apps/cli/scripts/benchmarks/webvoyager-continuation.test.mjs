// MP-08/MP-10/MP-11: later continuations preserve a verified original cleanup failure.
import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdtemp, mkdir, readFile, writeFile, rm, chmod } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { createHash } from 'node:crypto'
import { loadContinuation } from './webvoyager-continuation.mjs'

async function fixture(check) {
  const root = await mkdtemp(`${tmpdir()}/wvanalysis-lineage-`), originalPath = process.env.PATH
  const write = (file, value) => writeFile(file, JSON.stringify(value))
  try {
    const bin = `${root}/bin`; await mkdir(bin)
    await writeFile(`${bin}/docker`, '#!/bin/sh\ncase "$*" in "ps -aq --filter label=io.chariox.slice.owner-kernel-id=kernel-0123456789abcdef"|"volume ls -q --filter label=io.chariox.slice.owner-kernel-id=kernel-0123456789abcdef") exit 0;; *) exit 9;; esac\n')
    await chmod(`${bin}/docker`, 0o700); process.env.PATH = `${bin}:${originalPath}`
    const source = { image: 'fixture-image', runtimeSourceRevision: 'fixture-revision', protocol: '423', clientIpcSha256: 'fixture-client', artifacts: [] }
    const selection = { observationPolicy: 'vision-allowed', tasks: Array.from({ length: 643 }, (_, i) => ({ id: `S--${i}`, ...(i >= 632 ? { excludedReason: 'fixture exclusion' } : {}) })) }
    const first = `${root}/first`, second = `${root}/second`, third = `${root}/third`, proofFile = `${root}/settlement.json`
    for (const directory of [first, second]) await mkdir(`${directory}/admissions`, { recursive: true })
    const excluded = selection.tasks.filter(t => t.excludedReason).map(t => ({ taskId: t.id, excluded: true, excludedReason: t.excludedReason, observationPolicy: 'vision-allowed' }))
    const failed = { taskId: 'S--0', source, observationPolicy: 'vision-allowed', cleanupValid: false, harnessValid: false, judgeValid: true, judgeVerdict: 'SUCCESS', turnLifecycle: 'completed', providerError: false,
      owned: { sessionId: 'room-0', slices: [{ id: 'slice-0' }] }, slice: { ownerKernelId: 'kernel-0123456789abcdef' }, cleanup: { sessionGone: true, slices: [{ sliceId: 'slice-0', gone: true }] } }
    const closed = { stateRemoved: true, workspaceRemoved: true, unresolvedRooms: [], unresolvedSlices: [], processes: [{ pid: 2000000000, stopped: true }] }
    const campaign = { scope: 'round4-full', observationPolicy: 'vision-allowed', promptVariant: 'recovery-vision-v1', finishedAt: 'fixture', completed: false, source, model: 'gpt-6.1-sol', judgeModel: 'gpt-6.1-sol', effort: 'high', judgeEffort: 'low', maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000 }
    const runtime = { root: `${root}/removed-state`, workspace: `${root}/removed-workspace`, ownedPids: [2000000000], source, cleanup: { ...closed, stateRemoved: false, workspaceRemoved: false } }
    await write(`${first}/CAMPAIGN.json`, { ...campaign, settled: 1, cleanup: runtime.cleanup })
    await write(`${first}/RESULTS.json`, [...excluded, failed]); await write(`${first}/runtime.json`, runtime)
    for (const directory of [first, second]) {
      await write(`${directory}/FULL_SELECTION.json`, selection)
      await writeFile(`${directory}/runtime-resources.jsonl`, JSON.stringify({ memAvailable: 20 * 1024 ** 3, diskAvailable: 20 * 1024 ** 3 }) + '\n')
    }
    await write(`${first}/admissions/round4-full-S--0.json`, { taskId: 'S--0', state: 'reserved' })
    const priorReceipts = await Promise.all(['CAMPAIGN.json', 'RESULTS.json', 'FULL_SELECTION.json', 'runtime.json'].map(async name => ({ name, sha256: createHash('sha256').update(await readFile(`${first}/${name}`)).digest('hex') })))
    await write(proofFile, { priorEvidence: first, priorReceipts, root: runtime.root, workspace: runtime.workspace, source, providerSubmissions: 0, ...closed, settledRooms: [], settledSlices: [], settledTaskIds: ['S--0'] })
    const original = await loadContinuation({ priorEvidence: first, newEvidence: second, selection, cleanupSettlement: proofFile, scope: 'round4-full', observationPolicy: 'vision-allowed' })
    const rows = [...original.rows, { taskId: 'S--1', source, observationPolicy: 'vision-allowed', cleanupValid: true }]
    await write(`${second}/RESULTS.json`, rows)
    await write(`${second}/CAMPAIGN.json`, { ...campaign, settled: 2, cleanup: closed, continuation: original.provenance })
    await write(`${second}/admissions/round4-full-S--1.json`, { taskId: 'S--1', state: 'reserved' })
    const load = () => loadContinuation({ priorEvidence: second, newEvidence: third, selection, scope: 'round4-full', observationPolicy: 'vision-allowed' })
    await check({ root, first, second, proofFile, rows, load, write })
  } finally { process.env.PATH = originalPath; await rm(root, { recursive: true, force: true }) }
}

test('MP-08/MP-10/MP-11 carries original settled cleanup across later continuations without replay or requalification', async () => fixture(async ({ rows, load }) => {
  const next = await load()
  assert.deepEqual(next.rows, rows); assert.equal(next.rows[11].cleanupValid, false)
  assert.equal(next.tasks.length, 630); assert(!next.tasks.some(t => ['S--0', 'S--1'].includes(t.id)))
}))

test('MP-08/MP-10/MP-11 rejects changed ancestral cleanup proof, attempts and admissions', async () => fixture(async ({ first, second, proofFile, rows, load, write }) => {
  await load()
  const original = await readFile(proofFile); await writeFile(proofFile, Buffer.concat([original, Buffer.from('\n')]))
  await assert.rejects(load()); await writeFile(proofFile, original)
  await write(`${second}/RESULTS.json`, rows.map(r => r.taskId === 'S--0' ? { ...r, cleanupValid: true } : r))
  await assert.rejects(load()); await write(`${second}/RESULTS.json`, rows)
  await write(`${first}/admissions/round4-full-S--0.json`, { taskId: 'S--0', state: 'changed' })
  await assert.rejects(load())
  await write(`${first}/admissions/round4-full-S--0.json`, { taskId: 'S--0', state: 'reserved' })
  await write(`${first}/admissions/round4-full-S--2.json`, { taskId: 'S--2', state: 'reserved' })
  await assert.rejects(load()); await rm(`${first}/admissions/round4-full-S--2.json`)
  const campaign = JSON.parse(await readFile(`${second}/CAMPAIGN.json`, 'utf8'))
  await write(`${second}/CAMPAIGN.json`, { ...campaign, continuation: { ...campaign.continuation, priorEvidence: second } })
  await assert.rejects(load())
}))
