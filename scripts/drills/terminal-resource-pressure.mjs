// MP-08/09/10/11: real CLI terminals survive a kernel descriptor refusal.
// Build kernel-client and CLI first; state/evidence must be outside the checkout.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { closeSync, cpSync, mkdirSync, openSync, readFileSync, readdirSync, writeFileSync, writeSync } from 'node:fs'
import { createServer } from 'node:net'
import { join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { LocalIpcClient } from '../../packages/kernel-client/dist/ipc.js'

const flags = new Map()
for (let i = 2; i < process.argv.length; i += 2) flags.set(process.argv[i], process.argv[i + 1])
const binary = resolve(flags.get('--kernel-binary'))
const root = resolve(flags.get('--state-root'))
const evidence = resolve(flags.get('--evidence-dir'))
assert(root.startsWith('/root/.chariox/dev/fdexh/'), 'use lane-owned disposable state')
assert(!evidence.startsWith(resolve('.') + '/'), 'evidence belongs outside the checkout')
for (const sub of ['home', 'tmp', 'config', 'cache', 'state', 'chariox', 'work', 'logs', 'history']) mkdirSync(join(root, sub), { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
writeFileSync(join(root, 'chariox/config.toml'), 'version = 1\n[history.archive]\nmode = "disabled"\n[credential_vault]\nbackend = "process_memory"\n', { mode: 0o600 })
async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, '127.0.0.1', r))
  const port = server.address().port
  await new Promise(r => server.close(r))
  return port
}
const ports = await Promise.all(Array.from({ length: 4 }, freePort))
const env = { PATH: process.env.PATH, LANG: 'C.UTF-8', SHELL: '/bin/bash', TERM: 'xterm-256color',
  HOME: join(root, 'home'), TMPDIR: join(root, 'tmp'), XDG_CONFIG_HOME: join(root, 'config'),
  XDG_CACHE_HOME: join(root, 'cache'), XDG_STATE_HOME: join(root, 'state'),
  CHARIOX_HOME: join(root, 'chariox'), CHARIOX_LOG_DIR: join(root, 'logs'), CHARIOX_SESSION_HISTORY_DIR: join(root, 'history'),
  CHARIOX_DAEMON_SOCKET: join(root, 'daemon.sock'), CHARIOX_DAEMON_ALIAS: 'fdexh', CHARIOX_MACHINE_ALIAS: 'fdexh',
  CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]),
  CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT: '1', IS_SANDBOX: '1' }
