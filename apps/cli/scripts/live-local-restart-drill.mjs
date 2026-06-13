#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

const {
  createSessionRequest,
  attachToSessionRequest,
  spawnAgentRequest,
  launchProviderRunRequest,
  installMcpServerRequest,
  grantAgentExtensionRequest,
  installSkillRequest,
  createWorkflowRequest,
  addWorkflowNodeRequest,
  createWorkflowEndpointRequest,
  invokeWorkflowEndpointRequest,
  getSessionStateRequest,
  getSessionHistoryOutlineRequest,
  searchRecallRequest,
  listAgentsRequest,
  listSessionsRequest,
  endSessionRequest,
  submitPromptRequest,
  completePromptRequest,
} = requests

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-local-restart-drill.mjs [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 48000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[restart-drill] ${name}`)
  else console.log(`[restart-drill] ${name}`, JSON.stringify(details))
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms))
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

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  }
  return binary
}

function startDaemon(binary, env) {
  const logs = { stdout: '', stderr: '' }
  const child = spawn(binary, [], {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  child.stdout.on('data', (chunk) => { logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { logs.stderr += chunk.toString() })
  child.logs = logs
  return child
}

async function stopDaemon(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  await new Promise((resolve) => {
    const timeout = setTimeout(() => {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
    }, 3_000)
    child.once('exit', () => {
      clearTimeout(timeout)
      resolve()
    })
    child.kill('SIGTERM')
  })
}

async function waitForDaemon(kernelUrl) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? String(lastError)}`)
}

