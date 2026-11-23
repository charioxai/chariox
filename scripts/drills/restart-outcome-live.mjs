#!/usr/bin/env node
// Queue row 372 live cells O0-O3: what ended a turn, and the kernel pid file. Adapted from live-provider-restart-drill.mjs.
import assert from 'node:assert/strict'
import { createHash, randomBytes } from 'node:crypto'
import { existsSync } from 'node:fs'
import { access, chmod, copyFile, mkdir, readFile, readlink, readdir, rename, rm, writeFile } from 'node:fs/promises'
import { spawn } from 'node:child_process'
import { openSync } from 'node:fs'
import { createServer } from 'node:net'
import { setTimeout as delay } from 'node:timers/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repo = process.env.DRILL_REPO ?? path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const clientDist = process.env.KERNEL_CLIENT_DIST ?? path.join(repo, 'packages/kernel-client/dist')
const { LocalIpcClient } = await import(`${clientDist}/ipc.js`)
const req = await import(`${clientDist}/ipc-requests.js`)
const { isolatedKernelConfigToml } = await import(path.join(repo, 'apps/cli/scripts/lib/drill-kernel-storage.mjs'))
const options = Object.fromEntries(process.argv.slice(2).map(arg => {
  const [key, ...parts] = arg.replace(/^--/, '').split('=')
  return [key, parts.length ? parts.join('=') : true]
}))
if (options.help || options.h) {
  console.log('Usage: node scripts/drills/restart-outcome-live.mjs --scenario=o1|o2|o0|o4 --provider=claude --binary=KERNEL --root=STATE_PARENT --workspace=NEW_WORKSPACE --evidence=EVIDENCE_DIR [--token-file=PATH | --opencode-home=PATH]')
  process.exit(0)
}
for (const key of ['scenario', 'provider', 'binary', 'root', 'workspace', 'evidence']) {
  assert.equal(typeof options[key], 'string', `--${key}=... is required`)
}
assert.ok(['claude', 'opencode'].includes(options.provider), '--provider must be claude or opencode')
const scratchParent = path.resolve(options.root)
const workspace = path.resolve(options.workspace)
const evidenceDir = path.resolve(options.evidence)
const binary = path.resolve(options.binary)
const overlaps = (left, right) => left === right || left.startsWith(right + path.sep) || right.startsWith(left + path.sep)
for (const candidate of [scratchParent, workspace, evidenceDir]) {
  assert.ok(!overlaps(candidate, repo), 'state, workspace and evidence must be separate from the repository')
}
assert.ok(!overlaps(scratchParent, workspace), 'state and workspace must use separate paths')
assert.ok(!overlaps(evidenceDir, scratchParent) && !overlaps(evidenceDir, workspace), 'evidence must be outside state and workspace')
await access(binary).catch(() => { throw Error('--binary must name an existing kernel executable') })
if (options.provider === 'claude') assert.equal(typeof options['token-file'], 'string', 'Claude requires --token-file=PATH')
if (options.provider === 'opencode') assert.equal(typeof options['opencode-home'], 'string', 'OpenCode requires --opencode-home=PATH')
const sourceHead = (await readFile(`${binary}.head`, 'utf8')).trim()
assert.match(sourceHead, /^[0-9a-f]{40}$/i, 'kernel binary .head must contain a full git commit hash')
const binarySha256 = createHash('sha256').update(await readFile(binary)).digest('hex')
const vaultPassphrase = randomBytes(24).toString('hex')
const nonce = `${Date.now()}-${process.pid}`
const runRoot = path.join(scratchParent, `run-${nonce}`)
const stateHome = path.join(runRoot, 'home')
assert.notEqual(path.resolve(stateHome), path.resolve(workspace), 'workspace must not overlap state')
const resultPath = path.join(evidenceDir, `restart-outcome-${options.scenario}-${nonce}.json`)
await access(workspace).then(() => { throw Error('--workspace must be new and empty') }, () => {})
await mkdir(evidenceDir, { recursive: true, mode: 0o700 })
await mkdir(workspace, { recursive: true, mode: 0o700 })
const record = {
  scope: 'real official-provider warm and post-graceful-kernel-restart turns',
  provider: options.provider,
  candidate: binary,
  source_head: sourceHead,
  binary_sha256: binarySha256,
  repo_head: (await import('node:child_process')).execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repo, encoding: 'utf8' }).trim(),
  started_at: new Date().toISOString(),
  turns: [],
  cleanup: 'pending',
}
const persist = async () => writeFile(resultPath, `${JSON.stringify(record, null, 2)}\n`, { mode: 0o600 })
await persist()

