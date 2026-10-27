// MP-08 / MP-09 / MP-10 / MP-11: abandoned native clients through the real relay.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { Fixture, until } from './fixture.mjs'

const args = process.argv.slice(2)
const value = flag => { const index = args.indexOf(flag); if (index < 0 || !args[index + 1]) throw new Error(`missing ${flag}`); return path.resolve(args[index + 1]) }
const fixture = new Fixture({ home: value('--home'), evidence: value('--evidence'), kernelBinary: value('--kernel-bin'), relayBinary: value('--relay-bin'), cliBinary: value('--cli-bin') })
const results = { mp: ['MP-08', 'MP-09', 'MP-10', 'MP-11'], cells: [] }
const active = new Set()
let session
function signal(child, name) {
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1 || !active.has(child)) throw new Error('invalid owned CLI PID')
  if (child.exitCode === null && child.signalCode === null) child.kill(name)
}
function cli(argv) {
  const child = spawn(fixture.cliBinary, argv, { env: fixture.envs.A, stdio: ['pipe', 'pipe', 'pipe'] })
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error('invalid owned CLI PID')
  active.add(child)
  child.stdin.on('error', () => undefined)
  child.done = new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('close', (code, signal) => { active.delete(child); resolve({ code, signal }) })
  })
  child.done.catch(() => undefined)
  return child
}
const flags = () => ['--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET, '--kernel', fixture.target]
function command(script) { return cli(['exec', ...flags(), '--session', session, '--', '/bin/bash', '-c', script]) }
async function request(operation) {
  const child = cli(['request', ...flags()])
  const stdout = [], stderr = []
  child.stdout.on('data', bytes => stdout.push(bytes))
  child.stderr.on('data', bytes => stderr.push(bytes))
  child.stdin.end(JSON.stringify({ RoomTerminal: { session_id: session, operation } }))
  const status = await child.done
  assert.equal(status.code, 0, Buffer.concat(stderr).toString())
  return JSON.parse(Buffer.concat(stdout).toString()).RoomTerminal.result
}
async function exists(pid) {
  assert.ok(Number.isSafeInteger(pid) && pid > 1)
  try { await readFile(`/proc/${pid}/stat`); return true } catch (error) { if (error.code === 'ENOENT') return false; throw error }
}
async function start(script) {
  const child = command(`printf '%s\\n' "$$"; ${script}`)
  const stdout = [], stderr = []
  child.stdout.on('data', bytes => stdout.push(bytes))
  child.stderr.on('data', bytes => stderr.push(bytes))
  const pid = await until(() => {
    const text = Buffer.concat(stdout).toString()
    return /^\d+\n/.test(text) && Number(text.split('\n')[0])
  }, 'exec process PID')
  return { child, pid, stdout, stderr }
}
async function cell(id, run) {
  const only = args.indexOf("--only")
  if (only >= 0 && args[only + 1] !== id) return
  const began = Date.now()
  try {
    const details = await run()
    results.cells.push({ id, status: 'PASS', elapsed_ms: Date.now() - began, ...details })
    console.log(`${id}: PASS`)
  } catch (error) {
    results.cells.push({ id, status: 'FAIL', elapsed_ms: Date.now() - began, message: error.message })
    throw error
  } finally { await writeFile(path.join(fixture.evidence, 'results.json'), JSON.stringify(results, null, 2)) }
}
try {
  await fixture.start()
  await fixture.grant()
  const work = path.join(fixture.home, 'B/work')
  session = (await fixture.B.send({ CreateSession: { workspace_id: work, worktree_id: work, alias: null, no_subscribe: true } })).SessionCreated.session.id
  await cell('sigkill-list-reclaim', async () => {
    const { child, pid } = await start('cat')
    signal(child, 'SIGKILL')
    assert.equal((await child.done).signal, 'SIGKILL')
    const listed = (await request({ operation: 'exec_list' })).execs
    assert.equal(listed.length, 1)
    assert.equal(listed[0].state, 'running')
    assert.ok(listed[0].age_ms >= 0 && listed[0].buffered_bytes >= 0)
    await request({ operation: 'exec_close', exec_id: listed[0].exec_id })
    await until(async () => !(await exists(pid)), 'reclaimed process reaped')
    assert.deepEqual((await request({ operation: 'exec_list' })).execs, [])
    return { listed, reaped_pid: pid }
  })
  await cell('sighup-cancels', async () => {
    const { child, pid } = await start('cat')
    signal(child, 'SIGHUP')
    const status = await child.done
    assert.deepEqual(status, { code: 129, signal: null })
    await until(async () => !(await exists(pid)), 'SIGHUP process reaped')
    assert.deepEqual((await request({ operation: 'exec_list' })).execs, [])
    return { process_status: status, reaped_pid: pid }
  })
  await cell('cancel-truncated-output', async () => {
    const marker = path.join(fixture.home, 'B/work/cancel-process-exited')
    const opened = await request({ operation: 'exec_open', command: ['/bin/bash', '-c', 'head -c 1081344 /dev/zero; : > "$1"', 'exec', marker] })
    await until(() => readFile(marker).then(() => true, () => false), 'producer completed before cancel')
    await delay(100)
    await request({ operation: 'exec_cancel', exec_id: opened.exec_id })
    await delay(1000)
    let cursor = 0, bytes = 0, outcome
    do {
      const page = (await request({ operation: 'exec_read', exec_id: opened.exec_id, after_cursor: cursor, wait_ms: 1000 })).output
      bytes += page.chunks.reduce((sum, chunk) => sum + Buffer.from(chunk.data_base64, 'base64').length, 0)
      cursor = page.next_cursor; outcome = page.outcome
    } while (!outcome)
    await request({ operation: 'exec_close', exec_id: opened.exec_id })
    assert.ok(bytes < 1081344)
    assert.equal(outcome.kind, 'failed')
    assert.equal(outcome.message, 'command canceled before its output ended; the stream is incomplete')
    return { bytes, outcome }
  })
  await cell('sigkill-expiry-input-and-unread-output', async () => {
    const first = await start('cat')
    signal(first.child, 'SIGKILL'); await first.child.done
    const second = await start('head -c 8388608 /dev/zero')
    signal(second.child, 'SIGKILL'); await second.child.done
    const listed = await until(async () => {
      const rows = (await request({ operation: 'exec_list' })).execs
      return rows.length === 2 && rows.some(row => row.buffered_bytes > 1024 * 1024 - 32 * 1024) && rows
    }, 'unread bounded output full')
    assert.ok(await exists(first.pid))
    const began = Date.now()
    await until(async () => {
      const empty = (await request({ operation: 'exec_list' })).execs.length === 0
      if (!empty) await delay(5_000)
      return empty
    }, 'five minute reader lease expiry', 330_000)
    for (const pid of [first.pid, second.pid]) await until(async () => !(await exists(pid)), 'expired process reaped')
    return { listed, elapsed_since_full_ms: Date.now() - began, reaped_pids: [first.pid, second.pid], expiry_seconds: 300 }
  })
  await cell('real-kernel-connection-loss', async () => {
    const { child, stderr } = await start('cat')
    const origin = fixture.children.find(item => item.name === 'kernel-A').child
    if (!Number.isSafeInteger(origin.pid) || origin.pid <= 1) throw new Error('invalid owned origin kernel PID')
    origin.kill('SIGKILL')
    await origin.done
    const status = await child.done
    assert.equal(status.code, 5)
    const error = JSON.parse(Buffer.concat(stderr).toString().trim()).error
    assert.equal(error.code, 'kernel_connection_lost')
    assert.ok(error.exec_id && error.session_id === session)
    assert.equal(error.output_cursor >= 0, true)
    return { process_status: status, error }
  })

} finally {
  for (const child of [...active]) signal(child, 'SIGTERM')
  await Promise.allSettled([...active].map(child => child.done))
  if (session) await fixture.B.send({ EndSession: { session_id: session } }).catch(() => undefined)
  await fixture.cleanup()
}
