#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { createHmac } from 'node:crypto'
import { mkdir, readdir, readFile, rm, stat } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'
import { remoteEnvCommand, shellQuote, sshArgs } from './lib/native-tui-remote-execution.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const RELAY_ISSUER = 'arroba-relay-freeform-multi-user-drill'
const RELAY_SECRET = 'arroba-relay-freeform-multi-user-drill-secret'
const RELAY_REALM = 'relay-freeform-multi-user-drill'
const DEFAULT_PROVIDER = 'codex'
const DEFAULT_MODEL = 'gpt-5.4-mini'
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

function parseArgs(argv = process.argv.slice(2)) {
  const options = {
    hetznerRelay: false,
    hetznerHost: process.env.ARROBA_COLLAB_HETZNER_HOST ?? process.env.ARROBA_NATIVE_TUI_HETZNER_HOST ?? 'root@195.201.123.115',
    hetznerKey: process.env.ARROBA_COLLAB_HETZNER_KEY ?? process.env.ARROBA_NATIVE_TUI_HETZNER_KEY ?? path.join(os.homedir(), '.ssh/arroba_hetzner_staging'),
    hetznerRepo: process.env.ARROBA_COLLAB_HETZNER_REPO ?? process.env.ARROBA_NATIVE_TUI_HETZNER_REPO ?? '/tmp/arroba-native-remote-validate',
    provider: process.env.ARROBA_COLLAB_PROVIDER ?? DEFAULT_PROVIDER,
    model: process.env.ARROBA_COLLAB_MODEL ?? DEFAULT_MODEL,
    timeoutMs: Number.parseInt(process.env.ARROBA_COLLAB_TIMEOUT_MS ?? '300000', 10),
    pollMs: Number.parseInt(process.env.ARROBA_COLLAB_POLL_MS ?? '1000', 10),
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--hetzner-relay') {
      options.hetznerRelay = true
    } else if (arg === '--hetzner-host') {
      options.hetznerHost = argv[++index]
    } else if (arg === '--hetzner-key') {
      options.hetznerKey = argv[++index]
    } else if (arg === '--hetzner-repo') {
      options.hetznerRepo = argv[++index]
    } else if (arg === '--provider') {
      options.provider = argv[++index]
    } else if (arg === '--model') {
      options.model = argv[++index]
    } else if (arg === '--timeout-ms') {
      options.timeoutMs = Number.parseInt(argv[++index], 10)
    } else if (arg === '--poll-ms') {
      options.pollMs = Number.parseInt(argv[++index], 10)
    } else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-relay-freeform-multi-user-drill.mjs [options]',
        '',
        'Options:',
        '  --hetzner-relay       Run the scoped relay on the configured Hetzner host through an SSH tunnel',
        '  --hetzner-host HOST   SSH host for --hetzner-relay',
        '  --hetzner-key PATH    SSH key for --hetzner-relay',
        '  --hetzner-repo PATH   Remote Arroba checkout containing built relay binary',
        `  --provider PROVIDER   Provider for live model prompts (default ${DEFAULT_PROVIDER})`,
        `  --model MODEL         Model for live model prompts (default ${DEFAULT_MODEL})`,
        '  --timeout-ms MS       Prompt completion timeout',
        '  --poll-ms MS          Prompt completion poll interval',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown argument ${arg}`)
    }
  }
  return options
}

function log(name, details = null) {
  if (details == null) console.log(`[relay-freeform-multi-user-drill] ${name}`)
  else console.log(`[relay-freeform-multi-user-drill] ${name}`, JSON.stringify(details))
}

function assert(condition, message, details = null) {
  if (!condition) {
    throw new Error(`${message}${details == null ? '' : `\n${JSON.stringify(details, null, 2)}`}`)
  }
}

function modelForProvider(provider, model) {
  if (provider === 'opencode' && !model.includes('/')) return `opencode/${model}`
  if (provider === 'codex' && !model.includes('/')) return opencodeCodexModel(model)
  return model
}

function opencodeCodexModel(model) {
  if (model.endsWith('-codex')) return model
  if (/^gpt-5\.[23]$/.test(model)) return `${model}-codex`
  return model
}

function base64url(input) {
  return Buffer.from(input).toString('base64url')
}

function signRelayToken(claims) {
  const payload = base64url(JSON.stringify(claims))
  const signature = createHmac('sha256', RELAY_SECRET).update(payload).digest('base64url')
  return `arroba-scoped-v1.${payload}.${signature}`
}

function relayClaims({ subject, subjectKind, actions, userId = null }) {
  return {
    issuer: RELAY_ISSUER,
    subject,
    subject_kind: subjectKind,
    realm_id: RELAY_REALM,
    allowed_actions: actions,
    allowed_targets: null,
    issued_at_ms: Date.now(),
    expires_at_ms: Date.now() + 10 * 60_000,
    token_id: `${subject}-${Date.now()}`,
    account_id: 'relay-freeform-multi-user-drill-account',
    organization_id: null,
    user_id: userId,
    device_id: subject,
    machine_id: subjectKind === 'kernel' || subjectKind === 'machine' ? subject : null,
    client_id: subjectKind === 'client' ? subject : null,
    public_key_thumbprint: `${subject}-thumbprint`,
    entitlements_version: 'drill',
  }
}

function clientToken(userId) {
  return signRelayToken(relayClaims({
    subject: `client-${userId}-${process.pid}-${Date.now()}`,
    subjectKind: 'client',
    actions: ['client_connect', 'client_metadata_read', 'packet_route'],
    userId,
  }))
}

function makePorts() {
  const base = 50000 + Math.floor(Math.random() * 1000)
  return {
    relayPort: base,
    kernelPort: base + 1000,
    mcpPort: base + 2000,
    opencodePort: base + 3000,
    codexPort: base + 3001,
  }
}

function makeEnv(ports, rootDir) {
  const daemonId = `relay-freeform-multi-user-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `relay-freeform-multi-user-${process.pid}`
  const daemonRelayToken = signRelayToken(relayClaims({
    subject: daemonId,
    subjectKind: 'kernel',
    actions: ['daemon_register', 'daemon_heartbeat', 'peer_request', 'peer_event'],
    userId: 'kernel-owner',
  }))
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  return {
    daemonAlias,
    relayUrl,
    relayEnv: {
      ...process.env,
      ARROBA_RELAY_HOST: '127.0.0.1',
      ARROBA_RELAY_PORT: String(ports.relayPort),
      ARROBA_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
      ARROBA_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
    },
    daemonEnv: {
      ...process.env,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_RELAY_URL: relayUrl,
      ARROBA_RELAY_TOKEN: daemonRelayToken,
      ARROBA_DAEMON_ID: daemonId,
      ARROBA_DAEMON_ALIAS: daemonAlias,
      ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
      ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'session-history'),
    },
  }
}

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function buildKernelIfNeeded() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const exists = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (exists) return binary
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(2_000)])
  }
}

