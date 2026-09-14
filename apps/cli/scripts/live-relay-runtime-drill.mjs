import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHmac } from 'node:crypto'
import { mkdir, rm, symlink } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { finalizeDrillArtifacts } from './lib/drill-artifacts.mjs'
import {
  childTerminationStatus,
  makeAvailablePorts,
  resolveBuiltBinary,
} from './lib/drill-runtime-helpers.mjs'
import { sanitizeDrillMetadata } from './lib/drill-secrets.mjs'
import {
  collectPumpedTerminalOutput,
  terminalText,
} from './lib/relay-runtime-terminal-output.mjs'
import {
  assertAttachmentRecoveryState,
  assertNoSubmitPromptRequest,
  assertStaleAttachmentRejected,
  createAttachmentRecoveryLedger,
} from '../../../scripts/candidate-kernel-attachment-recovery-helpers.mjs'
import { assertCandidateProtocolOutput } from '../../../scripts/candidate-kernel-startup-smoke-drill-helpers.mjs'
import {
  assertExactCandidateBinary,
  claimExactChildProcess,
  durableAgentFingerprint,
  durableSessionFingerprint,
  stopOwnedChild,
} from '../../../scripts/staged-kernel-restart-persistence-drill-helpers.mjs'
import { runVersionProbe } from '../../../scripts/staged-kernel-restart-persistence-drill.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

export async function loadCliModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
  for (const rel of ['src/ipc.ts', 'src/ipc-requests.ts']) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, '.js'))
    const code = await (await import('node:fs/promises')).readFile(sourcePath, 'utf8')
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await (await import('node:fs/promises')).writeFile(outPath, transformed?.code ?? '', 'utf8')
  }
  await symlink(path.join(cliRoot, 'node_modules'), path.join(runtimeDir, 'node_modules'), 'dir')
  const ipcUrl = pathToFileURL(path.join(runtimeDir, 'ipc.js')).href
  const requestsUrl = pathToFileURL(path.join(runtimeDir, 'ipc-requests.js')).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

let LocalIpcClient
let createSessionRequest
let attachToSessionRequest
let detachFromSessionRequest
let getSessionStateRequest
let getDaemonHealthRequest
let listSessionsRequest
let launchProviderRunRequest
let submitPromptRequest
let pumpTerminalOutputRequest
let endSessionRequest

const DEFAULT_MODEL = 'kimi2.5'
const DEFAULT_PROVIDER = 'opencode'
const DEFAULT_WORKSPACE = repoRoot
const DEFAULT_WORKTREE = repoRoot
const DEFAULT_TIMEOUT_MS = 180_000
const DEFAULT_ATTACHMENT_RECOVERY_TIMEOUT_MS = 60_000
const MAX_ATTACHMENT_RECOVERY_TIMEOUT_MS = 120_000
const DEFAULT_POLL_MS = 1_000
const RELAY_ISSUER = 'chariox-relay-runtime-drill'
const RELAY_SECRET = 'chariox-relay-runtime-drill-secret'
const RELAY_REALM = 'relay-runtime-drill'

function base64url(input) {
  return Buffer.from(input).toString('base64url')
}

function signRelayToken(claims) {
  const header = base64url(JSON.stringify({ alg: 'HS256', typ: 'JWT' }))
  const payload = base64url(JSON.stringify({
    iss: claims.issuer,
    sub: claims.subject,
    subject_kind: claims.subject_kind,
    realm_id: claims.realm_id,
    allowed_actions: claims.allowed_actions.map(relayActionName),
    allowed_targets: claims.allowed_targets,
    iat: Math.floor(claims.issued_at_ms / 1_000),
    exp: Math.ceil(claims.expires_at_ms / 1_000),
    jti: claims.token_id,
    account_id: claims.account_id,
    organization_id: claims.organization_id,
    user_id: claims.user_id,
    device_id: claims.device_id,
    machine_id: claims.machine_id,
    client_id: claims.client_id,
    public_key_thumbprint: claims.public_key_thumbprint,
    entitlements_version: claims.entitlements_version,
  }))
  const signingInput = `${header}.${payload}`
  const signature = createHmac('sha256', RELAY_SECRET).update(signingInput).digest('base64url')
  return `${signingInput}.${signature}`
}

function relayActionName(action) {
  const names = {
    daemon_register: 'daemon.register',
    daemon_heartbeat: 'daemon.heartbeat',
    client_metadata_read: 'client.metadata.read',
    client_connect: 'client.connect',
    packet_route: 'packet.route',
    peer_request: 'peer.request',
    peer_event: 'peer.event',
  }
  const name = names[action]
  if (!name) throw new Error(`unsupported relay action ${action}`)
  return name
}

function relayClaims({ subject, subjectKind, actions, userId = null, targets = null, machineId = null }) {
  return {
    issuer: RELAY_ISSUER,
    subject,
    subject_kind: subjectKind,
    realm_id: RELAY_REALM,
    allowed_actions: actions,
    allowed_targets: targets,
    issued_at_ms: Date.now(),
    expires_at_ms: Date.now() + 10 * 60_000,
    token_id: `${subject}-${Date.now()}`,
    account_id: 'relay-runtime-drill-account',
    organization_id: null,
    user_id: userId,
    device_id: subject,
    machine_id: machineId ?? (subjectKind === 'kernel' || subjectKind === 'machine' ? subject : null),
    client_id: subjectKind === 'client' ? subject : null,
    // Local drill tokens are intentionally unbound. Production-issued tokens
    // bind this claim to the kernel's real relay key thumbprint; a fabricated
    // value makes the relay correctly reject the daemon registration.
    public_key_thumbprint: null,
    entitlements_version: 'drill',
  }
}

function readArgumentValue(argv, index, option) {
  const value = argv[index + 1]
  if (!value || value.startsWith('--')) throw new Error(`${option} requires a value`)
  return value
}

function parsePositiveInteger(value, option) {
  if (!/^\d+$/.test(String(value))) throw new Error(`${option} must be a positive integer`)
  const parsed = Number(value)
  if (!Number.isSafeInteger(parsed) || parsed < 1) throw new Error(`${option} must be a positive integer`)
  return parsed
}

