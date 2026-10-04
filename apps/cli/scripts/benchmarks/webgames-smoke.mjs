#!/usr/bin/env node
// MP-08 / MP-10, WP-12: frozen smoke policy; owner-approved full manifest ranges.
import assert from 'node:assert/strict'
import { spawn, execFile } from 'node:child_process'
import { createHash, randomBytes, createPublicKey, verify } from 'node:crypto'
import { createWriteStream } from 'node:fs'
import { mkdir, mkdtemp, readFile, writeFile, appendFile, rm, chmod, statfs } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'
const clientRoot = process.env.WEBGAMES_CLIENT_ROOT
assert.ok(clientRoot && path.isAbsolute(clientRoot), 'MP-08 / MP-10 pinned client root required')
const clientModuleRoot = process.env.WEBGAMES_CLIENT_MODULE_ROOT ?? `${clientRoot}/packages/kernel-client/dist`
assert.ok(path.isAbsolute(clientModuleRoot), 'MP-08 / MP-10 absolute built client module root required')
const { LocalIpcClient } = await import(`${clientModuleRoot}/ipc.js`)
const r = await import(`${clientModuleRoot}/ipc-requests.js`)
const { assembleSessionHistoryFinalMessage } = await import(`${clientModuleRoot}/session-history-fragments.js`)
import { sanitizeDrillMetadata } from '../lib/drill-secrets.mjs'
import { roomProviderToolName, roomProviderToolOutput } from '../lib/room-provider-tool-record.mjs'
import { observeKernelRpcErrors, rpcErrorRecord } from './round2/rpc-errors.mjs'
import { signalOwnedProcess } from './round2/owned-processes.mjs'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../..')
const lane = process.env.WEBGAMES_LANE_ROOT
const evidence = process.env.WEBGAMES_EVIDENCE_ROOT
const release = process.env.WEBGAMES_RELEASE_ROOT
const image = process.env.WEBGAMES_SLICE_IMAGE
for (const p of [lane, evidence, release]) assert.ok(p && path.isAbsolute(p), 'MP-08 / MP-10 roots must be absolute')
assert.match(image ?? '', /^sha256:[a-f0-9]{64}$/)
assert.ok(lane.startsWith('/root/.chariox/dev/'))
assert.ok(evidence.startsWith('/root/.codex/evidence/'))
const taskIds = ['date', 'dateeasy', 'datehard', 'buttons', 'buttons-easy', 'buttons-hard',
  'click-cubed', 'click-cubed-easy', 'click-cubed-hard', 'patience']
