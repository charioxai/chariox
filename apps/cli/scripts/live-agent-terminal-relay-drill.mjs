#!/usr/bin/env node
import { createHmac } from 'node:crypto'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const root = path.join(os.tmpdir(), `chariox-agent-terminal-relay-${process.pid}-${Date.now()}`)
const workspace = path.join(root, 'workspace')
const relayPort = 50000 + Math.floor(Math.random() * 500)
const kernelPort = relayPort + 1000
const relayUrl = `ws://127.0.0.1:${relayPort}`
const daemonId = `agent-terminal-relay-${process.pid}-${Date.now()}`
const daemonAlias = `agent-terminal-relay-${process.pid}`
const issuer = 'chariox-agent-terminal-relay-drill'
const secret = 'chariox-agent-terminal-relay-drill-secret'
const realm = 'agent-terminal-relay-drill'

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const base64url = (value) => Buffer.from(value).toString('base64url')
function token({ subject, kind, actions, userId = null }) {
  const claims = {
    issuer, subject, subject_kind: kind, realm_id: realm,
    allowed_actions: actions, allowed_targets: null,
    issued_at_ms: Date.now(), expires_at_ms: Date.now() + 10 * 60_000,
    token_id: `${subject}-${Date.now()}`, account_id: realm,
    organization_id: null, user_id: userId, device_id: subject,
    machine_id: kind === 'kernel' ? subject : null,
    client_id: kind === 'client' ? subject : null,
    public_key_thumbprint: `${subject}-thumbprint`, entitlements_version: 'drill',
  }
  const payload = base64url(JSON.stringify(claims))
  const signature = createHmac('sha256', secret).update(payload).digest('base64url')
  return `chariox-scoped-v1.${payload}.${signature}`
}

function start(command, args, env, stdio = ['ignore', 'ignore', 'pipe']) {
  return spawn(command, args, { cwd: repoRoot, env, stdio })
}

async function waitFor(fn, label, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let lastError
  while (Date.now() < deadline) {
    try { return await fn() } catch (error) { lastError = error; await sleep(250) }
  }
  throw new Error(`${label}: ${lastError?.message ?? lastError ?? 'timed out'}`)
}

function sendJsonLine(child, request) {
  return new Promise((resolve, reject) => {
    let buffer = ''
    const onData = (chunk) => {
      buffer += chunk.toString()
      const newline = buffer.indexOf('\n')
      if (newline < 0) return
      child.stdout.off('data', onData)
      try { resolve(JSON.parse(buffer.slice(0, newline))) } catch (error) { reject(error) }
    }
    child.stdout.on('data', onData)
    child.stdin.write(`${JSON.stringify(request)}\n`)
  })
}

function assert(condition, message, details) {
  if (!condition) throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
}

