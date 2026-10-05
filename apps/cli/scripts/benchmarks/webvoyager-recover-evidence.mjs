// MP-08/MP-10/MP-11: one gated evidence recovery; original solver and scoring receipts remain immutable.
import assert from 'node:assert/strict'
import { readFile, writeFile, mkdir, access, copyFile } from 'node:fs/promises'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { startOwnedRuntime } from './round2/runtime.mjs'
import { runWebVoyagerTask } from './webvoyager-task.mjs'
import { judgeWebVoyager } from './webvoyager-linked-judge.mjs'
import { submitOnce } from './webvoyager-admission.mjs'
import { assertRuntimeClosed, assertSameController } from './webvoyager-continuation.mjs'
const exec = promisify(execFile)
const digest = data => createHash('sha256').update(data).digest('hex')
const json = async file => JSON.parse(await readFile(file, 'utf8'))

function assertBoundary(row) {
  for (const [key, value] of Object.entries({ effort: 'high', judgeEffort: 'low', model: 'gpt-6.1-sol', judgeModel: 'gpt-6.1-sol',
    maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000, sandboxCompatibility: false, cleanupValid: true })) {
    assert.equal(row[key], value, `MP-10 frozen ${key} changed`)
  }
  assert(row.cleanup?.complete && row.cleanup.sessionGone && row.cleanup.attachmentDetached
    && row.cleanup.slices.every(slice => slice.gone) && !row.cleanup.errors.length, 'MP-11 original attempt cleanup incomplete')
  assert(!row.providerUnauthorized, 'MP-08 original account unauthorized')
}

export function assertUnactedRoomFailure(row) {
  assertBoundary(row)
  assert.equal(row.firstFailingSeam, 'room_create')
  assert.equal(row.harnessValid, false); assert.equal(row.judgeValid, false)
  for (const key of ['agentId', 'promptId', 'submissionRequestedAt', 'providerStartedAt', 'turnId', 'turnLifecycle']) {
    assert(!row[key], `MP-10 possible ${key}; never replay`)
  }
  assert(!row.owned?.agentId && !row.providerToolCalls && !row.providerError
    && !row.toolTrace?.length && !row.actions?.length && !row.providerErrors?.length, 'MP-10 possible provider action; never replay')
}

export function assertFailedJudge(row) {
  assertBoundary(row)
  assert.equal(row.firstFailingSeam, 'official_prompt_judge')
  assert.equal(row.harnessValid, true); assert.equal(row.judgeValid, false)
  assert(row.promptId && row.turnLifecycle === 'completed' && !row.providerError && row.answer
    && row.providerToolCalls > 0 && row.providerToolCalls <= 80 && row.mutatingActions <= 15
    && !row.forbiddenTools?.length && row.actions?.length && row.actions.every(action => action.mode === 'browser'), 'MP-10 solver not eligible for unchanged-input judging')
  assert(row.screenshots?.length && row.screenshots.every(name => /^screenshot[1-9]\d*\.png$/.test(name)), 'MP-10 only original post-prompt captures')
}