export function parseArgs(argv) {
  const options = {
    model: DEFAULT_MODEL,
    provider: DEFAULT_PROVIDER,
    workspace: DEFAULT_WORKSPACE,
    worktree: DEFAULT_WORKTREE,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    dryRun: false,
    attachmentRecovery: false,
    relayBinary: null,
    kernelBinary: null,
    expectedProtocol: null,
  }
  let timeoutSpecified = false
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider') options.provider = argv[++i]
    else if (arg === '--workspace') options.workspace = argv[++i]
    else if (arg === '--worktree') options.worktree = argv[++i]
    else if (arg === '--timeout-ms') {
      options.timeoutMs = Number(readArgumentValue(argv, i++))
      timeoutSpecified = true
    }
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--dry-run') options.dryRun = true
    else if (arg === '--attachment-recovery') options.attachmentRecovery = true
    else if (arg === '--relay-binary') options.relayBinary = readArgumentValue(argv, i++, '--relay-binary')
    else if (arg.startsWith('--relay-binary=')) options.relayBinary = arg.slice('--relay-binary='.length)
    else if (arg === '--kernel-binary') options.kernelBinary = readArgumentValue(argv, i++, '--kernel-binary')
    else if (arg.startsWith('--kernel-binary=')) options.kernelBinary = arg.slice('--kernel-binary='.length)
    else if (arg === '--expected-protocol') options.expectedProtocol = parsePositiveInteger(readArgumentValue(argv, i++, '--expected-protocol'), '--expected-protocol')
    else if (arg.startsWith('--expected-protocol=')) options.expectedProtocol = parsePositiveInteger(arg.slice('--expected-protocol='.length), '--expected-protocol')
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (options.attachmentRecovery && !timeoutSpecified) {
    options.timeoutMs = DEFAULT_ATTACHMENT_RECOVERY_TIMEOUT_MS
  }
  return options
}

export function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-relay-runtime-drill.mjs [options]',
    '',
    'Options:',
    '  --attachment-recovery  bounded two-client encrypted-relay attachment recovery mode',
    '  --relay-binary PATH    exact external chariox-relay binary (required by attachment mode)',
    '  --kernel-binary PATH   exact external chariox-kernel binary (required by attachment mode)',
    '  --expected-protocol N  exact local daemon protocol (required by attachment mode)',
    `  --model ${DEFAULT_MODEL}`,
    `  --provider ${DEFAULT_PROVIDER}`,
    `  --workspace ${DEFAULT_WORKSPACE}`,
    `  --worktree ${DEFAULT_WORKTREE}`,
    `  --timeout-ms N          attachment mode defaults to ${DEFAULT_ATTACHMENT_RECOVERY_TIMEOUT_MS}ms (max ${MAX_ATTACHMENT_RECOVERY_TIMEOUT_MS}ms)`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --dry-run',
  ].join('\n'))
}

