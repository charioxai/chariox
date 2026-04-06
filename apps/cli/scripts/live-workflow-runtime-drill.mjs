import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const distIpcUrl = pathToFileURL(path.join(cliRoot, 'dist', 'ipc.js')).href
const distRequestsUrl = pathToFileURL(path.join(cliRoot, 'dist', 'ipc-requests.js')).href
const { LocalIpcClient } = await import(distIpcUrl)
const requests = await import(distRequestsUrl)

const {
  createSessionRequest,
  attachToSessionRequest,
  createWorkflowRequest,
  addWorkflowNodeRequest,
  addWorkflowEdgeRequest,
  updateWorkflowNodeInstructionsRequest,
  createWorkflowEndpointRequest,
  invokeWorkflowEndpointRequest,
  getSessionStateRequest,
  spawnAgentRequest,
  launchProviderRunRequest,
} = requests

const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
const DEFAULT_WORKSPACE = repoRoot
const DEFAULT_WORKTREE = repoRoot
const DEFAULT_MODEL = 'gpt-5.4'
const DEFAULT_PROVIDERS = ['opencode', 'codex', 'opencode', 'codex', 'opencode', 'codex']
const DEFAULT_NUMBERS = ['1842', '7315', '4068', '5921', '8473', '2604']

function parseArgs(argv) {
  const options = {
    scenario: 'simple-chain',
    kernel: DEFAULT_KERNEL,
    workspace: DEFAULT_WORKSPACE,
    worktree: DEFAULT_WORKTREE,
    model: DEFAULT_MODEL,
    providers: DEFAULT_PROVIDERS,
    pollLimit: 120,
    pollIntervalMs: 2000,
    dryRun: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--scenario') options.scenario = argv[++index]
    else if (arg === '--kernel') options.kernel = argv[++index]
    else if (arg === '--workspace') options.workspace = argv[++index]
    else if (arg === '--worktree') options.worktree = argv[++index]
    else if (arg === '--model') options.model = argv[++index]
    else if (arg === '--providers') options.providers = argv[++index].split(',').map((v) => v.trim()).filter(Boolean)
    else if (arg === '--poll-limit') options.pollLimit = Number(argv[++index])
    else if (arg === '--poll-interval-ms') options.pollIntervalMs = Number(argv[++index])
    else if (arg === '--dry-run') options.dryRun = true
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
    '  --scenario simple-chain|validated-increment-chain|console-increment-chain',
    `  --kernel ${DEFAULT_KERNEL}`,
    `  --workspace ${DEFAULT_WORKSPACE}`,
    `  --worktree ${DEFAULT_WORKTREE}`,
    `  --model ${DEFAULT_MODEL}`,
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    '  --poll-limit 120',
    '  --poll-interval-ms 2000',
    '  --dry-run',
  ].join('\n'))
}

function workflowOutput(summary, messageJson) {
  return [
    '```json',
    JSON.stringify({ summary, output: { message: messageJson } }, null, 2),
    '```',
  ].join('\n')
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

function buildSimpleChainScenario(providers, model) {
  return {
    id: 'simple-chain',
    alias: 'simple-gpt54',
    providers,
    model,
    nodePrompt(index) {
      const value = DEFAULT_NUMBERS[index] ?? String(1842 + index)
      return [
        'Your only job is to emit the exact workflow output block below and nothing else.',
        'Do not use any tools unless required by the runtime.',
        workflowOutput(`emit ${value}`, JSON.stringify({ number: value, label: `node-${index + 1}` })),
      ].join('\n\n')
    },
    edgeRequest(sessionId, workflowId, fromNodeId, toNodeId) {
      return addWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId)
    },
  }
}

function buildValidatedIncrementScenario(providers, model, schemaPath) {
  return {
    id: 'validated-increment-chain',
    alias: 'value-chain',
    providers,
    model,
    nodePrompt(index) {
      if (index === 0) {
        return [
          'Produce output.message JSON with a single integer field `value`.',
          'Send exactly value 1842.',
          'Do not add any other fields.',
          'Your summary should be `sent 1842`.',
        ].join('\n\n')
      }
      return [
        'Read the upstream handoff payload for this workflow turn.',
        'Extract output.message JSON from the previous node.',
        'Read its integer field `value`.',
        'Add 1 to that integer.',
        'Produce output.message JSON with exactly one field: `value` set to the incremented integer.',
        'Do not add any other fields.',
        'Your summary should say `received X, sent Y`.',
      ].join('\n\n')
    },
    edgeRequest(sessionId, workflowId, fromNodeId, toNodeId) {
      return {
        AddWorkflowEdge: {
          session_id: sessionId,
          workflow_ref: workflowId,
          from_node_id: fromNodeId,
          to_node_id: toNodeId,
          output_schema_ref: schemaPath,
          validation_policy: 'warn',
        },
      }
    },
  }
}

function buildConsoleIncrementScenario(providers, model) {
  return {
    id: 'console-increment-chain',
    alias: 'console-chain',
    providers,
    model,
    nodePrompt(index) {
      if (index === 0) {
        return [
          'Use the workflow console MCP tools for this task.',
          'Call `workflow_console_write` exactly once.',
          'Write exactly `1842\\n` to the workflow console.',
          'Each number must be on its own line. Include the trailing newline in the write payload.',
          'Do not call `validate_workflow_output` for this task unless the runtime explicitly requires it.',
          'After the write succeeds, emit the normal workflow output block.',
        ].join('\n\n')
      }
      return [
        'Use the workflow console MCP tools for this task.',
        'Call `workflow_console_read` to read the current workflow console contents.',
        'Take the last non-empty line from the console.',
        'Interpret that last non-empty line as a base-10 integer.',
        'Add 1 to that integer.',
        'Call `workflow_console_write` exactly once to append the incremented integer followed by a newline.',
        'Each number must be on its own line. Include the trailing newline in the write payload.',
        'Do not write any extra prose to the console.',
        'Do not call `validate_workflow_output` for this task unless the runtime explicitly requires it.',
        'After the write succeeds, emit the normal workflow output block.',
      ].join('\n\n')
    },
    edgeRequest(sessionId, workflowId, fromNodeId, toNodeId) {
      return addWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId)
    },
  }
}

