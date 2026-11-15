// L10-MUST DEADLINE-01..04: official Codex, real kernel, relay, CLI and restart.
// Run after building kernel/relay and the kernel client. Every artifact stays outside git.
import assert from 'node:assert/strict'
import { spawn, execFileSync } from 'node:child_process'
import { randomBytes, createECDH } from 'node:crypto'
import { openSync, closeSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { LocalIpcClient, RelayClientIdentity } from '../../packages/kernel-client/dist/ipc.js'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const root = process.env.DEADLINE_DRILL_ROOT
const kernel = process.env.DEADLINE_KERNEL_BINARY
const relay = process.env.DEADLINE_RELAY_BINARY
assert(root?.startsWith('/') && !root.startsWith(repo + '/'), 'DEADLINE_DRILL_ROOT must be an absolute path outside the repo')
assert(kernel && relay, 'set DEADLINE_KERNEL_BINARY and DEADLINE_RELAY_BINARY to the private builds')
const base = Number(process.env.DEADLINE_DRILL_PORT ?? 57930)
const token = randomBytes(32).toString('hex')
const terminalKey = createECDH('prime256v1')
terminalKey.generateKeys()
const relayIdentity = new RelayClientIdentity(terminalKey.getPrivateKey())
let paired = false
const home = process.env.DEADLINE_DRILL_HOME
assert(home?.startsWith('/root/.chariox/dev/'), 'DEADLINE_DRILL_HOME must be private state under /root/.chariox/dev/')
const workspace = path.join(root, 'workspace')
const socket = path.join(root, 'kernel.sock')
const env = { ...process.env, CHARIOX_HOME: home, CHARIOX_KERNEL_URL: `ws://127.0.0.1:${base}/kernel`, CHARIOX_KERNEL_HOST: '127.0.0.1', CHARIOX_KERNEL_PORT: String(base), CHARIOX_MCP_PORT: String(base + 1),
  CHARIOX_OPENCODE_PORT: String(base + 2), CHARIOX_CODEX_PORT: String(base + 3), CHARIOX_DAEMON_ID: 'deadline-edit-drill',
  CHARIOX_DAEMON_ALIAS: 'deadline-edit', CHARIOX_MACHINE_ID: 'deadline-edit-drill', CHARIOX_MACHINE_ALIAS: 'deadline-edit',
  CHARIOX_DAEMON_SOCKET: socket, CHARIOX_SESSION_HISTORY_DIR: path.join(home, 'history'),
  CHARIOX_RELAY_URL: `ws://127.0.0.1:${base + 4}`, CHARIOX_RELAY_TOKEN: token }
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
let kernelChild, relayChild, client, local
const sessions = []
const result = { started: new Date().toISOString(), head: process.env.DEADLINE_DRILL_HEAD, cells: {}, status: 'FAIL' }
async function record(name, value) { await writeFile(path.join(root, `${name}.json`), JSON.stringify(value, null, 2) + '\n') }
function start(binary, name, vars) {
  const log = openSync(path.join(root, `${name}.log`), 'a', 0o600)
  try { return spawn(binary, [], { cwd: workspace, env: vars, stdio: ['ignore', log, log] }) } finally { closeSync(log) }
}
async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  await new Promise(resolve => {
    const timer = setTimeout(() => child.kill('SIGKILL'), 15000)
    child.once('exit', () => { clearTimeout(timer); resolve() })
    child.kill('SIGTERM')
  })
}
async function until(probe, timeout = 120000) {
  const end = Date.now() + timeout
  let last
  do { try { last = await probe(); if (last) return last } catch (error) { last = error.message } await sleep(500) } while (Date.now() < end)
  throw Error(`condition timed out: ${JSON.stringify(last)}`)
}
async function connect() {
  await until(async () => {
    const probe = new LocalIpcClient(`ws://127.0.0.1:${base}`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 5000 })
    try { await probe.send({ ListSessions: null }); return true } finally { await probe.close() }
  })
  local = new LocalIpcClient(`ws://127.0.0.1:${base}`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 0 })
  const status = await until(async () => { const s = (await local.send({ RelayStatus: null })).RelayStatus.status; return s.connected && s })
  const daemonId = status.daemon_id
  client = new LocalIpcClient(`ws://127.0.0.1:${base + 4}`, { relayAuthToken: token, relayIdentity, targetDaemonId: daemonId, localAuthEnvironment: env, controlRequestRetryDeadlineMs: 0 })
  if (!paired) {
    const pairing = (await local.send({ CreateTerminalPairingLink: { terminal_type: 'cli', alias: 'deadline-drill' } })).TerminalPairingLinkCreated.pairing
    await client.send({ JoinTerminalPairingLink: { pairing_link: pairing.pairing_link, public_key_thumbprint: relayIdentity.publicKeyThumbprint, terminal_type: 'cli', alias: 'deadline-drill' } })
    paired = true
  }
  await client.send({ ListSessions: null })
}
async function agent(label, seconds = 45) {
  const created = (await client.send({ CreateSession: { workspace_id: workspace, worktree_id: workspace, alias: label,
    agent_defaults: { provider: 'codex', model: 'gpt-6-luna', effort: 'low', account_profile: 'default', execution_mode: 'build', permission_level: 'yolo' } } })).SessionCreated
  const room = created.session.id, id = created.agent.id
  sessions.push(room)
  const attachment = (await client.send({ AttachToSession: { session_id: room, client_id: 'deadline-drill', capability_level: 'FullTerminal' } })).SessionAttached.attachment.id
  const marker = path.join(workspace, `${label}.started`)
  const done = path.join(workspace, `${label}.done`)
  const launched = await client.send({ LaunchProviderRun: { session_id: room, agent_id: id, adapter_key: 'codex', provider: 'codex', account_profile: 'default', model: 'gpt-6-luna', variant: 'low' } })
  await record(label + '-provider', { room, agent: id, providerRun: (launched.ProviderRunLaunched ?? launched.ProviderRunLaunchAccepted).provider_run })
  await client.send({ SubmitPrompt: { session_id: room, attachment_id: attachment, target_agent_id: id,
    prompt: `This is a runtime deadline drill. Use your official shell tool to run exactly: python3 -c "from pathlib import Path; import time; Path('${marker}').write_text('started'); time.sleep(${seconds}); Path('${done}').write_text('done')". Run it in the foreground and wait for it to finish. Then answer DEADLINE_DONE. Do not inspect credentials, create agents, or edit repository files.`, attachments: [] } })
  await until(async () => (await readFile(marker, 'utf8')) === 'started', 180000)
  return { room, id, attachment, done }
}
async function set(a, time) {
  return (await client.send({ SetAgentDeadline: { session_id: a.room, agent_id: a.id, deadline_ms: time } })).AgentDeadline.deadline
}
async function get(a) { return (await client.send({ GetAgentDeadline: { session_id: a.room, agent_id: a.id } })).AgentDeadline.deadline }
async function cli(a, value) {
  const child = spawn('bun', [path.join(repo, 'apps/cli/dist/index.js'), 'agent', 'deadline', a.id, value, '--session', a.room], { cwd: workspace, env, stdio: ['ignore', 'pipe', 'pipe'] })
  let output = '', error = ''
  child.stdout.on('data', bytes => { output += bytes })
  child.stderr.on('data', bytes => { error += bytes })
  const code = await new Promise(resolve => child.once('exit', resolve))
  assert.equal(code, 0, `CLI deadline failed: ${error}`)
  return JSON.parse(output)
}
try {
  await mkdir(workspace, { recursive: true }); await mkdir(home, { recursive: true })
  await writeFile(path.join(workspace, 'README.md'), 'Runtime deadline drill fixture.\n')
  execFileSync('git', ['init', '-q', workspace])
  execFileSync('git', ['-C', workspace, 'add', 'README.md'])
  execFileSync('git', ['-C', workspace, '-c', 'user.name=Deadline drill', '-c', 'user.email=deadline@example.invalid', 'commit', '-qm', 'Initialize deadline drill fixture'])
  relayChild = start(relay, 'relay', { ...process.env, CHARIOX_RELAY_HOST: '127.0.0.1', CHARIOX_RELAY_PORT: String(base + 4), CHARIOX_RELAY_TOKEN: token })
  kernelChild = start(kernel, 'kernel', env)
  await connect()
  const extended = await agent('DEADLINE-01')
  const old = Date.now() + 12000
  await set(extended, old)
  const extension = await cli(extended, '1m')
  assert(extension.AgentDeadline.deadline.deadline_ms > old)
  await record('DEADLINE-01-edit', { old, receipt: extension.AgentDeadline.deadline, at: Date.now() })
  await sleep(Math.max(0, old + 2000 - Date.now()))
  assert.equal((await get(extended)).state, 'armed')
  await assert.rejects(readFile(extended.done), { code: 'ENOENT' }, 'provider job must still be pending beyond the old deadline')
  await until(async () => (await readFile(extended.done, 'utf8')) === 'done')
  result.cells['DEADLINE-01'] = { status: 'PASS', old, extended: await get(extended), keptRunningPastOldDeadline: true }
  await record('DEADLINE-01', result.cells['DEADLINE-01'])

  const cleared = await agent('DEADLINE-02')
  const clearedOld = Date.now() + 12000
  await set(cleared, clearedOld)
  assert.equal((await cli(cleared, 'none')).AgentDeadline.deadline, null)
  await record('DEADLINE-02-edit', { old: clearedOld, receipt: await get(cleared), at: Date.now() })
  await sleep(Math.max(0, clearedOld + 2000 - Date.now()))
  assert.equal(await get(cleared), null)
  await assert.rejects(readFile(cleared.done), { code: 'ENOENT' }, 'cleared provider job must still be pending beyond the old deadline')
  await until(async () => (await readFile(cleared.done, 'utf8')) === 'done')
  result.cells['DEADLINE-02'] = { status: 'PASS', old: clearedOld, deadline: await get(cleared), completedPastOldDeadline: true }
  await record('DEADLINE-02', result.cells['DEADLINE-02'])

  const shortened = await agent('DEADLINE-03', 90)
  await set(shortened, Date.now() + 120000)
  await cli(shortened, '5s')
  const stopped = await until(async () => { const d = await get(shortened); return d?.stopped_at_ms && d })
  assert.equal(stopped.state, 'reached')
  await assert.rejects(client.send({ SubmitPrompt: { session_id: shortened.room, attachment_id: shortened.attachment, target_agent_id: shortened.id, prompt: 'Reply SHOULD_BE_REFUSED', attachments: [] } }), /deadline/)
  result.cells['DEADLINE-03'] = { status: 'PASS', receipt: stopped, laterAdmissionRefused: true }
  await record('DEADLINE-03', result.cells['DEADLINE-03'])

  const restarted = await agent('DEADLINE-04', 1)
  await until(async () => (await readFile(restarted.done, 'utf8')) === 'done')
  const restartOld = Date.now() + 12000
  await set(restarted, restartOld)
  const restartNew = Date.now() + 90000
  await set(restarted, restartNew)
  await client.close(); await local.close(); client = local = null
  await stop(kernelChild)
  kernelChild = start(kernel, 'kernel', env)
  await connect()
  assert.equal((await get(restarted)).deadline_ms, restartNew)
  assert.equal(await get(cleared), null, 'cleared deadline reappeared on restart')
  await sleep(Math.max(0, restartOld + 2000 - Date.now()))
  const attachment = (await client.send({ AttachToSession: { session_id: restarted.room, client_id: 'deadline-restart', capability_level: 'FullTerminal' } })).SessionAttached.attachment.id
  await client.send({ SubmitPrompt: { session_id: restarted.room, attachment_id: attachment, target_agent_id: restarted.id, prompt: 'Without tools, reply exactly RESTART_DEADLINE_OK.', attachments: [] } })
  result.cells['DEADLINE-04'] = { status: 'PASS', old: restartOld, retainedDeadline: await get(restarted), clearedDeadlineRetained: true, workAdmittedPastOldDeadline: true }
  await record('DEADLINE-04', result.cells['DEADLINE-04'])
  result.status = 'PASS'
} catch (error) { result.failure = error.stack; process.exitCode = 1 }
finally {
  for (const session_id of sessions) { try { await client?.send({ EndSession: { session_id } }) } catch {} }
  await client?.close(); await local?.close()
  await stop(kernelChild); await stop(relayChild)
  result.finished = new Date().toISOString()
  await record('result', result)
  console.log(JSON.stringify({ status: result.status, cells: result.cells, failure: result.failure }))
}