function unwrap(resp, ...keys) {
  for (const key of keys) {
    if (resp?.[key] != null) return resp[key]
  }
  return resp
}

function eventCounts(events) {
  return events.reduce((counts, event) => {
    const key = event.event ?? 'unknown'
    counts[key] = (counts[key] ?? 0) + 1
    return counts
  }, {})
}

function clientFor(LocalIpcClient, relayUrl, daemonAlias, userId) {
  return new LocalIpcClient(relayUrl, {
    relayAuthToken: clientToken(userId),
    targetDaemonAlias: daemonAlias,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
}

async function waitForRelayTarget(LocalIpcClient, requests, relayUrl, daemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const client = clientFor(LocalIpcClient, relayUrl, daemonAlias, 'probe-user')
    try {
      await Promise.race([
        client.send(requests.listSessionsRequest()),
        sleep(2_000).then(() => { throw new Error('probe timeout') }),
      ])
      await client.close()
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target did not become reachable: ${lastError?.message ?? lastError}`)
}

async function expectReject(promise, label, expectedText) {
  try {
    await promise
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    if (expectedText && !message.includes(expectedText)) {
      throw new Error(`${label} rejected with unexpected error: ${message}`)
    }
    return message
  }
  throw new Error(`${label} unexpectedly succeeded`)
}

async function subscribeForCompletions(client, sessionId, attachmentId) {
  const events = []
  if (typeof client.onKernelEvent === 'function') {
    client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
  }
  if (typeof client.subscribeToKernelEvents === 'function') {
    await client.subscribeToKernelEvents(sessionId, attachmentId)
  }
  return events
}

async function waitForCompletion(client, sessionId, attachmentId, events, previousCount, timeoutMs, pollMs, label) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const completions = events.filter((event) => event.event === 'assistant_message_completed')
    if (completions.length > previousCount) return completions.length
    await sleep(pollMs)
  }
  throw new Error(`${label} did not complete after ${timeoutMs}ms`)
}

async function waitForHistoryMarker(rootDir, sessionId, marker, timeoutMs, pollMs, label) {
  const historyDir = path.join(rootDir, 'session-history')
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const entries = await readdir(historyDir).catch(() => [])
    const sessionHistory = entries.filter((entry) => entry.startsWith(`${sessionId}-`) && entry.endsWith('.jsonl'))
    for (const entry of sessionHistory) {
      const text = await readFile(path.join(historyDir, entry), 'utf8').catch(() => '')
      if (text.includes(marker)) return true
    }
    await sleep(pollMs)
  }
  throw new Error(`${label} did not write expected marker ${marker} after ${timeoutMs}ms`)
}

async function main() {
  const options = parseArgs()
  const rootDir = path.join(repoRoot, '.artifacts', 'live-relay-freeform-multi-user-drill', nowStamp())
  const workspace = path.join(rootDir, 'workspace')
  await prepareDrillArtifacts(rootDir)
  await mkdir(workspace, { recursive: true })

  const ports = makePorts()
  const envs = makeEnv(ports, rootDir)
  let requests = null
  let relay = null
  let relayTunnel = null
  let daemon = null
  const clients = []
  let sessionId = null
  let passed = false
  let failure = null
  let providerModel = null
  let agent1 = null
  let agent2 = null
  let user1CompletionCount = 0
  let user2CompletionCount = 0
  let user3CompletionCount = 0
  let state1 = null
  let state2 = null
  let state3 = null
  let state4 = null
  let events1 = []
  let events2 = []
  let events3 = []
  try {
    const [{ LocalIpcClient }, ipcRequests] = await Promise.all([
      import('../../../packages/kernel-client/dist/ipc.js'),
      import('../../../packages/kernel-client/dist/ipc-requests.js'),
    ])
    requests = ipcRequests
    const kernelPath = await buildKernelIfNeeded()
    if (options.hetznerRelay) {
      const remoteRelayCheck = await run('ssh', sshArgs(options, [
        `test -x ${shellQuote(path.posix.join(options.hetznerRepo, 'apps/relay/target/debug/arroba-relay'))}`,
      ].join('; ')))
      if (remoteRelayCheck.code !== 0) {
        throw new Error(`Hetzner relay binary is not available in ${options.hetznerRepo}\n${remoteRelayCheck.stdout}\n${remoteRelayCheck.stderr}`)
      }
      relay = spawn('ssh', sshArgs(options, remoteEnvCommand({
        ARROBA_REMOTE_REPO: options.hetznerRepo,
        ARROBA_RELAY_HOST: '127.0.0.1',
        ARROBA_RELAY_PORT: String(ports.relayPort),
        ARROBA_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
        ARROBA_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
      }, './apps/relay/target/debug/arroba-relay')), {
        stdio: ['ignore', 'ignore', 'inherit'],
      })
      relayTunnel = spawn('ssh', [
        '-i',
        options.hetznerKey,
        '-o',
        'BatchMode=yes',
        '-o',
        'StrictHostKeyChecking=accept-new',
        '-N',
        '-L',
        `127.0.0.1:${ports.relayPort}:127.0.0.1:${ports.relayPort}`,
        options.hetznerHost,
      ], {
        stdio: ['ignore', 'ignore', 'inherit'],
      })
    } else {
      relay = spawn('cargo', ['run', '--manifest-path', path.join(repoRoot, 'apps/relay/Cargo.toml'), '--bin', 'arroba-relay'], {
        cwd: repoRoot,
        env: envs.relayEnv,
        stdio: ['ignore', 'ignore', 'inherit'],
      })
    }
    daemon = spawn(kernelPath, [], { cwd: repoRoot, env: envs.daemonEnv, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForRelayTarget(LocalIpcClient, requests, envs.relayUrl, envs.daemonAlias)
    log('relay-target-ready', { relayUrl: envs.relayUrl, daemonAlias: envs.daemonAlias })

    const user1 = clientFor(LocalIpcClient, envs.relayUrl, envs.daemonAlias, 'user-1')
    const user2 = clientFor(LocalIpcClient, envs.relayUrl, envs.daemonAlias, 'user-2')
    const user3 = clientFor(LocalIpcClient, envs.relayUrl, envs.daemonAlias, 'user-3')
    const user4 = clientFor(LocalIpcClient, envs.relayUrl, envs.daemonAlias, 'user-4')
    clients.push(user1, user2, user3, user4)

    const created = unwrap(
      await user1.send(requests.createSessionRequest(workspace, workspace)),
      'SessionCreated',
    )
    const session = created.session
    sessionId = session.id
    assert(session.owner_user_id === 'user-1', 'relay-created freeform session should be owned by user-1', session)
    const privateInvite = unwrap(
      await user1.send(requests.createSessionInviteRequest(session.id, null, 1, 'private')),
      'SessionInviteCreated',
    )
    const fullInvite = unwrap(
      await user1.send(requests.createSessionInviteRequest(session.id, null, 1, 'full')),
      'SessionInviteCreated',
    )
    const transparentInvite = unwrap(
      await user1.send(requests.createSessionInviteRequest(session.id, null, 1, 'transparent')),
      'SessionInviteCreated',
    )
    await user2.send(requests.joinSessionInviteRequest(privateInvite.invite.invite_token, 'user-2'))
    await user3.send(requests.joinSessionInviteRequest(fullInvite.invite.invite_token, 'user-3'))
    await user4.send(requests.joinSessionInviteRequest(transparentInvite.invite.invite_token, 'user-4'))

    const attachment1 = unwrap(
      await user1.send(requests.attachToSessionRequest(session.id, `freeform-user-1-${process.pid}`)),
      'SessionAttached',
    ).attachment
    const attachment2 = unwrap(
      await user2.send(requests.attachToSessionRequest(session.id, `freeform-user-2-${process.pid}`)),
      'SessionAttached',
    ).attachment
    const attachment3 = unwrap(
      await user3.send(requests.attachToSessionRequest(session.id, `freeform-user-3-${process.pid}`)),
      'SessionAttached',
    ).attachment
    const attachment4 = unwrap(
      await user4.send(requests.attachToSessionRequest(session.id, `freeform-user-4-${process.pid}`)),
      'SessionAttached',
    ).attachment

    providerModel = modelForProvider(options.provider, options.model)
    events1 = await subscribeForCompletions(user1, session.id, attachment1.id)
    events2 = await subscribeForCompletions(user2, session.id, attachment2.id)
    events3 = await subscribeForCompletions(user3, session.id, attachment3.id)

    agent1 = unwrap(
      await user1.send(requests.spawnAgentRequest(session.id, options.provider, 'freeform-user-one', providerModel, workspace, 'low')),
      'AgentSpawned',
    ).agent
    agent2 = unwrap(
      await user2.send(requests.spawnAgentRequest(session.id, options.provider, 'freeform-user-two', providerModel, workspace, 'low')),
      'AgentSpawned',
    ).agent
    assert(agent1.owner_user_id === 'user-1', 'user-1 freeform agent owner mismatch', agent1)
    assert(agent2.owner_user_id === 'user-2', 'user-2 freeform agent owner mismatch', agent2)

    const user1Agents = unwrap(await user1.send(requests.listAgentsRequest(session.id)), 'AgentsListed').agents
    const user2Agents = unwrap(await user2.send(requests.listAgentsRequest(session.id)), 'AgentsListed').agents
    const user3Agents = unwrap(await user3.send(requests.listAgentsRequest(session.id)), 'AgentsListed').agents
    const user4Agents = unwrap(await user4.send(requests.listAgentsRequest(session.id)), 'AgentsListed').agents
    assert(
      user1Agents.length >= 1
        && user1Agents.some((agent) => agent.id === agent1.id),
      'user-1 should include its spawned drill agent',
      user1Agents,
    )
    assert(user2Agents.some((agent) => agent.id === agent2.id), 'user-2 should include its own freeform agent', user2Agents)
    assert(user2Agents.some((agent) => agent.id === agent1.id && agent.provider === 'redacted'), 'private user should see redacted owner handle', user2Agents)
    assert(user3Agents.some((agent) => agent.id === agent1.id && agent.provider !== 'redacted'), 'full collaborator should see owner agent details', user3Agents)
    assert(
      user4Agents.some((agent) => agent.id === agent1.id && agent.provider === 'redacted' && agent.visible_in_freeform !== false),
      'transparent user should see other-user panes/traces with redacted parameters',
      user4Agents,
    )

    const user1Prompt = unwrap(
      await user1.send(requests.submitPromptRequest(session.id, attachment1.id, agent1.id, 'Reply with exactly USER1_FREEFORM_OK and nothing else.', [])),
      'PromptSubmitted',
    )
    assert(user1Prompt.outcome?.Started?.prompt?.target_agent_id === agent1.id, 'user-1 prompt should start for own agent', user1Prompt)
    user1CompletionCount = await waitForCompletion(user1, session.id, attachment1.id, events1, 0, options.timeoutMs, options.pollMs, 'user-1 owned prompt')
    await waitForHistoryMarker(rootDir, session.id, 'USER1_FREEFORM_OK', options.timeoutMs, options.pollMs, 'user-1 owned prompt')
    await expectReject(
      user2.send(requests.submitPromptRequest(session.id, attachment2.id, agent1.id, 'Cross-user freeform prompt should fail.', [])),
      'private user-2 submitting to user-1 freeform agent',
      'owned by `user-1`',
    )
    await expectReject(
      user4.send(requests.submitPromptRequest(session.id, attachment4.id, agent1.id, 'Transparent cross-user freeform prompt should fail.', [])),
      'transparent user-4 submitting to user-1 freeform agent',
      'owned by `user-1`',
    )

    const fullPrompt = unwrap(
      await user3.send(requests.submitPromptRequest(session.id, attachment3.id, agent1.id, 'Reply with exactly FULL_USER3_CAN_PROMPT_OWNER_AGENT and nothing else.', [])),
      'PromptSubmitted',
    )
    assert(fullPrompt.outcome?.Started?.prompt?.target_agent_id === agent1.id, 'full collaborator should start prompt for owner agent', fullPrompt)
    user3CompletionCount = await waitForCompletion(user3, session.id, attachment3.id, events3, 0, 30_000, options.pollMs, 'full collaborator cross-owner prompt')
      .catch(() => events3.filter((event) => event.event === 'assistant_message_completed').length)
    await waitForHistoryMarker(rootDir, session.id, 'FULL_USER3_CAN_PROMPT_OWNER_AGENT', options.timeoutMs, options.pollMs, 'full collaborator cross-owner prompt')
    user1CompletionCount = Math.max(user1CompletionCount, events1.filter((event) => event.event === 'assistant_message_completed').length)

    const user2Prompt = unwrap(
      await user2.send(requests.submitPromptRequest(session.id, attachment2.id, agent2.id, 'Reply with exactly USER2_PRIVATE_OWN_AGENT_OK and nothing else.', [])),
      'PromptSubmitted',
    )
    assert(user2Prompt.outcome?.Started?.prompt?.target_agent_id === agent2.id, 'user-2 prompt should start for own agent', user2Prompt)
    user2CompletionCount = await waitForCompletion(user2, session.id, attachment2.id, events2, 0, options.timeoutMs, options.pollMs, 'private user own prompt')
    await waitForHistoryMarker(rootDir, session.id, 'USER2_PRIVATE_OWN_AGENT_OK', options.timeoutMs, options.pollMs, 'private user own prompt')

    state1 = unwrap(await user1.send(requests.getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState').session
    state2 = unwrap(await user2.send(requests.getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState').session
    state3 = unwrap(await user3.send(requests.getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState').session
    state4 = unwrap(await user4.send(requests.getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState').session
    assert(state1.agents.some((agent) => agent.id === agent1.id), 'user-1 projection should retain own agent', state1.agents)
    assert(state2.agents.some((agent) => agent.id === agent1.id && agent.provider === 'redacted'), 'private user projection should redact user-1 agent details', state2.agents)
    assert(state3.agents.some((agent) => agent.id === agent1.id && agent.provider !== 'redacted'), 'full user projection should expose user-1 agent details', state3.agents)
    assert(
      state4.agents.some((agent) => agent.id === agent1.id && agent.provider === 'redacted' && agent.visible_in_freeform !== false),
      'transparent user projection should show other-user panes with redacted parameters',
      state4.agents,
    )

    for (const [label, state, ownedCount, otherCount] of [
      ['user-1', state1, 2, 1],
      ['user-2', state2, 1, 2],
      ['user-3', state3, 0, 3],
      ['user-4', state4, 0, 3],
    ]) {
      assert(state.collaboration_agent_counts?.owned_agent_count === ownedCount, `${label} owned agent count mismatch`, state.collaboration_agent_counts)
      assert(state.collaboration_agent_counts?.other_user_agent_count === otherCount, `${label} collaborator agent count mismatch`, state.collaboration_agent_counts)
      assert(state.collaboration_agent_counts?.total_agent_count === 3, `${label} total agent count mismatch`, state.collaboration_agent_counts)
      assert(state.collaboration_agent_counts?.collaborator_count === 3, `${label} collaborator count mismatch`, state.collaboration_agent_counts)
    }

    console.log(JSON.stringify({
      status: 'ok',
      mode: options.hetznerRelay ? 'hetzner-relay-freeform-multi-user' : 'relay-freeform-multi-user',
      relayUrl: envs.relayUrl,
      daemonAlias: envs.daemonAlias,
      sessionId: session.id,
      provider: options.provider,
      model: providerModel,
      agents: [
        { id: agent1.id, ownerUserId: agent1.owner_user_id },
        { id: agent2.id, ownerUserId: agent2.owner_user_id },
      ],
      completionCounts: {
        user1: user1CompletionCount,
        user2: user2CompletionCount,
        user3: user3CompletionCount,
      },
      assertions: [
        'four users share one scoped-relay freeform session',
        'private invite sees redacted collaborator agent handles',
        'transparent invite sees other-user panes/traces with parameters redacted',
        'full invite sees collaborator agent details',
        'collaboration agent counts report aggregate other-user agents without identities',
        'actual-model freeform prompt submit succeeds for owned agent',
        'private freeform prompt submit rejects another user agent',
        'transparent freeform prompt submit rejects another user agent',
        'full freeform prompt submit can prompt another user agent',
        'session state projection redacts other-user agent details for private invitees',
      ],
    }, null, 2))
    passed = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (sessionId && clients[0] && requests) await clients[0].send(requests.endSessionRequest(sessionId)).catch(() => {})
    await Promise.all(clients.map((client) => client.close().catch(() => {})))
    await terminateChild(daemon)
    await terminateChild(relay)
    await terminateChild(relayTunnel)
    await finalizeDrillArtifacts({
      rootDir,
      passed,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: 'live-relay-freeform-multi-user',
        mode: options.hetznerRelay ? 'hetzner-relay-freeform-multi-user' : 'relay-freeform-multi-user',
        relayUrl: envs.relayUrl,
        daemonAlias: envs.daemonAlias,
        sessionId,
        provider: options.provider,
        model: providerModel ?? modelForProvider(options.provider, options.model),
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        agents: [
          agent1 ? { id: agent1.id, ownerUserId: agent1.owner_user_id } : null,
          agent2 ? { id: agent2.id, ownerUserId: agent2.owner_user_id } : null,
        ].filter(Boolean),
        completionCounts: {
          user1: user1CompletionCount,
          user2: user2CompletionCount,
          user3: user3CompletionCount,
        },
        eventCounts: {
          user1: eventCounts(events1),
          user2: eventCounts(events2),
          user3: eventCounts(events3),
        },
        collaborationAgentCounts: {
          user1: state1?.collaboration_agent_counts ?? null,
          user2: state2?.collaboration_agent_counts ?? null,
          user3: state3?.collaboration_agent_counts ?? null,
          user4: state4?.collaboration_agent_counts ?? null,
        },
      },
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
