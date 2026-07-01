import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { mkdir, rm } from 'node:fs/promises'
import { finalizeDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const distIpcUrl = pathToFileURL(path.join(cliRoot, 'dist', 'ipc.js')).href
const distRequestsUrl = pathToFileURL(path.join(cliRoot, 'dist', 'ipc-requests.js')).href
const { LocalIpcClient } = await import(distIpcUrl)
const requests = await import(distRequestsUrl)

const {
  addWorkflowEdgeRequest,
  addWorkflowNodeRequest,
  attachToSessionRequest,
  createSessionRequest,
  createWorkflowEndpointRequest,
  createWorkflowRequest,
  createWorkflowScheduleRequest,
  getSessionStateRequest,
  launchProviderRunRequest,
  spawnAgentRequest,
  updateWorkflowNodeInstructionsRequest,
  endSessionRequest,
} = requests

const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_INTERVAL_SECONDS = 1
const DEFAULT_CRON = '*/1 * * * * *'
const DEFAULT_TIMEZONE = 'UTC'
const DEFAULT_PROVIDERS = [
  'opencode',
  'codex',
  'opencode',
  'codex',
  'opencode',
  'codex',
  'opencode',
  'codex',
  'opencode',
  'codex',
]

function parseArgs(argv) {
  const options = {
    kernel: DEFAULT_KERNEL,
    workspace: repoRoot,
    worktree: repoRoot,
    model: DEFAULT_MODEL,
    providerModels: {},
    trigger: 'interval',
    intervalSeconds: DEFAULT_INTERVAL_SECONDS,
    cron: DEFAULT_CRON,
    timezone: DEFAULT_TIMEZONE,
    overlap: 'skip',
    providers: DEFAULT_PROVIDERS,
    pollLimit: 90,
    pollIntervalMs: 1000,
    dryRun: false,
    spawnDaemon: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--kernel') options.kernel = argv[++index]
    else if (arg === '--workspace') options.workspace = argv[++index]
    else if (arg === '--worktree') options.worktree = argv[++index]
    else if (arg === '--model') options.model = argv[++index]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++index].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--trigger') options.trigger = argv[++index]
    else if (arg === '--interval-seconds') options.intervalSeconds = Number(argv[++index])
    else if (arg === '--cron') options.cron = argv[++index]
    else if (arg === '--timezone') options.timezone = argv[++index]
    else if (arg === '--overlap') options.overlap = argv[++index]
    else if (arg === '--providers') options.providers = argv[++index].split(',').map((v) => v.trim()).filter(Boolean)
    else if (arg === '--poll-limit') options.pollLimit = Number(argv[++index])
    else if (arg === '--poll-interval-ms') options.pollIntervalMs = Number(argv[++index])
    else if (arg === '--dry-run') options.dryRun = true
    else if (arg === '--spawn-daemon') options.spawnDaemon = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (!['skip', 'queue'].includes(options.overlap)) {
    throw new Error(`unsupported schedule overlap: ${options.overlap}`)
  }
  if (!['interval', 'cron'].includes(options.trigger)) {
    throw new Error(`unsupported schedule trigger: ${options.trigger}`)
  }
  if (!Number.isFinite(options.intervalSeconds) || options.intervalSeconds < 1) {
    throw new Error('--interval-seconds must be a positive number')
  }
  if (!options.cron || !options.cron.trim()) {
    throw new Error('--cron must not be empty')
  }
  if (!options.timezone || !options.timezone.trim()) {
    throw new Error('--timezone must not be empty')
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-workflow-scheduler-drill.mjs [options]',
    '',
    'Options:',
    `  --kernel ${DEFAULT_KERNEL}`,
    `  --workspace ${repoRoot}`,
    `  --worktree ${repoRoot}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL (for example opencode=opencode/gpt-5.2)',
    '  --trigger interval|cron',
    `  --interval-seconds ${DEFAULT_INTERVAL_SECONDS}`,
    `  --cron "${DEFAULT_CRON}"`,
    `  --timezone ${DEFAULT_TIMEZONE}`,
    '  --overlap skip|queue',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    '  --poll-limit 90',
    '  --poll-interval-ms 1000',
    '  --dry-run',
    '  --spawn-daemon',
  ].join('\n'))
}

function scheduleTrigger(options) {
  if (options.trigger === 'cron') {
    return {
      kind: 'cron',
      expression: options.cron,
      timezone: options.timezone,
    }
  }
  return {
    kind: 'interval',
    every_seconds: options.intervalSeconds,
  }
}

function nowStamp() {
  return new Date().toISOString().replace(/[-:]/g, '').replace(/\.\d{3}Z$/, 'Z')
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  if (provider === 'codex' && !options.model.includes('/')) return opencodeCodexModel(options.model)
  return options.model
}

function opencodeCodexModel(model) {
  if (model.endsWith('-codex')) return model
  if (/^gpt-5\.[23]$/.test(model)) return `${model}-codex`
  return model
}

function deriveSpawnedKernelUrl(rootDir) {
  const kernelPort = 45000 + Math.floor(Math.random() * 1000)
  const mcpPort = kernelPort + 1000
  const socketPath = path.join(rootDir, 'daemon.sock')
  return {
    kernelUrl: `ws://127.0.0.1:${kernelPort}`,
    env: {
      ...process.env,
      ARROBA_KERNEL_PORT: String(kernelPort),
      ARROBA_MCP_PORT: String(mcpPort),
      ARROBA_DAEMON_SOCKET: socketPath,
      ARROBA_DAEMON_ID: `workflow-scheduler-drill-${process.pid}-${Date.now()}`,
    },
  }
}

function spawnDaemon(env) {
  const child = spawn(
    'cargo',
    ['run', '--quiet', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'],
    { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] },
  )
  child.logs = { stdout: '', stderr: '' }
  child.stdout.on('data', (chunk) => { child.logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { child.logs.stderr += chunk.toString() })
  return child
}

async function stopDaemon(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill('SIGTERM')
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    new Promise((resolve) => setTimeout(resolve, 5_000)),
  ])
  if (child.exitCode === null && child.signalCode === null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      new Promise((resolve) => setTimeout(resolve, 2_000)),
    ])
  }
}

