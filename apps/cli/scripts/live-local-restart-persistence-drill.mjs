#!/usr/bin/env node
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const runtimeDir = path.join(cliRoot, '.tmp-live-local-restart-persistence-drill')

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-local-restart-persistence-drill.mjs [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

async function loadCliModules() {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
  await rm(runtimeDir, { recursive: true, force: true })
  await mkdir(runtimeDir, { recursive: true })
  for (const rel of ['src/ipc.ts', 'src/ipc-requests.ts']) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, '.js'))
    const code = await readFile(sourcePath, 'utf8')
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await writeFile(outPath, transformed?.code ?? '', 'utf8')
  }
  const { LocalIpcClient } = await import(new URL(`file://${path.join(runtimeDir, 'ipc.js')}`).href)
  const requests = await import(new URL(`file://${path.join(runtimeDir, 'ipc-requests.js')}`).href)
  return { LocalIpcClient, requests }
}

function makePorts() {
  const kernelPort = 49000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[m8-local-restart] ${name}`)
  else console.log(`[m8-local-restart] ${name}`, JSON.stringify(details))
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
  const existingBinary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const existing = await stat(existingBinary).then((info) => info.isFile()).catch(() => false)
  if (existing) return existingBinary
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  }
  return existingBinary
}

function unwrap(resp, key) {
  return resp?.[key] ?? resp
}

async function waitForDaemon(LocalIpcClient, kernelUrl) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send({ ListSessions: null })
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? lastError}`)
}

async function stopDaemon(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill('SIGTERM')
  const exited = await new Promise((resolve) => {
    const timeout = setTimeout(() => resolve(false), 5_000)
    child.once('exit', () => {
      clearTimeout(timeout)
      resolve(true)
    })
  })
  if (!exited && child.exitCode === null && child.signalCode === null) {
    child.kill('SIGKILL')
    await new Promise((resolve) => child.once('exit', resolve))
  }
}

async function startDaemon(kernelBinary, env) {
  return spawn(kernelBinary, [], {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'ignore', 'inherit'],
  })
}

