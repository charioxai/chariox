// MP-08 / MP-09 / MP-10 / MP-11: orphan sub-agent cleanup runs beside the wake scheduler, never in front of it.
// Real kernel (dev-stub provider) and the shared client; all state and evidence are outside the checkout.
// A room agent gets several sub-agents; the drill deletes the room agent and SIGKILLs the kernel the moment that
// deletion is durable, which leaves sub-agents whose parent no longer exists (the only way a user's kernel gets
// an orphan). After the restart the scheduler's sweep must end every one of them, and an agent deadline armed in
// another room must be reached on time while the sweep runs.
// usage: node scripts/drills/orphan-sub-agent-sweep.mjs --kernel-binary <path> --state-root /root/.chariox/dev/<task> \
//   --evidence-dir <dir> [--children 12] [--attempts 5]
// A coverage drill, not a reproduction of the stuck-cleanup bug: with the stub provider the sweep ends orphans in
// well under a second; the stuck case is pinned by subtree_cascade::a_stuck_orphan_cleanup_does_not_hold_back_due_wakes.
// Exit 0 = passed, 1 = a check failed, 2 = inconclusive (the kill never landed inside the cascade).
import assert from 'node:assert/strict'
import { spawn, execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, openSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { resolve, join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { LocalIpcClient } from '../../packages/kernel-client/dist/ipc.js'

const args = new Map()
for (let i = 2; i < process.argv.length; i += 2) args.set(process.argv[i], process.argv[i + 1])
const binary = resolve(args.get('--kernel-binary'))
const stateRoot = resolve(args.get('--state-root'))
const evidence = resolve(args.get('--evidence-dir'))
const children = Number(args.get('--children') ?? 12)
const attempts = Number(args.get('--attempts') ?? 5)
const ON_TIME_MS = 8000
assert(stateRoot.startsWith('/root/.chariox/dev/'), 'isolated state root required')
mkdirSync(stateRoot, { recursive: true })
mkdirSync(evidence, { recursive: true })
const root = mkdtempSync(join(stateRoot, 'run-'))
const result = { started_at: new Date().toISOString(), binary, children, attempts: [], checks: [] }
const save = () => writeFileSync(join(evidence, 'orphan-sweep.json'), JSON.stringify(result, null, 2) + '\n')
const check = (name, ok, detail = {}) => { result.checks.push({ name, ok, ...detail }); save(); console.log(`${ok ? 'ok  ' : 'FAIL'} ${name} ${JSON.stringify(detail)}`) }

async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, '127.0.0.1', r))
  const port = server.address().port
  await new Promise(r => server.close(r))
  return port
}
const ports = await Promise.all(Array.from({ length: 4 }, freePort))
for (const sub of ['home', 'config', 'cache', 'state', 'chariox', 'work', 'tmp', 'history', 'logs']) mkdirSync(join(root, sub), { recursive: true, mode: 0o700 })
writeFileSync(join(root, 'chariox/config.toml'), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${root}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${root}/chariox/state.db"\n`, { mode: 0o600 })
const work = join(root, 'work')
const env = { PATH: process.env.PATH, IS_SANDBOX: '1', SHELL: '/bin/bash', HOME: join(root, 'home'), TMPDIR: join(root, 'tmp'),
  XDG_CONFIG_HOME: join(root, 'config'), XDG_CACHE_HOME: join(root, 'cache'), XDG_STATE_HOME: join(root, 'state'),
  CHARIOX_HOME: join(root, 'chariox'), CHARIOX_LOG_DIR: join(root, 'logs'), CHARIOX_SESSION_HISTORY_DIR: join(root, 'history'),
  CHARIOX_DAEMON_SOCKET: join(root, 'daemon.sock'), CHARIOX_DAEMON_ALIAS: 'orphan-sweep', CHARIOX_MACHINE_ALIAS: 'orphan-sweep',
  CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]),
  CHARIOX_PROVIDER_DEV_STUB: '1' }
const db = join(root, 'chariox/state.db')
const log = openSync(join(evidence, 'kernel.log'), 'a', 0o600)
let child, client

