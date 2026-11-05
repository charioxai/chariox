// MP-08 / MP-09 / MP-10 / MP-11: native exec retains its pipes while no user program exists.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { access, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { until } from './fixture.mjs'
import { AdmissionFixture, marker as fixtureMarker, command as fixtureCommand } from './pre-spawn-fixture.mjs'

const args = process.argv.slice(2)
const value = flag => {
  const at = args.indexOf(flag)
  if (at < 0 || !args[at + 1]) throw Error(`missing ${flag}`)
  return path.resolve(args[at + 1])
}
const home = value('--home'), evidence = value('--evidence')
const marker = (name, host = 'A') => fixtureMarker(home, name, host)
const command = (name, host = 'A') => fixtureCommand(home, name, host)
const digest = bytes => createHash('sha256').update(bytes).digest('hex')
const exists = file => access(file).then(() => true, () => false)

const fixture = new AdmissionFixture({home, evidence, kernelBinary: value('--kernel-bin'), relayBinary: value('--relay-bin'), cliBinary: value('--cli-bin')})
const active = new Set(), cells = []
const result = {status: 'FAIL', mp: ['MP-08', 'MP-09', 'MP-10', 'MP-11'], scope: 'native CLI and real kernels, local scoped relay; deterministic learned peak; no remote placement or hosted/paid acceptance', cells}
let session, peerSession
const operation = async (operation, host = 'A') => (await fixture[host].send({RoomTerminal: {session_id: host === 'A' ? session : peerSession, operation}})).RoomTerminal.result

function run(name, bytes, host = 'A') {
  const child = spawn(fixture.cliBinary, ['exec', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET, ...(host === 'B' ? ['--kernel', fixture.target] : []), '--session', host === 'A' ? session : peerSession, '--', ...command(name, host)], {env: fixture.envs.A, stdio: ['pipe', 'pipe', 'pipe']})
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw Error('invalid owned native exec PID')
  active.add(child)
  const stdout = [], stderr = []
  child.stdout.on('data', chunk => stdout.push(chunk)); child.stderr.on('data', chunk => stderr.push(chunk))
  child.stdin.on('error', () => {})
  let timedOut = false, escalation
  const deadline = setTimeout(() => {
    timedOut = true
    if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw Error('invalid owned native exec PID')
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM')
    escalation = setTimeout(() => {
      if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw Error('invalid owned native exec PID')
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
    }, 5000)
  }, 60_000)
  const done = new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('close', (code, signal) => {active.delete(child); clearTimeout(deadline); clearTimeout(escalation); if (timedOut) reject(Error('native exec exceeded its deadline')); else resolve({code, signal, stdout: Buffer.concat(stdout), stderr: Buffer.concat(stderr)})})
  })
  done.catch(() => {})
  child.done = done
  child.stdin.end(bytes)
  return {child, done}
}

async function held(name, host = 'A') {
  return until(async () => {
    assert.equal(await exists(marker(name, host)), false, 'user code executed before resource admission')
    const listed = await operation({operation: 'exec_list'}, host)
    return listed.execs?.find(row => row.command?.at(-1) === name && row.state.kind === 'queued')
  }, `native exec ${name} held before start`)
}

async function cell(id, work) {
  const started = Date.now()
  try { cells.push({id, status: 'PASS', ...await work(), elapsed_ms: Date.now() - started}) }
  catch (error) { cells.push({id, status: 'FAIL', message: error.message, elapsed_ms: Date.now() - started}); throw error }
  finally { await writeFile(path.join(evidence, 'pre-spawn-results.json'), JSON.stringify(result, null, 2)) }
}

