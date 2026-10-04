#!/usr/bin/env node
// MP-08 / MP-10, WP-12: frozen solver; optional approved full scheduling.
import assert from 'node:assert/strict'
import { spawn, execFile } from 'node:child_process'
import { createHash, randomBytes } from 'node:crypto'
import { createWriteStream } from 'node:fs'
import { mkdir, mkdtemp, readFile, writeFile, appendFile, rm, chmod, statfs } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createInterface } from 'node:readline'
import { promisify } from 'node:util'
const clientModuleRoot = process.env.MINIWOB_CLIENT_MODULE_ROOT
assert.ok(clientModuleRoot && path.isAbsolute(clientModuleRoot), 'MP-08 / MP-10 absolute built client module root required')
const { LocalIpcClient } = await import(`${clientModuleRoot}/ipc.js`)
const r = await import(`${clientModuleRoot}/ipc-requests.js`)
import { sanitizeDrillMetadata } from '../lib/drill-secrets.mjs'
import { roomProviderToolName, roomProviderToolOutput } from '../lib/room-provider-tool-record.mjs'
import { observeKernelRpcErrors, rpcErrorRecord } from './round2/rpc-errors.mjs'
import { signalOwnedProcess } from './round2/owned-processes.mjs'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../..')
const lane = process.env.MINIWOB_LANE_ROOT
const evidence = process.env.MINIWOB_EVIDENCE_ROOT
const release = process.env.MINIWOB_RELEASE_ROOT
const image = process.env.MINIWOB_SLICE_IMAGE
for (const p of [lane, evidence, release]) assert.ok(p && path.isAbsolute(p), 'MP-08 / MP-10 roots must be absolute')
assert.match(image ?? '', /^sha256:[a-f0-9]{64}$/)
assert.ok(lane.startsWith('/root/.chariox/dev/'))
assert.ok(evidence.startsWith('/root/.codex/evidence/'))
const full = process.env.MINIWOB_SCOPE === 'full'
const shard = Number(process.env.MINIWOB_SHARD ?? 0)
const shards = Number(process.env.MINIWOB_SHARDS ?? 1)
const skipIndices = new Set((process.env.MINIWOB_SKIP_INDICES ?? '').split(',').filter(Boolean).map(Number))
assert.ok(Number.isInteger(shard) && Number.isInteger(shards) && shard >= 0 && shard < shards)
const templates = ['click-button', 'click-link', 'click-dialog', 'click-checkboxes',
  'click-option', 'enter-text', 'enter-password', 'login-user', 'simple-arithmetic', 'choose-list']
const mpItems = ['MP-08', 'MP-10', 'MP-11']
const observationKinds = new Set(['browser_status', 'browser_find', 'browser_text', 'browser_wait_for_text', 'browser_events', 'browser_downloads'])
const runId = `r2next-mini-${Date.now()}-${process.pid}`
const model = process.env.MINIWOB_MODEL ?? 'gpt-6.1-sol'
const upstream = `${lane}/upstream`
const expectedCommit = process.env.MINIWOB_SOURCE_COMMIT
assert.match(expectedCommit ?? '', /^[a-f0-9]{40}$/)
const exec = promisify(execFile)
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const unwrap = (v, k) => { assert.ok(v?.[k], `MP-10 expected ${k}; received ${Object.keys(v ?? {})}`); return v[k] }
await mkdir(evidence, { recursive: true, mode: 0o700 })
const root = await mkdtemp(`${lane}/runtime-`)
await chmod(root, 0o711)
const workspace = await mkdtemp('/tmp/benchmini-workspace-')
await chmod(workspace, 0o777)
const ownedChildren = [], slices = [], rows = [], resources = []
let client, session, slice, agent, attachment, grader, monitor, interrupted = false, stage = 'provenance'
let commandSequence = 0
const report = { experiment: 'r2next-unsigned', unsigned: true, effort: 'high', mpItems, runId, scope: full ? 'full' : 'smoke', shard, shards, benchmark: 'MiniWoB++', model,
  seed: full ? 'official RandomState(42) schedule' : 42, repeats: full ? 5 : 1, episodeMaxTimeMs: 1000000, wallTimeoutMs: 180000,
  browsergym: 'c05d7f38f4f788d6e4ee960512c3d5c8aabc6e42',
  farama: '33c3b4ddef8c6eb67c57a29663d844b1eda7e614', rows, resources }