async function reservePorts() {
  const servers = [createServer(), createServer()]
  await Promise.all(servers.map(server => new Promise(resolve => server.listen(0, '127.0.0.1', resolve))))
  const ports = servers.map(server => server.address().port)
  await Promise.all(servers.map(server => new Promise(resolve => server.close(resolve))))
  return ports
}
const [kernelPort, mcpPort] = await reservePorts()
let kernel
let client
let kernelGeneration = 0
const ownedDescendants = new Map()
const inheritedEnvKeys = [
  'PATH', 'USER', 'LANG', 'TERM', 'TMPDIR', 'HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY',
  'http_proxy', 'https_proxy', 'all_proxy', 'no_proxy', 'IS_SANDBOX',
]
const baseEnv = {
  ...Object.fromEntries(inheritedEnvKeys.flatMap(key => process.env[key] === undefined ? [] : [[key, process.env[key]]])),
  HOME: stateHome,
  USER: 'root',
  LANG: 'C.UTF-8',
  CHARIOX_HOME: stateHome,
  CHARIOX_KERNEL_HOST: '127.0.0.1',
  CHARIOX_KERNEL_PORT: String(kernelPort),
  CHARIOX_MCP_PORT: String(mcpPort),
  CHARIOX_DAEMON_ID: `resumefix-cross-${nonce}`,
  CHARIOX_MACHINE_ID: `resumefix-cross-machine-${nonce}`,
  XDG_DATA_HOME: options.provider === 'opencode' ? path.join(options['opencode-home'], '.local/share') : path.join(stateHome, 'xdg-data'),
  XDG_CONFIG_HOME: options.provider === 'opencode' ? path.join(options['opencode-home'], '.config') : path.join(stateHome, 'xdg-config'),
  XDG_STATE_HOME: options.provider === 'opencode' ? path.join(options['opencode-home'], '.local/state') : path.join(stateHome, 'xdg-state'),
  XDG_CACHE_HOME: options.provider === 'opencode' ? path.join(options['opencode-home'], '.cache') : path.join(stateHome, 'xdg-cache'),
}
const endpoint = `ws://127.0.0.1:${kernelPort}/kernel`
const unwrap = (reply, name) => {
  assert.ok(reply?.[name], `expected ${name}`)
  return reply[name]
}
const safeClass = error => {
  const text = String(error).toLowerCase()
  if (/401|unauthori[sz]ed|expired|not authenticated/.test(text)) return 'provider_authentication_refused'
  if (/quota|credit|fund|rate.?limit/.test(text)) return 'provider_quota_refused'
  if (/model.*(unavailable|disabled|not found)|unknown model/.test(text)) return 'model_unavailable'
  if (/timed? ?out|timeout/.test(text)) return 'timeout'
  return 'kernel_or_provider_error'
}
async function waitReady() {
  const deadline = Date.now() + 30000
  while (Date.now() < deadline) {
    if (kernel.exitCode !== null || kernel.signalCode !== null) throw new Error('kernel exited before readiness')
    client = new LocalIpcClient(endpoint, {
      localAuthEnvironment: baseEnv,
      controlRequestRetryDeadlineMs: 500,
      controlResponseStallMs: 1000,
    })
    try {
      await client.send(req.listProviderAccountProfilesRequest())
      return
    } catch {
      await client.close().catch(() => {})
      client = undefined
      await delay(200)
    }
  }
  throw new Error('candidate kernel did not become ready')
}
async function hotReload() {
  // A planned upgrade: install the program the way an upgrade does (write aside, rename over), then SIGUSR2.
  // The kernel keeps its PID and execs the installed program.
  const pidBefore = kernel.pid
  const staged = `${binary}.next`
  await copyFile(binary, staged)
  await chmod(staged, 0o755)
  await rename(staged, binary)
  record.exe_before_reload = await readlink(`/proc/${pidBefore}/exe`)
  await client?.close().catch(() => {})
  client = undefined
  process.kill(pidBefore, 'SIGUSR2')
  await until('the kernel runs the installed program again', async () => {
    const exe = await readlink(`/proc/${pidBefore}/exe`).catch(() => '')
    return exe && !exe.endsWith('(deleted)')
  }, 60000)
  record.exe_after_reload = await readlink(`/proc/${pidBefore}/exe`)
  assert.equal(kernel.exitCode, null)
  assert.equal(kernel.pid, pidBefore)
  await delay(3000)
  await waitReady()
}
async function startKernel() {
  kernelGeneration += 1
  const kernelLog = openSync(path.join(evidenceDir, `kernel-${options.scenario}-${nonce}.log`), 'a')
  kernel = spawn(binary, [], { cwd: workspace, env: baseEnv, stdio: ['ignore', kernelLog, kernelLog] })
  assert.ok(Number.isInteger(kernel.pid) && kernel.pid > 1, 'candidate kernel must have an exact valid PID')
  await waitReady()
}
async function unlockVault() {
  const reply = await client.send(req.unlockCredentialVaultRequest(vaultPassphrase, { kind: 'until_kernel_stops' }, 'terminal'))
  assert.ok(reply.CredentialVaultUnlocked, `kernel did not confirm Vault initialization/unlock (${Object.keys(reply ?? {}).join(',')})`)
}
async function initializeVaultThroughOwner(sessionId, agentId) {
  const pending = client.send(req.manageCredentialVaultRequest(sessionId, agentId))
  pending.catch(() => {})
  const deadline = Date.now() + 15000
  while (Date.now() < deadline) {
    const state = unwrap(await client.send(req.getSessionStateRequest(sessionId)), 'SessionState')
    const prompt = state.session.active_interactions?.find(item => item.id.startsWith('vault-unlock-'))
    if (prompt) {
      await client.send(req.respondToInteractionRequest(sessionId, prompt.id, 'passphrase', vaultPassphrase))
      await pending
      return
    }
    await delay(100)
  }
  throw new Error('kernel did not show the Vault owner passphrase interaction')
}
async function observeOwnedDescendants() {
  if (!kernel?.pid || kernel.pid <= 1) return
  const rows = []
  for (const entry of await readdir('/proc')) {
    if (!/^\d+$/.test(entry)) continue
    try {
      const stat = await readFile(`/proc/${entry}/stat`, 'utf8')
      const close = stat.lastIndexOf(')')
      const fields = stat.slice(close + 2).trim().split(/\s+/)
      rows.push({ pid: Number(entry), ppid: Number(fields[1]), starttime: fields[19] })
    } catch { /* exited while enumerating */ }
  }
  const descendants = new Map([[kernel.pid, 0]])
  let changed = true
  while (changed) {
    changed = false
    for (const row of rows) {
      const parentDepth = descendants.get(row.ppid)
      if (parentDepth === undefined || descendants.has(row.pid)) continue
      descendants.set(row.pid, parentDepth + 1)
      changed = true
    }
  }
  for (const [pid, depth] of descendants) {
    if (pid === kernel.pid) continue
    const row = rows.find(item => item.pid === pid)
    if (!row) continue
    try {
      const cwd = await readlink(`/proc/${pid}/cwd`)
      ownedDescendants.set(pid, { pid, depth, starttime: row.starttime, cwd })
    } catch { /* exited while observing */ }
  }
}
async function assertOwnedKernelCwd() {
  assert.ok(kernel && Number.isInteger(kernel.pid) && kernel.pid > 1, 'refusing signal without exact owned PID')
  assert.equal(await readlink(`/proc/${kernel.pid}/cwd`), workspace, 'refusing signal: PID cwd is not the owned workspace')
}
async function stopKernel() {
  await client?.close().catch(() => {})
  client = undefined
  if (kernel && kernel.exitCode === null && kernel.signalCode === null) {
    await observeOwnedDescendants()
    await assertOwnedKernelCwd()
    const exited = new Promise(resolve => kernel.once('exit', (code, signal) => resolve({ code, signal })))
    process.kill(kernel.pid, 'SIGTERM')
    let timer
    const outcome = await Promise.race([exited, new Promise(resolve => { timer = setTimeout(() => resolve(null), 15000) })])
    clearTimeout(timer)
    if (!outcome) {
      await assertOwnedKernelCwd()
      process.kill(kernel.pid, 'SIGKILL')
      await exited
      throw new Error('owned candidate kernel required SIGKILL after graceful shutdown timeout')
    }
    assert.equal(outcome.signal, null, 'candidate kernel should stop gracefully')
  }
  await cleanOwnedDescendants()
}
async function cleanOwnedDescendants() {
  const active = () => [...ownedDescendants.values()].filter(item => {
    try { return existsSync(`/proc/${item.pid}`) } catch { return false }
  }).sort((left, right) => right.depth - left.depth)
  for (const item of active()) {
    const signalOwned = async signal => {
      assert.ok(item.pid > 1, 'refusing to signal an invalid provider child PID')
      let stat
      let cwd
      try {
        stat = await readFile(`/proc/${item.pid}/stat`, 'utf8')
        cwd = await readlink(`/proc/${item.pid}/cwd`)
      } catch { return false }
      const fields = stat.slice(stat.lastIndexOf(')') + 2).trim().split(/\s+/)
      if (fields[19] !== item.starttime || cwd !== item.cwd || cwd !== workspace) return false
      process.kill(item.pid, signal)
      return true
    }
    if (!await signalOwned('SIGTERM')) continue
    for (let attempt = 0; attempt < 20; attempt++) {
      try { await readFile(`/proc/${item.pid}/stat`, 'utf8') } catch { break }
      await delay(100)
    }
    try {
      await readFile(`/proc/${item.pid}/stat`, 'utf8')
      await signalOwned('SIGKILL')
    } catch { /* exited after TERM */ }
  }
  for (const item of active()) {
    let cwd
    try { cwd = await readlink(`/proc/${item.pid}/cwd`) } catch { continue }
    if (cwd === workspace) throw new Error('owned provider child remained after cleanup')
  }
}
function profilePayload(reply) {
  const wrapped = reply.ProviderAccountProfile ?? reply.ProviderAccountProfileRefreshed
  assert.ok(wrapped?.profile, 'provider profile response was missing')
  return wrapped.profile
}
async function enrollClaudeSetupToken(setPhase) {
  setPhase('create_managed_profile')
  const created = profilePayload(await client.send(req.createProviderAccountProfileRequest('claude', `resumefix Claude ${nonce}`)))
  setPhase('read_product_setup_token')
  let tokenBuffer = await readFile(options['token-file'])
  try {
    const token = tokenBuffer.toString('utf8').trim()
    assert.ok(token.startsWith('sk-ant-oat01-'), 'Claude setup token file did not contain the expected token kind')
    setPhase('store_setup_token_via_kernel_api')
    const stored = await client.send(req.setProviderAccountCredentialRequest('claude', created.profile_id, token))
    assert.ok(stored.ProviderAccountCredentialStored, 'kernel did not confirm product credential storage')
  } finally {
    tokenBuffer.fill(0)
    tokenBuffer = Buffer.alloc(0)
  }
  setPhase('verify_setup_token_status')
  const auth = unwrap(await client.send(req.getProviderAuthStatusRequest('claude', created.profile_id)), 'ProviderAuthStatus').status
  return { profile_id: created.profile_id, auth_state: auth.auth_state }
}