async function waitForState(client, sessionId, predicate, label, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs
  let lastSession = null
  while (Date.now() < deadline) {
    const response = await client.send(getSessionStateRequest(sessionId))
    lastSession = variant(response, 'SessionState').session
    if (predicate(lastSession)) return lastSession
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${label}\nlast session:\n${JSON.stringify(lastSession, null, 2)}`)
}

function variant(response, key) {
  if (!response || !response[key]) {
    throw new Error(`expected ${key}, got ${JSON.stringify(response)}`)
  }
  return response[key]
}

function oneOfVariant(response, keys) {
  for (const key of keys) {
    if (response?.[key]) return response[key]
  }
  throw new Error(`expected one of ${keys.join(', ')}, got ${JSON.stringify(response)}`)
}

function assert(condition, message) {
  if (!condition) throw new Error(message)
}

function historyOutlineText(outline) {
  return (outline.agents ?? [])
    .flatMap((agent) => agent.turns ?? [])
    .flatMap((turn) => [
      turn.user_prompt,
      ...(turn.entries ?? []),
      ...(turn.summary ? [turn.summary] : []),
      ...(turn.blobs ?? []).map((blob) => ({ entry: { text: blob.summary } })),
    ])
    .map((row) => row?.entry?.text ?? '')
    .join('\n')
}

function asArray(value) {
  return Array.isArray(value) ? value : []
}

function hasExtensionGrant(agent, kind, name) {
  return asArray(agent?.extension_grants).some((grant) => grant.kind === kind && grant.name === name)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-local-restart-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const configHome = path.join(rootDir, 'config')
  const stateHome = path.join(rootDir, 'state')
  const skillDir = path.join(rootDir, 'skill-source')
  const mcpPath = path.join(rootDir, 'restart-echo-mcp.mjs')
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const daemonId = `restart-drill-${process.pid}-${Date.now()}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
  }

  let daemon = null
  let client = null
  let sessionId = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await mkdir(skillDir, { recursive: true })
    await writeFile(
      path.join(configHome, 'arroba', 'config.toml'),
      'version = 1\n\n[state]\nsnapshot_interval_events = 1\n',
      'utf8',
    )
    await writeFile(
      path.join(skillDir, 'SKILL.md'),
      '---\nname: m8-restart-skill\ndescription: Skill used by the M8 local restart drill.\n---\nM8 restart skill body.\n',
      'utf8',
    )
    await writeFile(mcpPath, "process.stdin.resume()\nprocess.stdin.on('data', () => {})\n", 'utf8')

    const kernelBinary = await buildKernel()
    daemon = startDaemon(kernelBinary, env)
    await waitForDaemon(kernelUrl)
    log('daemon-ready', { kernelUrl })

    client = new LocalIpcClient(kernelUrl)
    const created = variant(await client.send(createSessionRequest(workspace, workspace, 'm8-restart-session')), 'SessionCreated')
    const session = created.session
    sessionId = session.id
    const attached = variant(await client.send(attachToSessionRequest(sessionId, `restart-drill-client-${process.pid}`)), 'SessionAttached')
    const attachment = attached.attachment
    const spawned = variant(
      await client.send(spawnAgentRequest(sessionId, 'dev-stub', 'restart-worker', 'm8-restart-profile', workspace, 'low')),
      'AgentSpawned',
    )
    const agent = spawned.agent
    log('session-created', { sessionId, agentId: agent.id })

    await client.send(installMcpServerRequest(workspace, {
      name: 'm8_restart_echo',
      transport: {
        type: 'stdio',
        command: 'node',
        args: [mcpPath],
        env: {},
        env_vars: [],
        cwd: null,
      },
      enabled: true,
      required: false,
    }))
    await client.send(installSkillRequest(workspace, skillDir))
    await client.send(grantAgentExtensionRequest(workspace, agent.id, 'mcp', 'm8_restart_echo'))
    await client.send(grantAgentExtensionRequest(workspace, agent.id, 'skill', 'm8-restart-skill'))

    const launched = oneOfVariant(
      await client.send(launchProviderRunRequest(sessionId, 'dev-stub', 'default', 'm8-restart-profile', 'low', agent.id)),
      ['ProviderRunLaunchAccepted', 'ProviderRunLaunched'],
    )
    const providerRun = launched.provider_run
    log('provider-launched', { providerRunId: providerRun?.id ?? null })

    const historyMarker = `M8_RESTART_HISTORY_MARKER_${process.pid}_${Date.now()}`
    await client.send(submitPromptRequest(sessionId, attachment.id, agent.id, `persist this marker: ${historyMarker}`, []))
    await waitForState(
      client,
      sessionId,
      (state) => Boolean(state.active_prompt || state.active_prompt_for_agent || asArray(Object.values(state.prompt_states ?? {})).some((promptState) => promptState.active_prompt)),
      'normal active prompt before completion',
    )
    await client.send(completePromptRequest(sessionId))
    await waitForState(client, sessionId, (state) => !state.active_prompt, 'normal prompt completion')

    const workflowCreated = variant(await client.send(createWorkflowRequest(sessionId, 'm8-restart-workflow')), 'WorkflowCreated')
    const workflow = workflowCreated.workflow
    const nodeAdded = variant(await client.send(addWorkflowNodeRequest(sessionId, workflow.id, agent.id)), 'WorkflowNodeAdded')
    const node = nodeAdded.node
    const endpointCreated = variant(
      await client.send(createWorkflowEndpointRequest(sessionId, workflow.id, node.id, 'm8-restart-entry')),
      'WorkflowEndpointCreated',
    )
    const endpoint = endpointCreated.endpoint
    const workflowProvider = oneOfVariant(
      await client.send({
        LaunchProviderRun: {
          session_id: sessionId,
          agent_id: agent.id,
          adapter_key: 'dev-stub',
          provider: 'dev-stub',
          account_profile: 'default',
          model: 'workflow-drill-node-1',
          variant: 'low',
        },
      }),
      ['ProviderRunLaunchAccepted', 'ProviderRunLaunched'],
    )
    const workflowProviderRun = workflowProvider.provider_run
    await waitForState(
      client,
      sessionId,
      (state) => state.active_provider_run_id === workflowProviderRun.id,
      'workflow dev-stub provider run activation',
    )
    const workflowMarker = `M8_RESTART_WORKFLOW_MARKER_${process.pid}_${Date.now()}`
    const invoked = variant(
      await client.send(invokeWorkflowEndpointRequest(sessionId, workflow.id, endpoint.id, `leave workflow active: ${workflowMarker}`)),
      'WorkflowRunInvoked',
    )
    const workflowRun = invoked.workflow_run
    await waitForState(
      client,
      sessionId,
      (state) => asArray(state.workflow_runs).some((run) => run.id === workflowRun.id && ['Created', 'Running', 'Waiting'].includes(run.status)),
      'active workflow run before restart',
    )
    log('workflow-active', { workflowId: workflow.id, workflowRunId: workflowRun.id })

    await sleep(250)
    await client.close().catch(() => {})
    client = null
    await stopDaemon(daemon)
    log('daemon-stopped')

    daemon = startDaemon(kernelBinary, env)
    await waitForDaemon(kernelUrl)
    client = new LocalIpcClient(kernelUrl)
    log('daemon-restarted')

    const sessions = variant(await client.send(listSessionsRequest()), 'SessionsListed').sessions
    assert(sessions.some((candidate) => candidate.id === sessionId), 'restored session is missing from listSessions')
    const restored = variant(await client.send(getSessionStateRequest(sessionId)), 'SessionState').session
    const restoredAgents = variant(await client.send(listAgentsRequest(sessionId)), 'AgentsListed').agents
    const restoredAgent = restoredAgents.find((candidate) => candidate.id === agent.id)
    assert(restoredAgent, 'spawned agent did not restore')
    assert(restoredAgent.provider === 'dev-stub', `expected restored provider dev-stub, got ${restoredAgent.provider}`)
    assert(restoredAgent.model === 'workflow-drill-node-1', `expected restored model workflow-drill-node-1, got ${restoredAgent.model}`)
    assert(hasExtensionGrant(restoredAgent, 'mcp', 'm8_restart_echo'), 'MCP grant did not restore')
    assert(hasExtensionGrant(restoredAgent, 'skill', 'm8-restart-skill'), 'skill grant did not restore')
    assert(asArray(restored.workflows).some((candidate) => candidate.id === workflow.id), 'workflow definition did not restore')
    const restoredRun = asArray(restored.workflow_runs).find((candidate) => candidate.id === workflowRun.id)
    assert(restoredRun, 'workflow run did not restore')
    assert(restored.active_provider_run_id == null, `stale active provider run was not cleared: ${restored.active_provider_run_id}`)
    assert(restored.active_prompt == null, 'stale active prompt was not cleared')
    assert(restoredRun.status === 'Stopped', `workflow run was not stopped after restart: ${restoredRun.status}`)
    assert(
      JSON.stringify(restoredRun.failure_events ?? []).includes('interrupted by kernel restart'),
      `workflow run failure event does not mention kernel restart: ${JSON.stringify(restoredRun.failure_events ?? [])}`,
    )

    const transcript = variant(
      await client.send(getSessionHistoryOutlineRequest(sessionId, [restoredAgent.id], 10)),
      'SessionHistoryOutline',
    )
    assert(historyOutlineText(transcript).includes(historyMarker), 'session transcript did not retain completed prompt marker')
    const search = await client.send(searchRecallRequest(historyMarker, { session_id: sessionId, limit: 10 }))
    assert(JSON.stringify(search).includes(historyMarker), 'recall search did not find completed prompt marker')
    log('restored-state-verified', {
      sessionId,
      agentId: agent.id,
      workflowRunId: workflowRun.id,
      workflowRunStatus: restoredRun.status,
    })

    await client.send(endSessionRequest(sessionId)).catch(() => {})
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    await client?.close().catch(() => {})
    await stopDaemon(daemon)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'local-restart',
        kernelUrl,
        daemonId,
      },
      log,
    })
  }
}

main().catch((error) => {
  console.error(`[restart-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
