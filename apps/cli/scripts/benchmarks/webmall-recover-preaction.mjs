// MP-08/MP-10/MP-11: one fresh setup after a proven unacted failure; unchanged security and scoring.
import assert from 'node:assert/strict'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { spawn } from 'node:child_process'
import { pathToFileURL, fileURLToPath } from 'node:url'
import { startOwnedRuntime } from './round2/runtime.mjs'
import { stopOwnedProcess } from './round2/owned-processes.mjs'
export function assertUnactedSliceStart(row) {
  assert.equal(row.status, 'RED', 'MP-10 recovery requires original RED')
  assert.equal(row.firstFailingSeam, 'slice_start', 'MP-10 only pre-agent slice-start failure is eligible')
  for (const key of ['promptId', 'providerStartedAt', 'submissionRequestedAt', 'agentId', 'turnId', 'lifecycle']) assert(!row[key], `MP-10 ${key} proves possible provider action; never replay`)
  assert(!row.providerToolCalls && !row.providerUnauthorized)
  assert(row.cleanup.includes('owned_room_deleted') && row.cleanup.includes('owned_slice_deleted'))
}
export async function recover(options, originalEvidence, taskId) {
  assert.equal(options.runtime.sandboxCompatibility, true, 'MP-11 preserve original sandbox boundary')
  const original = JSON.parse(await readFile(`${originalEvidence}/RESULTS.json`, 'utf8'))
  const campaign = JSON.parse(await readFile(`${originalEvidence}/CAMPAIGN.json`, 'utf8'))
  assert(campaign.completed && campaign.finishedAt, 'MP-10 finish original campaign first')
  const matches = original.filter(row => row.taskId === taskId); assert.equal(matches.length, 1)
  const row = matches[0]; assertUnactedSliceStart(row)
  const priorCleanup = JSON.parse(await readFile(`${originalEvidence}/runtime.json`, 'utf8')).cleanup
  assert(priorCleanup.stateRemoved && priorCleanup.workspaceRemoved && !priorCleanup.unresolvedRooms.length && priorCleanup.processes.every(p => p.stopped), 'MP-11 original runtime not settled')
  const manifest = JSON.parse(await readFile(`${originalEvidence}/FULL_MANIFEST.json`, 'utf8'))
  assert(manifest.tasks.includes(taskId)); assert.equal(manifest.tasks.length, 91)
  const proof = JSON.parse(await readFile(`${originalEvidence}/SECOND_INVALID.json`, 'utf8'))
  assert.equal(proof.taskId, taskId); assert.equal(proof.originalRunId, row.runId); assert.equal(proof.physicalOwnedContainerMatches, 0); assert.equal(proof.physicalOwnedVolumeMatches, 0)
  const report = { mpItems: ['MP-08', 'MP-10', 'MP-11'], taskId, originalEvidence,
    originalRunId: row.runId, originalPromptSubmitted: false, startedAt: new Date().toISOString(),
    attempt: 2, actedRetries: 0, boundaryWeakened: false, originalReceiptPreserved: true }
  assert(options.runtime.evidence.startsWith('/root/.codex/evidence/browser-resume-20260930/r2next/round3/'))
  assert.notEqual(options.runtime.evidence, originalEvidence)
  await mkdir(options.runtime.evidence, { recursive: false, mode: 0o700 })
  await writeFile(`${options.runtime.evidence}/ADMISSION.json`, JSON.stringify(report) + '\n', { flag: 'wx', mode: 0o600 })
  let runtime, child
  try {
    runtime = await startOwnedRuntime(options.runtime)
    const fixture = JSON.parse(await readFile(`${options.fixtureEvidence}/sites-readiness.json`, 'utf8'))
    assert.equal(fixture.status, 'ready'); report.source = runtime.source
    assert.equal(runtime.source.image, campaign.runtime.image)
    assert.equal(runtime.source.runtimeSourceRevision, campaign.runtime.runtimeSourceRevision)
    const config = { source: runtime.source.sourceCommit, release: runtime.source.runtimeSourceRevision,
      kernelSha256: runtime.source.artifacts.find(item => item.name === 'chariox-kernel').sha256,
      sliceImage: options.runtime.image, kernelUrl: runtime.kernelUrl, clientRoot: options.runtime.clientRoot,
      workspace: runtime.workspace, accountProfile: runtime.profileId, agentInputs: options.agentInputs,
      siteUrls: `${options.fixtureLane}/site-urls.json`, siteNetwork: fixture.network, outputRoot: `${options.runtime.evidence}/attempts`,
      python: options.python, upstream: options.upstream, scratch: runtime.root, maxSeconds: 600, maxActions: 80,
      leaderboard: options.leaderboard, fixtureLane: options.fixtureLane, fixtureEvidence: options.fixtureEvidence,
      composeBinary: options.composeBinary }
    const path = `${runtime.root}/campaign-options.json`; await writeFile(path, JSON.stringify(config), { mode: 0o600 })
    child = spawn(process.execPath, [fileURLToPath(new URL('./webmall-task.mjs', import.meta.url)), path, taskId],
      { detached: true, stdio: ['ignore', 'inherit', 'ignore'] })
    const exit = new Promise((resolve, reject) => { child.once('exit', resolve); child.once('error', reject) })
    await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject) })
    assert(Number.isSafeInteger(child.pid) && child.pid > 1, 'MP-11 reject unsafe child PID')
    report.exitCode = await exit
    report.row = JSON.parse(await readFile(`${config.outputRoot}/${taskId}/run.json`, 'utf8'))
    report.row.exitCode = report.exitCode; report.row.harnessValid = report.row.status === 'scored'
  } finally {
    try {
      if (child) assert(await stopOwnedProcess(child, { detached: true }), 'MP-11 recovery child did not settle')
    } catch (error) { report.childCleanupFailure = { errorClass: error.name } }
    if (runtime) report.cleanup = await runtime.close({ preserveState: !!report.childCleanupFailure })
    report.finishedAt = new Date().toISOString()
    await writeFile(`${options.runtime.evidence}/RECOVERY.json`, JSON.stringify(report, null, 2) + '\n')
  }
  assert(!report.childCleanupFailure, 'MP-11 recovery child cleanup failed')
  assert(report.row?.harnessValid, 'MP-10 recovery remains RED; preserve both receipts')
  assert(report.cleanup.stateRemoved && report.cleanup.workspaceRemoved && !report.cleanup.unresolvedRooms.length && report.cleanup.processes.every(p => p.stopped), 'MP-11 recovery runtime cleanup incomplete')
  console.log('MP-08/MP-10/MP-11 single pre-action recovery settled; original RED preserved')
}
if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  const options = JSON.parse(await readFile(process.argv[2], 'utf8'))
  await recover(options, process.argv[3], process.argv[4])
}