function createScenario(options, schemaPath) {
  if (options.scenario === 'simple-chain') return buildSimpleChainScenario(options.providers, options.model)
  if (options.scenario === 'validated-increment-chain') {
    return buildValidatedIncrementScenario(options.providers, options.model, schemaPath)
  }
  if (options.scenario === 'console-increment-chain') {
    return buildConsoleIncrementScenario(options.providers, options.model)
  }
  throw new Error(`unsupported scenario: ${options.scenario}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
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

  const client = new LocalIpcClient(options.kernel)
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
  const unwrap = (resp, key) => resp?.[key] ?? resp
  const logStep = (name, details = null) => {
    const prefix = `[drill] ${name}`
    if (details == null) console.log(prefix)
    else console.log(prefix, JSON.stringify(details))
  }

  try {
    logStep('create_session')
    const session = unwrap(await client.send(createSessionRequest(options.workspace, options.worktree)), 'SessionCreated').session
    await client.send(attachToSessionRequest(session.id, `live-drill-${Date.now()}`))

    const agentIds = []
    const nodeIds = []
    for (let index = 0; index < scenario.providers.length; index += 1) {
      const provider = scenario.providers[index]
      logStep('spawn_agent', { index, provider })
      const agent = unwrap(
        await client.send(spawnAgentRequest(session.id, provider, `a${index + 1}`, scenario.model, options.worktree, 'medium')),
        'AgentSpawned',
      ).agent
      agentIds.push(agent.id)
      logStep('launch_provider_run', { index, provider, agentId: agent.id })
      await client.send(launchProviderRunRequest(session.id, provider, 'default', scenario.model, 'medium', agent.id))
    }

    logStep('create_workflow', { alias: scenario.alias })
    const workflow = unwrap(await client.send(createWorkflowRequest(session.id, scenario.alias)), 'WorkflowCreated').workflow
    for (let index = 0; index < scenario.providers.length; index += 1) {
      const provider = scenario.providers[index]
      logStep('add_node', { index, provider })
      const node = unwrap(await client.send(addWorkflowNodeRequest(session.id, workflow.id, agentIds[index])), 'WorkflowNodeAdded').node
      nodeIds.push(node.id)
      logStep('update_node_instructions', { index, nodeId: node.id })
      await client.send(updateWorkflowNodeInstructionsRequest(session.id, workflow.id, node.id, scenario.nodePrompt(index)))
      if (index > 0) {
        const edgeRequest = scenario.edgeRequest(session.id, workflow.id, nodeIds[index - 1], nodeIds[index])
        if (edgeRequest) {
          logStep('add_edge', { fromNodeId: nodeIds[index - 1], toNodeId: nodeIds[index] })
          await client.send(edgeRequest)
        }
      }
    }

    logStep('create_endpoint', { entryNodeId: nodeIds[0] })
    const endpoint = unwrap(
      await client.send(createWorkflowEndpointRequest(session.id, workflow.id, nodeIds[0], 'start')),
      'WorkflowEndpointCreated',
    ).endpoint

    logStep('invoke', { endpointId: endpoint.id })
    const workflowRun = unwrap(
      await client.send(invokeWorkflowEndpointRequest(session.id, workflow.id, endpoint.id, 'Run the workflow exactly as instructed.')),
      'WorkflowRunInvoked',
    ).workflow_run

    for (let index = 0; index < options.pollLimit; index += 1) {
      await sleep(options.pollIntervalMs)
      const stateResp = await client.send(getSessionStateRequest(session.id))
      const state = unwrap(stateResp, 'SessionStateLoaded')?.session ?? unwrap(stateResp, 'SessionState')?.session
      const run = (state.workflow_runs || []).find((entry) => entry.id === workflowRun.id)
      if (run && ['Completed', 'Failed', 'Stopped'].includes(run.status)) {
        console.log(JSON.stringify({
          sessionId: session.id,
          workflowId: workflow.id,
          workflowRunId: workflowRun.id,
          status: run.status,
          consoleEntries: run.console?.entries || [],
          nodeRuns: run.node_runs.map((nodeRun) => ({
            id: nodeRun.id,
            status: nodeRun.status,
            summary: nodeRun.summary,
            completion: nodeRun.completion,
            tools: nodeRun.turn_envelope?.runtime_tool_calls || [],
          })),
          failureEvents: run.failure_events || [],
        }, null, 2))
        await client.close()
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
      consoleEntries: run?.console?.entries || [],
      nodeRuns: run?.node_runs?.map((nodeRun) => ({
        id: nodeRun.id,
        status: nodeRun.status,
        summary: nodeRun.summary,
        completion: nodeRun.completion,
        tools: nodeRun.turn_envelope?.runtime_tool_calls || [],
      })),
      failureEvents: run?.failure_events || [],
    }, null, 2))
    await client.close()
    process.exitCode = 1
  } catch (error) {
    console.error(error)
    try { await client.close() } catch {}
    process.exitCode = 1
  }
}

await main()