const scope = process.env.WEBGAMES_SCOPE ?? 'smoke'
assert.ok(['smoke', 'full'].includes(scope))
const taskOffset = Number(process.env.WEBGAMES_TASK_OFFSET ?? 0)
const taskCount = Number(process.env.WEBGAMES_TASK_COUNT ?? 10)
assert.ok(Number.isInteger(taskOffset) && Number.isInteger(taskCount) && taskOffset >= 0 && taskCount > 0 && taskOffset + taskCount <= 150)
assert.ok(scope === 'full' || (taskOffset === 0 && taskCount === 10))
const mpItems = ['MP-08', 'MP-10', 'MP-11']
const observationKinds = new Set(['browser_status', 'browser_find', 'browser_text', 'browser_wait_for_text', 'browser_events', 'browser_downloads'])
const runId = `r2next-wg-${Date.now()}-${process.pid}`
const model = process.env.WEBGAMES_MODEL ?? 'gpt-6.1-sol'
const upstream = `${lane}/upstream`
const siteRoot = process.env.WEBGAMES_SITE_ROOT ?? `${upstream}/webgames/webgames/dist`
assert.ok(path.isAbsolute(siteRoot), 'MP-08 / MP-10 absolute WebGames build root required')
const expectedCommit = process.env.WEBGAMES_SOURCE_COMMIT
assert.match(expectedCommit ?? '', /^[a-f0-9]{40}$/)
const exec = promisify(execFile)
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const unwrap = (v, k) => { assert.ok(v?.[k], `MP-10 expected ${k}; received ${Object.keys(v ?? {})}`); return v[k] }
await mkdir(evidence, { recursive: true, mode: 0o700 })
const root = await mkdtemp(`${lane}/runtime-`)
await chmod(root, 0o711)
const workspace = await mkdtemp('/tmp/benchwg-workspace-')
await chmod(workspace, 0o777)
const ownedChildren = [], slices = [], rows = [], resources = []
const ownedAgents = new Set()
let client, session, slice, agent, attachment, monitor, interrupted = false, stage = 'provenance'
let commandSequence = 0
const report = { experiment: 'r2next-unsigned', unsigned: true, effort: 'high', mpItems, runId, scope, taskOffset, taskCount, benchmark: 'WebGames', model, startedAt: new Date().toISOString(),
  wallTimeoutMs: 180000, maxMutatingActions: 20, maxToolCalls: 80,
  upstreamRevision: '309866f64642b864414ca2c7ff956e59e01d3317',
  freshAgentPerTask: true, repeats: 1, taskIds, rows, resources }
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
async function screenshot(task) {
  const artifact = unwrap(await client.send(r.captureRoomEnvironmentScreenshotRequest(session.id, attachment.id)), 'RoomEnvironmentScreenshotCaptured').artifact
  assert.equal(artifact.media_type, 'image/png')
  assert.ok(artifact.size_bytes > 8 && artifact.size_bytes < 16 * 1024 ** 2)
  const chunks = []; let offset = 0
  while (offset < artifact.size_bytes) {
    const chunk = unwrap(await client.send(r.readRoomEnvironmentScreenshotChunkRequest(session.id, attachment.id, artifact.artifact_id, offset, 128 * 1024)), 'RoomEnvironmentScreenshotChunk').chunk
    assert.equal(chunk.offset, offset); assert.equal(chunk.artifact_id, artifact.artifact_id)
    const bytes = Buffer.from(chunk.data_base64, 'base64'); assert.ok(bytes.length > 0)
    chunks.push(bytes); offset += bytes.length; assert.ok(offset <= artifact.size_bytes)
    assert.equal(chunk.eof, offset === artifact.size_bytes)
  }
  const bytes = Buffer.concat(chunks)
  assert.equal(createHash('sha256').update(bytes).digest('hex'), artifact.sha256)
  const filename = `${runId}-${encodeURIComponent(task)}.png`
  await writeFile(`${evidence}/${filename}`, bytes, { mode: 0o600 })
  return { filename, sha256: artifact.sha256, sizeBytes: bytes.length }
}
async function score(task, answer) {
  return new Promise((resolve, reject) => {
    const child = spawn('python3', [`${repo}/apps/cli/scripts/benchmarks/webgames-grader.py`, `${upstream}/webgames/evals/browseruse_webgames/webgames_tasks.jsonl`, 'score'], { stdio: ['pipe', 'pipe', 'ignore'] })
    let output = ''; child.stdout.on('data', b => { output += b })
    child.on('error', reject); child.on('exit', code => { try { assert.equal(code, 0); resolve(JSON.parse(output)) } catch (error) { reject(error) } })
    child.stdin.end(JSON.stringify({ id: task, answer }))
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
    harnessSha256: await hash(fileURLToPath(import.meta.url)), graderSha256: await hash(`${repo}/apps/cli/scripts/benchmarks/webgames-grader.py`) }
  assert.equal(await command('git', ['-C', `${upstream}/webgames`, 'rev-parse', 'HEAD']), report.upstreamRevision)
  report.taskManifest = JSON.parse(await command('python3', [`${repo}/apps/cli/scripts/benchmarks/webgames-grader.py`, `${upstream}/webgames/evals/browseruse_webgames/webgames_tasks.jsonl`, 'manifest']))
  assert.equal(report.taskManifest.count, 150)
  assert.deepEqual(report.taskManifest.tasks.slice(0, 10).map(t => t.id), taskIds)
  report.source.clientRoot = clientRoot
  report.source.clientSource = await command('git', ['-C', clientRoot, 'rev-parse', 'HEAD'])
  await command('git', ['-C', clientRoot, 'diff', '--exit-code', expectedCommit, '--', 'packages/kernel-client', 'apps/kernel/slice-linux-docker'])
  report.source.clientAndProvisionerMatchRuntimeSource = expectedCommit
  report.source.provisionerSha256 = await hash(`${clientRoot}/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh`)
  report.source.siteBuildSha256 = await hash(`${siteRoot}/index.html`)
  report.source.taskManifestSha256 = await hash(`${upstream}/webgames/evals/browseruse_webgames/webgames_tasks.jsonl`)
  assert.equal(manifest.unsigned, true)
  report.source.signatureValid = false
  report.source.artifactKind = 'UNSIGNED dev experiment'
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
    CHARIOX_SLICE_DOCKER_PROVISIONER: `${clientRoot}/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh`,
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
  monitor = setInterval(() => guard().catch(() => { interrupted = true }), 5000)
  stage = 'linked provider account'
  const profile = unwrap(await client.send(r.linkProviderAccountProfileRequest('codex', 'benchwg-acct-b2',
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
  stage = 'loopback-only official site'
  await docker(['exec', '-u', 'root', cname(), 'mkdir', '-p', '/tmp/benchwg/site'])
  await docker(['cp', `${siteRoot}/.`, `${cname()}:/tmp/benchwg/site`])
  await docker(['cp', `${repo}/apps/cli/scripts/benchmarks/webgames-serve.py`, `${cname()}:/tmp/benchwg/serve.py`])
  await docker(['exec', '-d', '-u', 'slice', cname(), 'python3', '/tmp/benchwg/serve.py', '/tmp/benchwg/site'])
  await docker(['exec', '-u', 'slice', cname(), 'python3', '-c', 'import urllib.request; r=urllib.request.urlopen("http://127.0.0.1:8765/"); assert r.status==200; print("MP-08 / MP-10 loopback site ready")'])
  await docker(['exec', '-u', 'slice', cname(), '/opt/chariox-slice/slice-screen.sh', 'open-url', 'about:blank'])
  await client.send(r.startRoomEnvironmentRequest(session.id, { css_width: 1280, css_height: 800,
    device_scale_factor: 1, desktop_pixel_width: 1280, desktop_pixel_height: 800 }))
  const selectedTasks = report.taskManifest.tasks.slice(taskOffset, taskOffset + taskCount)
  report.taskIds = selectedTasks.map(task => task.id)
  for (const task of selectedTasks) {
    stage = `${scope} webgames.${task.id}`; await guard()
    const template = task.id, started = Date.now()
    stage = `fresh Codex agent ${template}`
    agent = unwrap(await client.send(r.spawnAgentRequest(session.id, 'codex', `benchwg-${template}`, model, workspace,
      'high', 'build', 'yolo', undefined, undefined, slice.id, profile.profile_id)), 'AgentSpawned').agent
    ownedAgents.add(agent.id)
    assert.equal(agent.remote_execution.worker_kernel_id, slice.worker_kernel_id)
    report.sessionId = session.id
    stage = `attach ${template}`
    const row = { mpItems, benchmark: 'WebGames', scope, task: template, repeat: 0,
      upstreamRevision: report.upstreamRevision, runId, model, agentId: agent.id, startedAt: new Date(started).toISOString(), harnessValid: false, evidenceComplete: false,
      leaderboard: report.leaderboard }
    rows.push(row)
    row.goal = task.description; row.path = task.path
    attachment = unwrap(await client.send(r.attachToSessionRequest(session.id, `${runId}-${template}`)), 'SessionAttached').attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)
    const prompt = `MP-08 / MP-10 WebGames smoke. Your task is: ${task.description}. You must go to http://127.0.0.1:8765/${task.path} and obtain the password for the game. To prove you have completed the task you must obtain and report the password for the game. If you do not have the password, you have not managed to complete the task.
The supplied URL is SLICE-LOCAL: the site server runs on loopback INSIDE this same browser slice, not on the host kernel. Keep 127.0.0.1 exactly; do NOT replace it with host.docker.internal. This is not a host-local URL.
Use ONLY Chariox first-party slice_open_url and slice_browser_* tools. Observe the rendered page; use returned opaque field IDs for actions. No shell, scripts, source inspection, HTTP, files, Computer tools, provider-native browser tools, or another browser. Do not inspect benchmark code, hidden answers, or grader data. One attempt only: do not reload or restart a failed game. Max 20 mutating Browser actions, 80 tool calls, 180 seconds. Stop if unsupported or failed and report honestly. Return the obtained password in your final reply, or state failure.`
    stage = `prompt submit ${template}`
    const submitted = unwrap(await client.send(r.submitPromptRequest(session.id, attachment.id, agent.id, prompt, [])), 'PromptSubmitted')
    row.promptId = (submitted.outcome.Started ?? submitted.outcome.Queued).prompt.id
    stage = `provider polling ${template}`
    let turn = null, timedOut = false, budgetExceeded = false
    const deadline = started + report.wallTimeoutMs
    while (Date.now() < deadline) {
      if (interrupted) throw new Error('MP-10 interrupted')
      await client.send({ PumpTerminalOutput: { session_id: session.id, attachment_id: attachment.id } })
      const liveActions = unwrap(await client.send(r.listRoomEnvironmentActionHistoryRequest(session.id, null, 1000)), 'RoomEnvironmentActionHistoryListed').page.actions.filter(a => a.actor_id === `agent:${agent.id}` && a.submitted_at_ms >= started)
      if (liveActions.filter(a => !observationKinds.has(a.kind)).length > report.maxMutatingActions) { budgetExceeded = true; break }
      const outline = unwrap(await client.send(r.getSessionHistoryOutlineRequest(session.id, [agent.id], 2)), 'SessionHistoryOutline')
      turn = outline.agents.find(a => a.agent_id === agent.id)?.turns.find(t => t.prompt_id === row.promptId)
      if (['completed', 'failed', 'cancelled'].includes(turn?.lifecycle)) break
      await sleep(1000)
    }
    if (!['completed', 'failed', 'cancelled'].includes(turn?.lifecycle)) {
      timedOut = !budgetExceeded; await client.send(r.cancelActivePromptRequest(session.id, attachment.id, agent.id))
      await wait(async () => !unwrap(await client.send(r.getSessionStateRequest(session.id)), 'SessionState').session.agents.find(a => a.id === agent.id).is_processing, 'provider cancellation')
    }
    const settled = unwrap(await client.send(r.getSessionHistoryOutlineRequest(session.id, [agent.id], 2)), 'SessionHistoryOutline')
    turn = settled.agents.find(a => a.agent_id === agent.id)?.turns.find(t => t.prompt_id === row.promptId)
    const actions = unwrap(await client.send(r.listRoomEnvironmentActionHistoryRequest(session.id, null, 1000)), 'RoomEnvironmentActionHistoryListed').page.actions
    row.actions = actions.filter(a => a.actor_id === `agent:${agent.id}` && a.submitted_at_ms >= started)
    row.elapsedMs = Date.now() - started; row.timedOut = timedOut; row.turnLifecycle = turn?.lifecycle ?? null
    row.mutatingActions = row.actions.filter(a => !observationKinds.has(a.kind)).length
    // Hydrate the official transcript to audit the tool track, retaining only
    // bounded, sanitized results from permitted first-party Browser tools.
    stage = `history export ${template}`
    row.toolTrace = []; row.toolAuditComplete = true; const outputEntries = []; const transcriptEntries = []
    for (const blob of turn?.blobs ?? []) {
      if (!['provider_tool', 'provider_error', 'provider_output'].includes(blob.kind)) continue
      const content = unwrap(await client.send(r.getSessionHistoryBlobContentRequest(session.id, agent.id, blob.blob_id)), 'SessionHistoryBlobContent')
      if (blob.kind === 'provider_output') { outputEntries.push(...content.entries); transcriptEntries.push(...content.entries); continue }
      if (blob.kind === 'provider_error') { row.providerError = true; continue }
      for (const item of content.entries) {
        // History blobs may batch prose and tools; the entry kind is authoritative.
        if (item.entry.kind === 'provider_output') { outputEntries.push(item); transcriptEntries.push(item); continue }
        if (item.entry.kind === 'provider_error') { row.providerError = true; continue }
        if (item.entry.kind !== 'provider_tool') continue
        try {
          const record = JSON.parse(item.entry.text), tool = roomProviderToolName(record.tool)
          const permitted = /^(?:slice_open_url|slice_browser_(?:status|find|text|wait_for_text|wait_for_idle|click|fill|submit|dialog|events|downloads|tab|history|upload))$/.test(tool)
          if (permitted) transcriptEntries.push({ ...item, entry: { ...item.entry,
            text: JSON.stringify(sanitizeDrillMetadata(record)) } })
          row.toolTrace.push({ id: record.id, tool, status: record.status, permitted,
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
    const inlineOutput = [...(turn?.entries ?? []), ...(turn?.summary ? [turn.summary] : [])].filter(x => x.entry?.kind === 'provider_output')
    let finalOutput = [...outputEntries, ...inlineOutput].sort((a, b) => (a.sequence ?? 0) - (b.sequence ?? 0)).at(-1)?.entry.text ?? ''
    if (turn?.lifecycle === 'completed') {
      try { finalOutput = assembleSessionHistoryFinalMessage(turn, [...outputEntries, ...inlineOutput]) }
      catch (error) { row.finalAssemblyError = rpcErrorRecord(error); finalOutput = '' }
    }
    await writeFile(`${evidence}/${runId}-${encodeURIComponent(template)}-final.txt`, finalOutput, { mode: 0o600 })
    await writeFile(`${evidence}/${runId}-${encodeURIComponent(template)}-transcript.json`, JSON.stringify(sanitizeDrillMetadata({ mpItems, entries: transcriptEntries }), null, 2), { mode: 0o600 })
    row.finalResponseSha256 = createHash('sha256').update(finalOutput).digest('hex')
    stage = `official scoring ${template}`
    const graded = await score(template, finalOutput)
    row.reward = graded.reward; row.scorer = graded.scorer; row.finalResponsePresent = finalOutput.length > 0
    const toolCalls = new Map(row.toolTrace.map((t, i) => [t.id ?? i, t])); row.toolCalls = toolCalls.size
    row.firstFailingSeam = row.actions.find(a => a.kind === 'navigate' && a.state === 'failed') ? 'benchmark_navigation' : null
    row.harnessValid = !row.firstFailingSeam && !row.actions.some(a => a.mode !== 'browser')
      && row.toolAuditComplete && row.toolTrace.length > 0 && row.toolTrace.every(t => t.permitted)
      && !row.providerError && !row.finalAssemblyError && (turn?.lifecycle === 'completed' || timedOut || budgetExceeded)
    row.budgetExceeded = budgetExceeded || row.mutatingActions > report.maxMutatingActions || toolCalls.size > report.maxToolCalls
    row.reward = row.budgetExceeded ? 0 : row.reward
    row.termination = timedOut ? 'wall_timeout' : row.budgetExceeded ? 'action_budget' : 'agent_finished'
    row.failureClass = row.providerError ? 'provider' : !row.harnessValid ? 'harness' : row.reward === 1 ? null
      : row.actions.some(a => a.state === 'failed') ? 'action' : 'task_unsuccessful_unanalysed_round1'
    row.screenshot = await screenshot(template)
    row.evidenceComplete = true
    await client.send(r.detachFromSessionRequest(attachment.id)); attachment = null
    await save('report', report)
    // MP-08/MP-10/MP-11: retire only this completed, evidence-captured fresh agent.
    stage = `agent retirement ${template}`
    row.agentRetired = await client.send(r.destroyAgentRequest(session.id, agent.id)).then(() => { ownedAgents.delete(agent.id); return true }, () => false)
    console.log(`MP-08 / MP-10 ${scope} ${template} reward=${row.reward} valid=${row.harnessValid} elapsedMs=${row.elapsedMs}`)
  }
  report.completed = rows.length === taskCount && rows.every(row => row.evidenceComplete)
  report.harnessValid = report.completed && rows.every(row => row.harnessValid)
  if (!report.harnessValid) process.exitCode = 1
} catch (error) {
  report.completed = false; report.firstFailingSeam = stage
  report.originalError = rpcErrorRecord(error); report.error = sanitizeDrillMetadata(error.message); process.exitCode = 1
  console.log(`MP-08 / MP-10 RED at ${stage}: ${error.message}`)
} finally {
  stage = 'cleanup'; clearInterval(monitor)
  const cleanup = { agents: [], slices: [], processes: [], stateRemoved: false, workspaceRemoved: false }
  if (client && attachment && agent) await client.send(r.cancelActivePromptRequest(session.id, attachment.id, agent.id)).catch(() => {})
  // MP-08/MP-10/MP-11: remove registered agents before product slice deletion.
  for (const agentId of ownedAgents) {
    const destroyed = await client?.send(r.destroyAgentRequest(session.id, agentId)).then(() => true, () => false) ?? false
    cleanup.agents.push({ id: agentId, destroyed })
  }
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
  report.cleanup = cleanup; report.finishedAt = new Date().toISOString(); await guard().catch(() => {})
  await save('report', report)
  // Append attempt rows without replacing another run. Track comparability remains separate.
  const resultsPath = `${evidence}/RESULTS.json`
  const previous = await readFile(resultsPath, 'utf8').then(JSON.parse, error => { if (error.code === 'ENOENT') return []; throw error })
  assert.ok(Array.isArray(previous)); await writeFile(resultsPath, JSON.stringify([...previous, ...rows], null, 2) + '\n')
}
