import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath, pathToFileURL } from 'node:url'
import os from 'node:os'

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
  createWorkflowWatchdogRequest,
  getSessionStateRequest,
  invokeWorkflowEndpointRequest,
  launchProviderRunRequest,
  spawnAgentRequest,
  updateWorkflowNodeInstructionsRequest,
  endSessionRequest,
} = requests

const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
const DEFAULT_MODEL = 'gpt-5.4'
const DEFAULT_INTERVAL_SECONDS = 1
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
    intervalSeconds: DEFAULT_INTERVAL_SECONDS,
    policy: 'skip',
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
    else if (arg === '--interval-seconds') options.intervalSeconds = Number(argv[++index])
    else if (arg === '--policy') options.policy = argv[++index]
    else if (arg === '--providers') options.providers = argv[++index].split(',').map((v) => v.trim()).filter(Boolean)
    else if (arg === '--poll-limit') options.pollLimit = Number(argv[++index])
    else if (arg === '--poll-interval-ms') options.pollIntervalMs = Number(argv[++index])
    else if (arg === '--dry-run') options.dryRun = true
    else if (arg === '--spawn-daemon') options.spawnDaemon = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (!['skip', 'queue'].includes(options.policy)) {
    throw new Error(`unsupported watchdog policy: ${options.policy}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-watchdog-drill.mjs [options]',
    '',
    'Options:',
    `  --kernel ${DEFAULT_KERNEL}`,
    `  --workspace ${repoRoot}`,
    `  --worktree ${repoRoot}`,
    `  --model ${DEFAULT_MODEL}`,
    `  --interval-seconds ${DEFAULT_INTERVAL_SECONDS}`,
    '  --policy skip|queue',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    '  --poll-limit 90',
    '  --poll-interval-ms 1000',
    '  --dry-run',
    '  --spawn-daemon',
  ].join('\n'))
}

function deriveSpawnedKernelUrl() {
  const kernelPort = 45000 + Math.floor(Math.random() * 1000)
  const mcpPort = kernelPort + 1000
  const socketPath = path.join(os.tmpdir(), `arroba-watchdog-drill-${process.pid}-${Date.now()}.sock`)
  return {
    kernelUrl: `ws://127.0.0.1:${kernelPort}`,
    env: {
      ...process.env,
      ARROBA_KERNEL_PORT: String(kernelPort),
      ARROBA_MCP_PORT: String(mcpPort),
      ARROBA_DAEMON_SOCKET: socketPath,
      ARROBA_DAEMON_ID: `watchdog-drill-${process.pid}-${Date.now()}`,
    },
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

  let daemonChild = null
  let kernelUrl = options.kernel
  if (options.spawnDaemon) {
    const spawned = deriveSpawnedKernelUrl()
    kernelUrl = spawned.kernelUrl
    daemonChild = spawn(
      'cargo',
      ['run', '--quiet', '--manifest-path', path.join(repoRoot, 'apps/daemon/Cargo.toml'), '--bin', 'arroba-daemon'],
      { cwd: repoRoot, env: spawned.env, stdio: ['ignore', 'ignore', 'inherit'] },
    )
  }
  const client = new LocalIpcClient(kernelUrl)
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
  const unwrap = (resp, key) => resp?.[key] ?? resp
  let session = null

  try {
    if (daemonChild) {
      for (let attempt = 0; attempt < 40; attempt += 1) {
        try {
          const probeClient = new LocalIpcClient(kernelUrl)
          const probeSession = unwrap(
            await probeClient.send(createSessionRequest(options.workspace, options.worktree)),
            'SessionCreated',
          ).session
          await probeClient.send(endSessionRequest(probeSession.id)).catch(() => {})
          await probeClient.close()
          break
        } catch {
          await sleep(250)
        }
      }
    }
    session = unwrap(
      await client.send(createSessionRequest(options.workspace, options.worktree)),
      'SessionCreated',
    ).session
    await client.send(attachToSessionRequest(session.id, `watchdog-drill-${Date.now()}`))

    const agentIds = []
    const nodeIds = []
    for (let index = 0; index < options.providers.length; index += 1) {
      const provider = options.providers[index]
      const agent = unwrap(
        await client.send(
          spawnAgentRequest(
            session.id,
            provider,
            `watchdog-${provider}-${index + 1}`,
            options.model,
            options.worktree,
          ),
        ),
        'AgentSpawned',
      ).agent
      agentIds.push(agent.id)
      await client.send(
        launchProviderRunRequest(session.id, provider, provider, options.model, 'default', agent.id),
      )
    }

    const workflow = unwrap(
      await client.send(createWorkflowRequest(session.id, `watchdog-${options.policy}`)),
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
      await client.send(createWorkflowEndpointRequest(session.id, workflow.id, nodeIds[0], `entry-${options.policy}`)),
      'WorkflowEndpointCreated',
    ).endpoint

    const watchdog = unwrap(
      await client.send(
        createWorkflowWatchdogRequest(
          session.id,
          workflow.id,
          endpoint.id,
          options.intervalSeconds,
          'Run the workflow exactly as instructed.',
          options.policy,
        ),
      ),
      'WorkflowWatchdogCreated',
    ).watchdog

    await client.send(
      invokeWorkflowEndpointRequest(
        session.id,
        workflow.id,
        endpoint.id,
        'Run the workflow exactly as instructed.',
      ),
    )

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
      const currentWatchdog = (sessionState.workflow_watchdogs ?? []).find((entry) => entry.id === watchdog.id)
      const runs = summarizeWorkflowRuns(sessionState, workflow.id, endpoint.id)
      if (!currentWatchdog) {
        throw new Error(`watchdog ${watchdog.id} disappeared from session state`)
      }

      const queueSatisfied =
        options.policy === 'queue'
          ? currentWatchdog.pending_run === true
            || (runs.length >= 2 && runs.some((run) => run.id === currentWatchdog.last_workflow_run_id))
          : currentWatchdog.last_status === 'skipped_running' && runs.length === 1
      if (queueSatisfied) {
        finalSession = sessionState
        break
      }
      await sleep(options.pollIntervalMs)
    }

    if (!finalSession) {
      throw new Error(`watchdog drill timed out for policy ${options.policy}`)
    }

    const finalWatchdog = (finalSession.workflow_watchdogs ?? []).find((entry) => entry.id === watchdog.id)
    const workflowRuns = summarizeWorkflowRuns(finalSession, workflow.id, endpoint.id)
    console.log(JSON.stringify({
      kernel: kernelUrl,
      session: session.id,
      workflow: workflow.id,
      endpoint: endpoint.id,
      watchdog: finalWatchdog,
      workflowRuns,
    }, null, 2))
  } finally {
    try {
      if (session?.id) {
        await client.send(endSessionRequest(session.id)).catch(() => {})
      }
    } catch {}
    await client.close().catch(() => {})
    if (daemonChild) {
      daemonChild.kill('SIGTERM')
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
