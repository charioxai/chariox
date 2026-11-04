#!/usr/bin/env node
// MP-08/09/10/11 live kernel + official Codex app-server resume/cancel drill.
import assert from 'node:assert/strict'
import { spawn, execFileSync } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { access, mkdir, open, realpath, readdir, readFile, stat, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createHash, randomBytes } from 'node:crypto'
import WebSocket from 'ws'
import * as api from '../../../packages/kernel-client/dist/index.js'
import { isolatedKernelConfigToml } from './lib/drill-kernel-storage.mjs'
const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const options = Object.fromEntries(process.argv.slice(2).map(arg => { const [k, ...v] = arg.replace(/^--/, '').split('='); return [k, v.length ? v.join('=') : true] }))
if (options.help || options.h) {
  console.log('Usage: node apps/cli/scripts/live-agent-resume-drill.mjs --scenario=resume|cancel --binary=KERNEL --root=STATE_PARENT --workspace=NEW_WORKSPACE --profile=PRODUCT_CODEX_HOME [--restart-binary=FIXED_KERNEL] [--cli-entry=BUILT_CLI] [--cancel-mode=task|timer|stop]')
  console.log('Resume: seed on the base binary and restart with the fixed binary. Cancel: start directly on the fixed binary. Each binary requires a full commit hash in its adjacent .head file. Logs/results remain under STATE_PARENT; successful runs remove state and workspace, failures preserve them.')
  process.exit(0)
}
for (const key of ['binary', 'root', 'workspace', 'profile'])
  assert.equal(typeof options[key], 'string', `--${key}=... is required`)
options['cancel-mode'] ??= 'task'
assert.ok(['resume', 'cancel'].includes(options.scenario), '--scenario=resume or --scenario=cancel is required')
assert.ok(['task', 'timer', 'stop'].includes(options['cancel-mode']), '--cancel-mode must be task, timer or stop')
if (options['cancel-mode'] !== 'task')
  assert.equal(typeof options['cli-entry'], 'string', `--cancel-mode=${options['cancel-mode']} requires --cli-entry`)