async function main() {
  await mkdir(workspace, { recursive: true })
  const clientToken = token({
    subject: `agent-terminal-client-${process.pid}`,
    kind: 'client',
    actions: ['client_connect', 'client_metadata_read', 'packet_route'],
    userId: 'agent-terminal-drill-user',
  })
  const daemonToken = token({
    subject: daemonId,
    kind: 'kernel',
    actions: ['daemon_register', 'daemon_heartbeat', 'peer_request', 'peer_event'],
    userId: 'agent-terminal-drill-user',
  })
  const baseEnv = {
    ...process.env,
    HOME: path.join(root, 'home'),
    CHARIOX_SESSION_HISTORY_DIR: path.join(root, 'history'),
    CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(kernelPort + 1),
    CHARIOX_OPENCODE_PORT: String(kernelPort + 2),
    CHARIOX_CODEX_PORT: String(kernelPort + 3),
    CHARIOX_DAEMON_ID: daemonId,
    CHARIOX_DAEMON_ALIAS: daemonAlias,
    CHARIOX_RELAY_URL: relayUrl,
    CHARIOX_RELAY_TOKEN: daemonToken,
    CHARIOX_TEST_TUI: '1',
  }
  const relayEnv = {
    ...process.env,
    CHARIOX_RELAY_HOST: '127.0.0.1',
    CHARIOX_RELAY_PORT: String(relayPort),
    CHARIOX_RELAY_SCOPED_ISSUER: issuer,
    CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret,
  }
  let relay
  let kernel
  let peer
  let shell
  let observer
  try {
    const [{ LocalIpcClient }, requests] = await Promise.all([
      import('../../../packages/kernel-client/dist/ipc.js'),
      import('../../../packages/kernel-client/dist/ipc-requests.js'),
    ])
    relay = start(path.join(repoRoot, 'target', 'debug', 'chariox-relay'), [], relayEnv)
    await waitFor(() => new Promise((resolve, reject) => {
      const socket = new net.Socket()
      socket.once('connect', () => { socket.destroy(); resolve(true) })
      socket.once('error', (error) => { socket.destroy(); reject(error) })
      socket.connect(relayPort, '127.0.0.1')
    }), 'relay did not become ready')
    kernel = start(path.join(repoRoot, 'target', 'debug', 'chariox-kernel'), [], baseEnv)
    await waitFor(async () => {
      const probe = new LocalIpcClient(relayUrl, { relayAuthToken: clientToken, targetDaemonAlias: daemonAlias })
      try { await probe.send(requests.listSessionsRequest()) } finally { await probe.close().catch(() => {}) }
    }, 'relay target did not become reachable')
    peer = start(process.execPath, [path.join(repoRoot, 'apps/shell/dist/agent-terminal-main.js')], {
      ...baseEnv,
      CHARIOX_KERNEL_URL: relayUrl,
      CHARIOX_RELAY_AUTH_TOKEN: clientToken,
      CHARIOX_RELAY_TARGET_DAEMON_ALIAS: daemonAlias,
    }, ['pipe', 'pipe', 'pipe'])
    const initialized = await sendJsonLine(peer, { jsonrpc: '2.0', id: 1, method: 'initialize' })
    assert(initialized.result?.serverInfo?.name === 'chariox-agent-terminal', 'relay MCP initialize failed', initialized)
    const listed = await sendJsonLine(peer, { jsonrpc: '2.0', id: 2, method: 'tools/list' })
    assert(listed.result?.tools?.length === 5, 'relay MCP tool surface drifted', listed)
    const context = { workspace, worktree: workspace }
    const created = await sendJsonLine(peer, { jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name: 'chariox_execute', arguments: { command: 'session new --dir . as relay_shared', context } } })
    const createdPayload = JSON.parse(created.result.content[0].text)
    assert(createdPayload.ok && createdPayload.context?.session_id, 'relay agent session creation failed', createdPayload)
    observer = new LocalIpcClient(relayUrl, { relayAuthToken: clientToken, targetDaemonAlias: daemonAlias })
    const state = await observer.send(requests.getSessionStateRequest(createdPayload.context.session_id))
    assert(state.SessionState?.session?.id === createdPayload.context.session_id, 'relay observer did not see agent-created session', state)
    const workflow = await sendJsonLine(peer, { jsonrpc: '2.0', id: 4, method: 'tools/call', params: { name: 'chariox_execute', arguments: { command: 'workflow new relay_agent_flow as workflow', context: createdPayload.context } } })
    const workflowPayload = JSON.parse(workflow.result.content[0].text)
    assert(workflowPayload.ok, 'relay agent workflow creation failed', workflowPayload)
    const observed = await observer.send(requests.getSessionStateRequest(createdPayload.context.session_id))
    assert(/relay_agent_flow/i.test(JSON.stringify(observed)), 'relay observer did not see agent workflow', observed)
    const shellScript = path.join(root, 'relay-shell.chariox')
    await writeFile(shellScript, `session use ${createdPayload.context.session_id}\nworkflow new relay_shell_flow as workflow\n`, 'utf8')
    shell = start(process.execPath, [path.join(repoRoot, 'apps/shell/dist/shell.js'), 'run', shellScript, '--kernel-url', relayUrl, '--workspace', workspace, '--worktree', workspace], {
      ...baseEnv,
      CHARIOX_RELAY_AUTH_TOKEN: clientToken,
      CHARIOX_RELAY_TARGET_DAEMON_ALIAS: daemonAlias,
    }, ['pipe', 'pipe', 'pipe'])
    let shellOutput = ''
    let shellError = ''
    shell.stdout.on('data', (chunk) => { shellOutput += chunk.toString() })
    shell.stderr.on('data', (chunk) => { shellError += chunk.toString() })
    shell.stdin.end()
    const shellExit = await new Promise((resolve) => shell.once('exit', (code) => resolve(code)))
    assert(shellExit === 0, 'relay shell mutation failed', { shellOutput, shellError })
    const listedWorkflows = await sendJsonLine(peer, { jsonrpc: '2.0', id: 5, method: 'tools/call', params: { name: 'chariox_execute', arguments: { command: 'workflow list', context: createdPayload.context } } })
    const listedWorkflowsPayload = JSON.parse(listedWorkflows.result.content[0].text)
    assert(listedWorkflowsPayload.ok && /relay_shell_flow/i.test(listedWorkflowsPayload.output), 'agent terminal did not observe relay shell mutation', listedWorkflowsPayload)
    const previousRelay = relay
    previousRelay.kill('SIGTERM')
    await new Promise((resolve) => previousRelay.once('exit', resolve))
    relay = start(path.join(repoRoot, 'target', 'debug', 'chariox-relay'), [], relayEnv)
    await waitFor(() => new Promise((resolve, reject) => {
      const socket = new net.Socket()
      socket.once('connect', () => { socket.destroy(); resolve(true) })
      socket.once('error', (error) => { socket.destroy(); reject(error) })
      socket.connect(relayPort, '127.0.0.1')
    }), 'relay did not recover after restart')
    await waitFor(async () => {
      const probe = new LocalIpcClient(relayUrl, { relayAuthToken: clientToken, targetDaemonAlias: daemonAlias })
      try { await probe.send(requests.listSessionsRequest()) } finally { await probe.close().catch(() => {}) }
    }, 'kernel target did not re-register after relay restart')
    const restarted = await sendJsonLine(peer, { jsonrpc: '2.0', id: 6, method: 'tools/call', params: { name: 'chariox_execute', arguments: { command: 'workflow new relay_after_restart as workflow', context: createdPayload.context } } })
    const restartedPayload = JSON.parse(restarted.result.content[0].text)
    assert(restartedPayload.ok, 'agent terminal did not recover its relay control path after restart', restartedPayload)
    const observedAfterRestart = await observer.send(requests.getSessionStateRequest(createdPayload.context.session_id))
    assert(/relay_after_restart/i.test(JSON.stringify(observedAfterRestart)), 'relay observer did not recover after relay restart', observedAfterRestart)
    console.log(JSON.stringify({ ok: true, relay: true, relay_restart: true, tools: listed.result.tools.map((tool) => tool.name), session_id: createdPayload.context.session_id }))
  } finally {
    await observer?.close().catch(() => {})
    peer?.stdin?.end()
    peer?.kill('SIGTERM')
    shell?.kill('SIGTERM')
    kernel?.kill('SIGTERM')
    relay?.kill('SIGTERM')
    await rm(root, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exitCode = 1
})
