#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { createHmac } from 'node:crypto'
import net from 'node:net'
import { access, mkdir, rm, stat } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const RELAY_ISSUER = 'arroba-multi-user-cli-workflow-drill'
const RELAY_SECRET = 'arroba-multi-user-cli-workflow-drill-secret'
const RELAY_REALM = 'multi-user-cli-workflow-drill'

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function log(name, details = null) {
  if (details == null) console.log(`[multi-user-cli-workflow-drill] ${name}`)
  else console.log(`[multi-user-cli-workflow-drill] ${name}`, JSON.stringify(details))
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
    account_id: 'multi-user-cli-workflow-drill-account',
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
    subject: `cli-${userId}-${process.pid}-${Date.now()}`,
    subjectKind: 'client',
    actions: ['client_connect', 'client_metadata_read', 'packet_route'],
    userId,
  }))
}

function makePorts() {
  const base = 49000 + Math.floor(Math.random() * 1000)
  return {
    relayPort: base,
    kernelPort: base + 1000,
    mcpPort: base + 2000,
    opencodePort: base + 3000,
    codexPort: base + 3001,
  }
}

function makeEnv(ports, rootDir) {
  const daemonId = `multi-user-cli-workflow-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `multi-user-cli-workflow-${process.pid}`
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
      ARROBA_TEST_TUI: '1',
    },
  }
}

async function buildKernelIfNeeded() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const exists = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (exists) return binary
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

async function requireBuiltCli() {
  const cli = path.join(cliRoot, 'dist/index.js')
  await access(cli).catch(() => {
    throw new Error(`missing built CLI ${cli}; run pnpm --filter @arroba/cli run build first`)
  })
  return cli
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

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(2_000)])
  }
}

async function waitForSocket(socketPath) {
  const deadline = Date.now() + 25_000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const socket = net.createConnection(socketPath)
      await new Promise((resolve, reject) => {
        socket.once('connect', resolve)
        socket.once('error', reject)
      })
      socket.destroy()
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`automation socket did not become ready: ${lastError?.message ?? lastError}`)
}

function automationClient(socketPath) {
  const socket = net.createConnection(socketPath)
  socket.setEncoding('utf8')
  let nextId = 1
  let buffer = ''
  const pending = new Map()
  socket.on('data', (chunk) => {
    buffer += chunk
    while (buffer.includes('\n')) {
      const index = buffer.indexOf('\n')
      const line = buffer.slice(0, index).trim()
      buffer = buffer.slice(index + 1)
      if (!line) continue
      const response = JSON.parse(line)
      const deferred = pending.get(response.id)
      if (!deferred) continue
      pending.delete(response.id)
      if (response.ok) deferred.resolve(response.data)
      else deferred.reject(new Error(response.error ?? 'automation command failed'))
    }
  })
  socket.on('error', (error) => {
    for (const deferred of pending.values()) deferred.reject(error)
    pending.clear()
  })
  return {
    send(action, fields = {}) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(`${JSON.stringify({ id, action, ...fields })}\n`)
      })
    },
    close() {
      socket.destroy()
    },
  }
}

async function waitForRelayTarget(LocalIpcClient, requests, relayUrl, daemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: clientToken('probe-user'),
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
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

function unwrap(resp, ...keys) {
  for (const key of keys) {
    if (resp?.[key] != null) return resp[key]
  }
  return resp
}

function startCli({ cliPath, env, relayUrl, relayToken, daemonAlias, socketPath, workspace, clientId, createSession, sessionId, launchProvider = true }) {
  const args = [
    '-q',
    '/dev/null',
    'env',
    ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
    'bun',
    cliPath,
    '--relay-url', relayUrl,
    '--relay-token', relayToken,
    '--target-daemon-alias', daemonAlias,
    '--automation-socket', socketPath,
    '--workspace', workspace,
    '--worktree', workspace,
    '--client-id', clientId,
  ]
  if (launchProvider) {
    args.push('--provider', 'dev-stub', '--model', 'cli-workflow-drill-model', '--effort', 'low')
  }
  if (createSession) args.push('--create-session')
  if (sessionId) args.push('--session', sessionId)
  const child = spawn('script', args, { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
  let stdout = ''
  let stderr = ''
  child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
  return { child, stdout: () => stdout, stderr: () => stderr }
}

async function waitForWorkflowGraph(automation, workflowId, alias, expectedNodes, expectedEdges, label) {
  const deadline = Date.now() + 10_000
  let lastSnapshot = null
  while (Date.now() < deadline) {
    const snapshot = await automation.send('snapshot')
    lastSnapshot = snapshot
    const workflow = snapshot.workflows?.find((entry) => entry.id === workflowId || entry.alias === alias)
    if (workflow?.nodeCount === expectedNodes && workflow?.edgeCount === expectedEdges) {
      return { snapshot, workflow }
    }
    await sleep(100)
  }
  assert(false, `${label} did not observe workflow graph ${expectedNodes} nodes/${expectedEdges} edges`, lastSnapshot)
}

async function main() {
  const rootDir = path.join(os.tmpdir(), `arroba-multi-user-cli-workflow-${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home1 = path.join(rootDir, 'home-user-1')
  const home2 = path.join(rootDir, 'home-user-2')
  const socket1 = path.join(os.tmpdir(), `arroba-cli-user1-${process.pid}-${Date.now()}.sock`)
  const socket2 = path.join(os.tmpdir(), `arroba-cli-user2-${process.pid}-${Date.now()}.sock`)
  await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(workspace, { recursive: true })
  await mkdir(home1, { recursive: true })
  await mkdir(home2, { recursive: true })

  const [{ LocalIpcClient }, requests] = await Promise.all([
    import('../../../packages/kernel-client/dist/ipc.js'),
    import('../../../packages/kernel-client/dist/ipc-requests.js'),
  ])
  const cliPath = await requireBuiltCli()
  const kernelPath = await buildKernelIfNeeded()
  const ports = makePorts()
  const envs = makeEnv(ports, rootDir)
  let relay = null
  let daemon = null
  let cli1 = null
  let cli2 = null
  let auto1 = null
  let auto2 = null
  let apiClient = null
  let joinClient = null
  let sessionId = null
  try {
    relay = spawn('cargo', ['run', '--manifest-path', path.join(repoRoot, 'apps/relay/Cargo.toml'), '--bin', 'arroba-relay'], {
      cwd: repoRoot,
      env: envs.relayEnv,
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    daemon = spawn(kernelPath, [], { cwd: repoRoot, env: envs.daemonEnv, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForRelayTarget(LocalIpcClient, requests, envs.relayUrl, envs.daemonAlias)
    log('relay-target-ready', { relayUrl: envs.relayUrl, daemonAlias: envs.daemonAlias })

    cli1 = startCli({
      cliPath,
      env: { ...envs.daemonEnv, HOME: home1 },
      relayUrl: envs.relayUrl,
      relayToken: clientToken('user-1'),
      daemonAlias: envs.daemonAlias,
      socketPath: socket1,
      workspace,
      clientId: `cli-user-1-${process.pid}`,
      createSession: true,
    })
    await waitForSocket(socket1).catch((error) => {
      throw new Error(`${error.message}\n--- cli1 stdout ---\n${cli1.stdout().slice(-4000)}\n--- cli1 stderr ---\n${cli1.stderr().slice(-4000)}`)
    })
    auto1 = automationClient(socket1)
    await auto1.send('ping')
    await auto1.send('switch_screen', { screen: 'workflow' })
    const firstSnapshot = await auto1.send('snapshot')
    sessionId = firstSnapshot.session?.id
    assert(sessionId, 'user-1 CLI did not create a session', firstSnapshot)

    const invite = await auto1.send('workspace_shell_exec', { command: 'session invite create 1' })
    assert(invite.result?.ok === true, 'user-1 CLI failed to create invite', invite)
    const inviteToken = invite.result?.data?.invite?.invite_token
      ?? invite.result?.message?.trim().split(/\s+/).at(-1)
      ?? invite.result?.output?.trim().split(/\s+/).at(-1)
    assert(inviteToken, 'invite token was not returned by shell command', invite)
    joinClient = new LocalIpcClient(envs.relayUrl, {
      relayAuthToken: clientToken('user-2'),
      targetDaemonAlias: envs.daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await joinClient.send(requests.joinSessionInviteRequest(inviteToken, 'user-2'))
    const user2PreAttachState = unwrap(
      await joinClient.send(requests.getSessionStateRequest(sessionId)),
      'SessionStateLoaded',
      'SessionState',
    ).session
    log('user-2-pre-cli-attach-projection', {
      focusedAgentId: user2PreAttachState.focused_agent_id,
      agentIds: user2PreAttachState.agents?.map((agent) => agent.id) ?? [],
    })

    const agent1 = await auto1.send('workspace_shell_exec', { command: 'agent spawn cli-user-one cli-workflow-drill-model as u1' })
    assert(agent1.result?.ok === true, 'user-1 CLI failed to spawn agent', agent1)
    const workflow = await auto1.send('workspace_shell_exec', { command: 'workflow new two-cli-flow as wf' })
    assert(workflow.result?.ok === true, 'user-1 CLI failed to create workflow', workflow)
    const workflowId = workflow.result?.data?.workflow?.id ?? workflow.snapshot?.selectedWorkflowId
    assert(workflowId, 'workflow id missing after CLI workflow creation', workflow)
    const node1 = await auto1.send('workspace_shell_exec', { command: 'workflow node add $wf $u1 as n1' })
    assert(node1.result?.ok === true, 'user-1 CLI failed to add workflow node', node1)
    const node1Id = node1.result?.data?.node?.id ?? node1.result?.context?.variables?.n1
    assert(node1Id, 'user-1 node id missing', node1)

    cli2 = startCli({
      cliPath,
      env: { ...envs.daemonEnv, HOME: home2 },
      relayUrl: envs.relayUrl,
      relayToken: clientToken('user-2'),
      daemonAlias: envs.daemonAlias,
      socketPath: socket2,
      workspace,
      clientId: `cli-user-2-${process.pid}`,
      sessionId,
      launchProvider: false,
    })
    await waitForSocket(socket2).catch((error) => {
      throw new Error(`${error.message}\n--- cli2 stdout ---\n${cli2.stdout().slice(-4000)}\n--- cli2 stderr ---\n${cli2.stderr().slice(-4000)}`)
    })
    auto2 = automationClient(socket2)
    await auto2.send('ping')
    await auto2.send('switch_screen', { screen: 'workflow' })

    const agent2 = await auto2.send('workspace_shell_exec', { command: 'agent spawn cli-user-two cli-workflow-drill-model as u2' })
    assert(agent2.result?.ok === true, 'user-2 CLI failed to spawn agent', agent2)
    const node2 = await auto2.send('workspace_shell_exec', { command: `workflow node add ${workflowId} $u2 as n2` })
    assert(node2.result?.ok === true, 'user-2 CLI failed to add own workflow node to shared workflow', node2)
    const node2Id = node2.result?.data?.node?.id ?? node2.result?.context?.variables?.n2
    assert(node2Id, 'user-2 node id missing', node2)
    const edge = await auto2.send('workspace_shell_exec', { command: `workflow edge add ${workflowId} ${node1Id} ${node2Id}` })
    assert(edge.result?.ok === true, 'user-2 CLI failed to add cross-owner edge', edge)
    const edgeId = edge.result?.data?.edge?.id ?? edge.result?.output?.match(/added workflow edge\s+(\S+)/)?.[1] ?? null
    assert(edgeId, 'edge id missing after user-2 cross-owner edge add', edge)

    const user1Api = new LocalIpcClient(envs.relayUrl, {
      relayAuthToken: clientToken('user-1'),
      targetDaemonAlias: envs.daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    const user1ApiState = unwrap(await user1Api.send(requests.getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState').session
    await user1Api.close().catch(() => {})
    const user1ApiWorkflow = user1ApiState.workflows?.find((entry) => entry.id === workflowId)
    log('user-1-post-edge-api-projection', {
      nodes: user1ApiWorkflow?.nodes?.length ?? 0,
      edges: user1ApiWorkflow?.edges?.length ?? 0,
    })

    const { snapshot: final1 } = await waitForWorkflowGraph(auto1, workflowId, 'two-cli-flow', 2, 1, 'user-1 CLI')
    const { snapshot: final2 } = await waitForWorkflowGraph(auto2, workflowId, 'two-cli-flow', 2, 1, 'user-2 CLI')

    const removeEdge = await auto1.send('workspace_shell_exec', { command: `workflow edge remove ${workflowId} ${edgeId}` })
    assert(removeEdge.result?.ok === true, 'user-1 CLI failed to remove edge incident to its node', removeEdge)
    await waitForWorkflowGraph(auto1, workflowId, 'two-cli-flow', 2, 0, 'user-1 CLI after edge removal')
    await waitForWorkflowGraph(auto2, workflowId, 'two-cli-flow', 2, 0, 'user-2 CLI after edge removal')

    apiClient = new LocalIpcClient(envs.relayUrl, {
      relayAuthToken: clientToken('user-2'),
      targetDaemonAlias: envs.daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    const state = unwrap(await apiClient.send(requests.getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState').session
    const resolved = state.workflows?.find((entry) => entry.id === workflowId)
    assert(resolved?.nodes?.length === 2 && resolved?.edges?.length === 0, 'API projection did not match CLI-created graph after edge removal', state)

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'multi-user-cli-workflow-relay',
      relayUrl: envs.relayUrl,
      daemonAlias: envs.daemonAlias,
      sessionId,
      workflowId,
      nodeIds: [node1Id, node2Id],
      removedEdgeId: edgeId,
      assertions: [
        'two TUI clients connected through scoped relay',
        'user-1 created session and invite from embedded shell',
        'user-2 joined the invited session over scoped relay',
        'each user added only its own agent as workflow node',
        'cross-owner edge add live-updates both workflow screens without manual refresh',
        'cross-owner edge removal live-updates both workflow screens without manual refresh',
      ],
    }, null, 2))
  } finally {
    await auto1?.send('exit').catch(() => {})
    await auto2?.send('exit').catch(() => {})
    auto1?.close()
    auto2?.close()
    if (apiClient && sessionId) await apiClient.send(requests.endSessionRequest(sessionId)).catch(() => {})
    await apiClient?.close().catch(() => {})
    await joinClient?.close().catch(() => {})
    await terminateChild(cli1?.child)
    await terminateChild(cli2?.child)
    await terminateChild(daemon)
    await terminateChild(relay)
    await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    await rm(socket1, { force: true }).catch(() => {})
    await rm(socket2, { force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
