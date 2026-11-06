#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11 official provider warm + restart drill.
import assert from 'node:assert/strict'
import { createHash, randomBytes } from 'node:crypto'
import { existsSync } from 'node:fs'
import { access, mkdir, readFile, readlink, readdir, rm, writeFile } from 'node:fs/promises'
import { spawn } from 'node:child_process'
import { createServer } from 'node:net'
import { setTimeout as delay } from 'node:timers/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'
import * as req from '../../../packages/kernel-client/dist/ipc-requests.js'
import { isolatedKernelConfigToml } from './lib/drill-kernel-storage.mjs'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const options = Object.fromEntries(process.argv.slice(2).map(arg => {
  const [key, ...parts] = arg.replace(/^--/, '').split('=')
  return [key, parts.length ? parts.join('=') : true]
}))
if (options.help || options.h) {
  console.log('Usage: node apps/cli/scripts/live-provider-restart-drill.mjs --provider=claude|opencode --binary=KERNEL --root=STATE_PARENT --workspace=NEW_WORKSPACE --evidence=EVIDENCE_DIR [--token-file=PATH | --opencode-home=PATH]')
  process.exit(0)
}
for (const key of ['provider', 'binary', 'root', 'workspace', 'evidence']) {
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
const resultPath = path.join(evidenceDir, `provider-restart-${options.provider}-${nonce}.json`)
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
async function startKernel() {
  kernelGeneration += 1
  kernel = spawn(binary, [], { cwd: workspace, env: baseEnv, stdio: 'ignore' })
  assert.ok(Number.isInteger(kernel.pid) && kernel.pid > 1, 'candidate kernel must have an exact valid PID')
  await waitReady()
}
async function unlockVault() {
  const reply = await client.send(req.unlockCredentialVaultRequest(vaultPassphrase))
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
async function importOpenCodeNative() {
  const imported = profilePayload(await client.send(req.importNativeProviderAccountProfileRequest('opencode')))
  const refreshed = profilePayload(await client.send(req.refreshProviderAccountProfileRequest('opencode', imported.profile_id)))
  return { profile_id: refreshed.profile_id, auth_state: refreshed.auth_state }
}
async function runCatalog(provider, profileId) {
  const response = unwrap(await client.send(req.getProviderCatalogRequest({ provider, accountProfile: profileId })), 'ProviderCatalog')
  return response.catalog
}
async function selectOpenCodeModel(profileId) {
  const catalog = await runCatalog('opencode', profileId)
  const available = new Set((catalog.all ?? []).flatMap(provider =>
    Object.values(provider.models ?? {}).map(model => `${provider.id}/${model.id}`)))
  const allowed = ['opencode-go/deepseek-v4.1-flash', 'opencode/deepseek-v4.1-flash', 'opencode/glm-5.3-flash']
  const model = allowed.find(candidate => available.has(candidate))
  if (model) return { model, selected_by: model === allowed[0] ? 'preferred_available' : 'preferred_absent_fallback_available' }
  throw Object.assign(new Error('no allowed OpenCode model is present in the linked-account catalog'), { safeClass: 'model_unavailable' })
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
async function runTurn(providerRow, tag) {
  const attached = unwrap(await client.send(req.attachToSessionRequest(providerRow.sessionId, `cross-${tag}`)), 'SessionAttached')
  const prompt = `This is a short runtime resume check. Reply with exactly: ${tag}-complete. Do not use tools.`
  const submitted = unwrap(await client.send(req.submitPromptRequest(providerRow.sessionId, attached.attachment.id, providerRow.agentId, prompt, [])), 'PromptSubmitted')
  const outcome = submitted.outcome?.Started ?? submitted.outcome?.Queued
  const promptId = outcome?.prompt?.id
  assert.ok(promptId, 'submitted prompt did not return a prompt ID')
  const deadline = Date.now() + 180000
  let observedRunId
  let completion
  while (Date.now() < deadline) {
    await observeOwnedDescendants()
    const state = unwrap(await client.send(req.getSessionStateRequest(providerRow.sessionId)), 'SessionState')
    const activity = state.agent_activity?.[providerRow.agentId]
    if (activity?.active_turn?.prompt_id === promptId) observedRunId ??= activity.active_turn.provider_run_id
    const last = activity?.last_completed_turn
    if (last?.prompt_id === promptId) {
      completion = last
      observedRunId ??= last.provider_run_id
      break
    }
    await delay(500)
  }
  assert.ok(completion, 'provider turn did not settle within three minutes')
  if (completion.settlement_status !== 'completed') {
    const runId = completion.provider_run_id ?? observedRunId
    let runKeys = []
    let runStatus
    if (runId) {
      try {
        const runReply = unwrap(await client.send(req.getProviderRunRequest(runId)), 'ProviderRun')
        runKeys = Object.keys(runReply.provider_run ?? {})
        runStatus = runReply.provider_run?.status ?? runReply.provider_run?.state
      } catch { /* preserve only the settlement class */ }
    }
    throw Object.assign(new Error('provider turn settled unsuccessfully'), {
      safeClass: 'provider_turn_failed',
      settlement: completion.settlement_status,
      outcomeKind: completion.outcome?.kind,
      outcomeRetryable: completion.outcome?.retryable,
      turnOutcomeReason: completion.outcome?.reason,
      terminationCategory: completion.provider_termination?.category,
      runKeys,
      runStatus,
    })
  }
  const runId = completion.provider_run_id ?? observedRunId
  assert.ok(runId, 'settled turn had no provider run identity')
  const runReply = unwrap(await client.send(req.getProviderRunRequest(runId)), 'ProviderRun')
  const nativeId = runReply.provider_run.provider_session_id
  assert.ok(typeof nativeId === 'string' && nativeId.length > 0, 'provider did not report a native session identity')
  return {
    turn: tag,
    status: 'completed',
    kernel_generation: kernelGeneration,
    provider_run_id: runId,
    native_session_sha256: createHash('sha256').update(nativeId).digest('hex'),
    model: runReply.provider_run.model,
  }
}
async function checkProvider(provider, label, model) {
  const row = { provider, status: 'running', turns: [] }
  record.turns.push(row)
  await persist()
  try {
    row.phase = 'profile_setup'
    const linked = provider === 'claude-p'
      ? await enrollClaudeSetupToken(phase => { row.phase = phase })
      : await importOpenCodeNative()
    row.profile_origin = provider === 'claude-p' ? 'product_setup_token' : 'product_native_import'
    row.auth_state = linked.auth_state
    if (linked.auth_state !== 'authenticated') {
      throw Object.assign(new Error('provider account status is not authenticated'), { safeClass: 'provider_authentication_refused' })
    }
    if (provider === 'opencode') {
      row.phase = 'model_catalog'
      const selected = await selectOpenCodeModel(linked.profile_id)
      model = selected.model
      row.model_selection = selected.selected_by
    }
    row.model = model
    row.phase = 'session_create'
    const identity = await createSession(provider, model, linked.profile_id, `resume ${label}`)
    row.phase = 'warm_turn'
    row.turns.push(await runTurn({ ...identity, profileId: linked.profile_id }, `${label}-warm`))
    await persist()
    const beforeRestart = row.turns.at(-1).native_session_sha256
    row.phase = 'graceful_restart'
    await stopKernel()
    await startKernel()
    row.phase = 'vault_unlock_after_restart'
    await unlockVault()
    row.phase = 'session_resume'
    const restored = unwrap(await client.send(req.getSessionStateRequest(identity.sessionId)), 'SessionState')
    const agent = restored.session.agents.find(item => item.id === identity.agentId)
    assert.ok(agent, 'session agent identity did not survive graceful kernel restart')
    row.phase = 'postrestart_turn'
    row.turns.push(await runTurn({ ...identity, profileId: linked.profile_id }, `${label}-postrestart`))
    await persist()
    const afterRestart = row.turns.at(-1).native_session_sha256
    row.native_identity_unchanged = beforeRestart === afterRestart
    assert.equal(row.native_identity_unchanged, true, 'provider native session identity changed across kernel restart')
    row.status = 'pass'
    await persist()
  } catch (error) {
    row.status = 'fail'
    row.phase ??= 'unknown'
    row.failure_class = error.safeClass ?? safeClass(error)
    if (typeof error?.code === 'string' && /^[a-z0-9_-]{1,80}$/i.test(error.code)) row.error_code = error.code
    row.error_type = error?.constructor?.name ?? 'Error'
    if (typeof error?.settlement === 'string') row.settlement_status = error.settlement
    if (typeof error?.outcomeKind === 'string') row.turn_outcome_kind = error.outcomeKind
    if (typeof error?.outcomeRetryable === 'boolean') row.turn_outcome_retryable = error.outcomeRetryable
    if (typeof error?.terminationCategory === 'string') row.provider_termination_category = error.terminationCategory
    if (Array.isArray(error?.runKeys)) row.provider_run_fields = error.runKeys
    if (typeof error?.runStatus === 'string') row.provider_run_status = error.runStatus
    if (typeof error?.turnOutcomeReason === 'string') row.turn_outcome_reason = error.turnOutcomeReason
    await persist()
    throw error
  }
}

try {
  await mkdir(stateHome, { recursive: true })
  await writeFile(path.join(stateHome, 'config.toml'), isolatedKernelConfigToml(path.join(stateHome, 'storage'), [
    '[credential_vault]',
    `path = ${JSON.stringify(path.join(stateHome, 'vault', 'vault.json'))}`,
    'unlock_policy = "kernel_init"',
  ]), { mode: 0o600 })
  await startKernel()
  try {
    const vaultSession = await createSession('dev-stub', 'default', null, 'vault setup')
    await initializeVaultThroughOwner(vaultSession.sessionId, vaultSession.agentId)
    if (options.provider === 'claude') await checkProvider('claude-p', 'claude', 'claude-haiku-5-5')
    else await checkProvider('opencode', 'opencode', 'opencode-go/deepseek-v4.1-flash')
  } catch (error) { record.failure_class = error.safeClass ?? safeClass(error) }
  record.status = record.turns.length === 1 && record.turns[0].status === 'pass' && !record.failure_class ? 'pass' : 'fail'
} catch (error) {
  record.status = 'fail'
  record.failure_class ??= error.safeClass ?? safeClass(error)
  record.failure_code = typeof error?.code === 'string' ? error.code : undefined
  record.failure_type = error?.constructor?.name ?? 'Error'
} finally {
  let cleanupSucceeded = true
  await stopKernel().catch(() => {
    cleanupSucceeded = false
    record.status = 'fail'
    record.cleanup = 'kernel_cleanup_failed'
    record.cleanup_failure_class = 'owned_process_shutdown_failed'
  })
  await client?.close().catch(() => {})
  client = undefined
  if (cleanupSucceeded) {
    await rm(runRoot, { recursive: true, force: true })
    await rm(workspace, { recursive: true, force: true })
    record.cleanup = 'kernel_stopped_and_owned_scratch_removed'
  } else {
    record.cleanup_preserved_paths = [runRoot, workspace]
  }
  record.finished_at = new Date().toISOString()
  await persist()
}
console.log(JSON.stringify({ status: record.status, provider: record.provider, turns: record.turns.map(({ status, model, model_selection, native_identity_unchanged, failure_class, settlement_status, turn_outcome_kind, turn_outcome_retryable, turn_outcome_reason, turns }) => ({ status, model, model_selection, native_identity_unchanged, failure_class, settlement_status, turn_outcome_kind, turn_outcome_retryable, turn_outcome_reason, turns: turns.map(({ turn, status, kernel_generation, native_session_sha256 }) => ({ turn, status, kernel_generation, native_session_sha256 })) })), source_head: record.source_head, binary_sha256: record.binary_sha256, cleanup: record.cleanup, evidence: resultPath }))
if (record.status !== 'pass' || record.cleanup !== 'kernel_stopped_and_owned_scratch_removed') process.exitCode = 1