export async function recoverEvidence({ options, originalEvidence, completedEvidence, taskId, mode }) {
  assert(['preaction', 'judge-only'].includes(mode))
  assert.equal(options.runtime.sandboxCompatibility, false, 'MP-11 unchanged sandbox boundary')
  const originalBytes = await readFile(`${originalEvidence}/RESULTS.json`), originalRows = JSON.parse(originalBytes)
  const original = await json(`${originalEvidence}/CAMPAIGN.json`), complete = await json(`${completedEvidence}/CAMPAIGN.json`)
  assert(original.finishedAt && complete.completed && complete.finishedAt, 'MP-10 complete frozen campaign before recovery')
  assertRuntimeClosed(original.cleanup); assertRuntimeClosed(complete.cleanup)
  assertSameController(original.source, complete.source)
  const matches = originalRows.filter(row => row.taskId === taskId); assert.equal(matches.length, 1)
  const row = matches[0], fullRows = await json(`${completedEvidence}/RESULTS.json`)
  assert.equal(fullRows.length, 643); assert.equal(new Set(fullRows.map(item => item.taskId)).size, 643)
  const fullMatches = fullRows.filter(item => item.taskId === taskId); assert.equal(fullMatches.length, 1)
  assert.deepEqual(fullMatches[0], row, 'MP-10 original attempt was already replaced')
  const selection = await json(`${completedEvidence}/FULL_SELECTION.json`)
  assert.deepEqual(await json(options.selection), selection, 'MP-10 frozen selection changed')
  assert.equal(selection.taskRevision, '5a7896738c10bfb8b9edccce6bb0e0411f8ae569')
  assert.equal(digest(await readFile(`${options.upstream}/data/WebVoyager_data.jsonl`)), selection.datasetSha256)
  const task = selection.tasks.find(item => item.id === taskId); assert(task && !task.excludedReason)
  const proof = { mpItems: ['MP-08', 'MP-10', 'MP-11'], taskId, mode, originalEvidence, completedEvidence,
    originalRunId: row.runId, originalResultsSha256: digest(originalBytes), solverRetries: 0,
    originalReceiptPreserved: true, boundaryWeakened: false, startedAt: new Date().toISOString() }
  if (mode === 'preaction') {
    assertUnactedRoomFailure(row)
    for (const root of new Set([originalEvidence, completedEvidence])) {
      const admission = `${root}/admissions/round3-full-${encodeURIComponent(taskId)}.json`
      await access(admission).then(() => { throw Error('MP-10 prompt reservation exists; never replay') }, error => assert.equal(error.code, 'ENOENT'))
    }
    const owners = new Set(originalRows.map(item => item.slice?.ownerKernelId).filter(Boolean))
    assert.equal(owners.size, 1, 'MP-11 failed slice owner ambiguous')
    const owner = [...owners][0]; assert(row.owned.slices.length)
    proof.physicalInventory = []
    for (const slice of row.owned.slices) {
      assert(slice.name.startsWith(`${row.runId}-`), 'MP-11 original slice ownership mismatch')
      const filters = ['--filter', `label=io.chariox.slice.id=${slice.id}`, '--filter', `label=io.chariox.slice.owner-kernel-id=${owner}`]
      const counts = await Promise.all([['ps', '-aq'], ['volume', 'ls', '-q']].map(async args =>
        (await exec('docker', [...args, ...filters], { timeout: 60000 })).stdout.trim().split('\n').filter(Boolean).length))
      assert.deepEqual(counts, [0, 0], 'MP-11 original physical slice resources remain')
      proof.physicalInventory.push({ sliceId: slice.id, containers: counts[0], volumes: counts[1] })
    }
    proof.originalPromptSubmitted = false
  } else assertFailedJudge(row)
  const evidence = options.runtime.evidence
  assert(evidence.startsWith('/root/.codex/evidence/browser-resume-20260930/r2next/round3/'))
  assert.notEqual(path.resolve(evidence), path.resolve(originalEvidence)); assert.notEqual(path.resolve(evidence), path.resolve(completedEvidence))
  await mkdir(evidence, { recursive: false, mode: 0o700 })
  await writeFile(`${evidence}/ADMISSION.json`, JSON.stringify(proof, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
  let runtime
  try {
    await submitOnce({ directory: '/root/.chariox/dev/browser-resume-20260930/agents/r2next/round3/recovery-admissions',
      scope: `evidence-${mode}`, taskId: `${row.runId}:${taskId}`, runId: path.basename(evidence), submit: async () => {
        runtime = await startOwnedRuntime(options.runtime); assertSameController(original.source, runtime.source)
        proof.recoverySource = runtime.source
        await mkdir(`${evidence}/attempts`, { mode: 0o700 })
        if (mode === 'preaction') proof.row = await runWebVoyagerTask({ task, runtime, options })
        else {
          const directory = `${evidence}/attempts/${taskId}`, origin = `${originalEvidence}/attempts/${taskId}`
          await mkdir(directory, { mode: 0o700 })
          const names = ['answer.txt', ...row.screenshots.slice(-15)]; proof.inputHashes = []
          for (const name of names) {
            const before = await readFile(`${origin}/${name}`)
            if (name === 'answer.txt') assert.equal(before.toString(), row.answer, 'MP-10 original answer bytes changed')
            await copyFile(`${origin}/${name}`, `${directory}/${name}`)
            assert.equal(digest(await readFile(`${directory}/${name}`)), digest(before))
            proof.inputHashes.push({ name, sha256: digest(before) })
          }
          const officialInput = await readFile(`${origin}/judge-input.json`)
          await exec('python3', [path.join(import.meta.dirname, 'webvoyager-judge.py'), `${options.upstream}/evaluation/auto_eval.py`,
            task.ques, `${directory}/answer.txt`, String(Math.min(row.screenshots.length, 15)), `${directory}/judge-input.json`])
          assert.equal(digest(await readFile(`${directory}/judge-input.json`)), digest(officialInput), 'MP-10 original judge prompt changed')
          proof.originalJudgePromptSha256 = digest(officialInput)
          proof.solverSubmissions = 0
          proof.row = { ...row, ...await judgeWebVoyager({ task, directory, screenshots: row.screenshots,
            upstream: options.upstream, root: runtime.root, accountHome: options.runtime.accountHome }) }
        }
      } })
  } catch (error) { proof.failureClass = error.name; if (error.judgeFailure) proof.judgeFailure = error.judgeFailure; throw error }
  finally {
    if (runtime) proof.cleanup = await runtime.close({ preserveState: !!proof.judgeFailure?.cleanupFailed || (!!proof.row && !proof.row.cleanupValid) })
    proof.finishedAt = new Date().toISOString()
    await writeFile(`${evidence}/RECOVERY.json`, JSON.stringify(proof, null, 2) + '\n', { mode: 0o600 })
  }
  assertRuntimeClosed(proof.cleanup)
  assert(proof.row?.harnessValid && proof.row.judgeValid && proof.row.cleanupValid, 'MP-10 recovery remains RED; preserve original and fresh receipts')
  assert.equal(digest(await readFile(`${originalEvidence}/RESULTS.json`)), proof.originalResultsSha256)
  console.log('MP-08/MP-10/MP-11 gated evidence recovery settled; original receipts preserved')
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  await recoverEvidence({ options: await json(process.argv[2]), originalEvidence: process.argv[3],
    completedEvidence: process.argv[4], taskId: process.argv[5], mode: process.argv[6] })
}