assert.ok(path.isAbsolute(options.profile), '--profile must name an existing product-linked Codex scope')
const root = path.resolve(options.root), workspace = path.resolve(options.workspace)
const kernelBinary = path.resolve(options.binary)
const restartBinary = options['restart-binary'] ? path.resolve(options['restart-binary']) : kernelBinary
const binaryHead = async (binary) => readFile(`${binary}.head`, 'utf8').then(value => value.trim()).catch(() => null)
assert.ok(root !== repo && !root.startsWith(repo + path.sep) && workspace !== repo && !workspace.startsWith(repo + path.sep), 'state and workspace must be outside the repository')
const runRoot = path.join(root, `codex-resume-${Date.now()}`), home = path.join(runRoot, 'home')
const kernelSocketPath = path.join('/var/tmp/resumefix', `resume-kernel-${process.pid}-${randomBytes(4).toString('hex')}.sock`)
await mkdir(path.dirname(kernelSocketPath), { recursive: true, mode: 0o700 })
await access(workspace).then(() => { throw Error('--workspace must be a new, empty scratch path') }, () => { })
await access(kernelBinary).catch(() => { throw Error('--binary must name an existing kernel executable') })
await access(restartBinary).catch(() => { throw Error('--restart-binary must name an existing kernel executable') })
await access(options.profile).catch(() => { throw Error('--profile must be an existing product-linked Codex scope') })
await mkdir(home, { recursive: true, mode: 0o700 })
await mkdir(workspace, { recursive: true, mode: 0o700 })
// Do not inspect or copy provider state. The kernel imports the selected scope.
const ports = []
for (let i = 0; i < 4; i++) {
  const s = net.createServer()
  await new Promise(r => s.listen(0, '127.0.0.1', r))
  ports.push(s.address().port)
  await new Promise(r => s.close(r))
}
const [kernelPort, mcpPort, codexPort] = ports
const vaultPhrase = randomBytes(32).toString('hex')
const env = {
  ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith('CHARIOX_') && k !== 'CODEX_HOME')),
  HOME: home, XDG_CONFIG_HOME: path.join(home, 'xdg-config'), XDG_STATE_HOME: path.join(home, 'xdg-state'),
  XDG_CACHE_HOME: path.join(home, 'xdg-cache'), XDG_DATA_HOME: path.join(home, 'xdg-data'), CHARIOX_HOME: home,
  CHARIOX_KERNEL_HOST: '127.0.0.1', CHARIOX_KERNEL_PORT: String(kernelPort), CHARIOX_MCP_PORT: String(mcpPort), CHARIOX_CODEX_PORT: String(codexPort),
  CHARIOX_DAEMON_SOCKET: kernelSocketPath, CHARIOX_DAEMON_ID: `resume-drill-${Date.now()}`,
  CHARIOX_SESSION_HISTORY_DIR: path.join(home, 'history'), CHARIOX_KERNEL_RUNTIME_ROLE: 'general'
}
delete env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN
delete env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
await writeFile(path.join(home, 'config.toml'), isolatedKernelConfigToml(path.join(home, 'storage'), ['[credential_vault]', `path = ${JSON.stringify(path.join(home, 'vault'))}`, 'unlock_policy = "kernel_init"']), { mode: 0o600 })
await access(path.join(workspace, '.git')).catch(() => execFileSync('git', ['init', '-q', workspace]))
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const unwrap = (value, key) => {
  assert.ok(value?.[key], `expected ${key}`)
  return value[key]
}
let kernel, client, room, attachment, agent, cancelAgent, providerRunId, stage = 'startup'
const pids = []
const records = {
  area: 'MP-08 / MP-09 / MP-10 / MP-11', provider: 'codex', checks: [], seeded_tool_output_bytes: 0,
  initial_binary: kernelBinary, restart_binary: restartBinary,
  source_head: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repo, encoding: 'utf8' }).trim(),
  initial_binary_source_head: await binaryHead(kernelBinary), restart_binary_source_head: await binaryHead(restartBinary),
  initial_binary_sha256: execFileSync('sha256sum', [kernelBinary], { encoding: 'utf8' }).split(/\s+/)[0],
  restart_binary_sha256: execFileSync('sha256sum', [restartBinary], { encoding: 'utf8' }).split(/\s+/)[0]
}
assert.match(records.initial_binary_source_head ?? '', /^[0-9a-f]{40}$/, 'initial binary requires a matching .head commit marker')
assert.match(records.restart_binary_source_head ?? '', /^[0-9a-f]{40}$/, 'restart binary requires a matching .head commit marker')
let activeBinary = kernelBinary
let cli, automation, cliSocketPath
async function cwdOf(pid) {
  assert.ok(Number.isSafeInteger(pid) && pid > 1, 'unsafe child PID')
  return await realpath(`/proc/${pid}/cwd`).catch(() => null)
}
async function stopKernel() {
  if (!kernel || kernel.exitCode !== null || kernel.signalCode !== null)
    return
  assert.equal(await cwdOf(kernel.pid), workspace, 'kernel PID cwd changed; refusing signal')
  kernel.kill('SIGTERM')
  await Promise.race([new Promise(resolve => kernel.once('exit', resolve)), pause(7000)])
  if (kernel.exitCode === null && kernel.signalCode === null) {
    assert.equal(await cwdOf(kernel.pid), workspace, 'kernel PID cwd changed; refusing signal')
    kernel.kill('SIGKILL')
    await new Promise(resolve => kernel.once('exit', resolve))
  }
}
async function start() {
  const log = createWriteStream(path.join(runRoot, `kernel-${pids.length}.log`), { mode: 0o600 })
  kernel = spawn(activeBinary, [], { cwd: workspace, env, stdio: ['ignore', 'pipe', 'pipe'] })
  assert.ok(Number.isSafeInteger(kernel.pid) && kernel.pid > 1, 'unsafe kernel PID')
  pids.push(kernel.pid)
  kernel.stdout.pipe(log, { end: false })
  kernel.stderr.pipe(log, { end: false })
  kernel.once('close', () => log.end())
  for (let attempt = 0; attempt < 120; attempt += 1) {
    if (kernel.exitCode !== null)
      throw Error('kernel exited before readiness')
    client = new api.LocalIpcClient(`ws://127.0.0.1:${kernelPort}/kernel`, { localAuthEnvironment: env })
    try {
      await client.send(api.listSessionsRequest())
      return
    }
    catch {
      await client.close().catch(() => { })
      await pause(200)
    }
  }
  throw Error('kernel readiness timeout')
}
async function send(request, responseKind) {
  return unwrap(await client.send(request), responseKind)
}
async function snapshot() {
  const response = await send(api.getSessionStateRequest(room), 'SessionState')
  assert.ok(response.session && response.agent_activity, 'SessionState omitted session or top-level agent_activity')
  return { ...response.session, agent_activity: response.agent_activity }
}
async function unlockVault() {
  if (!room || !client)
    return
  const state = await snapshot()
  for (const interaction of state.active_interactions ?? []) {
    if (interaction.id.startsWith('vault-unlock-')) {
      await client.send(api.respondToInteractionRequest(room, interaction.id, 'passphrase', vaultPhrase))
    }
  }
}
async function sendWithVault(request) {
  let done = false
  let response
  let failure
  const pending = client.send(request)
    .then(value => { response = value }, error => { failure = error })
    .finally(() => { done = true })
  while (!done) {
    await pause(50)
    if (!done)
      await unlockVault()
  }
  await pending
  if (failure)
    throw failure
  return response
}
async function waitFor(label, predicate, timeoutMs = 120000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    await unlockVault()
    if (await predicate())
      return
    await pause(200)
  }
  throw Error(`${label} timeout`)
}
async function openAutomation(socketPath) {
  const deadline = Date.now() + 20000
  while (Date.now() < deadline) {
    try {
      const socket = net.createConnection(socketPath)
      await new Promise((resolve, reject) => { socket.once('connect', resolve); socket.once('error', reject) })
      socket.setEncoding('utf8')
      let buffer = ''
      let nextId = 1
      const pending = new Map()
      socket.on('data', chunk => {
        buffer += chunk
        while (buffer.includes('\n')) {
          const lineEnd = buffer.indexOf('\n')
          const line = buffer.slice(0, lineEnd).trim()
          buffer = buffer.slice(lineEnd + 1)
          if (!line)
            continue
          const response = JSON.parse(line)
          const waiter = pending.get(response.id)
          if (!waiter)
            continue
          pending.delete(response.id)
          response.ok ? waiter.resolve(response.data) : waiter.reject(Error(response.error ?? 'CLI automation failed'))
        }
      })
      socket.on('error', error => {
        for (const waiter of pending.values())
          waiter.reject(error); pending.clear()
      })
      return {
        send(action, fields = {}) {
          const id = nextId++
          return new Promise((resolve, reject) => {
            pending.set(id, { resolve, reject })
            socket.write(`${JSON.stringify({ id, action, ...fields })}\n`)
          })
        },
        close() { socket.destroy() },
      }
    }
    catch {
      await pause(250)
    }
  }
  throw Error('CLI automation socket readiness timeout')
}
function cliTranscriptEntries(snapshot, agentId) {
  const pane = snapshot.agentPanes?.[agentId]
  return Array.isArray(pane) && pane.length > 0 ? pane : snapshot.transcript?.entries ?? []
}
async function startCli() {
  if (!options['cli-entry'])
    return
  await access(options['cli-entry'])
  const automationSocket = path.join('/var/tmp/resumefix', `resume-cli-${process.pid}.sock`)
  cliSocketPath = automationSocket
  const log = createWriteStream(path.join(runRoot, 'cli.log'), { mode: 0o600 })
  const cliEnv = { ...env, CHARIOX_HOME: home }
  const cliArgs = ['bun', path.resolve(options['cli-entry']), '--kernel-url', `ws://127.0.0.1:${kernelPort}/kernel`,
    '--automation-socket', automationSocket, '--session', room, '--workspace', workspace,
    '--worktree', workspace, '--client-id', `resume-live-${process.pid}`]
  const quote = value => `'${String(value).replaceAll("'", "'\\''")}'`
  const command = cliArgs.map(quote).join(' ')
  cli = spawn('script', ['-qefc', command, '/dev/null'], { cwd: workspace, env: cliEnv, stdio: ['ignore', 'pipe', 'pipe'] })
  assert.ok(Number.isSafeInteger(cli.pid) && cli.pid > 1, 'unsafe CLI PID')
  pids.push(cli.pid)
  cli.stdout.pipe(log, { end: false })
  cli.stderr.pipe(log, { end: false })
  cli.once('close', () => log.end())
  automation = await openAutomation(automationSocket)
  await automation.send('ping')
  const snapshot = await automation.send('snapshot')
  assert.equal(snapshot.session?.id, room, 'real CLI did not attach to the drill session')
  records.cli_attached = true
}
async function stopCli() {
  automation?.close()
  if (!cli || cli.exitCode !== null || cli.signalCode !== null)
    return
  const descendants = await processDescendants(cli.pid)
  for (const pid of descendants.reverse())
    await signalOwnedPid(pid, 'SIGTERM')
  await signalOwnedPid(cli.pid, 'SIGTERM')
  await Promise.race([new Promise(resolve => cli.once('exit', resolve)), pause(5000)])
  for (const pid of descendants) {
    if (await cwdOf(pid))
      await signalOwnedPid(pid, 'SIGKILL')
  }
  if (cli.exitCode === null && cli.signalCode === null) {
    await signalOwnedPid(cli.pid, 'SIGKILL')
    await new Promise(resolve => cli.once('exit', resolve))
  }
  for (const pid of descendants)
    assert.equal(await cwdOf(pid), null, 'CLI child remains after cleanup')
  assert.ok(cli.exitCode !== null || cli.signalCode !== null, 'CLI did not exit after SIGTERM')
}
async function signalOwnedPid(pid, signal) {
  assert.ok(Number.isSafeInteger(pid) && pid > 1, 'refuse unsafe process signal target')
  const cwd = await cwdOf(pid)
  if (cwd === null)
    return
  assert.equal(cwd, workspace, 'process cwd changed; refusing signal')
  process.kill(pid, signal)
}
async function processDescendants(rootPid) {
  const parentByPid = new Map()
  for (const name of await readdir('/proc')) {
    if (!/^\d+$/.test(name))
      continue
    const pid = Number(name)
    if (pid <= 1)
      continue
    try {
      const line = await readFile(`/proc/${pid}/stat`, 'utf8')
      const fields = line.slice(line.lastIndexOf(')') + 1).trim().split(/\s+/)
      parentByPid.set(pid, Number(fields[1]))
    }
    catch { }
  }
  const result = []
  const parents = new Set([rootPid])
  for (let changed = true; changed;) {
    changed = false
    for (const [pid, parent] of parentByPid) {
      if (!parents.has(pid) && parents.has(parent)) {
        parents.add(pid)
        result.push(pid)
        changed = true
      }
    }
  }
  for (const pid of result)
    assert.equal(await cwdOf(pid), workspace, 'CLI descendant cwd changed; refusing signal')
  return result
}
async function appServerCall(endpoint, method, params, maxPayload = 16 * 1024 * 1024) {
  const socket = new WebSocket(endpoint, { maxPayload })
  const pending = new Map()
  let nextId = 0
  socket.on('message', data => {
    const message = JSON.parse(data.toString())
    const request = pending.get(message.id)
    if (!request)
      return
    pending.delete(message.id)
    clearTimeout(request.timer)
    if (message.error)
      request.reject(Error(`Codex app-server rejected ${method}: ${message.error.message ?? message.error.code ?? 'protocol error'}`))
    else
      request.resolve(message.result)
  })
  await new Promise((resolve, reject) => { socket.once('open', resolve); socket.once('error', reject) })
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = ++nextId
    const timer = setTimeout(() => { pending.delete(id); reject(Error(`Codex ${method} timed out`)) }, 120000)
    pending.set(id, { resolve, reject, timer })
    socket.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
  try {
    await request('initialize', { clientInfo: { name: 'chariox-live-resume-drill', version: '1.0.0' }, capabilities: { experimentalApi: true } })
    socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'initialized', params: {} }))
    return await request(method, params)
  }
  finally {
    for (const request of pending.values()) {
      clearTimeout(request.timer)
      request.reject(Error('Codex app-server socket closed'))
    }
    pending.clear()
    socket.close()
  }
}
async function appServerMcpMetadata(endpoint, threadId) {
  const response = await appServerCall(endpoint, 'mcpServerStatus/list', { detail: 'full', threadId })
  const servers = Array.isArray(response?.data) ? response.data : []
  return servers.map(server => ({
    name: server.name,
    tools: server.tools && typeof server.tools === 'object' ? Object.keys(server.tools) : [],
  }))
}
async function measureFullResume(endpoint, threadId) {
  const socket = new WebSocket(endpoint, { maxPayload: 256 * 1024 * 1024 })
  let nextId = 0
  let currentRequest
  const opened = new Promise((resolve, reject) => { socket.once('open', resolve); socket.once('error', reject) })
  socket.on('message', data => {
    const header = Buffer.isBuffer(data) ? data.subarray(0, 128).toString('utf8') : String(data).slice(0, 128)
    const match = header.match(/^\{"id":(\d+),/)
    if (!match || Number(match[1]) !== currentRequest?.id)
      return
    clearTimeout(currentRequest.timer)
    currentRequest.resolve(Buffer.byteLength(data))
    currentRequest = undefined
  })
  socket.on('error', () => currentRequest?.reject(Error('Codex app-server socket failed')))
  await opened
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = ++nextId
    const timer = setTimeout(() => {
      currentRequest = undefined
      reject(Error(`Codex ${method} timed out`))
    }, 120000)
    currentRequest = { id, resolve, reject, timer }
    socket.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
  try {
    await request('initialize', { clientInfo: { name: 'chariox-live-resume-drill', version: '1.0.0' }, capabilities: { experimentalApi: true } })
    socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'initialized', params: {} }))
    return await request('thread/resume', { threadId, excludeTurns: false })
  }
  finally {
    if (currentRequest) {
      clearTimeout(currentRequest.timer)
      currentRequest.reject(Error('Codex app-server socket closed'))
      currentRequest = undefined
    }
    socket.close()
  }
}
async function nativeThreadPath(endpoint, threadId) {
  const response = await appServerCall(endpoint, 'thread/read', { threadId, includeTurns: false })
  const thread = response?.thread ?? response
  assert.equal(typeof thread?.path, 'string', 'official thread/read omitted native thread path')
  return thread.path
}
async function nativeRolloutBytes(rolloutPath) {
  const info = await stat(rolloutPath)
  assert.ok(info.isFile(), 'official native thread path is not a file')
  return info.size
}
try {
  await start()
  stage = 'profile'
  const profiles = await send(api.listProviderAccountProfilesRequest('codex'), 'ProviderAccountProfilesListed')
  let profile = profiles.profiles.find(p => p.label === 'resume-drill')
  if (!profile)
    profile = unwrap(await client.send(api.linkProviderAccountProfileRequest('codex', 'resume-drill', options.profile)), 'ProviderAccountProfile').profile
  await client.send(api.refreshProviderAccountProfileRequest('codex', profile.profile_id))
  stage = 'session'
  const created = await send(api.createSessionRequest(workspace, workspace, 'resume-live', { provider: 'codex', model: 'gpt-6-luna', effort: 'low', account_profile: profile.profile_id, execution_mode: 'build', permission_level: 'yolo' }), 'SessionCreated')
  room = created.session.id
  agent = created.agent.id
  cancelAgent = unwrap(await client.send(api.spawnAgentRequest(room, 'codex', 'cancel-check', 'gpt-6-luna', workspace, 'low', 'build', 'yolo', undefined, undefined, undefined, profile.profile_id)), 'AgentSpawned').agent.id
  attachment = (await send(api.attachToSessionRequest(room, 'resume-live'), 'SessionAttached')).attachment.id
  await startCli()
  const marker = `RESUME_${Date.now()}`
  let out = '', s, a, thread, run
  const pump = async (targetAgent = agent) => {
    const t = await send(api.pumpTerminalOutputRequest(room, attachment), 'TerminalOutput')
    for (const r of t.records ?? []) {
      if (r.agent_id === targetAgent && r.kind === 'provider_output')
        out += Buffer.from(r.bytes ?? []).toString('utf8')
    }
    const state = await snapshot()
    providerRunId ??= state.active_provider_run_id ?? state.agents.find(x => x.id === targetAgent)?.active_provider_run_id
  }
  if (options.scenario !== 'cancel') {
    stage = 'warmup'
    await sendWithVault(api.submitPromptRequest(room, attachment, agent, `Remember ${marker}. Reply only READY.`, []))
    await waitFor('warm provider turn', async () => { await pump(); const state = await snapshot(); return out.includes('READY') && !state.prompt_states?.[agent]?.active_prompt })
    s = await snapshot()
    a = s.agents.find(x => x.id === agent)
    thread = Object.values(a.provider_resume_state ?? {}).find(x => typeof x === 'string' && x.length)
    assert.ok(thread, 'Codex thread id missing')
    assert.ok(providerRunId, 'provider run id was never observed during warmup')
    run = await send(api.getProviderRunRequest(providerRunId), 'ProviderRun')
    assert.ok(run.provider_run?.structured_endpoint, 'live Codex app-server endpoint not published for provider run')
  }
  if (options.scenario !== 'resume') {
    try {
      stage = 'cancel-timer'
      let cancelTask
      const timerPrompt = 'Use chariox.events.timer once to arm a 10 minute timer labelled resume-cancel. Then use chariox.events.yield to wait for it. Keep the task waiting.'
      await sendWithVault(api.submitPromptRequest(room, attachment, cancelAgent, timerPrompt, []))
      await waitFor('parked timer', async () => { s = await snapshot(); return (s.agent_wakes ?? []).some(wake => wake.agent_id === cancelAgent && wake.state === 'scheduled') })
      await waitFor('parked timer task', async () => { s = await snapshot(); return (s.agent_tasks ?? []).some(task => task.agent_id === cancelAgent && task.state === 'waiting') })
      s = await snapshot()
      cancelTask = s.agent_tasks.find(task => task.agent_id === cancelAgent && task.state === 'waiting')
      assert.ok(cancelTask, 'parked timer task missing')
      records.cancel_target = {
        session_id: room,
        task_id: cancelTask.task_id,
        task_revision: cancelTask.revision,
        task_owner_matches_session: Boolean(cancelTask.owner_user_id && cancelTask.owner_user_id === s.owner_user_id),
      }
      assert.ok(!s.prompt_states?.[cancelAgent]?.active_prompt, 'parked timer has no active prompt')
      if (automation) {
        let stopResult
        if (options['cancel-mode'] === 'stop') {
          await automation.send('workspace_shell_exec', { command: '/agent focus cancel-check' })
          stopResult = await automation.send('workspace_shell_exec', { command: 'stop' })
        }
        else if (options['cancel-mode'] === 'timer') {
          const wake = s.agent_wakes.find(w => w.agent_id === cancelAgent && w.state === 'scheduled')
          assert.ok(wake, 'scheduled owner timer missing')
          records.cancel_target.wake_id = wake.id
          await automation.send('workspace_shell_exec', { command: '/agent focus cancel-check' })
          stopResult = await automation.send('workspace_shell_exec', { command: `stop timer ${wake.id}` })
          await waitFor('CLI timer cancellation', async () => {
            s = await snapshot()
            return s.agent_wakes?.some(w => w.id === wake.id && w.state === 'cancelled')
          })
          const stillWaiting = s.agent_tasks?.some(t => t.task_id === cancelTask.task_id && t.state === 'waiting')
          assert.ok(stillWaiting, 'timer cancellation should preserve the parked owner task')

        }
        else {
          stopResult = await automation.send('workspace_shell_exec', { command: `stop task ${cancelTask.task_id} ${cancelTask.revision}` })
        }
        assert.equal(stopResult.snapshot?.session?.id, room, 'CLI shell command returned a different session')
        if (options['cancel-mode'] !== 'timer')
          await waitFor('CLI task cancellation', async () => { s = await snapshot(); return s.agent_tasks?.some(t => t.task_id === cancelTask.task_id && t.state === 'cancelled') })
      }
      else {
        const cancel = unwrap(await client.send(api.cancelAgentTaskRequest(room, attachment, cancelTask.task_id, cancelTask.revision)), 'AgentTaskUpdated')
        assert.equal(cancel.task?.state, 'cancelled')
      }
      if (automation) {
        await automation.send('workspace_shell_exec', { command: '/agent focus cancel-check' })
        await automation.send('switch_screen', { screen: 'agents' })
        const ownerUpdatePrompt = 'Please check for any updates relevant to your work. Tell me what action they require and acknowledge any notice that needs acknowledgment.'
        const beforeOwnerUpdate = await snapshot()
        const priorCompletedOwnerPrompt = beforeOwnerUpdate.agent_activity?.[cancelAgent]?.last_completed_turn?.prompt_id ?? null
        await automation.send('submit_prompt', { prompt: ownerUpdatePrompt })
        let ownerUpdateTranscript = ''
        await waitFor('post-cancellation owner update', async () => {
          const current = await snapshot()
          const completedPrompt = current.agent_activity?.[cancelAgent]?.last_completed_turn?.prompt_id ?? null
          const newTurnSettled = Boolean(completedPrompt && completedPrompt !== priorCompletedOwnerPrompt)
          const view = await automation.send('snapshot')
          const entries = cliTranscriptEntries(view, cancelAgent)
          const promptIndex = entries.findIndex(entry => (entry.text ?? '').includes(ownerUpdatePrompt))
          ownerUpdateTranscript = promptIndex >= 0 ? entries.slice(promptIndex + 1).map(entry => entry.text ?? '').join('\n') : ''
          records.owner_update_prompt_active = Boolean(current.prompt_states?.[cancelAgent]?.active_prompt)
          records.owner_update_new_turn_settled = newTurnSettled
          return newTurnSettled && !records.owner_update_prompt_active && ownerUpdateTranscript.length > 0
        })
        await writeFile(path.join(runRoot, 'owner-update-transcript.txt'), ownerUpdateTranscript, { mode: 0o600 })
        records.owner_update_turn_completed = true
        records.owner_update_transcript_bytes_after_prompt = Buffer.byteLength(ownerUpdateTranscript)
        records.owner_update_mentions_cancellation = /cancel|stopped|keep .* work/i.test(ownerUpdateTranscript)
        records.owner_update_reports_timer_cancellation = /timer.{0,40}cancel|cancel.{0,40}timer/i.test(ownerUpdateTranscript)
        records.owner_update_acknowledgment_requested = true
        records.owner_update_acknowledgment_response = /acknowledg|handled/i.test(ownerUpdateTranscript)
        if (options['cancel-mode'] === 'timer') {
          assert.ok(records.owner_update_reports_timer_cancellation, 'next owner prompt did not report the cancelled timer receipt')
        }
      }
      await stopCli()
      records.cancellation = 'PASS'
      records.checks.push(automation ? `real CLI ${options['cancel-mode']} cancellation with no active prompt` : 'kernel IPC cancelled parked timer task with no active prompt')
    }
    catch (error) {
      records.cancellation = 'FAIL'
      records.cancellation_reason = String(error?.message ?? error).replace(/[A-Za-z0-9_+/=.:@-]{32,}/g, '<redacted>').slice(0, 250)
    }
  }
  if (options.scenario !== 'cancel') {
    stage = 'seed-history'
    const artifactName = 'resume-history-seed.txt'
    const artifactPath = path.join(workspace, artifactName)
    const artifactChunk = Buffer.alloc(1024 * 1024, 120)
    const artifact = await open(artifactPath, 'w', 0o600)
    try {
      await artifact.write(artifactChunk)
    }
    finally {
      await artifact.close()
    }
    const mcpMetadata = await appServerMcpMetadata(run.provider_run.structured_endpoint, thread)
    const charioxServer = mcpMetadata.find(server => server.name === 'chariox')
    assert.ok(charioxServer?.tools.includes('chariox.read_artifact'), 'official app-server MCP catalog does not expose chariox.read_artifact')
    records.mcp_seed_tool_available = true
    records.seeded_tool_output_bytes = 0
    records.seed_method = 'real Codex model turn invoking chariox.read_artifact 65 times'
    out = ''
    await sendWithVault(api.submitPromptRequest(room, attachment, agent, `Call chariox.read_artifact on ${artifactName} exactly 65 times, one call per invocation. Do not summarize file contents; after all calls, reply only SEED_DONE.`, []))
    await waitFor('large native MCP tool-history turn', async () => {
      await pump()
      const state = await snapshot()
      return out.includes('SEED_DONE') && !state.prompt_states?.[agent]?.active_prompt
    }, 600000)
    records.seeded_tool_output_bytes = 65 * artifactChunk.byteLength
    out = ''
    await sendWithVault(api.submitPromptRequest(room, attachment, agent, `Remember this marker for the next prompt: ${marker}. Reply only READY.`, []))
    await waitFor('post-seed marker turn', async () => {
      await pump()
      const state = await snapshot()
      return out.includes('READY') && !state.prompt_states?.[agent]?.active_prompt
    })
    records.post_seed_marker_turn_completed = true
    s = await snapshot()
    a = s.agents.find(x => x.id === agent)
    thread = Object.values(a.provider_resume_state ?? {}).find(x => typeof x === 'string' && x.length)
    assert.ok(thread, 'Codex native thread id disappeared after history seeding turn')
    records.native_thread_sha256 = createHash('sha256').update(thread).digest('hex')
    const rolloutPath = await nativeThreadPath(run.provider_run.structured_endpoint, thread)
    records.native_rollout_bytes = await nativeRolloutBytes(rolloutPath)
    assert.ok(records.seeded_tool_output_bytes > 64 * 1024 * 1024)
    assert.ok(records.native_rollout_bytes > 64 * 1024 * 1024, 'native Codex rollout file did not exceed 64 MiB after real MCP tool outputs')
    records.full_resume_reply_bytes = await measureFullResume(run.provider_run.structured_endpoint, thread)
    assert.ok(records.full_resume_reply_bytes > 64 * 1024 * 1024, 'official full thread/resume reply did not reproduce the 64 MiB boundary')
    stage = 'restart'
    await client.close()
    await stopKernel()
    activeBinary = restartBinary
    await start()
    s = await snapshot()
    a = s.agents.find(x => x.id === agent)
    assert.ok(a, 'agent missing after restart')
    thread = Object.values(a.provider_resume_state ?? {}).find(x => typeof x === 'string' && x.length)
    assert.equal(createHash('sha256').update(thread).digest('hex'), records.native_thread_sha256)
    attachment = (await send(api.attachToSessionRequest(room, 'resume-after-restart'), 'SessionAttached')).attachment.id
    if (automation)
      await waitFor('real CLI reattached after kernel restart', async () => { const view = await automation.send('snapshot'); return view.session?.id === room })
    if (automation) {
      await automation.send('workspace_shell_exec', { command: '/agent focus resume-live' })
      const focus = await automation.send('snapshot')
      records.cli_focused_agent_id = focus.session?.focusedAgentId ?? null
      assert.equal(records.cli_focused_agent_id, agent, 'real CLI focus did not select the resumed agent')
      await automation.send('switch_screen', { screen: 'agents' })
    }
    stage = 'next-turn'
    out = ''
    const recallPrompt = 'What marker did you memorize? Reply only with it.'
    const beforeRecall = await snapshot()
    const previousCompletedPrompt = beforeRecall.agent_activity?.[agent]?.last_completed_turn?.prompt_id ?? null
    if (automation)
      await automation.send('submit_prompt', { prompt: recallPrompt })
    else
      await sendWithVault(api.submitPromptRequest(room, attachment, agent, recallPrompt, []))
    let tuiEntries = []
    let postPromptTranscript = ''
    await waitFor('post-restart prompt completion', async () => {
      await pump()
      if (automation) {
        tuiEntries = cliTranscriptEntries(await automation.send('snapshot'), agent)
        const promptIndex = tuiEntries.findIndex(entry => (entry.text ?? '').includes(recallPrompt))
        postPromptTranscript = promptIndex >= 0 ? tuiEntries.slice(promptIndex + 1).map(entry => entry.text ?? '').join('\n') : ''
      }
      const state = await snapshot()
      const completedPrompt = state.agent_activity?.[agent]?.last_completed_turn?.prompt_id ?? null
      const settledNewTurn = Boolean(completedPrompt && completedPrompt !== previousCompletedPrompt)
      records.post_restart_prompt_active = Boolean(state.prompt_states?.[agent]?.active_prompt)
      records.post_restart_prompt_status = state.agent_activity?.[agent]?.prompt_status ?? null
      records.post_restart_new_turn_settled = settledNewTurn
      records.post_restart_output_bytes = Buffer.byteLength(automation ? postPromptTranscript : out)
      records.post_restart_marker_recalled = (automation ? postPromptTranscript : out).includes(marker)
      const completed = !state.prompt_states?.[agent]?.active_prompt && settledNewTurn
      return completed && (!automation || postPromptTranscript.length > 0)
    }, 600000)
    s = await snapshot()
    a = s.agents.find(x => x.id === agent)
    records.post_restart_prompt_completed = true
    const finalThread = Object.values(a.provider_resume_state ?? {}).find(x => typeof x === 'string' && x.length)
    assert.equal(a.id, agent, 'agent identity changed after post-restart turn')
    assert.equal(createHash('sha256').update(finalThread).digest('hex'), records.native_thread_sha256, 'provider thread identity changed during post-restart turn')
    assert.ok(records.post_restart_marker_recalled, 'post-restart prompt completed but did not recall the unprompted marker')
    records.checks.push(automation ? 'real CLI prompt recalled marker in same agent and native thread after kernel restart' : 'Codex same agent and native thread completed an unprompted marker recall after kernel restart')
  }
  records.status = records.cancellation === 'FAIL' ? 'FAIL' : 'PASS'
  if (records.status === 'FAIL')
    process.exitCode = 1
}
catch (e) {
  records.status = 'FAIL'
  records.stage = stage
  records.failure_reason = String(e?.message ?? e).replace(/[A-Za-z0-9_+/=.:@-]{32,}/g, '<redacted>').slice(0, 500)
  records.room_id = room ?? null
  records.attachment_id = attachment ?? null
  records.resume_agent_id = agent ?? null
  records.cancel_agent_id = cancelAgent ?? null
  if (client && room) {
    try {
      const state = await snapshot()
      const activity = state.agent_activity?.[agent] ?? {}
      records.failure_snapshot = {
        active_prompt: Boolean(state.prompt_states?.[agent]?.active_prompt),
        prompt_status: state.agent_activity?.[agent]?.prompt_status ?? null,
        last_completed_prompt_id: activity.last_completed_turn?.prompt_id ?? null,
        cli_focus_agent_id: automation ? (await automation.send('snapshot')).session?.focusedAgentId ?? null : null,
      }
    }
    catch { records.failure_snapshot = { unavailable: true } }
  }
  process.exitCode = 1
}
finally {
  const cleanupFailures = []
  try { await stopCli() }
  catch (error) { cleanupFailures.push(`CLI cleanup failed: ${String(error?.message ?? error).slice(0, 200)}`) }
  try { await client?.close() }
  catch (error) { cleanupFailures.push(`IPC cleanup failed: ${String(error?.message ?? error).slice(0, 200)}`) }
  let kernelDescendants = []
  if (kernel && kernel.exitCode === null && kernel.signalCode === null) {
    try { kernelDescendants = await processDescendants(kernel.pid) }
    catch (error) { cleanupFailures.push(`could not verify kernel descendants: ${String(error?.message ?? error).slice(0, 200)}`) }
  }
  records.kernel_descendants_before_stop = kernelDescendants
  try { await stopKernel() }
  catch (error) { cleanupFailures.push(`kernel cleanup failed: ${String(error?.message ?? error).slice(0, 200)}`) }
  const remainingKernelDescendants = []
  for (const pid of kernelDescendants) {
    if (await cwdOf(pid)) remainingKernelDescendants.push(pid)
  }
  records.kernel_descendants_remaining = remainingKernelDescendants
  if (remainingKernelDescendants.length)
    cleanupFailures.push('kernel descendants remain after shutdown')
  if (cliSocketPath) {
    try { await rm(cliSocketPath, { force: true }) }
    catch (error) { cleanupFailures.push(`CLI socket cleanup failed: ${String(error?.message ?? error).slice(0, 200)}`) }
  }
  try { await rm(kernelSocketPath, { force: true }) }
  catch (error) { cleanupFailures.push(`kernel socket cleanup failed: ${String(error?.message ?? error).slice(0, 200)}`) }
  records.cleanup_failures = cleanupFailures
  if (cleanupFailures.length) {
    records.status = 'FAIL'
    process.exitCode = 1
  }
  records.kernel_pids = pids
  if (records.status === 'FAIL') {
    records.preserved_home = home
    records.preserved_workspace = workspace
  }
  await writeFile(path.join(runRoot, 'result.json'), JSON.stringify(records, null, 2) + '\n', { mode: 0o600 })
  console.log(JSON.stringify({ status: records.status, result: path.join(runRoot, 'result.json') }))
  if (records.status !== 'FAIL') {
    await rm(home, { recursive: true, force: true })
    await rm(workspace, { recursive: true, force: true })
  }
}