try {
  await fixture.start()
  const grant = await fixture.grant()
  const work = path.join(home, 'A/work')
  session = (await fixture.A.send({CreateSession: {workspace_id: work, worktree_id: work, alias: null, no_subscribe: true}})).SessionCreated.session.id
  await cell('no-side-effect-and-cancel-before-claim', async () => {
    const pending = run('cancel', Buffer.from('never sent to a user program'))
    const queued = await held('cancel')
    assert.equal((await operation({operation: 'exec_cancel', exec_id: queued.exec_id})).result, 'exec_cancelled')
    const completed = await pending.done
    assert.notEqual(completed.code, 0)
    assert.equal(await exists(marker('cancel')), false)
    assert.equal(completed.stdout.length, 0)
    return {exec_id: queued.exec_id, native_exit: completed.code, marker: false}
  })
  await cell('start-now-preserves-binary-pipes-and-starts-once', async () => {
    const bytes = Buffer.concat([await readFile(new URL('./README.md', import.meta.url)), Buffer.from(Array.from({length: 600_000}, (_, i) => i % 256))])
    const pending = run('start', bytes), queued = await held('start')
    const started = await operation({operation: 'exec_start_now', exec_id: queued.exec_id})
    assert.equal(started.exec_id, queued.exec_id)
    const completed = await pending.done
    assert.equal(completed.code, 23)
    assert.deepEqual(completed.stdout, bytes)
    assert.equal(completed.stderr.toString(), digest(bytes))
    assert.equal((await readFile(marker('start'))).toString(), 'x')
    await assert.rejects(operation({operation: 'exec_start_now', exec_id: queued.exec_id}))
    return {exec_id: queued.exec_id, bytes: bytes.length, sha256: digest(bytes), native_exit: completed.code, executions: 1}
  })
  const peerWork = path.join(home, 'B/work')
  peerSession = (await fixture.B.send({CreateSession: {workspace_id: peerWork, worktree_id: peerWork, alias: null, no_subscribe: true}})).SessionCreated.session.id
  await cell('peer-queue-preserves-original-native-streams', async () => {
    const bytes = Buffer.from(Array.from({length: 700_017}, (_, i) => i % 256))
    const pending = run('peer-start', bytes, 'B'), queued = await held('peer-start', 'B')
    await operation({operation: 'exec_start_now', exec_id: queued.exec_id}, 'B')
    const completed = await pending.done
    assert.equal(completed.code, 23)
    assert.deepEqual(completed.stdout, bytes)
    assert.equal(completed.stderr.toString(), digest(bytes))
    assert.equal((await readFile(marker('peer-start', 'B'))).toString(), 'x')
    return {exec_id: queued.exec_id, bytes: bytes.length, sha256: digest(bytes), native_exit: completed.code, executions: 1, scope: 'native --kernel selection before submission; not post-hold placement'}
  })
  await cell('native-cancel-of-peer-queue-never-spawns', async () => {
    const pending = run('peer-cancel', Buffer.alloc(0), 'B'), queued = await held('peer-cancel', 'B')
    if (!Number.isSafeInteger(pending.child.pid) || pending.child.pid <= 1) throw Error('invalid owned native exec PID')
    pending.child.kill('SIGINT')
    const completed = await pending.done
    assert.equal(completed.code, 130)
    assert.equal(await exists(marker('peer-cancel', 'B')), false)
    return {exec_id: queued.exec_id, native_exit: completed.code, marker: false}
  })
  await cell('grant-revocation-cannot-dispatch-a-queued-launch', async () => {
    const pending = run('revoke', Buffer.from('must remain unexecuted')), queued = await held('revoke')
    const remote = run('peer-revoke', Buffer.alloc(0), 'B'), remoteQueued = await held('peer-revoke', 'B')
    await fixture.A.send({RevokeKernelAccessGrant: {grant_id: grant}})
    const completed = await pending.done
    assert.notEqual(completed.code, 0)
    await until(async () => (await operation({operation: 'exec_list'})).execs?.find(row => row.exec_id === queued.exec_id && row.state.kind === 'finished'), 'queued launch settles after revocation')
    assert.equal(await exists(marker('revoke')), false)
    assert.notEqual((await remote.done).code, 0)
    await until(async () => { const row = (await operation({operation: 'exec_list'}, 'B')).execs?.find(row => row.exec_id === remoteQueued.exec_id); return !row || row.state.kind === 'finished' }, 'peer queue settles after original grant revocation')
    assert.equal(await exists(marker('peer-revoke', 'B')), false)
    return {exec_id: queued.exec_id, marker: false, native_exit: completed.code}
  })
  await cell('target-revocation-cannot-dispatch-a-queued-launch', async () => {
    fixture.prompts.length = 0
    await fixture.grant()
    const pending = run('peer-target-revoke', Buffer.alloc(0), 'B')
    const queued = await held('peer-target-revoke', 'B')
    const clients = (await fixture.B.send({ListPairedClients: null})).PairedClientsListed.clients
    const gateway = clients.find(client => client.client_id.startsWith('agent-gateway:') && !client.revoked)
    assert.ok(gateway, 'the normal paired gateway is listed on the target')
    await fixture.B.send({RevokePairedClient: {client_id: gateway.client_id}})
    await until(async () => (await operation({operation: 'exec_list'}, 'B')).execs?.find(row => row.exec_id === queued.exec_id && row.state.kind === 'finished'), 'target revocation settles the queued launch')
    assert.notEqual((await pending.done).code, 0)
    assert.equal(await exists(marker('peer-target-revoke', 'B')), false)
    return {exec_id: queued.exec_id, marker: false}
  })
  result.status = 'PASS'
} finally {
  for (const child of active) {
    if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw Error('invalid owned native exec PID')
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM')
  }
  await Promise.allSettled([...active].map(async child => {
    await Promise.race([child.done, delay(5000)])
    if (child.exitCode === null && child.signalCode === null) {
      if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw Error('invalid owned native exec PID')
      child.kill('SIGKILL')
    }
    await child.done
  }))
  await fixture.cleanup()
  await writeFile(path.join(evidence, 'pre-spawn-results.json'), JSON.stringify(result, null, 2))
}