function workflowOutput(summary, message) {
  return [
    '```json',
    JSON.stringify({ summary, output: { message } }, null, 2),
    '```',
  ].join('\n')
}

function nodeInstructions(index) {
  return [
    `You are node ${index + 1}.`,
    'Emit the exact workflow output block below and nothing else.',
    'Do not use any optional tools.',
    workflowOutput(`node ${index + 1} complete`, JSON.stringify({ node: index + 1 })),
  ].join('\n\n')
}

function summarizeWorkflowRuns(session, workflowId, endpointId) {
  return (session.workflow_runs ?? [])
    .filter((run) => run.workflow_id === workflowId && run.endpoint_id === endpointId)
    .map((run) => ({
      id: run.id,
      status: run.status,
      node_runs: (run.node_runs ?? []).map((nodeRun) => ({
        id: nodeRun.id,
        node_id: nodeRun.node_id,
        status: nodeRun.status,
      })),
    }))
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

  const artifactRoot = path.join(repoRoot, '.artifacts', 'workflow-scheduler', nowStamp())
  await rm(artifactRoot, { recursive: true, force: true }).catch(() => {})
  await mkdir(artifactRoot, { recursive: true })
  let daemonChild = null
  let kernelUrl = options.kernel
  let failure = null
  let succeeded = false
  if (options.spawnDaemon) {
    const spawned = deriveSpawnedKernelUrl(artifactRoot)
    kernelUrl = spawned.kernelUrl
    daemonChild = spawnDaemon(spawned.env)
  }
  const client = new LocalIpcClient(kernelUrl)
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
  const unwrap = (resp, key) => resp?.[key] ?? resp
  let session = null

  try {
    if (daemonChild) {
      let daemonReady = false
      for (let attempt = 0; attempt < 40; attempt += 1) {
        try {
          const probeClient = new LocalIpcClient(kernelUrl)
          const probeSession = unwrap(
            await probeClient.send(createSessionRequest(options.workspace, options.worktree)),
            'SessionCreated',
          ).session
          await probeClient.send(endSessionRequest(probeSession.id)).catch(() => {})
          await probeClient.close()
          daemonReady = true
          break
        } catch {
          await sleep(250)
        }
      }
      if (!daemonReady) throw new Error(`spawned workflow scheduler drill daemon did not become ready at ${kernelUrl}`)
    }
    session = unwrap(
      await client.send(createSessionRequest(options.workspace, options.worktree)),
      'SessionCreated',
    ).session
    await client.send(attachToSessionRequest(session.id, `workflow-scheduler-drill-${Date.now()}`))

    const agentIds = []
    const nodeIds = []
    for (let index = 0; index < options.providers.length; index += 1) {
      const provider = options.providers[index]
      const providerModel = modelForProvider(provider, options)
      const agent = unwrap(
        await client.send(
          spawnAgentRequest(
            session.id,
            provider,
            `schedule-${provider}-${index + 1}`,
            providerModel,
            options.worktree,
            'low',
          ),
        ),
        'AgentSpawned',
      ).agent
      agentIds.push(agent.id)
      await client.send(
        launchProviderRunRequest(session.id, provider, provider, providerModel, 'low', agent.id),
      )
    }

    const workflow = unwrap(
      await client.send(createWorkflowRequest(session.id, `schedule-${options.trigger}-${options.overlap}`)),
      'WorkflowCreated',
    ).workflow

    for (let index = 0; index < agentIds.length; index += 1) {
      const node = unwrap(
        await client.send(addWorkflowNodeRequest(session.id, workflow.id, agentIds[index])),
        'WorkflowNodeAdded',
      ).node
      nodeIds.push(node.id)
      await client.send(
        updateWorkflowNodeInstructionsRequest(session.id, workflow.id, node.id, nodeInstructions(index)),
      )
    }

    for (let index = 0; index < nodeIds.length - 1; index += 1) {
      await client.send(addWorkflowEdgeRequest(session.id, workflow.id, nodeIds[index], nodeIds[index + 1]))
    }

    const endpoint = unwrap(
      await client.send(createWorkflowEndpointRequest(session.id, workflow.id, nodeIds[0], `entry-${options.overlap}`)),
      'WorkflowEndpointCreated',
    ).endpoint

    const schedule = unwrap(
      await client.send(
        createWorkflowScheduleRequest(
          session.id,
          workflow.id,
          endpoint.id,
          scheduleTrigger(options),
          'Run the workflow exactly as instructed.',
          options.overlap,
        ),
      ),
      'WorkflowScheduleCreated',
    ).schedule

    let finalSession = null
    for (let attempt = 0; attempt < options.pollLimit; attempt += 1) {
      const sessionResponse = await client.send(getSessionStateRequest(session.id))
      const sessionState =
        sessionResponse?.SessionStateLoaded?.session
        ?? sessionResponse?.SessionState?.session
        ?? sessionResponse?.session
        ?? null
      if (!sessionState) {
        throw new Error(`session state response did not include a session snapshot: ${JSON.stringify(sessionResponse)}`)
      }
      const currentSchedule = (sessionState.workflow_schedules ?? sessionState.workflow_watchdogs ?? []).find((entry) => entry.id === schedule.id)
      const runs = summarizeWorkflowRuns(sessionState, workflow.id, endpoint.id)
      if (!currentSchedule) {
        throw new Error(`schedule ${schedule.id} disappeared from session state`)
      }

      const scheduleStarted =
        (currentSchedule.runs_started ?? currentSchedule.wakeups_executed ?? 0) >= 1
        || typeof currentSchedule.last_workflow_run_id === 'string'
        || runs.length >= 1
      if (scheduleStarted) {
        finalSession = sessionState
        break
      }
      await sleep(options.pollIntervalMs)
    }

    if (!finalSession) {
      throw new Error(`workflow scheduler drill timed out before ${options.trigger} schedule started a run`)
    }

    const finalSchedule = (finalSession.workflow_schedules ?? finalSession.workflow_watchdogs ?? []).find((entry) => entry.id === schedule.id)
    const workflowRuns = summarizeWorkflowRuns(finalSession, workflow.id, endpoint.id)
    console.log(JSON.stringify({
      kernel: kernelUrl,
      session: session.id,
      workflow: workflow.id,
      endpoint: endpoint.id,
      schedule: finalSchedule,
      workflowRuns,
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    try {
      if (session?.id) {
        await client.send(endSessionRequest(session.id)).catch(() => {})
      }
    } catch {}
    await client.close().catch(() => {})
    await stopDaemon(daemonChild)
    await finalizeDrillArtifacts({
      rootDir: artifactRoot,
      passed: succeeded,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: 'workflow-scheduler',
        kernelUrl,
        workspace: options.workspace,
        worktree: options.worktree,
        trigger: options.trigger,
        cron: options.trigger === 'cron' ? options.cron : null,
        timezone: options.trigger === 'cron' ? options.timezone : null,
        overlap: options.overlap,
        providers: options.providers,
        intervalSeconds: options.intervalSeconds,
        sessionId: session?.id ?? null,
        daemonStdoutTail: daemonChild?.logs?.stdout?.slice(-4000) ?? '',
        daemonStderrTail: daemonChild?.logs?.stderr?.slice(-4000) ?? '',
      },
      log: (name, details) => {
        if (details === undefined) console.log(`[workflow-scheduler-drill] ${name}`)
        else console.log(`[workflow-scheduler-drill] ${name}`, JSON.stringify(details))
      },
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
