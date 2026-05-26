#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { createHmac } from 'node:crypto'
import { mkdir, rm, stat } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { remoteEnvCommand, shellQuote, sshArgs } from './lib/native-tui-remote-execution.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const RELAY_ISSUER = 'arroba-relay-freeform-multi-user-drill'
const RELAY_SECRET = 'arroba-relay-freeform-multi-user-drill-secret'
const RELAY_REALM = 'relay-freeform-multi-user-drill'
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function parseArgs(argv = process.argv.slice(2)) {
  const options = {
    hetznerRelay: false,
    hetznerHost: process.env.ARROBA_COLLAB_HETZNER_HOST ?? process.env.ARROBA_NATIVE_TUI_HETZNER_HOST ?? 'root@195.201.123.115',
    hetznerKey: process.env.ARROBA_COLLAB_HETZNER_KEY ?? process.env.ARROBA_NATIVE_TUI_HETZNER_KEY ?? path.join(os.homedir(), '.ssh/arroba_hetzner_staging'),
    hetznerRepo: process.env.ARROBA_COLLAB_HETZNER_REPO ?? process.env.ARROBA_NATIVE_TUI_HETZNER_REPO ?? '/tmp/arroba-native-remote-validate',
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
    } else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-relay-freeform-multi-user-drill.mjs [options]',
        '',
        'Options:',
        '  --hetzner-relay       Run the scoped relay on the configured Hetzner host through an SSH tunnel',
        '  --hetzner-host HOST   SSH host for --hetzner-relay',
        '  --hetzner-key PATH    SSH key for --hetzner-relay',
        '  --hetzner-repo PATH   Remote Arroba checkout containing built relay binary',
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

async function main() {
  const options = parseArgs()
  const rootDir = path.join(os.tmpdir(), `arroba-relay-freeform-multi-user-${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(workspace, { recursive: true })

  const [{ LocalIpcClient }, requests] = await Promise.all([
    import('../../../packages/kernel-client/dist/ipc.js'),
    import('../../../packages/kernel-client/dist/ipc-requests.js'),
  ])
  const kernelPath = await buildKernelIfNeeded()
  const ports = makePorts()
  const envs = makeEnv(ports, rootDir)
  let relay = null
  let relayTunnel = null
  let daemon = null
  const clients = []
  let sessionId = null
  try {
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
    clients.push(user1, user2, user3)

    const created = unwrap(
      await user1.send(requests.createSessionRequest(workspace, workspace)),
      'SessionCreated',
    )
    const session = created.session
    sessionId = session.id
    assert(session.owner_user_id === 'user-1', 'relay-created freeform session should be owned by user-1', session)
    const invite = unwrap(
      await user1.send(requests.createSessionInviteRequest(session.id, null, 2)),
      'SessionInviteCreated',
    )
    await user2.send(requests.joinSessionInviteRequest(invite.invite.invite_token, 'user-2'))
    await user3.send(requests.joinSessionInviteRequest(invite.invite.invite_token, 'user-3'))

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

    const agent1 = unwrap(
      await user1.send(requests.spawnAgentRequest(session.id, 'dev-stub', 'freeform-user-one', 'freeform-drill-model', workspace, 'low')),
      'AgentSpawned',
    ).agent
    const agent2 = unwrap(
      await user2.send(requests.spawnAgentRequest(session.id, 'dev-stub', 'freeform-user-two', 'freeform-drill-model', workspace, 'low')),
      'AgentSpawned',
    ).agent
    const agent3 = unwrap(
      await user3.send(requests.spawnAgentRequest(session.id, 'dev-stub', 'freeform-user-three', 'freeform-drill-model', workspace, 'low')),
      'AgentSpawned',
    ).agent
    assert(agent1.owner_user_id === 'user-1', 'user-1 freeform agent owner mismatch', agent1)
    assert(agent2.owner_user_id === 'user-2', 'user-2 freeform agent owner mismatch', agent2)
    assert(agent3.owner_user_id === 'user-3', 'user-3 freeform agent owner mismatch', agent3)

    const user1Agents = unwrap(await user1.send(requests.listAgentsRequest(session.id)), 'AgentsListed').agents
    const user2Agents = unwrap(await user2.send(requests.listAgentsRequest(session.id)), 'AgentsListed').agents
    const user3Agents = unwrap(await user3.send(requests.listAgentsRequest(session.id)), 'AgentsListed').agents
    assert(
      user1Agents.length >= 1
        && user1Agents.every((agent) => agent.owner_user_id === 'user-1')
        && user1Agents.some((agent) => agent.id === agent1.id),
      'user-1 should list only its own freeform agents and include its spawned drill agent',
      user1Agents,
    )
    assert(user2Agents.length === 1 && user2Agents[0].id === agent2.id, 'user-2 should list only its own freeform agent', user2Agents)
    assert(user3Agents.length === 1 && user3Agents[0].id === agent3.id, 'user-3 should list only its own freeform agent', user3Agents)

    await user1.send(requests.launchProviderRunRequest(session.id, 'dev-stub', 'default', 'freeform-drill-model', 'low', agent1.id))
    await user2.send(requests.launchProviderRunRequest(session.id, 'dev-stub', 'default', 'freeform-drill-model', 'low', agent2.id))
    await user3.send(requests.launchProviderRunRequest(session.id, 'dev-stub', 'default', 'freeform-drill-model', 'low', agent3.id))

    const user1Prompt = unwrap(
      await user1.send(requests.submitPromptRequest(session.id, attachment1.id, agent1.id, 'Freeform user-1 prompt.', [])),
      'PromptSubmitted',
    )
    assert(user1Prompt.outcome?.Started?.prompt?.target_agent_id === agent1.id, 'user-1 prompt should start for own agent', user1Prompt)
    await expectReject(
      user2.send(requests.submitPromptRequest(session.id, attachment2.id, agent1.id, 'Cross-user freeform prompt should fail.', [])),
      'user-2 submitting to user-1 freeform agent',
      'owned by `user-1`',
    )
    await user1.send(requests.completePromptRequest(session.id)).catch(() => {})

    const user2Prompt = unwrap(
      await user2.send(requests.submitPromptRequest(session.id, attachment2.id, agent2.id, 'Freeform user-2 prompt.', [])),
      'PromptSubmitted',
    )
    assert(user2Prompt.outcome?.Started?.prompt?.target_agent_id === agent2.id, 'user-2 prompt should start for own agent', user2Prompt)
    await user2.send(requests.completePromptRequest(session.id)).catch(() => {})

    const user3Prompt = unwrap(
      await user3.send(requests.submitPromptRequest(session.id, attachment3.id, agent3.id, 'Freeform user-3 prompt.', [])),
      'PromptSubmitted',
    )
    assert(user3Prompt.outcome?.Started?.prompt?.target_agent_id === agent3.id, 'user-3 prompt should start for own agent', user3Prompt)
    await user3.send(requests.completePromptRequest(session.id)).catch(() => {})

    const state1 = unwrap(await user1.send(requests.getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState').session
    const state2 = unwrap(await user2.send(requests.getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState').session
    const state3 = unwrap(await user3.send(requests.getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState').session
    assert(
      state1.agents.every((agent) => agent.owner_user_id === 'user-1') && state1.agents.some((agent) => agent.id === agent1.id),
      'user-1 freeform projection should redact user-2 agents and retain user-1 agents',
      state1.agents,
    )
    assert(state2.agents.length === 1 && state2.agents[0].id === agent2.id, 'user-2 freeform projection should redact user-1 agent', state2.agents)
    assert(state3.agents.length === 1 && state3.agents[0].id === agent3.id, 'user-3 freeform projection should redact user-1/user-2 agents', state3.agents)

    for (const [label, state, ownedCount, otherCount] of [
      ['user-1', state1, 2, 2],
      ['user-2', state2, 1, 3],
      ['user-3', state3, 1, 3],
    ]) {
      assert(state.collaboration_agent_counts?.owned_agent_count === ownedCount, `${label} owned agent count mismatch`, state.collaboration_agent_counts)
      assert(state.collaboration_agent_counts?.other_user_agent_count === otherCount, `${label} collaborator agent count mismatch`, state.collaboration_agent_counts)
      assert(state.collaboration_agent_counts?.total_agent_count === 4, `${label} total agent count mismatch`, state.collaboration_agent_counts)
      assert(state.collaboration_agent_counts?.collaborator_count === 2, `${label} collaborator count mismatch`, state.collaboration_agent_counts)
    }

    console.log(JSON.stringify({
      status: 'ok',
      mode: options.hetznerRelay ? 'hetzner-relay-freeform-multi-user' : 'relay-freeform-multi-user',
      relayUrl: envs.relayUrl,
      daemonAlias: envs.daemonAlias,
      sessionId: session.id,
      agents: [
        { id: agent1.id, ownerUserId: agent1.owner_user_id },
        { id: agent2.id, ownerUserId: agent2.owner_user_id },
        { id: agent3.id, ownerUserId: agent3.owner_user_id },
      ],
      assertions: [
        'three users share one scoped-relay freeform session',
        'each user sees only its own agents outside workflow mode',
        'collaboration agent counts report aggregate other-user agents without identities',
        'freeform prompt submit succeeds for owned agent',
        'freeform prompt submit rejects another user agent',
        'session state projection redacts other-user agents',
      ],
    }, null, 2))
  } finally {
    if (sessionId && clients[0]) await clients[0].send(requests.endSessionRequest(sessionId)).catch(() => {})
    await Promise.all(clients.map((client) => client.close().catch(() => {})))
    await terminateChild(daemon)
    await terminateChild(relay)
    await terminateChild(relayTunnel)
    await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
