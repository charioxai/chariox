// MP-08 / MP-09 / MP-10 / MP-11: a started target launch needs a durable receipt.
// This is a known-red contract drill until target execution recovery is implemented.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'
import { Fixture, until } from './fixture.mjs'

const args = process.argv.slice(2)
const value = flag => { const index = args.indexOf(flag); if (index < 0 || !args[index + 1]) throw Error(`missing ${flag}`); return path.resolve(args[index + 1]) }
const fixture = new Fixture({home: value('--home'), evidence: value('--evidence'), kernelBinary: value('--kernel-bin'), relayBinary: value('--relay-bin'), cliBinary: value('--cli-bin')})
const result = {mp: ['MP-08', 'MP-09', 'MP-10', 'MP-11'], status: 'FAIL', scope: 'direct preselected peer; target restart receipt contract, not post-hold placement or VM acceptance'}
let native, deadline, escalation, nativeOwned = false
const signal = (child, name) => {
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw Error('invalid owned process PID')
  if (child.exitCode === null && child.signalCode === null) child.kill(name)
}
try {
  await fixture.start()
  await fixture.grant()
  const work = path.join(fixture.home, 'B/work'), marker = path.join(work, 'started-once')
  const session = (await fixture.B.send({CreateSession: {workspace_id: work, worktree_id: work, alias: null, no_subscribe: true}})).SessionCreated.session.id
  const program = 'import sys; open("started-once","ab").write(b"x"); print("ready",flush=True); sys.stdin.buffer.read(); sys.exit(23)'
  native = spawn(fixture.cliBinary, ['exec', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET, '--kernel', fixture.target, '--session', session, '--', '/usr/bin/python3', '-c', program], {env: fixture.envs.A, stdio: ['pipe', 'pipe', 'pipe']})
  let output = ''
  const errors = []
  native.stdout.on('data', bytes => { output += bytes.toString() })
  native.stderr.on('data', bytes => errors.push(bytes))
  native.stdin.on('error', () => {})
  native.done = new Promise((resolve, reject) => {
    native.once('error', reject)
    native.once('close', (code, signal) => resolve({code, signal}))
  })
  native.done.catch(() => {})
  if (!Number.isSafeInteger(native.pid) || native.pid <= 1) throw Error('invalid owned native exec PID')
  nativeOwned = true
  deadline = setTimeout(() => {
    signal(native, 'SIGTERM')
    escalation = setTimeout(() => signal(native, 'SIGKILL'), 5000)
  }, 60_000)
  await until(() => output === 'ready\n', 'target user program executed once')
  assert.equal((await readFile(marker)).toString(), 'x')
  const list = await fixture.B.send({RoomTerminal: {session_id: session, operation: {operation: 'exec_list'}}})
  assert.equal(list.RoomTerminal.result.execs.length, 1)
  const id = list.RoomTerminal.result.execs[0].exec_id
  result.exec_id = id
  const target = fixture.children.find(row => row.name === 'kernel-B').child
  signal(target, 'SIGKILL')
  await target.done
  await fixture.B.close()
  await fixture.launch('kernel-B-restarted', fixture.kernelBinary, fixture.envs.B)
  const restored = new LocalIpcClient(`ws://127.0.0.1:${fixture.ports.B}/kernel`, {localAuthEnvironment: fixture.envs.B, controlRequestRetryDeadlineMs: 0})
  fixture.clients.push(restored)
  await until(async () => {
    try { return (await restored.send({RelayStatus: null})).RelayStatus.status.connected } catch { return false }
  }, 'same target kernel restarted with its original state', 60_000)
  result.marker_after_restart = (await readFile(marker)).toString()
  assert.equal(result.marker_after_restart, 'x', 'the launch must not be replayed')
  let response
  try {
    response = await restored.send({RoomTerminal: {session_id: session, operation: {operation: 'exec_read', exec_id: id, after_cursor: 0, wait_ms: 0}}})
  } catch (error) {
    result.receipt_error_code = typeof error.code === 'string' ? error.code : error.name
  }
  native.stdin.end()
  result.native_exit = await native.done
  try { result.native_error_code = JSON.parse(Buffer.concat(errors).toString().trim()).error.code } catch {}
  const outcome = response?.RoomTerminal?.result?.output?.outcome
  if (outcome) result.receipt_outcome = outcome
  const unknown = outcome?.kind === 'unknown' || (outcome?.kind === 'failed' && /unknown|incomplete|not confirmed/i.test(outcome.message))
  assert.ok(unknown, `target restart lost the durable launch receipt (${result.receipt_error_code ?? 'no explicit unknown outcome'})`)
  result.status = 'PASS'
} catch (error) {
  result.failure = error.message
  throw error
} finally {
  clearTimeout(deadline); clearTimeout(escalation)
  if (nativeOwned) {
    signal(native, 'SIGTERM')
    await Promise.race([native.done.catch(() => {}), delay(5000)])
    signal(native, 'SIGKILL')
    await native.done.catch(() => {})
  }
  await fixture.cleanup()
  await writeFile(path.join(fixture.evidence, 'results.json'), JSON.stringify(result, null, 2))
}