async function createSession(provider, model, profileId, label) {
  const created = unwrap(await client.send(req.createSessionRequest(
    workspace,
    workspace,
    label,
    { provider, model, effort: 'low', account_profile: profileId, permission_level: 'yolo' },
  )), 'SessionCreated')
  return { sessionId: created.session.id, agentId: created.agent.id }
}

const sleepSeconds = 100
const activeTurn = async (sessionId, agentId) => {
  const state = unwrap(await client.send(req.getSessionStateRequest(sessionId)), 'SessionState')
  return state.agent_activity?.[agentId]
}
async function submit(sessionId, agentId, text, tag) {
  const attached = unwrap(await client.send(req.attachToSessionRequest(sessionId, `outcome-${tag}`)), 'SessionAttached')
  const submitted = unwrap(await client.send(req.submitPromptRequest(sessionId, attached.attachment.id, agentId, text, [])), 'PromptSubmitted')
  const promptId = (submitted.outcome?.Started ?? submitted.outcome?.Queued)?.prompt?.id
  assert.ok(promptId, 'submitted prompt did not return a prompt ID')
  return promptId
}
async function until(what, fn, ms = 180000) {
  const deadline = Date.now() + ms
  while (Date.now() < deadline) {
    const value = await fn()
    if (value) return value
    await delay(500)
  }
  throw new Error(`timed out: ${what}`)
}
async function toolRunning(sessionId, agentId, promptId) {
  try {
    return await until('the provider runs the long command', async () => {
      const activity = await activeTurn(sessionId, agentId)
      record.debug = { turn: activity?.active_turn, last: activity?.last_completed_turn && summary(activity.last_completed_turn), prompt: promptId }
      const turn = activity?.active_turn
      if (turn?.prompt_id !== promptId) return undefined
      await observeOwnedDescendants()
      const sleeper = (await pidsWithCommand('tail -f /dev/null'))[0]
      return sleeper && { runId: turn.provider_run_id }
    }, 120000)
  } catch (error) {
    const dump = unwrap(await client.send(req.getSessionStateRequest(sessionId)), 'SessionState')
    record.debug_state = JSON.stringify({ activity: dump.agent_activity?.[agentId], keys: Object.keys(dump), sessionKeys: Object.keys(dump.session) }).slice(0, 2500)
    record.debug_history = JSON.stringify(await client.send(req.getSessionHistoryOutlineRequest(sessionId, [agentId], 3, null, true))).slice(0, 3500)
    record.debug_processes = (await pidsWithCommand('')).filter(item => /claude|sleep|bash/.test(item.cmdline)).slice(0, 12)
    throw error
  }
}
async function pidsWithCommand(needle) {
  const found = []
  for (const entry of await readdir('/proc')) {
    if (!/^\d+$/.test(entry)) continue
    try {
      const cmdline = (await readFile(`/proc/${entry}/cmdline`, 'utf8')).replaceAll('\0', ' ')
      const cwd = await readlink(`/proc/${entry}/cwd`)
      if (cmdline.includes(needle) && cwd === workspace) found.push({ pid: Number(entry), cmdline: cmdline.slice(0, 120) })
    } catch { /* gone */ }
  }
  return found
}
async function completedTurn(sessionId, agentId, promptId) {
  return until('the turn settles', async () => {
    const last = (await activeTurn(sessionId, agentId))?.last_completed_turn
    return last?.prompt_id === promptId ? last : undefined
  })
}
async function pidFile() {
  const file = path.join(stateHome, 'state/kernel-pid', `${kernelPort}.pid`)
  return (await readFile(file, 'utf8').catch(() => '')).trim()
}
const summary = last => ({ settlement: last.settlement_status, kind: last.outcome?.kind, retryable: last.outcome?.retryable, reason: last.outcome?.reason, termination: last.provider_termination?.category })

