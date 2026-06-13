#!/usr/bin/env node
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

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

async function loadCliModules(runtimeDir) {
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

function hasExtensionGrant(agent, kind, name) {
  return Array.isArray(agent?.extension_grants) && agent.extension_grants.some((grant) => grant.kind === kind && grant.name === name)
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

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const runId = `${process.pid}-${Date.now()}`
  const rootDir = path.join(repoRoot, 'target', 'live-local-restart-persistence-drill', runId)
  const runtimeDir = path.join(cliRoot, `.tmp-live-local-restart-persistence-drill-${runId}`)
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
  let failure = null
  let LocalIpcClient = null
  let createSessionRequest = null
  let attachToSessionRequest = null
  let spawnAgentRequest = null
  let launchProviderRunRequest = null
  let submitPromptRequest = null
  let completePromptRequest = null
  let getSessionStateRequest = null
  let getSessionHistoryOutlineRequest = null
  let searchRecallRequest = null
  let installMcpServerRequest = null
  let installSkillRequest = null
  let grantAgentExtensionRequest = null
  let listAgentsRequest = null
  let createWorkflowRequest = null
  let addWorkflowNodeRequest = null
  let updateWorkflowNodeInstructionsRequest = null
  let createWorkflowEndpointRequest = null
  let invokeWorkflowEndpointRequest = null
  let endSessionRequest = null

  try {
    await prepareDrillArtifacts(rootDir)
    const loaded = await loadCliModules(runtimeDir)
    LocalIpcClient = loaded.LocalIpcClient
    createSessionRequest = loaded.requests.createSessionRequest
    attachToSessionRequest = loaded.requests.attachToSessionRequest
    spawnAgentRequest = loaded.requests.spawnAgentRequest
    launchProviderRunRequest = loaded.requests.launchProviderRunRequest
    submitPromptRequest = loaded.requests.submitPromptRequest
    completePromptRequest = loaded.requests.completePromptRequest
    getSessionStateRequest = loaded.requests.getSessionStateRequest
    getSessionHistoryOutlineRequest = loaded.requests.getSessionHistoryOutlineRequest
    searchRecallRequest = loaded.requests.searchRecallRequest
    installMcpServerRequest = loaded.requests.installMcpServerRequest
    installSkillRequest = loaded.requests.installSkillRequest
    grantAgentExtensionRequest = loaded.requests.grantAgentExtensionRequest
    listAgentsRequest = loaded.requests.listAgentsRequest
    createWorkflowRequest = loaded.requests.createWorkflowRequest
    addWorkflowNodeRequest = loaded.requests.addWorkflowNodeRequest
    updateWorkflowNodeInstructionsRequest = loaded.requests.updateWorkflowNodeInstructionsRequest
    createWorkflowEndpointRequest = loaded.requests.createWorkflowEndpointRequest
    invokeWorkflowEndpointRequest = loaded.requests.invokeWorkflowEndpointRequest
    endSessionRequest = loaded.requests.endSessionRequest

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
    const grantedMcp = unwrap(await client.send(grantAgentExtensionRequest(workspace, workerAgent.id, 'mcp', 'm8_restart_echo')), 'AgentExtensionGranted').agent
    const grantedSkill = unwrap(await client.send(grantAgentExtensionRequest(workspace, workerAgent.id, 'skill', 'm8-restart-skill')), 'AgentExtensionGranted').agent
    assert.equal(hasExtensionGrant(grantedMcp, 'mcp', 'm8_restart_echo'), true)
    assert.equal(hasExtensionGrant(grantedSkill, 'skill', 'm8-restart-skill'), true)

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
    const startedPrompt = promptResponse.outcome?.Started?.prompt ?? promptResponse.outcome?.started?.prompt ?? null
    assert.ok(startedPrompt, `prompt should start immediately: ${JSON.stringify(promptResponse.outcome)}`)
    assert.equal(startedPrompt.status, 'Running')
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
    assert.equal(hasExtensionGrant(restoredWorker, 'mcp', 'm8_restart_echo'), true)
    assert.equal(hasExtensionGrant(restoredWorker, 'skill', 'm8-restart-skill'), true)

    const history = unwrap(await client.send(getSessionHistoryOutlineRequest(sessionId, [restoredWorker.id], 10)), 'SessionHistoryOutline')
    assert.equal(
      historyOutlineText(history).includes(promptMarker),
      true,
    )
    const search = unwrap(await client.send(searchRecallRequest(promptMarker, { session_id: sessionId, limit: 10 })), 'RecallEvents')
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
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) await client.close().catch(() => {})
    if (daemon) await stopDaemon(daemon).catch(() => {})
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'local-restart-persistence',
        kernelUrl,
        daemonId,
        runtimeDir,
      },
      log,
    })
    await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(`[m8-local-restart] failed: ${error.stack || error.message || error}`)
  process.exit(1)
})
