import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath, pathToFileURL } from 'node:url'
import os from 'node:os'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'
import {
  createWorkflowRuntimeScenario,
  workflowOutput,
} from './lib/live-workflow-runtime-drill-scenarios.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const cliRuntimeDir = path.join(cliRoot, '.tmp-live-workflow-runtime-drill')

async function loadCliModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
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
  const ipcUrl = new URL(`file://${path.join(runtimeDir, 'ipc.js')}`).href
  const requestsUrl = new URL(`file://${path.join(runtimeDir, 'ipc-requests.js')}`).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
await mkdir(cliRuntimeDir, { recursive: true })
const { LocalIpcClient, requests } = await loadCliModules(cliRuntimeDir)

const {
  createSessionRequest,
  attachToSessionRequest,
  createWorkflowRequest,
  addWorkflowNodeRequest,
  updateWorkflowNodeInstructionsRequest,
  createWorkflowEndpointRequest,
  invokeWorkflowEndpointRequest,
  getSessionStateRequest,
  pumpTerminalOutputRequest,
  getProviderRunRequest,
  spawnAgentRequest,
  launchProviderRunRequest,
  listProviderProcessesRequest,
  setWorkflowFlushContextRequest,
  endSessionRequest,
} = requests

const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
const DEFAULT_WORKSPACE = repoRoot
const DEFAULT_WORKTREE = repoRoot
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_PROVIDERS = ['opencode', 'codex', 'opencode', 'codex', 'opencode', 'codex']

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

function parseArgs(argv) {
  const options = {
    scenario: 'simple-chain',
    kernel: DEFAULT_KERNEL,
    relayUrl: null,
    relayToken: null,
    targetDaemonId: null,
    targetDaemonAlias: null,
    machineRef: null,
    workspace: DEFAULT_WORKSPACE,
    worktree: DEFAULT_WORKTREE,
    model: DEFAULT_MODEL,
    providerModels: {},
    providers: DEFAULT_PROVIDERS,
    pollLimit: 120,
    pollIntervalMs: 2000,
    dryRun: false,
    spawnDaemon: false,
    noEarlyPass: false,
    workspaceLiveSyncMode: null,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--scenario') options.scenario = argv[++index]
    else if (arg === '--kernel') options.kernel = argv[++index]
    else if (arg === '--relay-url') options.relayUrl = argv[++index]
    else if (arg === '--relay-token') options.relayToken = argv[++index]
    else if (arg === '--target-daemon-id') options.targetDaemonId = argv[++index]
    else if (arg === '--target-daemon-alias') options.targetDaemonAlias = argv[++index]
    else if (arg === '--machine-ref') options.machineRef = argv[++index]
    else if (arg === '--workspace') options.workspace = argv[++index]
    else if (arg === '--worktree') options.worktree = argv[++index]
    else if (arg === '--model') options.model = argv[++index]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++index].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--providers') options.providers = argv[++index].split(',').map((v) => v.trim()).filter(Boolean)
    else if (arg === '--poll-limit') options.pollLimit = Number(argv[++index])
    else if (arg === '--poll-interval-ms') options.pollIntervalMs = Number(argv[++index])
    else if (arg === '--dry-run') options.dryRun = true
    else if (arg === '--spawn-daemon') options.spawnDaemon = true
    else if (arg === '--no-early-pass') options.noEarlyPass = true
    else if (arg === '--workspace-live-sync-mode') options.workspaceLiveSyncMode = argv[++index]
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-workflow-runtime-drill.mjs [options]',
    '',
    'Options:',
    '  --scenario simple-chain|validated-increment-chain|console-increment-chain|final-run-output-chain|cyclic-final-run-output-chain|cyclic-budgeted-final-run-output-chain|cyclic-final-run-with-intermediate-output-chain|conditional-branch-subset|immediate-release-downstream|mcp-echo-workflow',
    `  --kernel ${DEFAULT_KERNEL}`,
    '  --relay-url ws://127.0.0.1:45168 --relay-token TOKEN --target-daemon-alias NAME',
    '  --machine-ref MACHINE_ALIAS',
    `  --workspace ${DEFAULT_WORKSPACE}`,
    `  --worktree ${DEFAULT_WORKTREE}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL (for example opencode=opencode/gpt-5.2)',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    '  --poll-limit 120',
    '  --poll-interval-ms 2000',
    '  --dry-run',
    '  --spawn-daemon',
    '  --no-early-pass',
    '  --workspace-live-sync-mode off|managed|tracked',
  ].join('\n'))
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  return options.model
}

function opencodeCodexModel(model) {
  if (model.endsWith('-codex')) return model
  if (/^gpt-5\.[23]$/.test(model)) return `${model}-codex`
  return model
}

function validateConnectionOptions(options) {
  const usingRelay = Boolean(options.relayUrl)
  if (usingRelay && (!options.relayToken || (!options.targetDaemonId && !options.targetDaemonAlias))) {
    throw new Error('--relay-url requires --relay-token and one of --target-daemon-id/--target-daemon-alias')
  }
  if (usingRelay && options.kernel !== DEFAULT_KERNEL && options.kernel) {
    throw new Error('--kernel cannot be combined with relay connection options')
  }
  if (!usingRelay && options.machineRef && options.spawnDaemon) {
    return
  }
}

function deriveSpawnedKernelUrl() {
  const kernelPort = 44000 + Math.floor(Math.random() * 1000)
  const mcpPort = kernelPort + 1000
  const opencodePort = kernelPort + 2000
  const codexPort = kernelPort + 2001
  const socketPath = path.join(os.tmpdir(), `arroba-drill-${process.pid}-${Date.now()}.sock`)
  return {
    kernelUrl: `ws://127.0.0.1:${kernelPort}`,
    env: {
      ...process.env,
      ARROBA_KERNEL_PORT: String(kernelPort),
      ARROBA_MCP_PORT: String(mcpPort),
      ARROBA_OPENCODE_PORT: String(opencodePort),
      ARROBA_CODEX_PORT: String(codexPort),
      ARROBA_DAEMON_SOCKET: socketPath,
      ARROBA_DAEMON_ID: `drill-${process.pid}-${Date.now()}`,
    },
  }
}

