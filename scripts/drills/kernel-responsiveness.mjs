// Real kernel and shared client; all state and evidence are outside the checkout.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdirSync, mkdtempSync, openSync, writeFileSync, readFileSync, fsyncSync, closeSync } from 'node:fs'
import { createServer } from 'node:net'
import { resolve, join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { createHash } from 'node:crypto'
import { LocalIpcClient } from '../../packages/kernel-client/dist/ipc.js'

const args = new Map()
for (let i = 2; i < process.argv.length; i += 2) args.set(process.argv[i], process.argv[i + 1])
const binary = resolve(args.get('--kernel-binary'))
const stateRoot = resolve(args.get('--state-root'))
const evidence = resolve(args.get('--evidence-dir'))
const cycles = Number(args.get('--cycles') ?? 1000)
const samples = Number(args.get('--samples') ?? 30)
assert(stateRoot.startsWith('/root/.chariox/dev/'), 'isolated state root required')
mkdirSync(stateRoot, { recursive: true })
mkdirSync(evidence, { recursive: true })
const root = mkdtempSync(join(stateRoot, 'run-'))
const output = { started_at: new Date().toISOString(), host_load: readFileSync("/proc/loadavg", "utf8").trim(), binary_sha256: createHash('sha256').update(readFileSync(binary)).digest('hex'), head: args.get('--head'), state_root: root, cycles, samples, checkpoints: [], processes: [] }
function save() { writeFileSync(join(evidence, 'measurements.json'), JSON.stringify(output, null, 2) + '\n') }
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
const env = { PATH: process.env.PATH, IS_SANDBOX: '1', SHELL: '/bin/bash', HOME: join(root, 'home'), TMPDIR: join(root, 'tmp'),
  XDG_CONFIG_HOME: join(root, 'config'), XDG_CACHE_HOME: join(root, 'cache'), XDG_STATE_HOME: join(root, 'state'),
  CHARIOX_HOME: join(root, 'chariox'), CHARIOX_LOG_DIR: join(root, 'logs'), CHARIOX_SESSION_HISTORY_DIR: join(root, 'history'),
  CHARIOX_DAEMON_SOCKET: join(root, 'daemon.sock'), CHARIOX_DAEMON_ALIAS: 'responsiveness', CHARIOX_MACHINE_ALIAS: 'responsiveness',
  CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]),
  ...(args.has('--preload') ? { LD_PRELOAD: resolve(args.get('--preload')), CHARIOX_SYNC_STALL_ROOT: root } : {}) }