async function start() {
  child = spawn(binary, [], { env, stdio: ['ignore', log, log] })
  assert(Number.isInteger(child.pid) && child.pid > 1)
  client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
  for (let i = 0; ; i++) {
    try { if ((await client.send({ RelayStatus: null })).RelayStatus) return } catch {}
    assert(child.exitCode === null && child.signalCode === null, 'kernel exited')
    assert(i < 300, 'kernel startup timed out')
    await delay(100)
  }
}
async function stop() {
  await client?.close()
  if (child?.exitCode === null && child.signalCode === null) {
    assert(Number.isInteger(child.pid) && child.pid > 1)
    child.kill('SIGTERM')
    for (let i = 0; i < 100 && child.exitCode === null && child.signalCode === null; i++) await delay(100)
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
  }
}
async function rpc(request) {
  const response = await client.send(request)
  if (response.Error) throw new Error(`${Object.keys(request)[0]}: ${response.Error.code}: ${response.Error.message}`)
  return response
}
const deleted = () => {
  try { return new Set(execFileSync('sqlite3', ['-readonly', db, "select subject_id from durable_state_events where kind='agent.deleted'"]).toString().split('\n').filter(Boolean)) } catch { return new Set() }
}
const room = async alias => {
  const made = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias, agent_defaults: { provider: 'dev-stub', model: 'native-tui-idle', account_profile: 'default' } } })).SessionCreated
  return { id: made.session.id, agent: made.agent.id }
}

try {
  await start()
  let hit = null
  for (let attempt = 1; attempt <= attempts && !hit; attempt++) {
    const a = await room(`orphan-room-${attempt}`)
    // The other room: an agent deadline is a timer the wake scheduler loop must keep ticking.
    const other = await room(`timer-room-${attempt}`)
    const ids = []
    for (let i = 0; i < children; i++) {
      const spawned = await rpc({ SpawnAgent: { session_id: a.id, alias: `sub-${i}`, provider: 'dev-stub', model: 'native-tui-idle', effort: null, account_profile: 'default', worktree_id: work } })
      const id = spawned.AgentSpawned.agent.id
      await rpc({ SetAgentParent: { session_id: a.id, agent_id: id, parent_agent_id: a.agent } })
      ids.push(id)
    }
    const deadlineMs = Date.now() + 25000
    await rpc({ SetAgentDeadline: { session_id: other.id, agent_id: other.agent, deadline_ms: deadlineMs } })
    const destroying = rpc({ DestroyAgent: { session_id: a.id, agent_id: a.agent } }).catch(() => {})
    for (const end = Date.now() + 60000; !deleted().has(a.agent) && Date.now() < end;) await delay(2)
    child.kill('SIGKILL')
    await new Promise(r => child.exitCode !== null || child.signalCode !== null ? r() : child.once('exit', r))
    await Promise.race([destroying, delay(2000)])
    const gone = deleted()
    const left = ids.filter(id => !gone.has(id))
    result.attempts.push({ attempt, parent_deleted: gone.has(a.agent), children_left_at_kill: left.length, deadline_ms: deadlineMs })
    save()
    await client.close()
    await start()
    if (gone.has(a.agent) && left.length > 0) hit = { other, ids, left, deadlineMs }
  }
  if (!hit) {
    check('the kill landed inside the cascade (orphans existed)', false, { attempts: result.attempts })
    process.exitCode = 2
  } else {
    const { other, ids, left, deadlineMs } = hit
    check('the kill left orphans behind', left.length > 0, { left: left.length, of: ids.length })
    for (const end = Date.now() + 60000; !ids.every(id => deleted().has(id)) && Date.now() < end;) await delay(250)
    const remaining = ids.filter(id => !deleted().has(id))
    check('the sweep ended every orphan', remaining.length === 0, { remaining: remaining.length })
    for (const end = Date.now() + 40000; Date.now() < end;) {
      const state = (await rpc({ GetAgentDeadline: { session_id: other.id, agent_id: other.agent } })).AgentDeadline.deadline
      if (state?.stopped_at_ms) {
        check('the deadline in the other room was reached on time', state.reached_at_ms - deadlineMs <= ON_TIME_MS, { late_ms: state.reached_at_ms - deadlineMs, limit_ms: ON_TIME_MS })
        break
      }
      if (Date.now() + 250 >= end) check('the deadline in the other room was reached', false, { state })
      await delay(250)
    }
    if (result.checks.some(c => !c.ok)) process.exitCode = 1
  }
} catch (error) {
  check('the drill ran to the end', false, { message: String(error.message).slice(0, 300) })
  process.exitCode = 1
} finally {
  await stop()
  result.finished_at = new Date().toISOString()
  save()
}
