// MP-08/MP-10/MP-11: fresh frozen 91-task campaign, at most two owned Rooms.
import assert from 'node:assert/strict'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { startOwnedRuntime } from './round2/runtime.mjs'
import { stopOwnedProcess } from './round2/owned-processes.mjs'

const options = JSON.parse(await readFile(process.argv[2], 'utf8'))
const { evidence } = options.runtime
await mkdir(evidence, { recursive: true, mode: 0o700 })
const tasks = JSON.parse(await readFile(options.agentInputs, 'utf8')).map(task => task.task_id)
assert.equal(tasks.length, 91); assert.equal(new Set(tasks).size, 91)
assert(options.rooms === 1 || options.rooms === 2)
const manifest = { mpItems: ['MP-08', 'MP-10', 'MP-11'], scope: 'round3-full', tasks,
  carriedSmoke: [], effort: 'high', maxSeconds: 600, maxActions: 80, rooms: options.rooms,
  publicInputsSha256: createHash('sha256').update(await readFile(options.agentInputs)).digest('hex') }
assert.equal(manifest.publicInputsSha256, 'cf6b4eaba34ca29943b016055c1708fd2b6c621f5dcaaf7c4d08384b71948164')
await writeFile(`${evidence}/FULL_MANIFEST.json`, JSON.stringify(manifest, null, 2) + '\n', { flag: 'wx' })
const report = { ...manifest, startedAt: new Date().toISOString(), completed: false, attempts: [] }
let runtime, interrupted = false, next = 0
const active = new Set()
const interrupt = () => { interrupted = true }
process.on('SIGTERM', interrupt); process.on('SIGINT', interrupt)
const save = async () => {
  // Mutations are synchronous between awaits; snapshot before writing so two
  // workers cannot overwrite newer results with a stale task list.
  await writeFile(`${evidence}/CAMPAIGN.json`, JSON.stringify(report, null, 2) + '\n')
  const rows = [...report.attempts].sort((a, b) => tasks.indexOf(a.taskId) - tasks.indexOf(b.taskId))
  await writeFile(`${evidence}/RESULTS.json`, JSON.stringify(rows, null, 2) + '\n')
}
let writes = Promise.resolve()
const checkpoint = () => (writes = writes.then(save))
async function worker(configPath, outputRoot) {
  while (next < tasks.length && !interrupted) {
    const taskId = tasks[next++]
    await runtime.guard()
    const started = Date.now()
    const child = spawn(process.execPath, [path.join(import.meta.dirname, 'webmall-task.mjs'), configPath, taskId],
      { detached: true, stdio: ['ignore', 'inherit', 'ignore'] })
    await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject) })
    assert(Number.isSafeInteger(child.pid) && child.pid > 1)
    active.add(child)
    const exitCode = await new Promise(resolve => child.once('exit', resolve))
    active.delete(child)
    const row = JSON.parse(await readFile(`${outputRoot}/${taskId}/run.json`, 'utf8'))
    row.exitCode = exitCode; row.totalWallSeconds = (Date.now() - started) / 1000
    row.harnessValid = row.status === 'scored'
    report.attempts.push(row); await checkpoint()
    console.log(JSON.stringify({ mpItems: manifest.mpItems, taskId, settled: report.attempts.length,
      valid: report.attempts.filter(row => row.harnessValid).length,
      wins: report.attempts.filter(row => row.harnessValid && row.grade?.officialMetrics?.avg_task_completion_rate === 1).length }))
    if (row.providerUnauthorized || row.containerGone === false || row.volumesGone === false || row.cleanup.some(item => item.endsWith('_failed'))) {
      interrupted = true
      throw new Error('MP-08/MP-10 provider authorization or owned cleanup needs coordinator attention')
    }
    // One prompt per task; no repeats after an RPC ambiguity or solver failure.
  }
}
try {
  runtime = await startOwnedRuntime(options.runtime)
  report.runtime = runtime.source
  const fixture = JSON.parse(await readFile(`${options.fixtureEvidence}/sites-readiness.json`, 'utf8'))
  assert.equal(fixture.status, 'ready', 'MP-10 fixture admission RED')
  const outputRoot = `${evidence}/attempts`
  const config = { source: runtime.source.sourceCommit, release: runtime.source.runtimeSourceRevision,
    kernelSha256: runtime.source.artifacts.find(item => item.name === 'chariox-kernel').sha256,
    sliceImage: options.runtime.image, kernelUrl: runtime.kernelUrl, clientRoot: options.runtime.clientRoot,
    workspace: runtime.workspace, accountProfile: runtime.profileId, agentInputs: options.agentInputs,
    siteUrls: `${options.fixtureLane}/site-urls.json`, siteNetwork: fixture.network, outputRoot,
    python: options.python, upstream: options.upstream, scratch: runtime.root, maxSeconds: 600, maxActions: 80,
    leaderboard: options.leaderboard, fixtureLane: options.fixtureLane, fixtureEvidence: options.fixtureEvidence,
    composeBinary: options.composeBinary }
  const configPath = `${runtime.root}/campaign-options.json`
  await writeFile(configPath, JSON.stringify(config), { mode: 0o600 })
  const workers = await Promise.allSettled(Array.from({ length: options.rooms }, () => worker(configPath, outputRoot)))
  const failure = workers.find(result => result.status === 'rejected')
  if (failure) throw failure.reason
  assert(!interrupted && report.attempts.length === 91, 'MP-10 campaign interrupted/incomplete')
  report.completed = true
} catch (error) {
  interrupted = true; report.failureClass = error.name; process.exitCode = 1
  console.log('MP-08/MP-10 campaign RED; original task/RPC receipts retained')
} finally {
  for (const child of active) {
    try { assert(await stopOwnedProcess(child, { detached: true, graceMs: 15000 }), 'MP-11 task exit missing') }
    catch { report.cleanupFailed = true }
  }
  if (runtime) report.cleanup = await runtime.close()
  report.finishedAt = new Date().toISOString(); await checkpoint()
  process.off('SIGTERM', interrupt); process.off('SIGINT', interrupt)
}