report.leaderboard = JSON.parse(await readFile(`${evidence}/LEADERBOARD_SNAPSHOT.json`, 'utf8'))
const save = (name, data) => writeFile(`${evidence}/${runId}-${name}.json`, JSON.stringify(sanitizeDrillMetadata({ mpItems, ...data }), null, 2) + '\n')
async function command(program, args, timeout = 120000) {
  const start = Date.now()
  const receipt = { mpItems, sequence: ++commandSequence, program, args, startedAt: new Date(start).toISOString() }
  try {
    const result = await exec(program, args, { timeout, maxBuffer: 16 * 1024 ** 2 })
    await appendFile(`${evidence}/commands.jsonl`, JSON.stringify({ ...receipt, exitCode: 0, elapsedMs: Date.now() - start }) + '\n')
    return result.stdout.trim()
  } catch (error) {
    await appendFile(`${evidence}/commands.jsonl`, JSON.stringify({ ...receipt, exitCode: error.code, elapsedMs: Date.now() - start }) + '\n')
    // Child stderr may contain credentials; keep only the failure class.
    throw new Error(`MP-10 ${program} ${args.slice(0, 2).join(' ')} failed (${error.code})`)
  }
}
const docker = args => command('docker', args)
const cname = () => `chariox-slice-${slice.name}`
async function guard() {
  const memory = await readFile('/proc/meminfo', 'utf8')
  const fs = await statfs('/')
  const sample = { at: new Date().toISOString(), stage,
    memAvailable: Number(memory.match(/MemAvailable:\s+(\d+)/)[1]) * 1024,
    diskAvailable: fs.bavail * fs.bsize }
  resources.push(sample)
  await appendFile(`${evidence}/resources.jsonl`, JSON.stringify({ mpItems, runId, ...sample }) + '\n')
  if (sample.memAvailable < 16 * 1024 ** 3 || sample.diskAvailable < 10 * 1024 ** 3) {
    interrupted = true
    throw new Error('MP-10 resource floor reached')
  }
}
async function wait(fn, label, ms = 60000) {
  const deadline = Date.now() + ms
  while (Date.now() < deadline) {
    if (interrupted) throw new Error('MP-10 interrupted/resource floor')
    const result = await fn()
    if (result) return result
    await sleep(500)
  }
  throw new Error(`MP-10 ${label} timeout`)
}
function port() {
  return new Promise(resolve => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => resolve(p)) }) })
}
function launch(binary, env, label) {
  const child = spawn(binary, [], { cwd: root, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] })
  const log = createWriteStream(`${root}/${label}.log`, { mode: 0o600 })
  child.stdout.pipe(log); child.stderr.pipe(log); ownedChildren.push(child)
  return child
}
function graderRequests(child) {
  let pending = null
  createInterface({ input: child.stdout }).on('line', line => {
    if (!pending) return
    const { resolve, reject, timer } = pending; pending = null; clearTimeout(timer)
    try { const result = JSON.parse(line); assert.ok(result.ok, `MP-10 grader ${result.operation} ${result.error_class}`); resolve(result) } catch (error) { reject(error) }
  })
  child.on('exit', () => { if (pending) { clearTimeout(pending.timer); pending.reject(new Error('MP-10 grader exited')); pending = null } })
  return request => new Promise((resolve, reject) => {
    assert.equal(pending, null)
    const timer = setTimeout(() => { pending = null; reject(new Error('MP-10 grader response timeout')) }, 15000)
    pending = { resolve, reject, timer }; child.stdin.write(JSON.stringify(request) + '\n')
  })
}
process.on('SIGTERM', () => { interrupted = true })
process.on('SIGINT', () => { interrupted = true })
try {
  await guard()
  const manifest = JSON.parse(await readFile(`${release}/usr/lib/chariox/r2next-dev-manifest.json`, 'utf8'))
  assert.equal(manifest.sourceCommit, expectedCommit)
  const kernel = `${release}/usr/local/bin/chariox-kernel`
  const relay = `${release}/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay`
  const hash = async p => createHash('sha256').update(await readFile(p)).digest('hex')
  assert.equal(`sha256:${await hash(kernel)}`, manifest.artifacts.find(a => a.name === 'chariox-kernel').sha256)
  const labels = JSON.parse(await docker(['image', 'inspect', image, '--format', '{{json .Config.Labels}}']))
  report.source = { commit: manifest.sourceCommit, tree: manifest.sourceTree, kernelSha256: await hash(kernel),
    relaySha256: await hash(relay), image, labels, protocol: await command(kernel, ['--print-local-daemon-protocol-version']),
    unsignedManifestDigest: `sha256:${await hash(`${release}/usr/lib/chariox/r2next-dev-manifest.json`)}`,
    harnessCommit: await command('git', ['-C', repo, 'rev-parse', 'HEAD']),
    harnessSha256: await hash(fileURLToPath(import.meta.url)), graderSha256: await hash(`${repo}/apps/cli/scripts/benchmarks/miniwob-grader.py`) }
  for (const [name, revision] of [['BrowserGym', report.browsergym], ['miniwob-plusplus', report.farama]])
    assert.equal(await command('git', ['-C', `${upstream}/${name}`, 'rev-parse', 'HEAD']), revision)
  await command('git', ['init', '-q', workspace])
  for (const d of ['home', 'runtime', 'codex', 'claude', 'opencode', 'config', 'data', 'cache', 'xdg-state']) await mkdir(`${root}/${d}`, { mode: 0o700 })
  await writeFile(`${root}/runtime/config.toml`, `version = 1\n[state]\npath = "${root}/state.db"\n[slices]\nroot = "${root}/slices"\n[slices.linux]\ndocker_image = "${image}"\nbuild_image = "never"\nallow_provider_sandbox_compatibility = true\nmemory_mb = 2048\ncpus = "1"\nscreen_width = 1280\nscreen_height = 800\n[credential_vault]\nbackend = "process_memory"\npath = "${root}/vault.db"\nservice = "${runId}"\nagent_management = "allow"\n`, { mode: 0o600 })
  const ports = []; for (let i = 0; i < 5; i++) ports.push(await port())
  // Normal local/self-hosted relay configuration; identities come from kernels.
  const env = { PATH: process.env.PATH, LANG: 'C.UTF-8', HOME: `${root}/home`,
    CODEX_HOME: `${root}/codex`, CLAUDE_CONFIG_DIR: `${root}/claude`, OPENCODE_CONFIG_DIR: `${root}/opencode`,
    XDG_CONFIG_HOME: `${root}/config`, XDG_DATA_HOME: `${root}/data`, XDG_STATE_HOME: `${root}/xdg-state`, XDG_CACHE_HOME: `${root}/cache`,
    CHARIOX_HOME: `${root}/runtime`, CHARIOX_LOG_DIR: `${root}/logs`, CHARIOX_DAEMON_SOCKET: `${root}/runtime/kernel.sock`,
    CHARIOX_KERNEL_HOST: '127.0.0.1', CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]),
    CHARIOX_CODEX_PORT: String(ports[2]), CHARIOX_OPENCODE_PORT: String(ports[3]),
    CHARIOX_RELAY_HOST: '0.0.0.0', CHARIOX_RELAY_PORT: String(ports[4]),
    CHARIOX_RELAY_URL: `ws://host.docker.internal:${ports[4]}`, CHARIOX_RELAY_TOKEN: randomBytes(32).toString('hex'),
    CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT: '1',
    CHARIOX_DAEMON_ALIAS: runId,
    CHARIOX_SLICE_LOCAL_DEV_OWNER_UID: '0',
    CHARIOX_SLICE_LOCAL_DEV_RUNTIME_REVISION: manifest.runtimeSourceRevision,
    CHARIOX_SLICE_DOCKER_PROVISIONER: `${repo}/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh`,
    CHARIOX_SLICE_DOCKER_PIDS_LIMIT: '1024' }
  // Host resolves its own relay directly; slice configuration rewrites host access.
  env.CHARIOX_RELAY_URL = `ws://127.0.0.1:${ports[4]}`
  const relayChild = launch(relay, env, 'relay'), kernelChild = launch(kernel, env, 'kernel')
  report.owned = { root, workspace, ports, kernelPid: kernelChild.pid, relayPid: relayChild.pid, slices: [] }
  await save('ownership', report.owned)
  stage = 'kernel readiness'
  client = await wait(async () => {
    const c = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { controlResponseStallMs: 240000 })
    try { await c.send(r.listSessionsRequest()); return c } catch { await c.close(); return false }
  }, stage)
  observeKernelRpcErrors(client, async failure => {
    (report.rpcErrors ??= []).push({ stage, ...failure })
    await save('rpc-errors', { failures: report.rpcErrors })
  })
  const health = unwrap(await client.send({ GetDaemonHealth: null }), 'DaemonHealth').projection
  assert.equal(health.process.process_id, kernelChild.pid)
  monitor = setInterval(() => guard().catch(() => { interrupted = true }), 10000)
  stage = 'linked provider account'
  const profile = unwrap(await client.send(r.linkProviderAccountProfileRequest('codex', 'benchmini-acct-b2',
    '/root/.chariox/dev/provider-runtimes/browser-computer/acct-b2/codex')), 'ProviderAccountProfile').profile
  const refreshed = unwrap(await client.send(r.refreshProviderAccountProfileRequest('codex', profile.profile_id)), 'ProviderAccountProfile').profile
  report.account = { profileId: profile.profile_id, authState: refreshed.auth_state }
  assert.equal(refreshed.auth_state, 'authenticated', 'MP-08 / MP-10 approved linked Codex profile must be authenticated')
  session = unwrap(await client.send(r.createSessionRequest(workspace, workspace, runId,
    { provider: 'codex', model, account_profile: profile.profile_id }, null, 'off')), 'SessionCreated').session
  stage = 'Room slice startup'
  for (let attempt = 0; attempt < 3; attempt++) {
    slice = unwrap(await client.send(r.createSliceRequest({ name: `${runId}-${attempt}`, displayMode: 'headed', workspaceMount: workspace, base: 'clean' })), 'SliceCreated').slice
    slices.push(slice); report.owned.slices.push({ id: slice.id, name: slice.name }); await save('ownership', report.owned)
    await client.send(r.bindRoomEnvironmentSliceRequest(session.id, slice.id))
    try { await client.send(r.startSliceRequest(slice.id)); break } catch (error) {
      if (!/host port.*already in use/.test(error.message) || attempt === 2) throw error
      report.portRetries = attempt + 1; await client.send(r.deleteSliceRequest(slice.id))
    }
  }
  slice = unwrap(await client.send(r.getSliceRequest(slice.id)), 'Slice').slice
  assert.equal(slice.status, 'running')
  const inspect = JSON.parse(await docker(['inspect', cname()]))[0]
  assert.equal(inspect.Image, image)
  assert.equal(inspect.Config.Labels['io.chariox.slice.id'], slice.id)
  assert.equal(inspect.HostConfig.Memory, 2048 * 1024 ** 2)
  assert.equal(inspect.HostConfig.NanoCpus, 1e9)
  report.slice = { id: slice.id, container: cname(), memory: inspect.HostConfig.Memory,
    nanoCpus: inspect.HostConfig.NanoCpus, volumes: inspect.Mounts.filter(m => m.Type === 'volume').map(m => m.Name) }
  assert.equal((await docker(['exec', cname(), 'sha256sum', '/opt/chariox-slice/bin/chariox-kernel'])).split(/\s+/)[0], report.source.kernelSha256)
  stage = 'official grader dependencies'
  await docker(['exec', '-u', 'root', cname(), 'mkdir', '-p', '/tmp/benchmini'])
  await docker(['cp', `${upstream}/BrowserGym/browsergym`, `${cname()}:/tmp/benchmini/browsergym`])
  await docker(['cp', `${upstream}/miniwob-plusplus/miniwob/html`, `${cname()}:/tmp/benchmini/html`])
  await docker(['cp', `${repo}/apps/cli/scripts/benchmarks/miniwob-grader.py`, `${cname()}:/tmp/benchmini/grader.py`])
  await docker(['exec', '-u', 'root', cname(), 'mkdir', '-p', '/tmp/benchmini/screenshots'])
  await docker(['exec', '-u', 'root', cname(), 'chown', 'slice:slice', '/tmp/benchmini/screenshots'])
  await docker(['exec', '-u', 'root', cname(), 'python3', '-m', 'venv', '/tmp/benchmini/venv'])
  await command('docker', ['exec', '-u', 'root', cname(), '/tmp/benchmini/venv/bin/pip', 'install', '--quiet',
    '/tmp/benchmini/browsergym/core', '/tmp/benchmini/browsergym/miniwob'], 240000)
  await docker(['exec', '-d', '-u', 'slice', cname(), 'python3', '-m', 'http.server', '8765', '--bind', '127.0.0.1', '--directory', '/tmp/benchmini/html'])
  await docker(['exec', '-u', 'slice', cname(), '/opt/chariox-slice/slice-screen.sh', 'open-url', 'about:blank'])
  await client.send(r.startRoomEnvironmentRequest(session.id, { css_width: 1280, css_height: 800,
    device_scale_factor: 1, desktop_pixel_width: 1280, desktop_pixel_height: 800 }))
  grader = spawn('docker', ['exec', '-i', '-u', 'slice', cname(), '/tmp/benchmini/venv/bin/python', '-u', '/tmp/benchmini/grader.py'], { stdio: ['pipe', 'pipe', 'pipe'] })
  grader.stderr.pipe(createWriteStream(`${root}/grader.log`, { mode: 0o600 }))
  const grade = graderRequests(grader)
  report.taskManifest = await grade({ op: 'manifest' }); assert.equal(report.taskManifest.count, 125)
  const episodes = full
    ? (await grade({ op: 'schedule' })).episodes.filter(e => Math.floor(e.index / 5) % shards === shard && !skipIndices.has(e.index))
    : templates.map((template, index) => ({ task_name: `miniwob.${template}`, task_seed: 42, repeat: 0, index }))
  report.schedule = episodes
  report.expectedEpisodes = episodes.length
  stage = 'official Codex agent startup'
  agent = unwrap(await client.send(r.spawnAgentRequest(session.id, 'codex', 'benchmini-codex', model, workspace,
    'high', 'build', 'yolo', undefined, undefined, slice.id, profile.profile_id)), 'AgentSpawned').agent
  assert.equal(agent.remote_execution.worker_kernel_id, slice.worker_kernel_id)
  report.sessionId = session.id; report.agentId = agent.id
  for (const episode of episodes) {
    const { task_name: task, task_seed: seed, repeat, index } = episode
    const template = task.slice('miniwob.'.length)
    const episodeKey = full ? `${index}-${template}-${repeat}` : template
    stage = `${report.scope} ${task}`; await guard()
    const started = Date.now()
    const row = { mpItems, benchmark: 'MiniWoB++', scope: report.scope, task, seed, repeat, episodeIndex: index,
      runId, model, startedAt: new Date(started).toISOString(), harnessValid: false, evidenceComplete: false,
      leaderboard: { capturedAt: report.leaderboard.capturedAt, boardUrl: report.leaderboard.boardUrl,
        boardRevision: report.leaderboard.boardRevision, numberOne: report.leaderboard.numberOne,
        wouldBeRank: null, rankReason: report.leaderboard.rankReason } }
    rows.push(row)
    if (interrupted) throw new Error('MP-10 interrupted before episode setup')
    try {
    const setup = await grade({ op: 'setup', task, seed, episode_max_time_ms: report.episodeMaxTimeMs })
    row.goal = setup.goal; row.initialInfo = setup.info
    attachment = unwrap(await client.send(r.attachToSessionRequest(session.id, `${runId}-${template}`)), 'SessionAttached').attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)
    const prompt = `MP-08 / MP-10 benchmark smoke. Solve this task in the Room browser already open: ${setup.goal}\nUse ONLY Chariox first-party slice_browser_* tools. Observe the page and use returned opaque field IDs for actions. Do not use shell, scripts, file edits, Computer tools, provider-native browser tools, or another browser. Do not inspect benchmark code or reward globals. Stop when the task is complete or unsupported. Maximum 10 mutating Browser actions; wall timeout 180 seconds. Report a short result.`
    if (interrupted) throw new Error('MP-10 interrupted before prompt submission')
    const submitted = unwrap(await client.send(r.submitPromptRequest(session.id, attachment.id, agent.id, prompt, [])), 'PromptSubmitted')
    row.promptId = (submitted.outcome.Started ?? submitted.outcome.Queued).prompt.id
    let turn = null, timedOut = false, reward = null
    const deadline = started + report.wallTimeoutMs
    while (Date.now() < deadline) {
      if (interrupted) throw new Error('MP-10 interrupted')
      await client.send({ PumpTerminalOutput: { session_id: session.id, attachment_id: attachment.id } })
      reward = await grade({ op: 'validate' })
      const outline = unwrap(await client.send(r.getSessionHistoryOutlineRequest(session.id, [agent.id], 2)), 'SessionHistoryOutline')
      turn = outline.agents.find(a => a.agent_id === agent.id)?.turns.find(t => t.prompt_id === row.promptId)
      if (['completed', 'failed', 'cancelled'].includes(turn?.lifecycle)) break
      await sleep(1000)
    }
    if (!['completed', 'failed', 'cancelled'].includes(turn?.lifecycle)) {
      timedOut = true; await client.send(r.cancelActivePromptRequest(session.id, attachment.id, agent.id))
      await wait(async () => !unwrap(await client.send(r.getSessionStateRequest(session.id)), 'SessionState').session.agents.find(a => a.id === agent.id).is_processing, 'provider cancellation')
    }
    reward = await grade({ op: 'validate' })
    const actions = unwrap(await client.send(r.listRoomEnvironmentActionHistoryRequest(session.id, null, 100)), 'RoomEnvironmentActionHistoryListed').page.actions
    row.actions = actions.filter(a => a.actor_id === `agent:${agent.id}` && a.submitted_at_ms >= started)
    row.reward = reward.reward; row.done = reward.done; row.officialInfo = reward.info
    row.elapsedMs = Date.now() - started; row.timedOut = timedOut; row.turnLifecycle = turn?.lifecycle ?? null
    row.mutatingActions = row.actions.filter(a => !observationKinds.has(a.kind)).length
    // Hydrate the official transcript to audit the tool track, retaining only
    // bounded, sanitized results from permitted first-party Browser tools.
    row.toolTrace = []; row.toolAuditComplete = true
    for (const blob of turn?.blobs ?? []) {
      if (!['provider_tool', 'provider_error'].includes(blob.kind)) continue
      const content = unwrap(await client.send(r.getSessionHistoryBlobContentRequest(session.id, agent.id, blob.blob_id)), 'SessionHistoryBlobContent')
      if (blob.kind === 'provider_error') {
        row.providerError = true
        row.providerUnauthorized = content.entries.some(item => /(?:401|unauthorized)/i.test(item.entry.text))
        continue
      }
      for (const item of content.entries) {
        if (item.entry.kind === 'provider_error') { row.providerError = true; continue }
        if (item.entry.kind !== 'provider_tool') continue
        try {
          const record = JSON.parse(item.entry.text), tool = roomProviderToolName(record.tool)
          const permitted = /^slice_browser_(?:status|find|text|wait_for_text|wait_for_idle|click|fill|submit|dialog|events|downloads|tab|history|upload)$/.test(tool)
          row.toolTrace.push({ tool, status: record.status, permitted,
            ...(permitted ? { input: sanitizeDrillMetadata(record.input),
              output: sanitizeDrillMetadata(roomProviderToolOutput(record.output)),
              error: sanitizeDrillMetadata(record.error ?? null) } : {}) })
        } catch { row.toolAuditComplete = false }
      }
    }
    const state = unwrap(await client.send(r.getSessionStateRequest(session.id)), 'SessionState')
    const currentAgent = state.session.agents.find(a => a.id === agent.id)
    const providerRunId = state.agent_activity?.[agent.id]?.last_completed_turn?.provider_run_id
      ?? currentAgent.remote_execution?.active_worker_provider_run_id
    row.usageTokensTotal = null
    if (providerRunId) {
      try { row.usageTokensTotal = unwrap(await client.send(r.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run.usage_tokens_total } catch { /* Explicitly unavailable; no billing guess. */ }
    }
    row.harnessValid = !row.actions.some(a => a.mode !== 'browser') && row.mutatingActions <= 10 && !timedOut
      && row.toolAuditComplete && row.toolTrace.length > 0 && row.toolTrace.every(t => t.permitted)
      && !row.providerError && turn?.lifecycle === 'completed'
    row.truncated = !row.done
    row.termination = timedOut ? 'wall_timeout' : row.done ? 'official_task_done' : 'agent_finished_before_task_done'
    row.failureClass = timedOut ? 'provider' : !row.harnessValid ? 'action' : reward.reward === 1 ? null
      : row.actions.some(a => a.state === 'failed') ? 'action' : row.mutatingActions === 0 ? 'element grounding' : 'planning'
    await grade({ op: 'screenshot', path: `/tmp/benchmini/screenshots/${episodeKey}.png` })
    await docker(['cp', `${cname()}:/tmp/benchmini/screenshots/${episodeKey}.png`, `${evidence}/${runId}-${episodeKey}.png`])
    row.evidenceComplete = true
    if (full) await appendFile(`${evidence}/${runId}-rows.jsonl`, JSON.stringify(sanitizeDrillMetadata(row)) + '\n')
    await client.send(r.detachFromSessionRequest(attachment.id)); attachment = null
    await save('report', report)
    } catch (error) {
      if (!full) throw error
      row.errorClass = error.name
      row.originalError = rpcErrorRecord(error)
      row.failureClass = 'benchmark infrastructure'
      row.firstFailingSeam = stage
      row.executionInterrupted = interrupted
      row.retryAllowed = !row.promptId
      row.harnessValid = false
      // Salvage grading and actor actions before deleting any state. Never retry
      // a submitted prompt unless no solver action can be independently proved.
      const finalReward = await grade({ op: 'validate' }).catch(() => null)
      if (finalReward) { row.reward = finalReward.reward; row.done = finalReward.done; row.officialInfo = finalReward.info }
      const history = await client.send(r.listRoomEnvironmentActionHistoryRequest(session.id, null, 100)).catch(() => null)
      if (history?.RoomEnvironmentActionHistoryListed) {
        row.actions = history.RoomEnvironmentActionHistoryListed.page.actions.filter(a => a.actor_id === `agent:${agent.id}` && a.submitted_at_ms >= started)
      }
      if (attachment && row.promptId) {
        await client.send(r.cancelActivePromptRequest(session.id, attachment.id, agent.id)).catch(() => {})
        await wait(async () => !unwrap(await client.send(r.getSessionStateRequest(session.id)), 'SessionState').session.agents.find(a => a.id === agent.id).is_processing, 'failed episode cancellation').catch(() => {})
      }
      if (attachment) { await client.send(r.detachFromSessionRequest(attachment.id)).catch(() => {}); attachment = null }
      await save('report', report)
      console.log(`MP-08 / MP-10 RED episode ${task} index=${index} errorClass=${row.errorClass} submitted=${Boolean(row.promptId)}`)
      if (interrupted) throw error
    }
    console.log(`MP-08 / MP-10 ${report.scope} ${task} seed=${seed} repeat=${repeat} reward=${row.reward} valid=${row.harnessValid} elapsedMs=${row.elapsedMs}`)
  }
  report.completed = rows.length === episodes.length && rows.every(row => row.evidenceComplete)
} catch (error) {
  report.completed = false; report.firstFailingSeam = stage
  report.originalError = rpcErrorRecord(error); report.error = sanitizeDrillMetadata(error.message); process.exitCode = 1
  console.log(`MP-08 / MP-10 RED at ${stage}: ${error.message}`)
} finally {
  stage = 'cleanup'; clearInterval(monitor)
  grader?.stdin.end(); if (grader) await signalOwnedProcess(grader, 'SIGTERM')
  const cleanup = { slices: [], processes: [], stateRemoved: false, workspaceRemoved: false }
  if (client && attachment && agent) await client.send(r.cancelActivePromptRequest(session.id, attachment.id, agent.id)).catch(() => {})
  for (const s of slices.toReversed()) {
    const name = `chariox-slice-${s.name}`
    assert.ok(s.name.startsWith(runId))
    await client?.send(r.deleteSliceRequest(s.id)).catch(() => {})
    // Only exact named, ID-labelled owned containers/volumes can be removed.
    const existing = await docker(['inspect', name]).then(text => JSON.parse(text)[0], () => null)
    if (existing) {
      assert.equal(existing.Config.Labels['io.chariox.slice.id'], s.id)
      await docker(['rm', '-f', name])
      for (const m of existing.Mounts.filter(m => m.Type === 'volume')) {
        assert.ok(m.Name.startsWith(`${name}-`)); await docker(['volume', 'rm', m.Name])
      }
    }
    cleanup.slices.push({ id: s.id, containerGone: await docker(['inspect', name]).then(() => false, () => true) })
  }
  await client?.close()
  for (const child of ownedChildren.toReversed()) {
    await signalOwnedProcess(child, 'SIGTERM', { detached: true })
    for (let n = 0; n < 20 && child.exitCode === null; n++) await sleep(250)
    await signalOwnedProcess(child, 'SIGKILL', { detached: true })
    cleanup.processes.push({ pid: child.pid, stopped: child.exitCode !== null || child.signalCode !== null })
  }
  await rm(root, { recursive: true }); cleanup.stateRemoved = true
  await rm(workspace, { recursive: true }); cleanup.workspaceRemoved = true
  report.cleanup = cleanup; await guard().catch(() => {})
  await save('report', report)
  // Append smoke rows without replacing another run. Full campaign ranking is invalid.
  const resultsPath = full ? `${evidence}/${runId}-RESULTS.json` : `${evidence}/RESULTS.json`
  const previous = await readFile(resultsPath, 'utf8').then(JSON.parse, error => { if (error.code === 'ENOENT') return []; throw error })
  assert.ok(Array.isArray(previous)); await writeFile(resultsPath, JSON.stringify([...previous, ...rows], null, 2) + '\n')
}