async function setup() {
  await mkdir(stateHome, { recursive: true })
  await writeFile(path.join(stateHome, 'config.toml'), isolatedKernelConfigToml(path.join(stateHome, 'storage'), [
    '[credential_vault]',
    `path = ${JSON.stringify(path.join(stateHome, 'vault', 'vault.json'))}`,
    'unlock_policy = "kernel_init"',
  ]), { mode: 0o600 })
  await startKernel()
  record.pid_file_after_start = { file: await pidFile(), kernel_pid: kernel.pid }
  await client.send(req.createCredentialVaultRequest(vaultPassphrase))
  await unlockVault()
  const linked = await enrollClaudeSetupToken(() => {})
  assert.equal(linked.auth_state, 'authenticated')
  return createSession('claude-p', ['o0', 'o4'].includes(options.scenario) ? 'claude-sonnet-5-5' : 'claude-haiku-5-5', linked.profile_id, `outcome ${options.scenario}`)
}
const longCommandPrompt = `Call the Bash tool exactly once with the command: timeout ${sleepSeconds} tail -f /dev/null\nDo not set run_in_background, do not change the command, do not shorten it, call no other tool. After the tool returns, reply with the single word finished.`

const scenarios = {
  // O1: the provider process dies mid-turn, the kernel keeps running.
  async o1(identity) {
    const promptId = await submit(identity.sessionId, identity.agentId, longCommandPrompt, 'o1')
    await toolRunning(identity.sessionId, identity.agentId, promptId)
    const kernelPidBefore = kernel.pid
    const providers = (await pidsWithCommand('--session-id')).filter(item => item.cmdline.includes('claude'))
    assert.equal(providers.length >= 1, true, 'the provider process was not found')
    const victim = providers.at(-1)
    record.killed_provider = victim
    process.kill(victim.pid, 'SIGKILL')
    const last = await completedTurn(identity.sessionId, identity.agentId, promptId)
    record.result = { ...summary(last), kernel_restarted: kernel.pid !== kernelPidBefore || kernel.exitCode !== null }
    assert.notEqual(last.outcome?.kind, 'interrupted_by_restart', 'a provider death with no restart must not say restart')
    assert.equal(kernel.exitCode, null)
  },
  // O0: a child's long command is cut by a real restart; its turn continues and completes. What its parent hears.
  async o0(identity) {
    const task = `Call the Bash tool exactly once with the command: timeout ${sleepSeconds} tail -f /dev/null\nDo not set run_in_background, do not change the command, call no other tool. After the tool returns, reply with the single word finished.`
    const promptId = await submit(identity.sessionId, identity.agentId,
      `Start exactly one sub-agent with the Chariox tool that starts sub-agents, giving it this task verbatim:\n${task}\nThen wait until the sub-agent has finished (wait on its finish report, do not poll in a loop, do not do its work yourself). When its finish report arrives, reply with exactly one line: CHILD_OUTCOME=<the outcome kind named in that report>`, 'o0')
    await until('the sub-agent runs its long command', async () => (await pidsWithCommand('tail -f /dev/null'))[0], 240000)
    const pidBefore = kernel.pid
    await stopKernel()
    await startKernel()
    await unlockVault()
    record.pid_file_after_restart = { file: await pidFile(), kernel_pid: kernel.pid, old_pid: pidBefore }
    let quietSince
    await until('the parent and its sub-agent are idle again', async () => {
      const state = unwrap(await client.send(req.getSessionStateRequest(identity.sessionId)), 'SessionState')
      const busy = Object.values(state.agent_activity ?? {}).some(item => item.status !== 'idle')
      const running = (await pidsWithCommand('tail -f /dev/null')).length > 0
      if (busy || running) { quietSince = undefined; return false }
      quietSince ??= Date.now()
      return Date.now() - quietSince > 20000
    }, 420000)
    const outline = JSON.stringify(await client.send(req.getSessionHistoryOutlineRequest(identity.sessionId, null, 6, null, true)))
    const found = [...outline.matchAll(/CHILD_OUTCOME=([a-z_]+)/g)].map(match => match[1])
    const state = unwrap(await client.send(req.getSessionStateRequest(identity.sessionId)), 'SessionState')
    record.agents = Object.fromEntries(Object.entries(state.agent_activity ?? {}).map(([id, item]) => [id, item.last_completed_turn && summary(item.last_completed_turn)]))
    record.tasks = (state.session.agent_tasks ?? []).map(item => ({ agent: item.agent_id, state: item.state, reason: String(item.reason ?? '').slice(0, 120) }))
    record.child_outcome_the_parent_reported = found
    record.result = { reported: found.at(-1) }
    assert.ok(found.length > 0, 'the parent never reported the child outcome')
    assert.notEqual(found.at(-1), 'interrupted_by_restart', 'a child that finished its work must not report a restart')
    assert.equal(found.at(-1), 'completed')
  },
  // O4: a child's turn ends while its background call runs (the kernel holds the task in memory); a real restart drops the hold. What its parent hears: not a cancellation nobody asked for.
  async o4(identity) {
    const task = `Call the Bash tool exactly once with the command: timeout ${sleepSeconds} tail -f /dev/null\nSet run_in_background to true, do not change the command, call no other tool. When the tool returns, reply with the single word started and end your turn.`
    const promptId = await submit(identity.sessionId, identity.agentId,
      `Start exactly one sub-agent with the Chariox tool that starts sub-agents, giving it this task verbatim:\n${task}\nThen wait until the sub-agent has finished (wait on its finish report, do not poll in a loop, do not do its work yourself). When its finish report arrives, reply with exactly one line: CHILD_OUTCOME=<the outcome kind named in that report>`, 'o4')
    await until('the sub-agent runs its long command', async () => (await pidsWithCommand('tail -f /dev/null'))[0], 240000)
    // the child's own turn has ended and its call still runs: the held state
    await delay(30000)
    assert.equal((await pidsWithCommand('tail -f /dev/null')).length > 0, true, 'the background call must still run when the kernel restarts')
    const pidBefore = kernel.pid
    await hotReload()
    record.pid_file_after_reload = { file: await pidFile(), kernel_pid: kernel.pid, old_pid: pidBefore }
    // The sweep ends a task that lost its executor only after the delivery timeout (120 s) has passed.
    let outline = ''
    let found = []
    await until('the parent reports the child outcome', async () => {
      outline = JSON.stringify(await client.send(req.getSessionHistoryOutlineRequest(identity.sessionId, null, 6, null, true)))
      found = [...outline.matchAll(/CHILD_OUTCOME=([a-z_]+)/g)].map(match => match[1])
      return found.length > 0
    }, 480000)
    const state = unwrap(await client.send(req.getSessionStateRequest(identity.sessionId)), 'SessionState')
    record.agents = Object.fromEntries(Object.entries(state.agent_activity ?? {}).map(([id, item]) => [id, item.last_completed_turn && summary(item.last_completed_turn)]))
    record.tasks = (state.session.agent_tasks ?? []).map(item => ({ agent: item.agent_id, state: item.state, reason: String(item.reason ?? '').slice(0, 120) }))
    record.child_outcome_the_parent_reported = found
    record.result = { reported: found.at(-1) }
    assert.ok(found.length > 0, 'the parent never reported the child outcome')
    assert.notEqual(found.at(-1), 'cancelled', 'nobody cancelled this child')
    // What the parent hears is whatever really happened to the child: here it finished (completed) or was
    // interrupted_by_restart; it is never a cancellation. The pid file must name the running kernel after the reload.
    assert.ok(['completed', 'interrupted_by_restart'].includes(found.at(-1)), `unexpected outcome ${found.at(-1)}`)
    assert.equal(record.pid_file_after_reload.file, String(kernel.pid), 'the pid file names the running kernel after a hot reload')
  },
  // O2: a real kernel restart while a person's question is open still ends as a restart.
  async o2(identity) {
    const promptId = await submit(identity.sessionId, identity.agentId,
      'Ask the person one yes/no question with the chariox request_popup tool (title "restart check") and wait for the answer. Do not answer it yourself.', 'o2')
    await until('the question is open', async () => {
      const state = unwrap(await client.send(req.getSessionStateRequest(identity.sessionId)), 'SessionState')
      return (state.session.active_interactions ?? []).length > 0
    })
    const pidBefore = kernel.pid
    await stopKernel()
    await startKernel()
    await unlockVault()
    record.pid_file_after_restart = { file: await pidFile(), kernel_pid: kernel.pid, old_pid: pidBefore }
    const last = await completedTurn(identity.sessionId, identity.agentId, promptId)
    record.result = summary(last)
    assert.equal(last.outcome?.kind, 'interrupted_by_restart')
    assert.equal(record.pid_file_after_restart.file, String(kernel.pid))
  },
}
const tail = `
`
try {
  const identity = await setup()
  await scenarios[options.scenario](identity)
  record.status = 'pass'
} catch (error) {
  record.status = 'fail'
  record.failure = String(error?.message ?? error).slice(0, 300)
} finally {
  let cleanupSucceeded = true
  await stopKernel().catch(() => { cleanupSucceeded = false; record.cleanup = 'kernel_cleanup_failed' })
  await client?.close().catch(() => {})
  client = undefined
  record.pid_file_after_stop = await pidFile()
  if (cleanupSucceeded) {
    await rm(runRoot, { recursive: true, force: true })
    await rm(workspace, { recursive: true, force: true })
    record.cleanup = 'kernel_stopped_and_owned_scratch_removed'
  }
  record.finished_at = new Date().toISOString()
  await persist()
}
console.log(JSON.stringify(record, null, 1))
if (record.status !== 'pass') process.exitCode = 1