function agentByAlias(agents, alias) {
  return agents.find((agent) => agent.alias === alias)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const { LocalIpcClient, requests } = await loadCliModules()
  const {
    createSessionRequest,
    attachToSessionRequest,
    spawnAgentRequest,
    launchProviderRunRequest,
    submitPromptRequest,
    completePromptRequest,
    getSessionStateRequest,
    getSessionHistoryRequest,
    searchHistoryRequest,
    installMcpServerRequest,
    installSkillRequest,
    grantAgentCapabilityRequest,
    listAgentsRequest,
    createWorkflowRequest,
    addWorkflowNodeRequest,
    updateWorkflowNodeInstructionsRequest,
    createWorkflowEndpointRequest,
    invokeWorkflowEndpointRequest,
    endSessionRequest,
  } = requests

  const rootDir = path.join(repoRoot, 'target', 'live-local-restart-persistence-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const configRoot = path.join(rootDir, 'config')
  const stateRoot = path.join(rootDir, 'state')
  const skillDir = path.join(rootDir, 'm8-restart-skill')
  const mcpPath = path.join(rootDir, 'echo-mcp.mjs')
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const daemonId = `m8-local-restart-${process.pid}-${Date.now()}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configRoot,
    XDG_STATE_HOME: stateRoot,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history-jsonl'),
  }

  let daemon = null
  let client = null
  let sessionId = null
  let succeeded = false

  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configRoot, 'arroba'), { recursive: true })
    await mkdir(stateRoot, { recursive: true })
    await mkdir(home, { recursive: true })
    await mkdir(skillDir, { recursive: true })
    await writeFile(path.join(configRoot, 'arroba', 'config.toml'), [
      'version = 1',
      '',
      '[state]',
      'snapshot_interval_events = 1',
      '',
      '[history.archive]',
      'mode = "disabled"',
      '',
    ].join('\n'), 'utf8')
    await writeFile(path.join(skillDir, 'SKILL.md'), [
      '---',
      'name: m8-restart-skill',
      'description: Skill used by the M8 local restart persistence drill.',
      '---',
      'Use this only for M8 local restart persistence drills.',
      '',
    ].join('\n'), 'utf8')
    await writeFile(mcpPath, [
      'process.stdin.resume()',
      "process.stdin.on('data', () => {})",
      '',
    ].join('\n'), 'utf8')

    const kernelBinary = await buildKernel()
    daemon = await startDaemon(kernelBinary, env)
    await waitForDaemon(LocalIpcClient, kernelUrl)
    log('daemon-ready', { kernelUrl })

    client = new LocalIpcClient(kernelUrl)
    const created = unwrap(await client.send(createSessionRequest(workspace, workspace, 'm8-restart-session')), 'SessionCreated')
    const session = created.session
    sessionId = session.id
    const defaultAgent = created.agent
    const attachment = unwrap(await client.send(attachToSessionRequest(sessionId, 'm8-restart-client')), 'SessionAttached').attachment
    const workerAgent = unwrap(
      await client.send(spawnAgentRequest(sessionId, 'dev-stub', 'persisted-worker', 'm8-restart-profile-model', workspace, 'low')),
      'AgentSpawned',
    ).agent
    log('session-created', { sessionId, defaultAgentId: defaultAgent.id, workerAgentId: workerAgent.id })

    await client.send(installMcpServerRequest(workspace, {
      name: 'm8_restart_echo',
      enabled: true,
      transport: {
        type: 'stdio',
        command: 'node',
        args: [mcpPath],
      },
    }))
    await client.send(installSkillRequest(workspace, skillDir))
    const grantedMcp = unwrap(await client.send(grantAgentCapabilityRequest(workspace, workerAgent.id, 'mcp', 'm8_restart_echo')), 'AgentCapabilityGranted').agent
    const grantedSkill = unwrap(await client.send(grantAgentCapabilityRequest(workspace, workerAgent.id, 'skill', 'm8-restart-skill')), 'AgentCapabilityGranted').agent
    assert.deepEqual(grantedMcp.mcp_grants, ['m8_restart_echo'])
    assert.deepEqual(grantedSkill.skill_grants, ['m8-restart-skill'])

    const providerRun = unwrap(
      await client.send(launchProviderRunRequest(sessionId, 'dev-stub', 'default', 'm8-restart-profile-model', 'low', workerAgent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    log('provider-launched', { providerRunId: providerRun.id })

    const promptMarker = `M8_RESTART_HISTORY_MARKER_${process.pid}_${Date.now()}`
    const promptResponse = unwrap(
      await client.send(submitPromptRequest(sessionId, attachment.id, workerAgent.id, promptMarker, [])),
      'PromptSubmitted',
    )
    assert.equal(promptResponse.outcome.Started.prompt.status, 'Running')
    await client.send(completePromptRequest(sessionId))

    const workflow = unwrap(await client.send(createWorkflowRequest(sessionId, 'm8-restart-flow')), 'WorkflowCreated').workflow
    const node = unwrap(await client.send(addWorkflowNodeRequest(sessionId, workflow.id, workerAgent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(sessionId, workflow.id, node.id, [
      'This workflow turn is intentionally left in-flight for the M8 restart drill.',
      'Do not complete the workflow.',
    ].join('\n')))
    const endpoint = unwrap(await client.send(createWorkflowEndpointRequest(sessionId, workflow.id, node.id, 'start')), 'WorkflowEndpointCreated').endpoint
    const workflowPrompt = `M8_RESTART_WORKFLOW_MARKER_${process.pid}_${Date.now()}`
    const invoked = unwrap(
      await client.send(invokeWorkflowEndpointRequest(sessionId, workflow.id, endpoint.id, workflowPrompt)),
      'WorkflowRunInvoked',
    )
    const workflowRun = invoked.workflow_run
    log('workflow-invoked', { workflowId: workflow.id, workflowRunId: workflowRun.id })

    const preRestartState = unwrap(await client.send(getSessionStateRequest(sessionId)), 'SessionState').session
    assert.equal(preRestartState.active_provider_run_id, providerRun.id)
    assert.equal(preRestartState.scheduler_state, 'Running')
    assert.equal(preRestartState.active_prompt.status, 'Running')
    assert.equal(
      preRestartState.workflow_runs.find((run) => run.id === workflowRun.id)?.status,
      'Running',
    )

    await new Promise((resolve) => setTimeout(resolve, 6_500))
    await client.close()
    client = null
    await stopDaemon(daemon)
    daemon = null
    log('daemon-stopped')

    daemon = await startDaemon(kernelBinary, env)
    await waitForDaemon(LocalIpcClient, kernelUrl)
    client = new LocalIpcClient(kernelUrl)
    log('daemon-restarted')

    const restored = unwrap(await client.send(getSessionStateRequest(sessionId)), 'SessionState').session
    assert.equal(restored.id, sessionId)
    assert.equal(restored.active_provider_run_id, null)
    assert.equal(restored.active_prompt, null)
    assert.equal(restored.scheduler_state, 'Idle')
    assert.equal(restored.workflows.some((entry) => entry.id === workflow.id), true)
    assert.equal(restored.workflows.find((entry) => entry.id === workflow.id)?.endpoints?.[0]?.id, endpoint.id)
    const restoredRun = restored.workflow_runs.find((run) => run.id === workflowRun.id)
    assert.equal(restoredRun?.status, 'Stopped')
    assert.equal(restoredRun?.active_node_run_id, null)
    assert.equal(restoredRun?.node_runs?.[0]?.status, 'Stopped')
    assert.equal(
      restoredRun?.failure_events?.some((event) => String(event.message).includes('interrupted by kernel restart')),
      true,
    )

    const agents = unwrap(await client.send(listAgentsRequest(sessionId)), 'AgentsListed').agents
    const restoredWorker = agentByAlias(agents, 'persisted-worker')
    assert.ok(restoredWorker, 'persisted-worker agent should restore')
    assert.equal(restoredWorker.provider, 'dev-stub')
    assert.equal(restoredWorker.model, 'm8-restart-profile-model')
    assert.deepEqual(restoredWorker.mcp_grants, ['m8_restart_echo'])
    assert.deepEqual(restoredWorker.skill_grants, ['m8-restart-skill'])

    const history = unwrap(await client.send(getSessionHistoryRequest(sessionId, 10, 20_000, null, workerAgent.id)), 'SessionHistory')
    assert.equal(
      history.entries.some((entry) => String(entry.text ?? '').includes(promptMarker)),
      true,
    )
    const search = unwrap(await client.send(searchHistoryRequest(promptMarker, { session_id: sessionId, limit: 10 })), 'HistoryEvents')
    assert.equal(search.events.length > 0, true)

    log('verified', {
      sessionId,
      agentId: restoredWorker.id,
      workflowId: workflow.id,
      workflowRunId: workflowRun.id,
      historyEvents: search.events.length,
      artifactRoot: rootDir,
    })
    await client.send(endSessionRequest(sessionId)).catch(() => {})
    succeeded = true
  } finally {
    if (client) await client.close().catch(() => {})
    if (daemon) await stopDaemon(daemon).catch(() => {})
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      log('artifacts-kept', { rootDir })
    }
    await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(`[m8-local-restart] failed: ${error.stack || error.message || error}`)
  process.exit(1)
})