export function makeChildrenEnv(ports, rootDir) {
  const daemonId = `relay-drill-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `relay-drill-${process.pid}`
  const machineId = `relay-drill-machine-${process.pid}`
  const daemonRelayToken = signRelayToken(relayClaims({
    subject: daemonId,
    subjectKind: 'kernel',
    actions: ['daemon_register', 'daemon_heartbeat', 'packet_route', 'peer_request', 'peer_event'],
    userId: 'local',
    machineId,
  }))
  const clientRelayToken = signRelayToken(relayClaims({
    subject: `relay-drill-client-${process.pid}`,
    subjectKind: 'client',
    actions: ['client_connect', 'client_metadata_read', 'packet_route'],
    userId: 'local',
  }))
  return {
    relayToken: clientRelayToken,
    daemonId,
    daemonAlias,
    relayEnv: {
      ...process.env,
      CHARIOX_RELAY_HOST: '127.0.0.1',
      CHARIOX_RELAY_PORT: String(ports.relayPort),
      CHARIOX_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
      CHARIOX_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
    },
    daemonEnv: {
      ...process.env,
      CHARIOX_HOME: path.join(rootDir, 'home'),
      CHARIOX_LOG_DIR: path.join(rootDir, 'logs'),
      CHARIOX_KERNEL_PORT: String(ports.kernelPort),
      CHARIOX_MCP_PORT: String(ports.mcpPort),
      CHARIOX_OPENCODE_PORT: String(ports.openCodePort),
      CHARIOX_CODEX_PORT: String(ports.codexPort),
      CHARIOX_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
      CHARIOX_RELAY_TOKEN: daemonRelayToken,
      CHARIOX_DAEMON_ID: daemonId,
      CHARIOX_DAEMON_ALIAS: daemonAlias,
      CHARIOX_MACHINE_ID: machineId,
      CHARIOX_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
      CHARIOX_SESSION_HISTORY_DIR: path.join(rootDir, 'session-history'),
    },
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp

function unwrapVariant(resp, ...keys) {
  for (const key of keys) {
    if (resp?.[key] != null) return resp[key]
  }
  return resp
}

function logStep(name, details = null) {
  if (details == null) console.log(`[relay-drill] ${name}`)
  else console.log(`[relay-drill] ${name}`, JSON.stringify(sanitizeDrillMetadata(details)))
}

function nowStamp() {
  return new Date().toISOString().replace(/[-:]/g, '').replace(/\.\d{3}Z$/, 'Z')
}

function resolveCliDependencies(overrides = {}) {
  return {
    LocalIpcClient: overrides.LocalIpcClient ?? LocalIpcClient,
    createSessionRequest: overrides.createSessionRequest ?? createSessionRequest,
    endSessionRequest: overrides.endSessionRequest ?? endSessionRequest,
    listSessionsRequest: overrides.listSessionsRequest ?? listSessionsRequest,
  }
}

export async function waitForLocalDaemon(kernelUrl, workspace, worktree, daemonChild, overrides = {}) {
  const dependencies = resolveCliDependencies(overrides)
  const deadline = overrides.deadline ?? Date.now() + 15_000
  for (let attempt = 0; attempt < 60 && Date.now() < deadline; attempt += 1) {
    const probe = new dependencies.LocalIpcClient(kernelUrl)
    try {
      const probeTimeoutMs = Math.min(2_000, Math.max(1, deadline - Date.now()))
      const session = unwrap(await Promise.race([
        probe.send(dependencies.createSessionRequest(workspace, worktree, undefined, undefined, null, 'off')),
        sleep(probeTimeoutMs).then(() => { throw new Error('local daemon probe timeout') }),
      ]), 'SessionCreated').session
      await Promise.race([
        probe.send(dependencies.endSessionRequest(session.id)),
        sleep(Math.min(2_000, Math.max(1, deadline - Date.now()))),
      ]).catch(() => {})
      await probe.close()
      return session.id
    } catch {
      await probe.close().catch(() => {})
      const termination = childTerminationStatus(daemonChild)
      if (termination) {
        const stderr = sanitizeDrillMetadata({ stderr: daemonChild.logs?.stderr?.slice(-2000) ?? '' }).stderr
        throw new Error(`local daemon exited before readiness with ${termination}: ${stderr}`)
      }
      await sleep(Math.min(250, Math.max(1, deadline - Date.now())))
    }
  }
  throw new Error('local daemon did not become ready')
}

export async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias, overrides = {}) {
  const dependencies = resolveCliDependencies(overrides)
  const deadline = overrides.deadline ?? Date.now() + 20_000
  let lastError = null
  for (let attempt = 0; attempt < 80 && Date.now() < deadline; attempt += 1) {
    const client = new dependencies.LocalIpcClient(relayUrl, { relayAuthToken: relayToken, targetDaemonAlias })
    try {
      const probeTimeoutMs = Math.min(2_000, Math.max(1, deadline - Date.now()))
      await Promise.race([
        client.send(dependencies.listSessionsRequest()),
        sleep(probeTimeoutMs).then(() => { throw new Error('probe timeout') }),
      ])
      await client.close()
      return
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
      if ((attempt + 1) % 10 === 0) {
        logStep('relay_target_wait_retry', { attempt: attempt + 1, error: lastError })
      }
      await client.close().catch(() => {})
      await sleep(Math.min(250, Math.max(1, deadline - Date.now())))
    }
  }
  throw new Error(`relay target daemon did not become reachable: ${lastError ?? 'unknown error'}`)
}

async function waitForEvent(predicate, bucket, timeoutMs, description) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const match = bucket.find(predicate)
    if (match) return match
    await sleep(100)
  }
  throw new Error(`timed out waiting for ${description}`)
}

async function waitForCompletion(remoteClient, sessionId, attachmentId, eventBucket, timeoutMs, pollMs, baselineCount = 0) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const completions = eventBucket.filter((event) => event.event === 'assistant_message_completed')
    if (completions.length > baselineCount) {
      return completions[completions.length - 1]
    }
    const response = await remoteClient.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => null)
    collectPumpedTerminalOutput(response, eventBucket)
    await sleep(pollMs)
  }
  throw new Error('timed out waiting for assistant message completion')
}

async function waitForPromptEvidence(remoteClient, sessionId, attachmentId, eventBucket, options) {
  if (options.provider !== 'dev-stub') {
    return await waitForCompletion(
      remoteClient,
      sessionId,
      attachmentId,
      eventBucket,
      options.timeoutMs,
      options.pollMs,
      options.completionBaseline,
    )
  }
  const started = Date.now()
  while (Date.now() - started < options.timeoutMs) {
    if (terminalText(eventBucket).includes(options.expectedText)) {
      return { completed_at_ms: Date.now(), evidence: 'terminal_output' }
    }
    const response = await remoteClient.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => null)
    collectPumpedTerminalOutput(response, eventBucket)
    await sleep(options.pollMs)
  }
  throw new Error(`timed out waiting for dev-stub terminal output ${options.expectedText}`)
}

export function spawnProcess(command, args, options) {
  const child = spawn(command, args, { ...options, stdio: ['ignore', 'pipe', 'pipe'] })
  child.logs = { stdout: '', stderr: '' }
  child.stdout.on('data', (chunk) => { child.logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { child.logs.stderr += chunk.toString() })
  return child
}

export async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

function withTimeout(promise, label, timeoutMs) {
  let timer
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs)
      timer.unref?.()
    }),
  ]).finally(() => clearTimeout(timer))
}

function remainingMs(deadline, label, capMs) {
  const remaining = deadline - Date.now()
  if (remaining <= 0) throw new Error(`${label} exceeded the bounded attachment recovery timeout`)
  return Math.max(1, Math.min(remaining, capMs))
}

async function boundedPublicRequest(client, request, label, deadline) {
  assertNoSubmitPromptRequest(request, label)
  return await withTimeout(
    client.send(request),
    label,
    remainingMs(deadline, label, 5_000),
  )
}

function assertRelayHealth(health, daemonChild, relayUrl) {
  assert.equal(health?.projection?.process?.process_id, daemonChild.pid, 'relay health must identify the owned kernel process')
  assert.equal(
    health?.projection?.transport?.relay_last_connected_url,
    relayUrl,
    'relay health must report the candidate relay endpoint',
  )
  assert.equal(
    health?.projection?.transport?.relay_reconnect_attempts,
    0,
    'attachment recovery must not require a relay reconnect',
  )
}

function assertTerminalOutput(response, label) {
  const output = unwrap(response, 'TerminalOutput')
  assert.ok(Array.isArray(output.records), `${label} must return terminal output records`)
  return output
}

function childHasExited(child) {
  return child?.exitCode != null || child?.signalCode != null
}

function validateAttachmentRecoveryOptions(options) {
  if (!options.relayBinary || !path.isAbsolute(options.relayBinary)) {
    throw new Error('--attachment-recovery requires an absolute --relay-binary path')
  }
  if (!options.kernelBinary || !path.isAbsolute(options.kernelBinary)) {
    throw new Error('--attachment-recovery requires an absolute --kernel-binary path')
  }
  if (!Number.isSafeInteger(options.expectedProtocol) || options.expectedProtocol < 1) {
    throw new Error('--attachment-recovery requires a positive --expected-protocol')
  }
  if (!Number.isSafeInteger(options.timeoutMs) || options.timeoutMs < 1 || options.timeoutMs > MAX_ATTACHMENT_RECOVERY_TIMEOUT_MS) {
    throw new Error(`--attachment-recovery --timeout-ms must be between 1 and ${MAX_ATTACHMENT_RECOVERY_TIMEOUT_MS}`)
  }
}

async function claimAttachmentRecoveryChild(child, binary, label) {
  try {
    return await claimExactChildProcess({ child, binary })
  } catch (error) {
    const failure = error instanceof Error ? error : new Error(String(error))
    if (!childHasExited(child)) {
      failure.message = `${label} ownership was not established; no signal will be sent to PID ${child.pid ?? '<missing>'}: ${failure.message}`
    }
    throw failure
  }
}

async function stopAttachmentRecoveryChild(child, ownership, label, cleanupErrors) {
  if (!child || childHasExited(child)) return
  if (!ownership) {
    cleanupErrors.push(new Error(`${label} ownership was not established; refusing to signal PID ${child.pid ?? '<missing>'}`))
    return
  }
  try {
    await stopOwnedChild(child, ownership, {
      graceMs: 5_000,
      killGraceMs: 2_000,
    })
  } catch (error) {
    cleanupErrors.push(new Error(`${label} cleanup failed: ${error?.message ?? error}`))
  }
  if (!childHasExited(child)) {
    cleanupErrors.push(new Error(`${label} cleanup left PID ${ownership.pid} running`))
  }
}

async function runAttachmentRecoveryScenario({
  options,
  deadline,
  kernelUrl,
  relayUrl,
  envs,
  daemonChild,
  workspace,
  readinessSessionId,
  context,
}) {
  const ledger = createAttachmentRecoveryLedger()
  const setupClient = new LocalIpcClient(kernelUrl)
  const clientOptions = {
    relayAuthToken: envs.relayToken,
    targetDaemonAlias: envs.daemonAlias,
    controlRequestRetryDeadlineMs: 0,
    controlResponseStallMs: 500,
  }
  const clientA = new LocalIpcClient(relayUrl, clientOptions)
  const clientB = new LocalIpcClient(relayUrl, clientOptions)
  // Session creation follows the existing relay drill's direct local readiness
  // path. Both attachment/recovery clients, including their event lanes, use
  // the encrypted relay transport below.
  context.clients.push(setupClient, clientA, clientB)
  const eventLogA = []
  const eventLogB = []
  clientA.onKernelEvent((event) => eventLogA.push({ ...event, observed_at_ms: Date.now() }))
  clientB.onKernelEvent((event) => eventLogB.push({ ...event, observed_at_ms: Date.now() }))

  const health = unwrap(
    await boundedPublicRequest(clientA, getDaemonHealthRequest(), 'first daemon health', deadline),
    'DaemonHealth',
  )
  ledger.record('first daemon health')
  assertRelayHealth(health, daemonChild, relayUrl)

  const initialList = unwrap(
    await boundedPublicRequest(clientA, listSessionsRequest(), 'initial session list', deadline),
    'SessionsListed',
  )
  ledger.record('initial session list')
  const initialSessionIds = new Set(initialList.sessions.map((session) => session.id))
  assert.ok(
    initialSessionIds.has(readinessSessionId),
    'initial relay candidate state must retain the known readiness session',
  )

  const created = unwrap(
    await boundedPublicRequest(
      setupClient,
      createSessionRequest(workspace, workspace, 'candidate-relay-attachment-recovery'),
      'session create',
      deadline,
    ),
    'SessionCreated',
  )
  ledger.record('session create')
  const sessionId = created.session.id
  context.sessionId = sessionId
  const agentId = created.agent.id
  assert.ok(sessionId, 'relay candidate must create a session')
  assert.ok(agentId, 'relay candidate session must include its default agent')
  assert.notEqual(sessionId, readinessSessionId, 'acceptance session must differ from the readiness session')
  assert.equal(created.agent.session_id, sessionId, 'created agent must belong to the created session')

  const attachmentA = unwrap(
    await boundedPublicRequest(
      clientA,
      attachToSessionRequest(sessionId, `candidate-relay-attachment-recovery-${process.pid}-a`),
      'attachment A attach',
      deadline,
    ),
    'SessionAttached',
  ).attachment
  ledger.record('attachment A attach')
  assert.ok(attachmentA.id, 'attachment A must have a public identity')
  assert.equal(attachmentA.session_id, sessionId)

  const stateWithA = unwrap(
    await boundedPublicRequest(clientA, getSessionStateRequest(sessionId), 'attachment A state', deadline),
    'SessionState',
  )
  ledger.record('attachment A state')
  const stateWithAAgent = stateWithA.session.agents.find((agent) => agent.id === agentId)
  assert.ok(stateWithAAgent, 'created agent must be present after relay attachment A')
  const expectedSession = durableSessionFingerprint(stateWithA.session)
  const expectedAgent = durableAgentFingerprint(stateWithAAgent)
  assertAttachmentRecoveryState({
    session: stateWithA.session,
    expectedSessionId: sessionId,
    expectedAttachmentIds: [attachmentA.id],
    expectedSession,
    expectedAgent,
  })

  const attachmentB = unwrap(
    await boundedPublicRequest(
      clientB,
      attachToSessionRequest(sessionId, `candidate-relay-attachment-recovery-${process.pid}-b`),
      'attachment B attach',
      deadline,
    ),
    'SessionAttached',
  ).attachment
  ledger.record('attachment B attach')
  assert.ok(attachmentB.id, 'attachment B must have a public identity')
  assert.notEqual(attachmentB.id, attachmentA.id, 'two relay clients must receive distinct attachment identities')
  assert.equal(attachmentB.session_id, sessionId)

  await withTimeout(
    clientA.subscribeToKernelEvents(sessionId, attachmentA.id),
    'attachment A relay event subscription',
    remainingMs(deadline, 'attachment A relay event subscription', 5_000),
  )
  await withTimeout(
    clientB.subscribeToKernelEvents(sessionId, attachmentB.id),
    'attachment B relay event subscription',
    remainingMs(deadline, 'attachment B relay event subscription', 5_000),
  )
  await waitForEvent(
    (event) => event.event === 'session_snapshot',
    eventLogA,
    remainingMs(deadline, 'attachment A relay event subscription', 5_000),
    'attachment A relay session snapshot',
  )
  await waitForEvent(
    (event) => event.event === 'session_snapshot',
    eventLogB,
    remainingMs(deadline, 'attachment B relay event subscription', 5_000),
    'attachment B relay session snapshot',
  )

  const twoAttachmentState = unwrap(
    await boundedPublicRequest(clientB, getSessionStateRequest(sessionId), 'two-attachment authoritative state', deadline),
    'SessionState',
  )
  ledger.record('two-attachment authoritative state')
  assertAttachmentRecoveryState({
    session: twoAttachmentState.session,
    expectedSessionId: sessionId,
    expectedAttachmentIds: [attachmentA.id, attachmentB.id],
    expectedSession,
    expectedAgent,
  })

  let recoveryDisconnected = false
  let recoveryCompleted = false
  let recoveryReset = false
  let recoverySynced = false
  let recoveryPaneRefreshes = 0
  let recoveryBusyCleared = false
  let recoveryResetPromise = null
  let recoveredAttachment = null
  let recoveredSession = null
  const recoveryFailures = []
  const controllerModulePath = path.join(cliRoot, 'dist', 'kernel-restart-recovery-controller.js')
  let createKernelRestartRecoveryController
  try {
    ({ createKernelRestartRecoveryController } = await import(pathToFileURL(controllerModulePath).href))
  } catch (error) {
    throw new Error(
      `relay attachment recovery requires the prebuilt production controller at ${controllerModulePath}: ${error?.message ?? error}`,
    )
  }
  const recoveryController = createKernelRestartRecoveryController({
    initialDelayMs: 1,
    maxDelayMs: 10,
    isClosing: () => false,
    isAttached: () => true,
    isDisconnected: () => recoveryDisconnected,
    getSessionId: () => sessionId,
    getSessionState: async (recoverySessionId) => {
      const state = unwrap(
        await boundedPublicRequest(
          clientA,
          getSessionStateRequest(recoverySessionId),
          'controller recovery authoritative state',
          deadline,
        ),
        'SessionState',
      )
      ledger.record('controller recovery authoritative state')
      assertAttachmentRecoveryState({
        session: state.session,
        expectedSessionId: sessionId,
        expectedAttachmentIds: [attachmentB.id],
        expectedSession,
        expectedAgent,
      })
      return state.session
    },
    attachToSession: async (recoverySessionId) => {
      const attached = unwrap(
        await boundedPublicRequest(
          clientA,
          attachToSessionRequest(
            recoverySessionId,
            `candidate-relay-attachment-recovery-${process.pid}-a-recovered`,
          ),
          'controller recovery replacement attach',
          deadline,
        ),
        'SessionAttached',
      )
      ledger.record('controller recovery replacement attach')
      return attached.attachment
    },
    projectSession: (session) => session,
    applyAttachment: (attachment) => {
      recoveredAttachment = attachment
    },
    applySession: (session) => {
      recoveredSession = session
    },
    resetKernelEventSubscription: () => {
      recoveryReset = true
      recoveryResetPromise = withTimeout(
        clientA.unsubscribeFromKernelEvents(),
        'controller recovery relay event reset',
        remainingMs(deadline, 'controller recovery relay event reset', 5_000),
      )
    },
    syncKernelEventSubscription: async () => {
      await recoveryResetPromise
      assert.ok(recoveredAttachment?.id, 'controller must apply the replacement before resubscribing')
      await withTimeout(
        clientA.subscribeToKernelEvents(sessionId, recoveredAttachment.id),
        'controller recovery relay event resync',
        remainingMs(deadline, 'controller recovery relay event resync', 5_000),
      )
      recoverySynced = true
    },
    refreshAgentPanes: async () => {
      recoveryPaneRefreshes += 1
    },
    clearLocalBusyStateForAuthoritativeIdle: () => {
      recoveryBusyCleared = true
    },
    onRecovered: () => {
      recoveryCompleted = true
      recoveryDisconnected = false
    },
    onAttemptFailed: (_sessionId, error) => {
      recoveryFailures.push(error instanceof Error ? error.message : String(error))
    },
    sleep: async (delayMs) => {
      const remaining = deadline - Date.now()
      if (remaining <= 0) throw new Error('relay attachment recovery exceeded its bounded timeout')
      await new Promise((resolve) => {
        const timer = setTimeout(resolve, Math.min(delayMs, remaining))
        timer.unref?.()
      })
    },
  })

  assertTerminalOutput(
    await boundedPublicRequest(
      clientB,
      pumpTerminalOutputRequest(sessionId, attachmentB.id),
      'sibling pump while both attached',
      deadline,
    ),
    'sibling pump while both attached',
  )
  ledger.record('sibling pump while both attached')

  const detachedA = unwrap(
    await boundedPublicRequest(clientA, detachFromSessionRequest(attachmentA.id), 'attachment A detach', deadline),
    'SessionDetached',
  )
  ledger.record('attachment A detach')
  assert.equal(detachedA.attachment.id, attachmentA.id)
  assert.equal(detachedA.attachment.session_id, sessionId)

  let staleAttachmentError = null
  await assert.rejects(
    () => boundedPublicRequest(
      clientA,
      pumpTerminalOutputRequest(sessionId, attachmentA.id),
      'stale attachment output',
      deadline,
    ),
    (error) => {
      staleAttachmentError = error
      return assertStaleAttachmentRejected(error)
    },
  )
  logStep('stale_attachment_rejected', { attachmentId: attachmentA.id, code: staleAttachmentError.code })

  // The real production recovery controller is triggered only after the
  // public stale-attachment request has returned the kernel rejection.
  recoveryDisconnected = true
  const recoveryPromise = recoveryController.recover()
  assert.ok(recoveryPromise, 'stale relay attachment must trigger the production recovery controller')
  await recoveryPromise
  assert.deepEqual(recoveryFailures, [], 'production recovery controller must recover without a failed retry')
  assert.equal(recoveryCompleted, true, 'production recovery controller did not report relay recovery')
  assert.equal(recoveryReset, true, 'production recovery controller did not reset the relay event subscription')
  assert.equal(recoverySynced, true, 'production recovery controller did not resync the relay event subscription')
  assert.equal(recoveryPaneRefreshes, 1, 'production recovery controller did not refresh its session projection')
  assert.equal(recoveryBusyCleared, true, 'production recovery controller did not clear stale local busy state')
  assertAttachmentRecoveryState({
    session: recoveredSession,
    expectedSessionId: sessionId,
    expectedAttachmentIds: [attachmentB.id],
    expectedSession,
    expectedAgent,
  })
  assert.ok(recoveredAttachment?.id, 'controller recovery must return a replacement attachment')
  assert.notEqual(recoveredAttachment.id, attachmentA.id, 'recovery must replace the stale relay attachment identity')
  assert.notEqual(recoveredAttachment.id, attachmentB.id, 'recovery must preserve the sibling relay attachment')
  assert.equal(recoveredAttachment.session_id, sessionId)

  const postRecoveryState = unwrap(
    await boundedPublicRequest(clientB, getSessionStateRequest(sessionId), 'post-recovery authoritative state', deadline),
    'SessionState',
  )
  ledger.record('post-recovery authoritative state')
  assertAttachmentRecoveryState({
    session: postRecoveryState.session,
    expectedSessionId: sessionId,
    expectedAttachmentIds: [attachmentB.id, recoveredAttachment.id],
    expectedSession,
    expectedAgent,
  })

  const postRecoveryList = unwrap(
    await boundedPublicRequest(clientB, listSessionsRequest(), 'post-recovery session list', deadline),
    'SessionsListed',
  )
  ledger.record('post-recovery session list')
  const postRecoverySessionIds = new Set(postRecoveryList.sessions.map((session) => session.id))
  assert.ok(
    postRecoverySessionIds.has(readinessSessionId),
    'relay attachment recovery must retain the known readiness session',
  )
  const newAcceptanceSessions = postRecoveryList.sessions.filter(
    (session) => !initialSessionIds.has(session.id),
  )
  assert.equal(
    newAcceptanceSessions.length,
    1,
    'relay attachment recovery must create exactly one new acceptance session',
  )
  assert.equal(
    newAcceptanceSessions[0].id,
    sessionId,
    'the only new relay session must be the acceptance session',
  )
  assertAttachmentRecoveryState({
    session: newAcceptanceSessions[0],
    expectedSessionId: sessionId,
    expectedAttachmentIds: [attachmentB.id, recoveredAttachment.id],
    expectedSession,
    expectedAgent,
  })

  assertTerminalOutput(
    await boundedPublicRequest(
      clientB,
      pumpTerminalOutputRequest(sessionId, attachmentB.id),
      'sibling pump after replacement',
      deadline,
    ),
    'sibling pump after replacement',
  )
  ledger.record('sibling pump after replacement')

  const detachedReplacement = unwrap(
    await boundedPublicRequest(
      clientA,
      detachFromSessionRequest(recoveredAttachment.id),
      'replacement attachment detach',
      deadline,
    ),
    'SessionDetached',
  )
  ledger.record('replacement attachment detach')
  assert.equal(detachedReplacement.attachment.id, recoveredAttachment.id)

  const detachedSibling = unwrap(
    await boundedPublicRequest(clientB, detachFromSessionRequest(attachmentB.id), 'sibling attachment detach', deadline),
    'SessionDetached',
  )
  ledger.record('sibling attachment detach')
  assert.equal(detachedSibling.attachment.id, attachmentB.id)

  const finalState = unwrap(
    await boundedPublicRequest(clientB, getSessionStateRequest(sessionId), 'final authoritative state', deadline),
    'SessionState',
  )
  ledger.record('final authoritative state')
  assertAttachmentRecoveryState({
    session: finalState.session,
    expectedSessionId: sessionId,
    expectedAttachmentIds: [],
    expectedSession,
    expectedAgent,
  })
  ledger.assertComplete()
  return {
    sessionId,
    agentId,
    attachmentAId: attachmentA.id,
    attachmentBId: attachmentB.id,
    replacementAttachmentId: recoveredAttachment.id,
    staleAttachmentError: staleAttachmentError.code,
    successfulRequests: ledger.count,
    eventCounts: {
      attachmentA: eventLogA.length,
      attachmentB: eventLogB.length,
      sessionSnapshotsA: eventLogA.filter((event) => event.event === 'session_snapshot').length,
      sessionSnapshotsB: eventLogB.filter((event) => event.event === 'session_snapshot').length,
    },
    protocol: options.expectedProtocol,
    transport: 'encrypted relay payloads over LocalIpcClient relay mode',
    relayUrl,
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.dryRun) {
    console.log(JSON.stringify(options, null, 2))
    return
  }
  if (options.attachmentRecovery) validateAttachmentRecoveryOptions(options)

  const ports = await makeAvailablePorts()
  const rootDir = path.join(
    os.homedir(),
    '.chariox',
    'dev',
    'browser-computer-use',
    'relay-runtime',
    nowStamp(),
  )
  await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(rootDir, { recursive: true })
  const cliRuntimeDir = path.join(rootDir, 'cli-runtime')
  await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(cliRuntimeDir, { recursive: true })

  let relayChild = null
  let daemonChild = null
  let relayOwnership = null
  let daemonOwnership = null
  let localClient = null
  let remoteClient = null
  const attachmentClients = []
  const attachmentContext = { clients: attachmentClients, sessionId: null }
  let sessionId = null
  let eventLog = []
  let succeeded = false
  let failure = null
  const cleanupErrors = []
  let attachmentRecoveryDetails = null
  let envs = null
  let relayBinary = null
  let daemonBinary = null
  let readinessSessionId = null
  let kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  let relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const attachmentDeadline = options.attachmentRecovery
    ? Date.now() + options.timeoutMs
    : null

  try {
    ;({ LocalIpcClient, requests: {
      createSessionRequest,
      attachToSessionRequest,
      detachFromSessionRequest,
      getDaemonHealthRequest,
      getSessionStateRequest,
      listSessionsRequest,
      launchProviderRunRequest,
      submitPromptRequest,
      pumpTerminalOutputRequest,
      endSessionRequest,
    } } = await loadCliModules(cliRuntimeDir))
    envs = makeChildrenEnv(ports, rootDir)
    relayBinary = options.attachmentRecovery
      ? await assertExactCandidateBinary(options.relayBinary)
      : await resolveBuiltBinary(
          path.join(repoRoot, 'apps/relay/target/debug/chariox-relay'),
          path.join(repoRoot, 'apps/relay/Cargo.toml'),
          'chariox-relay',
        )
    daemonBinary = options.attachmentRecovery
      ? await assertExactCandidateBinary(options.kernelBinary)
      : await resolveBuiltBinary(
          path.join(repoRoot, 'apps/kernel/target/debug/chariox-kernel'),
          path.join(repoRoot, 'apps/kernel/Cargo.toml'),
          'chariox-kernel',
        )
    if (options.attachmentRecovery) {
      const version = await runVersionProbe(
        daemonBinary,
        Math.min(5_000, Math.max(1, attachmentDeadline - Date.now())),
      )
      assertCandidateProtocolOutput(version.stdout, options.expectedProtocol)
      logStep('attachment_recovery_protocol_preflight_passed', {
        kernelBinary: daemonBinary,
        relayBinary,
        expectedProtocol: options.expectedProtocol,
      })
    }
    logStep('spawn_relay', { relayUrl, daemonAlias: envs.daemonAlias, relayToken: envs.relayToken })
    relayChild = spawnProcess(
      relayBinary,
      [],
      { cwd: repoRoot, env: envs.relayEnv },
    )
    if (options.attachmentRecovery) {
      relayOwnership = await claimAttachmentRecoveryChild(relayChild, relayBinary, 'relay')
    }

    daemonChild = spawnProcess(
      daemonBinary,
      [],
      { cwd: repoRoot, env: envs.daemonEnv },
    )
    if (options.attachmentRecovery) {
      daemonOwnership = await claimAttachmentRecoveryChild(daemonChild, daemonBinary, 'kernel')
    }

    const startupWorkspace = options.attachmentRecovery
      ? path.join(rootDir, 'workspace')
      : options.workspace
    if (options.attachmentRecovery) await mkdir(startupWorkspace, { recursive: true })
    readinessSessionId = await waitForLocalDaemon(
      kernelUrl,
      startupWorkspace,
      startupWorkspace,
      daemonChild,
      options.attachmentRecovery ? { deadline: attachmentDeadline } : {},
    )
    await waitForRelayTarget(
      relayUrl,
      envs.relayToken,
      envs.daemonAlias,
      options.attachmentRecovery ? { deadline: attachmentDeadline } : {},
    )

    if (options.attachmentRecovery) {
      const recovery = await runAttachmentRecoveryScenario({
        options,
        deadline: attachmentDeadline,
        kernelUrl,
        relayUrl,
        envs,
        daemonChild,
        workspace: startupWorkspace,
        readinessSessionId,
        context: attachmentContext,
      })
      sessionId = recovery.sessionId
      attachmentRecoveryDetails = {
        ...recovery,
        kernelUrl,
        daemonId: envs.daemonId,
        daemonAlias: envs.daemonAlias,
        readinessSessionId,
        relayBinary,
        kernelBinary: daemonBinary,
        status: 'ok',
      }
      succeeded = true
    } else {

      localClient = new LocalIpcClient(kernelUrl)
    const created = unwrap(await localClient.send(createSessionRequest(options.workspace, options.worktree, undefined, undefined, null, 'off')), 'SessionCreated')
    sessionId = created.session.id
    const defaultAgentId = created.agent.id
    logStep('local_session_created', { sessionId, defaultAgentId })

    remoteClient = new LocalIpcClient(relayUrl, {
      relayAuthToken: envs.relayToken,
      targetDaemonAlias: envs.daemonAlias,
    })
    remoteClient.onKernelEvent((event) => {
      eventLog.push({ ...event, observed_at_ms: Date.now() })
    })

    const listed = unwrapVariant(await remoteClient.send(listSessionsRequest()), 'SessionsListed', 'Sessions')
    const sessionIds = (listed.sessions || []).map((session) => session.id)
    if (!sessionIds.includes(sessionId)) {
      throw new Error(`relay list_sessions did not include ${sessionId}`)
    }
    logStep('relay_list_sessions_ok', { count: sessionIds.length })

    const remoteStateBeforeAttach = unwrapVariant(await remoteClient.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const remoteSessionBeforeAttach = remoteStateBeforeAttach.session ?? remoteStateBeforeAttach.SessionStateLoaded?.session ?? null
    if (remoteSessionBeforeAttach?.id !== sessionId) {
      throw new Error(`remote get_session_state returned unexpected session: ${JSON.stringify(remoteStateBeforeAttach)}`)
    }
    logStep('relay_get_session_state_ok', { sessionId })

    const attachment = unwrap(
      await remoteClient.send(attachToSessionRequest(sessionId, `relay-drill-client-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    logStep('relay_attach_ok', { attachmentId: attachment.id })

    await remoteClient.subscribeToKernelEvents(sessionId, attachment.id)
    await waitForEvent((event) => event.event === 'session_snapshot', eventLog, 30_000, 'initial session snapshot')
    logStep('relay_subscribe_ok', { events: eventLog.length })

    const launchResponse = unwrapVariant(
      await remoteClient.send(launchProviderRunRequest(sessionId, options.provider, 'default', options.model, 'medium', defaultAgentId)),
      'ProviderRunLaunchAccepted',
      'ProviderRunLaunched',
    )
    const providerRunId = launchResponse.provider_run?.id ?? launchResponse.provider_run_id ?? null
    logStep('relay_launch_provider_run_ok', { providerRunId, provider: options.provider, model: options.model })

    const firstPrompt = 'Reply with exactly RELAY_OK and nothing else.'
    await remoteClient.send(submitPromptRequest(sessionId, attachment.id, defaultAgentId, firstPrompt, []))
    logStep('relay_submit_prompt_ok', { prompt: firstPrompt })

    const firstCompletion = await waitForPromptEvidence(
      remoteClient,
      sessionId,
      attachment.id,
      eventLog,
      {
        ...options,
        completionBaseline: 0,
        expectedText: 'RELAY_OK',
      },
    )
    const firstTerminalOutputs = eventLog.filter((event) => event.event === 'terminal_output').length
    logStep('relay_prompt_completed', {
      completedAtMs: firstCompletion.completed_at_ms,
      terminalOutputEvents: firstTerminalOutputs,
    })

    logStep('restart_relay_begin')
    await terminateChild(relayChild)
    await waitForEvent((event) => event.event === 'transport_closed', eventLog, 30_000, 'transport_closed event')
    logStep('relay_down_observed')

    relayChild = spawnProcess(
      relayBinary,
      [],
      { cwd: repoRoot, env: envs.relayEnv },
    )

    await waitForRelayTarget(relayUrl, envs.relayToken, envs.daemonAlias)
    logStep('relay_target_reachable_after_restart')
    const resumed = await waitForEvent((event) => event.event === 'transport_resumed', eventLog, 45_000, 'transport_resumed event')
    logStep('relay_reconnect_ok', { resumedFromEventId: resumed.resumed_from_event_id ?? null })

    const stateAfterResume = unwrapVariant(await remoteClient.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const resumedSession = stateAfterResume.session ?? stateAfterResume.SessionStateLoaded?.session ?? null
    if (resumedSession?.id !== sessionId) {
      throw new Error(`relay get_session_state failed after reconnect: ${JSON.stringify(stateAfterResume)}`)
    }
    logStep('relay_request_after_reconnect_ok', { sessionId })

    const completionBaseline = eventLog.filter((event) => event.event === 'assistant_message_completed').length
    const secondPrompt = 'Reply with exactly RELAY_RECONNECT_OK and nothing else.'
    await remoteClient.send(submitPromptRequest(sessionId, attachment.id, defaultAgentId, secondPrompt, []))
    logStep('relay_submit_prompt_after_reconnect_ok', { prompt: secondPrompt })

    const secondCompletion = await waitForPromptEvidence(
      remoteClient,
      sessionId,
      attachment.id,
      eventLog,
      {
        ...options,
        completionBaseline,
        expectedText: 'RELAY_RECONNECT_OK',
      },
    )
    logStep('relay_prompt_after_reconnect_completed', {
      completedAtMs: secondCompletion.completed_at_ms,
      totalEvents: eventLog.length,
    })

    console.log(JSON.stringify({
      relayUrl,
      kernelUrl,
      daemonId: envs.daemonId,
      daemonAlias: envs.daemonAlias,
      sessionId,
      provider: options.provider,
      model: options.model,
      events: {
        total: eventLog.length,
        sessionSnapshots: eventLog.filter((event) => event.event === 'session_snapshot').length,
        terminalOutput: eventLog.filter((event) => event.event === 'terminal_output').length,
        completed: eventLog.filter((event) => event.event === 'assistant_message_completed').length,
        transportClosed: eventLog.filter((event) => event.event === 'transport_closed').length,
        transportResumed: eventLog.filter((event) => event.event === 'transport_resumed').length,
      },
      reconnect: {
        resumedFromEventId: resumed.resumed_from_event_id ?? null,
      },
      status: 'ok',
    }, null, 2))
      succeeded = true
    }
  } catch (error) {
    failure = error
  } finally {
    if (options.attachmentRecovery && !sessionId) sessionId = attachmentContext.sessionId
    if (options.attachmentRecovery) {
      const cleanupClient = attachmentClients.at(-1) ?? localClient
      if (sessionId && cleanupClient) {
        try {
          const cleanupRequest = endSessionRequest(sessionId)
          assertNoSubmitPromptRequest(cleanupRequest, 'attachment recovery session cleanup')
          await withTimeout(
            cleanupClient.send(cleanupRequest),
            'attachment recovery session cleanup',
            5_000,
          )
        } catch (error) {
          cleanupErrors.push(new Error(`attachment recovery session cleanup failed: ${error?.message ?? error}`))
        }
      }
      for (const [label, client] of attachmentClients.entries()) {
        try {
          await withTimeout(client.close(), `attachment recovery client ${label + 1} cleanup`, 2_500)
        } catch (error) {
          cleanupErrors.push(new Error(`attachment recovery client ${label + 1} cleanup failed: ${error?.message ?? error}`))
        }
      }
      await stopAttachmentRecoveryChild(daemonChild, daemonOwnership, 'kernel', cleanupErrors)
      await stopAttachmentRecoveryChild(relayChild, relayOwnership, 'relay', cleanupErrors)
    } else {
      if (remoteClient) {
        if (sessionId) await remoteClient.send(endSessionRequest(sessionId)).catch(() => {})
        await remoteClient.close().catch(() => {})
      } else if (localClient && sessionId) {
        await localClient.send(endSessionRequest(sessionId)).catch(() => {})
      }
      if (localClient) await localClient.close().catch(() => {})
      await terminateChild(daemonChild)
      await terminateChild(relayChild)
    }
    if (cleanupErrors.length > 0) {
      const cleanupFailure = new Error(
        cleanupErrors.map((error) => error.stack ?? error.message).join('\n'),
      )
      succeeded = false
      failure = failure
        ? new AggregateError([failure, cleanupFailure], 'relay drill failed with cleanup errors')
        : cleanupFailure
    }
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: options.attachmentRecovery ? 'relay-kernel-attachment-recovery' : 'relay-runtime',
        relayUrl,
        kernelUrl,
        daemonId: envs?.daemonId ?? null,
        daemonAlias: envs?.daemonAlias ?? null,
        readinessSessionId,
        sessionId,
        attachmentRecovery: options.attachmentRecovery,
        attachmentRecoveryDetails,
        provider: options.provider,
        model: options.model,
        eventCounts: {
          total: eventLog.length,
          sessionSnapshots: eventLog.filter((event) => event.event === 'session_snapshot').length,
          terminalOutput: eventLog.filter((event) => event.event === 'terminal_output').length,
          completed: eventLog.filter((event) => event.event === 'assistant_message_completed').length,
          transportClosed: eventLog.filter((event) => event.event === 'transport_closed').length,
          transportResumed: eventLog.filter((event) => event.event === 'transport_resumed').length,
        },
        relayStdoutTail: relayChild?.logs?.stdout?.slice(-4000) ?? '',
        relayStderrTail: relayChild?.logs?.stderr?.slice(-4000) ?? '',
        daemonStdoutTail: daemonChild?.logs?.stdout?.slice(-4000) ?? '',
        daemonStderrTail: daemonChild?.logs?.stderr?.slice(-4000) ?? '',
      },
      log: logStep,
    })
    if (options.attachmentRecovery && succeeded && !failure && attachmentRecoveryDetails) {
      logStep('passed', attachmentRecoveryDetails)
    }
  }
  if (failure) throw failure
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