async function ensureSchemaFile() {
  const dir = path.join(repoRoot, 'tmp', 'live-drills')
  await mkdir(dir, { recursive: true })
  const schemaPath = path.join(dir, 'value-schema.json')
  await writeFile(
    schemaPath,
    JSON.stringify(
      {
        $schema: 'https://json-schema.org/draft/2020-12/schema',
        type: 'object',
        required: ['value'],
        properties: { value: { type: 'integer' } },
        additionalProperties: false,
      },
      null,
      2,
    ),
  )
  return schemaPath
}

async function resolveBinary(binaryPath, manifestPath, binName) {
  try {
    await import('node:fs/promises').then(({ access }) => access(binaryPath))
    return binaryPath
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
}

function createScenario(options, schemaPath) {
  return createWorkflowRuntimeScenario(options, schemaPath, requests)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  validateConnectionOptions(options)
  if (options.help) {
    printHelp()
    return
  }
  const schemaPath = await ensureSchemaFile()
  const scenario = createScenario(options, schemaPath)
  if (options.dryRun) {
    console.log(JSON.stringify({
      kernel: options.kernel,
      workspace: options.workspace,
      worktree: options.worktree,
      scenario: scenario.id,
      providers: scenario.providers,
      model: scenario.model,
      schemaPath,
      samplePrompts: scenario.providers.map((_, index) => ({ index, prompt: scenario.nodePrompt(index) })),
    }, null, 2))
    return
  }

  const artifactRoot = path.join(repoRoot, '.artifacts', 'live-workflow-runtime-drill', nowStamp())
  await prepareDrillArtifacts(artifactRoot)
  let passed = false
  let failure = null
  let daemonChild = null
  let kernelUrl = options.kernel
  if (options.spawnDaemon) {
    const spawned = deriveSpawnedKernelUrl()
    kernelUrl = spawned.kernelUrl
    const daemonBinary = await resolveBinary(
      path.join(repoRoot, 'target/debug/arroba-kernel'),
      path.join(repoRoot, 'apps/kernel/Cargo.toml'),
      'arroba-kernel',
    )
    daemonChild = spawn(
      daemonBinary,
      [],
      { cwd: repoRoot, env: spawned.env, stdio: ['ignore', 'ignore', 'inherit'] },
    )
  }
  const client = options.relayUrl
    ? new LocalIpcClient(options.relayUrl, {
        relayAuthToken: options.relayToken,
        targetDaemonId: options.targetDaemonId,
        targetDaemonAlias: options.targetDaemonAlias,
        kernelPingIntervalMs: 60_000,
        kernelMaxMissedPongs: 10,
      })
    : new LocalIpcClient(kernelUrl, {
        kernelPingIntervalMs: 60_000,
        kernelMaxMissedPongs: 10,
      })
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
  const unwrap = (resp, key) => resp?.[key] ?? resp
  const pidExists = (pid) => {
    if (typeof pid !== 'number' || pid <= 0) return false
    try {
      process.kill(pid, 0)
      return true
    } catch {
      return false
    }
  }
  const consoleEntriesFor = (state, workflowId) =>
    ((state.workflow_consoles || []).find((entry) => entry.workflow_id === workflowId)?.entries) || []
  const buildWorkflowResult = (session, workflow, workflowRun, run, state) => ({
    sessionId: session.id,
    workflowId: workflow.id,
    workflowRunId: workflowRun.id,
    status: run.status,
    finalOutput: run.final_output ?? null,
    finalOutputValid: run.final_output_valid ?? null,
    finalOutputWarning: run.final_output_warning ?? null,
    completedByNodeRunId: run.completed_by_node_run_id ?? null,
    intermediateOutputs: run.intermediate_outputs ?? [],
    consoleEntries: consoleEntriesFor(state, workflow.id),
    nodeRuns: run.node_runs.map((nodeRun) => ({
      id: nodeRun.id,
      nodeId: nodeRun.node_id,
      status: nodeRun.status,
      summary: nodeRun.summary,
      completion: nodeRun.completion,
      createdAtMs: nodeRun.created_at_ms,
      startedAtMs: nodeRun.started_at_ms,
      completedAtMs: nodeRun.completed_at_ms,
      tools: nodeRun.turn_envelope?.runtime_tool_calls || [],
    })),
    failureEvents: run.failure_events || [],
    terminalRecords: terminalRecords.slice(-20).map((record) => ({
      kind: record.kind,
      providerRunId: record.provider_run_id ?? null,
      text: Array.isArray(record.bytes)
        ? Buffer.from(record.bytes).toString('utf8').slice(0, 800)
        : '',
    })),
  })
  const logStep = (name, details = null) => {
    const prefix = `[drill] ${name}`
    if (details == null) console.log(prefix)
    else console.log(prefix, JSON.stringify(details))
  }
  const waitForRemoteKernel = async (machineRef, provider) => {
    let last = []
    for (let attempt = 0; attempt < 80; attempt += 1) {
      const response = unwrap(
        await client.send({ ListRemoteMachineKernels: { machine_ref: machineRef } }),
        'RemoteMachineKernelsListed',
      )
      last = response.kernels ?? []
      const kernel = last.find((candidate) => candidate.accepting_remote_leases && (candidate.available_providers || []).includes(provider))
      if (kernel) return kernel
      await sleep(500)
    }
    throw new Error(`remote machine ${machineRef} did not advertise provider ${provider}; last=${JSON.stringify(last)}`)
  }
  const requireRemotePlacement = (agent, workerKernel) => {
    if (!agent.remote_execution?.leased_agent_id) {
      throw new Error(`agent ${agent.id} was expected to be remote-backed\n${JSON.stringify(agent, null, 2)}`)
    }
    if (agent.remote_execution.worker_kernel_id !== workerKernel.kernel_id) {
      throw new Error(`agent ${agent.id} ran on ${agent.remote_execution.worker_kernel_id}, expected ${workerKernel.kernel_id}`)
    }
    if (agent.remote_execution.worker_machine_id !== workerKernel.machine_id) {
      throw new Error(`agent ${agent.id} ran on machine ${agent.remote_execution.worker_machine_id}, expected ${workerKernel.machine_id}`)
    }
  }
  const waitForProviderRunReady = async (providerRunId) => {
    for (let attempt = 0; attempt < 120; attempt += 1) {
      const response = await client.send(getProviderRunRequest(providerRunId))
      const providerRun = unwrap(response, 'ProviderRun')?.provider_run
      if (providerRun && providerRun.state !== 'Starting') {
        if (providerRun.state !== 'Running' && providerRun.state !== 'Parked') {
          throw new Error(`provider run ${providerRunId} reached unexpected state ${providerRun.state}`)
        }
        return providerRun
      }
      await sleep(250)
    }
    throw new Error(`provider run ${providerRunId} did not become ready`)
  }

  let sessionId = null
  let workflowId = null
  let workflowRunId = null
  let attachmentId = null
  let trackedProviderProcesses = []
  let cleanupReport = null
  const terminalRecords = []
  const agentIds = []
  const nodeIds = []
  const captureTrackedProviderProcesses = async () => {
    if (!daemonChild) return
    const listed = unwrap(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed')?.processes || []
    trackedProviderProcesses = listed.map((processInfo) => ({
      processId: processInfo.process_id,
      provider: processInfo.provider,
      pid: processInfo.pid ?? null,
      ownerRunIds: processInfo.owner_provider_run_ids || [],
    }))
  }
  try {
    if (daemonChild) {
      let ready = false
      for (let attempt = 0; attempt < 80; attempt += 1) {
        try {
          const probeClient = new LocalIpcClient(kernelUrl)
          const probeSession = unwrap(
            await probeClient.send(createSessionRequest(options.workspace, options.worktree, undefined, undefined, null, options.workspaceLiveSyncMode)),
            'SessionCreated',
          ).session
          await probeClient.send(endSessionRequest(probeSession.id)).catch(() => {})
          await probeClient.close()
          ready = true
          break
        } catch {
          await sleep(250)
        }
      }
      if (!ready) {
        throw new Error(`spawned daemon did not become ready at ${kernelUrl}`)
      }
    }
    logStep('create_session')
    const session = unwrap(await client.send(createSessionRequest(options.workspace, options.worktree, undefined, undefined, null, options.workspaceLiveSyncMode)), 'SessionCreated').session
    sessionId = session.id
    const attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `live-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    attachmentId = attachment.id

    if (typeof scenario.beforeAgents === 'function') {
      logStep('scenario_before_agents', { scenario: scenario.id })
      await scenario.beforeAgents(client, options)
    }

    for (let index = 0; index < scenario.providers.length; index += 1) {
      const provider = scenario.providers[index]
      const providerModel = modelForProvider(provider, options)
      const agentWorktree = typeof scenario.agentWorktree === 'function'
        ? scenario.agentWorktree(index, options)
        : options.worktree
      await mkdir(agentWorktree, { recursive: true })
      logStep('spawn_agent', { index, provider })
      let expectedWorkerKernel = null
      const spawnRequest = options.machineRef
        ? await (async () => {
            const workerKernel = await waitForRemoteKernel(options.machineRef, provider)
            expectedWorkerKernel = workerKernel
            return {
              SpawnAgent: {
                session_id: session.id,
                provider,
                alias: `a${index + 1}`,
                model: providerModel,
                effort: 'low',
                worktree_id: agentWorktree,
                kernel_ref: workerKernel.kernel_id,
              },
            }
          })()
        : spawnAgentRequest(session.id, provider, `a${index + 1}`, providerModel, agentWorktree, 'low')
      const agent = unwrap(
        await client.send(spawnRequest),
        'AgentSpawned',
      ).agent
      if (expectedWorkerKernel) {
        requireRemotePlacement(agent, expectedWorkerKernel)
      }
      agentIds.push(agent.id)
      if (typeof scenario.afterAgentSpawn === 'function') {
        logStep('scenario_after_agent_spawn', { index, agentId: agent.id, scenario: scenario.id })
        await scenario.afterAgentSpawn(client, options, { agent, index, provider })
      }
      if (!options.machineRef) {
        logStep('launch_provider', { index, provider, agentId: agent.id })
        const launchResponse = await client.send(
          launchProviderRunRequest(session.id, provider, 'default', providerModel, 'low', agent.id),
        )
        const providerRun = unwrap(launchResponse, 'ProviderRunLaunchAccepted')?.provider_run
        if (!providerRun?.id) {
          throw new Error(`provider launch for agent ${agent.id} did not return a provider run`)
        }
        logStep('wait_provider_ready', { index, providerRunId: providerRun.id })
        await waitForProviderRunReady(providerRun.id)
      }
    }

    logStep('create_workflow', { alias: scenario.alias })
    const workflow = unwrap(await client.send(createWorkflowRequest(session.id, scenario.alias)), 'WorkflowCreated').workflow
    workflowId = workflow.id
    logStep('set_workflow_flush_context', { workflowId: workflow.id, flushAgentContextBeforeRun: false })
    await client.send(setWorkflowFlushContextRequest(session.id, workflow.id, false))
    for (let index = 0; index < scenario.providers.length; index += 1) {
      const provider = scenario.providers[index]
      logStep('add_node', { index, provider })
      const node = unwrap(await client.send(addWorkflowNodeRequest(session.id, workflow.id, agentIds[index])), 'WorkflowNodeAdded').node
      nodeIds.push(node.id)
      logStep('update_node_instructions', { index, nodeId: node.id })
      await client.send(updateWorkflowNodeInstructionsRequest(session.id, workflow.id, node.id, scenario.nodePrompt(index)))
      if (index > 0 && scenario.autoChainEdges !== false) {
        const edgeRequest = scenario.edgeRequest(session.id, workflow.id, nodeIds[index - 1], nodeIds[index])
        if (edgeRequest) {
          logStep('add_edge', { fromNodeId: nodeIds[index - 1], toNodeId: nodeIds[index] })
          await client.send(edgeRequest)
        }
      }
    }

    if (typeof scenario.extraEdges === 'function') {
      for (const [fromNodeId, toNodeId] of scenario.extraEdges(nodeIds)) {
        const edgeRequest = scenario.edgeRequest(session.id, workflow.id, fromNodeId, toNodeId)
        if (edgeRequest) {
          logStep('add_edge', { fromNodeId, toNodeId })
          await client.send(edgeRequest)
        }
      }
    }

    if (typeof scenario.configureWorkflow === 'function') {
      logStep('configure_workflow', { workflowId: workflow.id, scenario: scenario.id })
      await scenario.configureWorkflow(client, session.id, workflow.id, nodeIds)
    }

    logStep('create_endpoint', { entryNodeId: nodeIds[0] })
    const endpoint = unwrap(
      await client.send(createWorkflowEndpointRequest(session.id, workflow.id, nodeIds[0], 'start')),
      'WorkflowEndpointCreated',
    ).endpoint

    logStep('invoke', { endpointId: endpoint.id })
    const workflowRun = unwrap(
      await client.send(invokeWorkflowEndpointRequest(session.id, workflow.id, endpoint.id, scenario.entryPrompt ?? 'Run the workflow exactly as instructed.')),
      'WorkflowRunInvoked',
    ).workflow_run
    workflowRunId = workflowRun.id

    for (let index = 0; index < options.pollLimit; index += 1) {
      await sleep(options.pollIntervalMs)
      const outputResp = await client.send(pumpTerminalOutputRequest(session.id, attachment.id)).catch(() => null)
      const outputRecords = unwrap(outputResp, 'TerminalOutput')?.records || []
      terminalRecords.push(...outputRecords)
      const stateResp = await client.send(getSessionStateRequest(session.id))
      const state = unwrap(stateResp, 'SessionStateLoaded')?.session ?? unwrap(stateResp, 'SessionState')?.session
      const run = (state.workflow_runs || []).find((entry) => entry.id === workflowRun.id)
      if (!options.noEarlyPass && run && typeof scenario.assertEarlyResult === 'function') {
        const result = buildWorkflowResult(session, workflow, workflowRun, run, state)
        if (scenario.assertEarlyResult(result, { nodeIds, agentIds, workflowId: workflow.id, workflowRunId: workflowRun.id })) {
          console.log(JSON.stringify({ ...result, earlyPass: true }, null, 2))
          await captureTrackedProviderProcesses()
          await client.send(endSessionRequest(session.id)).catch(() => {})
          await client.close()
          passed = true
          return
        }
      }
      if (run && ['Completed', 'Failed', 'Stopped'].includes(run.status)) {
        const result = buildWorkflowResult(session, workflow, workflowRun, run, state)
        console.log(JSON.stringify(result, null, 2))
        await captureTrackedProviderProcesses()
        if (run.status !== 'Completed') {
          throw new Error(`workflow drill ${scenario.id} ended with status ${run.status}`)
        }
        const failureEvents = run.failure_events || []
        if (failureEvents.length > 0) {
          const expectedKinds = new Set(scenario.expectedFailureEventKindsWhenCompleted || [])
          const unexpected = failureEvents.filter((event) => !expectedKinds.has(event.kind))
          if (unexpected.length > 0) {
            throw new Error(`workflow drill ${scenario.id} recorded unexpected failure events`)
          }
        }
        if (scenario.expectedFinalOutput !== undefined) {
          const actualFinalOutput = run.final_output?.message
          if (actualFinalOutput !== scenario.expectedFinalOutput) {
            throw new Error(
              `workflow drill ${scenario.id} final output mismatch: expected ${scenario.expectedFinalOutput}, got ${actualFinalOutput}`,
            )
          }
        }
        if (typeof scenario.assertResult === 'function') {
          await scenario.assertResult(result, { nodeIds, agentIds, workflowId: workflow.id, workflowRunId: workflowRun.id, options })
        }
        await client.send(endSessionRequest(session.id)).catch(() => {})
        await client.close()
        passed = true
        return
      }
    }

    const stateResp = await client.send(getSessionStateRequest(session.id))
    const state = unwrap(stateResp, 'SessionStateLoaded')?.session ?? unwrap(stateResp, 'SessionState')?.session
    const run = (state.workflow_runs || []).find((entry) => entry.id === workflowRun.id)
    console.log(JSON.stringify({
      sessionId: session.id,
      workflowId: workflow.id,
      workflowRunId: workflowRun.id,
      status: run?.status,
      finalOutput: run?.final_output ?? null,
      finalOutputValid: run?.final_output_valid ?? null,
      finalOutputWarning: run?.final_output_warning ?? null,
      completedByNodeRunId: run?.completed_by_node_run_id ?? null,
      intermediateOutputs: run?.intermediate_outputs ?? [],
      consoleEntries: consoleEntriesFor(state, workflow.id),
      nodeRuns: run?.node_runs?.map((nodeRun) => ({
        id: nodeRun.id,
        nodeId: nodeRun.node_id,
        status: nodeRun.status,
        summary: nodeRun.summary,
        completion: nodeRun.completion,
        createdAtMs: nodeRun.created_at_ms,
        startedAtMs: nodeRun.started_at_ms,
        completedAtMs: nodeRun.completed_at_ms,
        tools: nodeRun.turn_envelope?.runtime_tool_calls || [],
      })),
      failureEvents: run?.failure_events || [],
      terminalRecords: terminalRecords.map((record) => ({
        kind: record.kind,
        providerRunId: record.provider_run_id,
        text: typeof record.bytes === 'string'
          ? record.bytes
          : Buffer.from(record.bytes || []).toString('utf8'),
      })).slice(-120),
    }, null, 2))
    await captureTrackedProviderProcesses()
    if (sessionId) await client.send(endSessionRequest(sessionId)).catch(() => {})
    await client.close()
    process.exitCode = 1
  } catch (error) {
    failure = error
    console.error(error)
    if (sessionId) {
      try { await client.send(endSessionRequest(sessionId)) } catch {}
    }
    try { await client.close() } catch {}
    process.exitCode = 1
  } finally {
    if (daemonChild) {
      daemonChild.kill('SIGTERM')
      await sleep(1000)
      cleanupReport = {
        daemonPid: daemonChild.pid ?? null,
        daemonAliveAfterKill: daemonChild.pid ? pidExists(daemonChild.pid) : false,
        trackedProviderProcessesBeforeKill: trackedProviderProcesses,
        trackedProviderProcessesAliveAfterKill: trackedProviderProcesses.map((processInfo) => ({
          ...processInfo,
          alive: processInfo.pid != null ? pidExists(processInfo.pid) : false,
        })),
      }
      console.log(JSON.stringify({ cleanupReport }, null, 2))
    }
    await finalizeDrillArtifacts({
      rootDir: artifactRoot,
      passed,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: 'live-workflow-runtime',
        scenario: scenario.id,
        kernelUrl,
        relayUrl: options.relayUrl,
        targetDaemonId: options.targetDaemonId,
        targetDaemonAlias: options.targetDaemonAlias,
        machineRef: options.machineRef,
        workspace: options.workspace,
        worktree: options.worktree,
        providers: scenario.providers,
        model: options.model,
        providerModels: options.providerModels,
        pollLimit: options.pollLimit,
        pollIntervalMs: options.pollIntervalMs,
        spawnDaemon: options.spawnDaemon,
        workspaceLiveSyncMode: options.workspaceLiveSyncMode,
        sessionId,
        attachmentId,
        workflowId,
        workflowRunId,
        agentIds,
        nodeIds,
        trackedProviderProcesses,
        cleanupReport,
        terminalRecords: terminalRecords.map((record) => ({
          kind: record.kind,
          providerRunId: record.provider_run_id ?? null,
          text: typeof record.bytes === 'string'
            ? record.bytes.slice(0, 1000)
            : Buffer.from(record.bytes || []).toString('utf8').slice(0, 1000),
        })).slice(-80),
      },
    })
  }
}

export {
  createScenario,
  ensureSchemaFile,
  modelForProvider,
  workflowOutput,
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  await main()
}