const probe = openSync(join(root, 'sync-probe'), 'w', 0o600), syncProbe = []
for (let i = 0; i < 100; i++) { writeFileSync(probe, 'x'); const at = performance.now(); fsyncSync(probe); syncProbe.push(performance.now()-at) }
closeSync(probe)
syncProbe.sort((a,b) => a-b)
output.fsync_probe = { n: syncProbe.length, p50_ms: syncProbe[49], p95_ms: syncProbe[94], max_ms: syncProbe.at(-1) }
const log = openSync(join(evidence, 'kernel.log'), 'a', 0o600)
let child, client, load
async function start() {
  child = spawn(binary, [], { env, stdio: ['ignore', log, log] })
  assert(Number.isInteger(child.pid) && child.pid > 1)
  output.processes.push(child.pid); save()
  const since = performance.now()
  client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
  for (let i = 0; ; i++) {
    try { if ((await client.send({ RelayStatus: null })).RelayStatus) break } catch {}
    assert(child.exitCode === null && child.signalCode === null, 'kernel exited')
    assert(i < 300, 'kernel startup timed out'); await delay(100)
  }
  return performance.now() - since
}
async function stop() {
  await client?.close()
  if (child?.exitCode === null && child.signalCode === null) {
    assert(Number.isInteger(child.pid) && child.pid > 1)
    child.kill('SIGTERM')
    for (let i = 0; i < 100 && child.exitCode === null && child.signalCode === null; i++) await delay(100)
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
    await new Promise(r => child.exitCode !== null || child.signalCode !== null ? r() : child.once('exit', r))
  }
}
const timings = new Map()
async function rpc(request, measure = false) {
  const at = performance.now(), kind = request.RoomTerminal ? `RoomTerminal/${request.RoomTerminal.operation.operation}` : Object.keys(request)[0]
  const result = await client.send(request)
  if (result.Error) throw new Error(`${kind}: ${result.Error.code}: ${result.Error.message}`)
  if (measure) {
    if (!timings.has(kind)) timings.set(kind, [])
    timings.get(kind).push(performance.now() - at)
  }
  return result
}
function distribution(values) {
  values.sort((a,b) => a-b)
  const at = p => values[Math.max(0, Math.ceil(values.length * p) - 1)]
  return { n: values.length, p50_ms: at(.5), p95_ms: at(.95), p99_ms: at(.99), max_ms: at(1) }
}
async function create(measure) {
  return (await rpc({ CreateSession: { workspace_id: join(root, 'work'), worktree_id: join(root, 'work') } }, measure)).SessionCreated
}
async function cycle(measure = false, remove = false) {
  const made = await create(measure)
  const sessionId = made.session.id
  const attached = (await rpc({ AttachToSession: { session_id: sessionId, client_id: 'responsiveness', capability_level: 'FullTerminal' } }, measure)).SessionAttached
  await rpc({ RunShellCommand: { session_id: sessionId, attachment_id: attached.attachment.id, command: '/bin/true', args: [], timeout_ms: 1000 } }, measure)
  await rpc({ DetachFromSession: { session_id: sessionId, attachment_id: attached.attachment.id } }, measure)
  await rpc(remove ? { DeleteSession: { session_ref: sessionId, workspace_id: null } } : { EndSession: { session_id: sessionId } }, measure)
  return sessionId
}
async function checkpoint(ended) {
  await delay(1100)
  const cycleTimings = Object.fromEntries([...timings].map(([kind, values]) => [kind, distribution(values)]))
  timings.clear()
  const healthSizes = []
  for (let i = 0; i < samples; i++) {
    await rpc({ ListSessions: {} }, true)
    const health = await rpc({ GetDaemonHealth: null }, true)
    healthSizes.push(Buffer.byteLength(JSON.stringify(health)))
  }
  const health = (await rpc({ GetDaemonHealth: null })).DaemonHealth.projection
  const status = readFileSync(`/proc/${child.pid}/status`, 'utf8')
  output.checkpoints.push({ ended, cycle_timings: cycleTimings, rss_kb: Number(status.match(/^VmRSS:\s+(\d+)/m)[1]),
    requests: Object.fromEntries([...timings].map(([kind, values]) => [kind, distribution(values)])),
    health_bytes: Math.max(...healthSizes), health, ...(load ? { load_status: await load.status() } : {}) })
  save(); console.log(`checkpoint ${ended}: ${Math.max(...healthSizes)} bytes`)
}
async function beginLoad() {
  const { startProviderLoad } = await import('./kernel-responsiveness/load.mjs')
  return startProviderLoad({ rpc, root, profilePath: process.env.KR_CODEX_PROFILE,
    count: Number(args.get('--load-agents')), objects: Number(args.get('--load-objects') ?? 1000), report: output, save })
}
try {
  output.startup_ms = await start()
  const warmup = Number(args.get('--warmup') ?? 0)
  for (let i = 0; i < warmup; i++) await cycle(false, true)
  output.warmup_deleted_sessions = warmup
  if (args.has('--mismatch-sessions')) {
    const { injectMissingAgents } = await import('./kernel-responsiveness/corruption.mjs')
    const sessions = [], count = Number(args.get('--mismatch-sessions'))
    for (let i=0;i<count;i++) { sessions.push((await create(false)).session); if(i%100===0)console.log(`fixture sessions ${i}`) }
    await stop()
    injectMissingAgents(root,sessions,Number(args.get('--mismatches') ?? 8000))
    await start()
  }
  if (args.get('--load-before-cycles') === '1') load = await beginLoad()
  if (args.get('--injections') === '1') {
    const { isolationDrills } = await import('./kernel-responsiveness/isolation.mjs')
    await isolationDrills({ rpc, create, root, report: output, save, connect: () => new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: env }), relay: args.has('--relay-binary') ? { binary, relayBinary: resolve(args.get('--relay-binary')), env, root, evidence, freePort } : null })
  }
  await checkpoint(0)
  let lastId
  for (let i = 1; i <= cycles; i++) {
    lastId = await cycle(true)
    if (i % 100 === 0) { console.log(`ended ${i}`); output.completed_cycles = i; save() }
    if (i === 1000 || i === cycles) await checkpoint(i)
  }
  if (cycles === 0) lastId = await cycle(true)
  if (!load && Number(args.get('--load-agents') ?? 0) > 0) load = await beginLoad()
  if (Number(args.get('--latency-samples') ?? 0) > 0) {
    const { measureLatency } = await import('./kernel-responsiveness/latency.mjs')
    await measureLatency({ rpc, create, cycle, root, mcpUrl: `http://127.0.0.1:${ports[1]}/mcp`, samples: Number(args.get('--latency-samples')),
      spacing: Number(args.get('--sample-spacing') ?? 1000), report: output, save, load, timings, distribution })
  }
  if (args.has('--mismatch-sessions')) {
    const { measureCorruptHealth } = await import('./kernel-responsiveness/corruption.mjs')
    await measureCorruptHealth({ rpc, report: output, save, expected: Number(args.get('--mismatches') ?? 8000), root, distribution })
  }
  if (load) { await load.stop(); load = undefined }
  output.last_session_id = lastId
  output.history_readable = !!(await rpc({ GetSessionState: { session_id: lastId } })).SessionState
  await stop()
  output.restart_ms = await start()
  output.history_readable_after_restart = !!(await rpc({ GetSessionState: { session_id: lastId } })).SessionState
  await checkpoint(cycles)
  output.completed = true; save()
} finally {
  for (const agent of output.provider_load?.agents ?? []) {
    try { await rpc({ EndSession: { session_id: agent.session_id } }) } catch { output.provider_cleanup_failed = true }
  }
  await stop(); output.cleaned_processes = true; save() }