const report = { head: flags.get('--head'), binary_sha256: createHash('sha256').update(readFileSync(binary)).digest('hex'), started_at: new Date().toISOString(), nofile: 320, rooms: [], terminals: [], cli_processes: [], samples: [], witness_checkpoints: [] }
const save = () => writeFileSync(join(evidence, 'result.json'), JSON.stringify(report, null, 2) + '\n')
const log = openSync(join(evidence, 'kernel.log'), 'w', 0o600)
const kernel = spawn('/usr/bin/prlimit', ['--nofile=320:320', binary], { env, cwd: root, stdio: ['ignore', log, log] })
assert(Number.isInteger(kernel.pid) && kernel.pid > 1)
report.kernel_pid = kernel.pid
closeSync(log)
const client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { timeoutMs: 10000, localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
const clients = []
const pressureClients = []
const fds = () => readdirSync(`/proc/${kernel.pid}/fd`).length
async function until(check, label, ms = 30000) {
  const end = Date.now() + ms
  while (Date.now() < end) { const result = await check(); if (result) return result; await delay(100) }
  throw new Error(`timed out: ${label}`)
}
async function rpc(request) {
  const reply = await client.send(request)
  if (reply.Error) throw new Error(JSON.stringify(reply.Error))
  return reply
}
const term = (session, operation) => rpc({ RoomTerminal: { session_id: session, operation } }).then(r => r.RoomTerminal.result)
async function room() {
  const made = (await rpc({ CreateSession: { workspace_id: join(root, 'work'), worktree_id: join(root, 'work'), no_subscribe: true } })).SessionCreated
  report.rooms.push(made.session.id); save()
  return made.session.id
}
function cli(session, args) {
  const child = spawn('bun', [resolve('apps/cli/dist/index.js'), 'terminal', '--session', session, '--kernel-url', `ws://127.0.0.1:${ports[0]}/kernel`, ...args], { env, stdio: ['pipe', 'pipe', 'pipe'] })
  assert(Number.isInteger(child.pid) && child.pid > 1)
  const handle = { child, output: '', error: '' }
  const file = openSync(join(evidence, `cli-${child.pid}.txt`), 'w', 0o600)
  // Keep the complete user-visible stream on disk, a bounded tail in memory.
  child.stdout.on('data', bytes => { handle.output = (handle.output + bytes.toString()).slice(-65536); writeSync(file, bytes) })
  child.stderr.on('data', bytes => { handle.error = (handle.error + bytes.toString()).slice(-8192); writeSync(file, bytes) })
  child.once('close', () => closeSync(file))
  report.cli_processes.push(child.pid); clients.push(handle)
  return handle
}
const exited = h => h.child.exitCode !== null || h.child.signalCode !== null
async function detach(h) {
  if (!exited(h)) h.child.stdin.write('\x1d')
  await until(() => exited(h), 'CLI detach')
}
async function marker(h, text) {
  const before = h.output.length
  // Split the literal so shell input echo cannot satisfy the assertion.
  h.child.stdin.write(`printf '%s%s\\n' 'fdexh-' '${text}'\n`)
  await until(() => h.output.slice(before).includes(`fdexh-${text}`) || exited(h), `echo ${text}`)
  return !exited(h) && h.output.slice(before).includes(`fdexh-${text}`)
}
try {
  await until(async () => { try { return Boolean((await client.send({ RelayStatus: null })).RelayStatus) } catch { assert(kernel.exitCode === null && kernel.signalCode === null, 'kernel startup failed'); return false } }, 'kernel startup', 90000)
  report.initial_fds = fds(); save()
  let session = await room()
  const witnesses = []
  for (let i = 0; i < 100; i++) {
    if (i > 0 && i % 16 === 0) session = await room()
    const title = `fdexh-${i}`
    const h = cli(session, ['new', title])
    const opened = await until(async () => {
      if (exited(h)) return { refused: true }
      const terminals = (await term(session, { operation: 'list' })).terminals
      const found = terminals.find(t => t.title === title)
      if (found && h.output.includes('\x1b[2J')) return { terminal: found }
      return false
    }, 'CLI terminal creation')
    if (opened.refused) {
      report.refusal = { opened: report.terminals.length, exit_code: h.child.exitCode, message: h.error, fds: fds() }
      break
    }
    const record = { session, terminal_id: opened.terminal.terminal_id }
    report.terminals.push(record)
    if (i < 10) { assert(await marker(h, `before-${i}`)); witnesses.push({ h, ...record }) }
    else await detach(h)
    if (i >= 10 && i % 8 === 0) {
      for (let j = 0; j < witnesses.length; j++) {
        const echoes = await marker(witnesses[j].h, `during-${i}-${j}`)
        report.witness_checkpoints.push({ opened: i + 1, witness: j, echoes })
      }
    }
    save()
  }
  assert(report.refusal, 'never reached descriptor refusal')
  // Repeated failed opens must release their descriptors.
  for (let attempt = 0; report.samples.length < 10 && attempt < 20; attempt++) {
    const before = fds()
    let refusal
    try {
      // The refused CLI closes its connection, which may free enough to admit
      // one more terminal. Keep that real terminal and fill to refusal again.
      const opened = await term(session, { operation: 'open', cols: 80, rows: 24, title: `retry-${attempt}` })
      report.terminals.push({ session, terminal_id: opened.terminal.terminal_id })
      continue
    }
    catch (error) {
      refusal = { code: error.code, message: error.message }
    }
    await delay(300)
    report.samples.push({ before, after: fds(), refusal })
  }
  assert.equal(report.samples.length, 10, 'did not reach ten refused opens')
  // One free descriptor cannot allocate a PTY pair. Real client connections
  // exercise portable-pty's allocation failure, before a child can be spawned.
  while (fds() < 319 && pressureClients.length < 8) {
    const extra = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { timeoutMs: 10000, localAuthEnvironment: env })
    pressureClients.push(extra)
    await extra.send({ RelayStatus: null })
  }
  report.fds_at_allocation = fds()
  assert(report.fds_at_allocation >= 319, 'did not reach PTY pair allocation pressure')
  try { await term(session, { operation: 'open', cols: 80, rows: 24, title: 'allocation' }); throw new Error('unexpected successful allocation') }
  catch (error) { report.allocation_refusal = { code: error.code, message: error.message } }
  report.fds_after_allocation = fds()
  await delay(3000)
  report.witnesses = []
  for (let i = 0; i < witnesses.length; i++) {
    const w = witnesses[i]
    const snapshot = await term(w.session, { operation: 'snapshot', terminal_id: w.terminal_id })
    let echoes = false
    try { echoes = await marker(w.h, `after-${i}`) } catch {}
    report.witnesses.push({ terminal_id: w.terminal_id, echoes, status: snapshot.snapshot.status })
  }
  save()
  await Promise.all(pressureClients.map(c => c.close()))
  for (const t of report.terminals.slice(10, 20)) await term(t.session, { operation: 'close', terminal_id: t.terminal_id })
  await delay(1000)
  const recovered = cli(session, ['new', 'recovered'])
  await until(() => recovered.output.includes('\x1b[2J') || exited(recovered), 'open after closing')
  report.recovered = !exited(recovered) && await marker(recovered, 'recovered')
  if (!exited(recovered)) await detach(recovered)
  report.fds_after_closing = fds()
  report.actionable = report.refusal.message.includes('too many open files in this kernel; close a terminal and try again')
  report.no_fd_growth = report.samples.every(s => s.after <= s.before)
  report.retry_refusals_actionable = report.samples.every(s => s.refusal.code === 'too_many_open_files' && s.refusal.message.includes('too many open files in this kernel; close a terminal and try again'))
  report.allocation_refusal_actionable = report.allocation_refusal.code === 'too_many_open_files' && report.allocation_refusal.message.includes('close a terminal and try again') && report.fds_after_allocation <= report.fds_at_allocation
  report.pass = report.actionable && report.retry_refusals_actionable && report.allocation_refusal_actionable && report.no_fd_growth && report.witness_checkpoints.every(w => w.echoes) && report.witnesses.length === 10 && report.witnesses.every(w => w.echoes) && report.recovered
  console.log(JSON.stringify({ opened: report.terminals.length, refusal: report.refusal, witnesses_echoing: report.witnesses.filter(w => w.echoes).length, recovered: report.recovered, no_fd_growth: report.no_fd_growth, pass: report.pass }))
  process.exitCode = report.pass ? 0 : 1
} catch (error) {
  report.error = String(error); process.exitCode = 1; console.error(report.error)
} finally {
  await Promise.all(pressureClients.map(c => c.close()))
  for (const h of clients) if (!exited(h)) {
    await detach(h).catch(async () => {
      assert(Number.isInteger(h.child.pid) && h.child.pid > 1)
      h.child.kill('SIGTERM')
      await until(() => exited(h), 'CLI exit', 5000).catch(() => h.child.kill('SIGKILL'))
    })
  }
  for (const session of report.rooms) await rpc({ EndSession: { session_id: session } }).catch(() => {})
  await client.close()
  if (kernel.exitCode === null && kernel.signalCode === null) {
    assert(Number.isInteger(kernel.pid) && kernel.pid > 1)
    kernel.kill('SIGTERM')
    await until(() => kernel.exitCode !== null || kernel.signalCode !== null, 'kernel exit').catch(() => { kernel.kill('SIGKILL') })
  }
  cpSync(join(root, 'logs'), join(evidence, 'kernel-logs'), { recursive: true })
  report.cleaned_processes = clients.every(exited) && (kernel.exitCode !== null || kernel.signalCode !== null)
  save()
}
