import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath, pathToFileURL } from 'node:url'
import os from 'node:os'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

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
  addWorkflowEdgeRequest,
  updateWorkflowNodeInstructionsRequest,
  createWorkflowEndpointRequest,
  invokeWorkflowEndpointRequest,
  getSessionStateRequest,
  pumpTerminalOutputRequest,
  getProviderRunRequest,
  spawnAgentRequest,
  launchProviderRunRequest,
  listProviderProcessesRequest,
  setWorkflowNodeCanCompleteRunRequest,
  setWorkflowNodeCanEmitIntermediateOutputRequest,
  setWorkflowNodeIntermediateOutputSchemaRequest,
  setWorkflowIntermediateOutputSchemaRequest,
  setWorkflowRunOutputSchemaRequest,
  setWorkflowFlushContextRequest,
  endSessionRequest,
  installMcpServerRequest,
  grantAgentExtensionRequest,
} = requests

const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
const DEFAULT_WORKSPACE = repoRoot
const DEFAULT_WORKTREE = repoRoot
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_PROVIDERS = ['opencode', 'codex', 'opencode', 'codex', 'opencode', 'codex']
const DEFAULT_NUMBERS = ['1842', '7315', '4068', '5921', '8473', '2604']

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

function workflowOutput(summary, messageJson) {
  return [
    '```json',
    JSON.stringify({ summary, output: { message: messageJson } }, null, 2),
    '```',
  ].join('\n')
}

function addValidatedWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId, schemaPath) {
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


async function createWorkflowEchoMcp(rootDir) {
  const mcpPath = path.join(rootDir, 'workflow-echo-mcp.mjs')
  await mkdir(rootDir, { recursive: true })
  await writeFile(mcpPath, [
    "let buffer = Buffer.alloc(0)",
    "function write(message) {",
    "  const body = JSON.stringify(message)",
    "  process.stdout.write(`${body}\\n`)",
    "}",
    "function handle(message) {",
    "  const { id, method, params } = message",
    "  if (method === 'notifications/initialized') return",
    "  if (method === 'initialize') {",
    "    write({ jsonrpc: '2.0', id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'arroba-workflow-echo', version: '1.0.0' } } })",
    "    return",
    "  }",
    "  if (method === 'tools/list') {",
    "    write({ jsonrpc: '2.0', id, result: { tools: [{ name: 'echo_marker', description: 'Echoes a marker for Arroba workflow MCP drills.', inputSchema: { type: 'object', properties: { marker: { type: 'string' } }, required: ['marker'] } }] } })",
    "    return",
    "  }",
    "  if (method === 'tools/call' && params?.name === 'echo_marker') {",
    "    write({ jsonrpc: '2.0', id, result: { content: [{ type: 'text', text: `ECHO:${params?.arguments?.marker ?? ''}` }] } })",
    "    return",
    "  }",
    "  write({ jsonrpc: '2.0', id, error: { code: -32601, message: `unknown method ${method}` } })",
    "}",
    "process.stdin.on('data', (chunk) => {",
    "  buffer = Buffer.concat([buffer, chunk])",
    "  while (true) {",
    "    const newline = buffer.indexOf('\\n')",
    "    if (newline >= 0) {",
    "      const line = buffer.subarray(0, newline).toString('utf8').trim()",
    "      buffer = buffer.subarray(newline + 1)",
    "      if (line) handle(JSON.parse(line))",
    "      continue",
    "    }",
    "    const headerEnd = buffer.indexOf('\\r\\n\\r\\n')",
    "    if (headerEnd < 0) return",
    "    const header = buffer.subarray(0, headerEnd).toString('utf8')",
    "    const match = /^content-length:\\s*(\\d+)$/im.exec(header)",
    "    if (!match) throw new Error(`missing Content-Length: ${header}`)",
    "    const length = Number(match[1])",
    "    const bodyStart = headerEnd + 4",
    "    const frameEnd = bodyStart + length",
    "    if (buffer.length < frameEnd) return",
    "    const message = JSON.parse(buffer.subarray(bodyStart, frameEnd).toString('utf8'))",
    "    buffer = buffer.subarray(frameEnd)",
    "    handle(message)",
    "  }",
    "})",
  ].join('\n'), 'utf8')
  return mcpPath
}

function workflowEchoMcpConfig(mcpPath) {
  return {
    name: 'workflow_echo',
    transport: { type: 'stdio', command: 'node', args: [mcpPath], env: {}, env_vars: [] },
    enabled: true,
    required: false,
    startup_timeout_sec: 5,
    tool_timeout_sec: 10,
  }
}

async function resolveBinary(binaryPath, manifestPath, binName) {
  try {
    await import('node:fs/promises').then(({ access }) => access(binaryPath))
    return binaryPath
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
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
      return addValidatedWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId, schemaPath)
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
          'Set `output.message` to JSON with exactly one integer field: `value` set to 1842.',
          'Your summary should be `wrote 1842`.',
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
        'Set `output.message` to JSON with exactly one integer field: `value` set to the incremented integer.',
        'Your summary should say `read X, wrote Y`.',
      ].join('\n\n')
    },
    edgeRequest(sessionId, workflowId, fromNodeId, toNodeId) {
      return addWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId)
    },
  }
}

function buildFinalRunOutputScenario(providers, model, schemaPath) {
  if (providers.length !== 2) {
    throw new Error('final-run-output-chain requires exactly 2 providers')
  }
  return {
    id: 'final-run-output-chain',
    alias: 'final-run-output-chain',
    providers,
    model,
    runOutputSchemaPath: schemaPath,
    entryPrompt: 'Start the workflow with integer 1842. The workflow should return the incremented final result.',
    nodePrompt(index) {
      if (index === 0) {
        return [
          'Read the endpoint prompt for the starting integer.',
          'Produce normal node-to-node workflow output for the downstream node.',
          'Set `output.message` to JSON with exactly one integer field: `value`.',
          'Use the integer from the endpoint prompt unchanged.',
          'Do not add any other fields.',
          'Your summary should be `sent 1842`.',
          workflowOutput('sent 1842', JSON.stringify({ value: 1842 })),
        ].join('\n\n')
      }
      return [
        'Read the upstream handoff payload for this workflow turn.',
        'Extract `output.message` JSON from the previous node.',
        'Read its integer field `value`.',
        'Add 1 to that integer.',
        'This node is the final workflow node. Generate final workflow run output JSON with exactly one integer field: `value` set to the incremented integer.',
        'Do not generate normal node-to-node output for this final result.',
        'Use the runtime MCP tool for final workflow run output submission and then finish the turn.',
        'Your summary should be `received 1842, completed 1843`.',
      ].join('\n\n')
    },
    edgeRequest(sessionId, workflowId, fromNodeId, toNodeId) {
      return addValidatedWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId, schemaPath)
    },
    async configureWorkflow(client, sessionId, workflowId, nodeIds) {
      await client.send(setWorkflowRunOutputSchemaRequest(sessionId, workflowId, schemaPath))
      await client.send(setWorkflowNodeCanCompleteRunRequest(sessionId, workflowId, nodeIds[1], true))
    },
  }
}

function buildCyclicFinalRunOutputScenario(providers, model, schemaPath) {
  if (providers.length !== 2) {
    throw new Error('cyclic-final-run-output-chain requires exactly 2 providers')
  }
  const original = 1842
  const threshold = original + 1
  return {
    id: 'cyclic-final-run-output-chain',
    alias: 'cyclic-final-run-output-chain',
    providers,
    model,
    runOutputSchemaPath: schemaPath,
    entryPrompt: `Start the workflow with original integer ${original}. Stop when the value reaches ${threshold} and return that final result.`,
    nodePrompt(index) {
      if (index === 0) {
        return [
          `The original number for this workflow is ${original}.`,
          `If this is your first turn and there is no upstream handoff payload, generate normal node-to-node output with JSON {"value":${original}}.`,
          'On later turns, read the upstream handoff payload and extract `output.message` JSON with integer field `value`.',
          `If that value is smaller than ${threshold}, add 1 and forward it as normal node-to-node output JSON with exactly one integer field: \`value\`.`,
          `If that value is ${threshold} or greater, complete the workflow run and submit final workflow run output JSON with exactly one integer field: \`value\` set to that received value.`,
          'When you are generating final workflow run output, do not generate normal node-to-node output.',
          `Use summaries like \`started ${original}\`, \`forwarded X\`, or \`completed ${threshold}\`.`,
          workflowOutput(`started ${original}`, JSON.stringify({ value: original })),
        ].join('\n\n')
      }
      return [
        'Read the upstream handoff payload for this workflow turn.',
        'Extract `output.message` JSON from the previous node.',
        'Read its integer field `value`.',
        'Add 1 to that integer.',
        'Produce normal node-to-node workflow output JSON with exactly one integer field: `value` set to the incremented integer.',
        'Do not add any other fields.',
        'Your summary should say `received X, sent Y`.',
      ].join('\n\n')
    },
    edgeRequest(sessionId, workflowId, fromNodeId, toNodeId) {
      return addValidatedWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId, schemaPath)
    },
    async configureWorkflow(client, sessionId, workflowId, nodeIds) {
      await client.send(setWorkflowRunOutputSchemaRequest(sessionId, workflowId, schemaPath))
      await client.send(setWorkflowNodeCanCompleteRunRequest(sessionId, workflowId, nodeIds[0], true))
    },
    extraEdges(nodeIds) {
      return [[nodeIds[1], nodeIds[0]]]
    },
  }
}

function buildCyclicBudgetedFinalRunOutputScenario(providers, model, schemaPath) {
  if (providers.length !== 2) {
    throw new Error('cyclic-budgeted-final-run-output-chain requires exactly 2 providers')
  }
  const original = 1842
  const aMaxTurns = 2
  return {
    id: 'cyclic-budgeted-final-run-output-chain',
    alias: 'cyclic-budgeted-final-run-output-chain',
    providers,
    model,
    runOutputSchemaPath: schemaPath,
    entryPrompt: `Start the workflow with original integer ${original}. Node A must stop on its last allowed turn and return the current value as final output.`,
    nodePrompt(index) {
      if (index === 0) {
        return [
          `The original number for this workflow is ${original}.`,
          `If this is your first turn and there is no upstream handoff payload, generate normal node-to-node output with JSON {"value":${original}}.`,
          'On later turns, read the upstream handoff payload and extract `output.message` JSON with integer field `value`.',
          `On later turns only, if this is not your last allowed turn, add 1 and forward it as normal node-to-node output JSON with exactly one integer field: \`value\`.`,
          'If this is your last allowed turn, do not forward. Instead, submit final workflow run output JSON with exactly one integer field: `value` set to the received current value.',
          'When you are generating final workflow run output, do not generate normal node-to-node output.',
          `Use summaries like \`started ${original}\`, \`forwarded X\`, or \`completed X\`.`,
          workflowOutput(`started ${original}`, JSON.stringify({ value: original })),
        ].join('\n\n')
      }
      return [
        'Read the upstream handoff payload for this workflow turn.',
        'Extract `output.message` JSON from the previous node.',
        'Read its integer field `value`.',
        'Add 1 to that integer.',
        'Produce normal node-to-node workflow output JSON with exactly one integer field: `value` set to the incremented integer.',
        'Do not add any other fields.',
        'Your summary should say `received X, sent Y`.',
      ].join('\n\n')
    },
    edgeRequest(sessionId, workflowId, fromNodeId, toNodeId) {
      return addValidatedWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId, schemaPath)
    },
    async configureWorkflow(client, sessionId, workflowId, nodeIds) {
      await client.send(setWorkflowRunOutputSchemaRequest(sessionId, workflowId, schemaPath))
      await client.send(setWorkflowNodeCanCompleteRunRequest(sessionId, workflowId, nodeIds[0], true))
      await client.send(requests.setWorkflowNodeMaxTurnsRequest(sessionId, workflowId, nodeIds[0], aMaxTurns))
    },
    expectedFinalOutput: JSON.stringify({ value: original + 1 }),
    expectedFailureEventKindsWhenCompleted: ['missing_structured_output'],
    extraEdges(nodeIds) {
      return [[nodeIds[1], nodeIds[0]]]
    },
  }
}

function buildCyclicFinalRunWithIntermediateOutputScenario(providers, model, schemaPath) {
  if (providers.length !== 2) {
    throw new Error('cyclic-final-run-with-intermediate-output-chain requires exactly 2 providers')
  }
  const original = 1842
  const threshold = original + 1
  return {
    id: 'cyclic-final-run-with-intermediate-output-chain',
    alias: 'cyclic-final-run-with-intermediate-output-chain',
    providers,
    model,
    runOutputSchemaPath: schemaPath,
    entryPrompt: `Start the workflow with original integer ${original}. Stop when the value reaches ${threshold} and return that final result. Each node should also send its computed number to the endpoint as intermediate workflow output.`,
    nodePrompt(index) {
      if (index === 0) {
        return [
          `The original number for this workflow is ${original}.`,
          `If this is your first turn and there is no upstream handoff payload, your computed number is ${original}. Submit that computed number as intermediate workflow run output JSON with exactly one integer field: \`value\`. Then generate normal node-to-node output with JSON {"value":${original}}.`,
          'On later turns, read the upstream handoff payload and extract `output.message` JSON with integer field `value`.',
          `If that value is smaller than ${threshold}, add 1. The incremented integer is your computed number for this turn. Submit that computed number as intermediate workflow run output JSON with exactly one integer field: \`value\`. Then forward it as normal node-to-node output JSON with exactly one integer field: \`value\`.`,
          `If that value is ${threshold} or greater, the received value is your computed number for this turn. Submit that computed number as intermediate workflow run output JSON with exactly one integer field: \`value\`. Then submit final workflow run output JSON with exactly one integer field: \`value\` set to that same received value.`,
          'When you are generating final workflow run output, do not generate normal node-to-node output.',
          `Use summaries like \`started ${original}\`, \`forwarded X\`, or \`completed ${threshold}\`.`,
          workflowOutput(`started ${original}`, JSON.stringify({ value: original })),
        ].join('\n\n')
      }
      return [
        'Read the upstream handoff payload for this workflow turn.',
        'Extract `output.message` JSON from the previous node.',
        'Read its integer field `value`.',
        'Add 1 to that integer. The incremented integer is your computed number for this turn.',
        'Submit that computed number as intermediate workflow run output JSON with exactly one integer field: `value`.',
        `If the computed number is ${threshold} or greater, submit final workflow run output JSON with exactly one integer field: \`value\` set to the computed number. Do not generate normal node-to-node output in that case.`,
        `If the computed number is smaller than ${threshold}, produce normal node-to-node workflow output JSON with exactly one integer field: \`value\` set to the computed number.`,
        'Do not add any other fields.',
        `Your summary should say \`received X, completed ${threshold}\` when you submit final workflow run output, otherwise \`received X, sent Y\`.`,
      ].join('\n\n')
    },
    edgeRequest(sessionId, workflowId, fromNodeId, toNodeId) {
      return addValidatedWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId, schemaPath)
    },
    async configureWorkflow(client, sessionId, workflowId, nodeIds) {
      await client.send(setWorkflowRunOutputSchemaRequest(sessionId, workflowId, schemaPath))
      await client.send(setWorkflowIntermediateOutputSchemaRequest(sessionId, workflowId, schemaPath))
      await client.send(setWorkflowNodeCanCompleteRunRequest(sessionId, workflowId, nodeIds[0], true))
      await client.send(setWorkflowNodeCanCompleteRunRequest(sessionId, workflowId, nodeIds[1], true))
      await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(sessionId, workflowId, nodeIds[0], true))
      await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(sessionId, workflowId, nodeIds[1], true))
      await client.send(setWorkflowNodeIntermediateOutputSchemaRequest(sessionId, workflowId, nodeIds[0], schemaPath))
      await client.send(setWorkflowNodeIntermediateOutputSchemaRequest(sessionId, workflowId, nodeIds[1], schemaPath))
    },
    extraEdges(nodeIds) {
      return [[nodeIds[1], nodeIds[0]]]
    },
  }
}

function buildConditionalBranchSubsetScenario(providers, model) {
  if (providers.length !== 3) {
    throw new Error('conditional-branch-subset requires exactly 3 providers')
  }
  return {
    id: 'conditional-branch-subset',
    alias: 'conditional-branch-subset',
    providers,
    model,
    autoChainEdges: false,
    entryPrompt: 'Route the configured numbers into the selected downstream branches.',
    nodePrompt(index) {
      if (index === 0) {
        return [
          'Wait for the daemon-managed node instructions to be updated with concrete target node ids.',
          'Then emit the exact workflow output block from those instructions.',
        ].join('\n\n')
      }
      if (index === 1) {
        return [
          'Read the upstream handoff payload for this workflow turn.',
          'Extract `output.message` JSON.',
          'It must have `bucket` equal to `even` and `values` equal to `[2]`.',
          'Emit normal workflow output.message JSON with exactly `{"bucket":"even","values":[2]}`.',
          'Your summary should be `even branch received 1 value`.',
          workflowOutput('even branch received 1 value', JSON.stringify({ bucket: 'even', values: [2] })),
        ].join('\n\n')
      }
      if (index === 2) {
        return [
          'This branch should not receive a handoff in this drill.',
          'If you are invoked, emit output.message JSON with exactly `{"bucket":"large","values":[]}`.',
          workflowOutput('large branch should not run', JSON.stringify({ bucket: 'large', values: [] })),
        ].join('\n\n')
      }
      throw new Error(`unexpected conditional branch node index ${index}`)
    },
    edgeRequest(sessionId, workflowId, fromNodeId, toNodeId) {
      return addWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId)
    },
    extraEdges(nodeIds) {
      return [
        [nodeIds[0], nodeIds[1]],
        [nodeIds[0], nodeIds[2]],
      ]
    },
    async configureWorkflow(client, sessionId, workflowId, nodeIds) {
      const routedOutput = {
        workflow_handoffs: [
          {
            to_node_id: nodeIds[1],
            summary: 'selected even values',
            message: { bucket: 'even', values: [2] },
          },
        ],
      }
      await client.send(updateWorkflowNodeInstructionsRequest(sessionId, workflowId, nodeIds[0], [
        'You are the router node for a conditional branching workflow.',
        'The input numbers are exactly `[1,2,3]`.',
        `Send even values only to target node \`${nodeIds[1]}\`.`,
        `Do not send any handoff to target node \`${nodeIds[2]}\` because no value is >= 8.`,
        'Emit exactly this final fenced json block and nothing else:',
        workflowOutput('routed 1 selected branch', JSON.stringify(routedOutput)),
      ].join('\n\n')))
    },
    assertResult(result, { nodeIds }) {
      const byNodeId = new Map(result.nodeRuns.map((nodeRun) => [nodeRun.nodeId, nodeRun]))
      const evenRun = byNodeId.get(nodeIds[1])
      const excludedRun = byNodeId.get(nodeIds[2])
      if (!evenRun || evenRun.status !== 'Completed') {
        throw new Error('conditional branch drill did not complete the even branch')
      }
      if (excludedRun) {
        throw new Error('conditional branch drill invoked the excluded large-value branch')
      }
      const evenOutput = evenRun.completion?.output?.message
      if (evenOutput !== JSON.stringify({ bucket: 'even', values: [2] })) {
        throw new Error(`conditional branch even output mismatch: ${evenOutput}`)
      }
    },
  }
}

function buildImmediateReleaseDownstreamScenario(providers, model, schemaPath) {
  if (providers.length !== 2) {
    throw new Error('immediate-release-downstream requires exactly 2 providers')
  }
  return {
    id: 'immediate-release-downstream',
    alias: 'immediate-release-downstream',
    providers,
    model,
    autoChainEdges: true,
    entryPrompt: 'Start the producer. The consumer must receive the producer intermediate output before the producer turn completes.',
    agentWorktree(index, options) {
      return path.join(options.worktree, 'tmp', 'live-drills', 'immediate-release-worktrees', `agent-${index + 1}`)
    },
    nodePrompt(index) {
      if (index === 0) {
        return [
          'This is an async/immediate-release workflow drill.',
          'First, acknowledge the workflow turn as required by the runtime.',
          'Then call `validate_and_submit_intermediate_workflow_run_output` with workflow output JSON exactly `{"value":1842}`.',
          'After that tool returns valid, you do not need to produce normal node-to-node output for this drill.',
          'Do not call final workflow run output tools.',
          'Your summary should be `submitted intermediate 1842`.',
        ].join('\n\n')
      }
      return [
        'Read the upstream handoff payload for this workflow turn.',
        'Extract `output.message` JSON.',
        'It must be exactly `{"value":1842}` from the producer intermediate output, not the producer final normal output.',
        'Add 1 and submit final workflow run output JSON exactly `{"value":1843}`.',
        'Do not generate normal node-to-node output.',
        'Your summary should be `received immediate 1842, completed 1843`.',
      ].join('\n\n')
    },
    edgeRequest(sessionId, workflowId, fromNodeId, toNodeId) {
      return addWorkflowEdgeRequest(sessionId, workflowId, fromNodeId, toNodeId)
    },
    async configureWorkflow(client, sessionId, workflowId, nodeIds) {
      await client.send(setWorkflowRunOutputSchemaRequest(sessionId, workflowId, schemaPath))
      await client.send(setWorkflowIntermediateOutputSchemaRequest(sessionId, workflowId, schemaPath))
      await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(sessionId, workflowId, nodeIds[0], true))
      await client.send(setWorkflowNodeIntermediateOutputSchemaRequest(sessionId, workflowId, nodeIds[0], schemaPath))
      await client.send(setWorkflowNodeCanCompleteRunRequest(sessionId, workflowId, nodeIds[1], true))
    },
    assertResult(result, { nodeIds }) {
      const producerRun = result.nodeRuns.find((nodeRun) => nodeRun.nodeId === nodeIds[0])
      const consumerRun = result.nodeRuns.find((nodeRun) => nodeRun.nodeId === nodeIds[1])
      if (!producerRun || !consumerRun) {
        throw new Error('immediate release drill did not create both producer and consumer node runs')
      }
      if (producerRun.completedAtMs != null && consumerRun.createdAtMs >= producerRun.completedAtMs) {
        throw new Error(
          `immediate release drill did not create consumer before producer completion: consumer created ${consumerRun.createdAtMs}, producer completed ${producerRun.completedAtMs}`,
        )
      }
      const intermediate = result.intermediateOutputs.find((output) => output.source_node_run_id === producerRun.id)
      if (intermediate?.output?.message !== JSON.stringify({ value: 1842 })) {
        throw new Error(`immediate release drill intermediate output mismatch: ${intermediate?.output?.message}`)
      }
      if (result.finalOutput?.message !== JSON.stringify({ value: 1843 })) {
        throw new Error(`immediate release drill final output mismatch: ${result.finalOutput?.message}`)
      }
    },
    assertEarlyResult(result, { nodeIds }) {
      const producerRun = result.nodeRuns.find((nodeRun) => nodeRun.nodeId === nodeIds[0])
      const consumerRun = result.nodeRuns.find((nodeRun) => nodeRun.nodeId === nodeIds[1])
      if (!producerRun || !consumerRun) return false
      const intermediate = result.intermediateOutputs.find((output) => output.source_node_run_id === producerRun.id)
      if (intermediate?.output?.message !== JSON.stringify({ value: 1842 })) return false
      if (consumerRun.createdAtMs < producerRun.startedAtMs) return false
      if (producerRun.completedAtMs != null && consumerRun.createdAtMs >= producerRun.completedAtMs) return false
      return true
    },
  }
}


function buildMcpEchoWorkflowScenario(providers, model) {
  if (providers.length !== 1) {
    throw new Error('mcp-echo-workflow requires exactly 1 provider')
  }
  const markerFile = `workflow-echo-mcp-${process.pid}-${Date.now()}.txt`
  return {
    id: 'mcp-echo-workflow',
    alias: 'mcp-echo-workflow',
    providers,
    model,
    autoChainEdges: false,
    entryPrompt: 'Use the workflow echo MCP and finish with deterministic evidence.',
    async beforeAgents(client, options) {
      const mcpPath = await createWorkflowEchoMcp(path.join(options.worktree, 'tmp', 'live-drills'))
      await client.send(installMcpServerRequest(options.workspace, workflowEchoMcpConfig(mcpPath)))
    },
    async afterAgentSpawn(client, options, { agent }) {
      await client.send(grantAgentExtensionRequest(options.workspace, agent.id, 'mcp', 'workflow_echo'))
    },
    nodePrompt() {
      return [
        'This is a workflow MCP grant drill.',
        'Use the provider-native workflow_echo MCP tool exactly once with marker M7_WORKFLOW_ECHO_OK.',
        'The tool is usually named `workflow_echo_echo_marker`, `mcp__workflow_echo__echo_marker`, `echo_marker`, or similar.',
        'The MCP tool result must contain exactly `ECHO:M7_WORKFLOW_ECHO_OK`.',
        `After the MCP tool call succeeds, use Arroba workspace live sync to create \`outputs/${markerFile}\` with exactly \`ECHO:M7_WORKFLOW_ECHO_OK\`.`,
        'Then call the Arroba runtime MCP tool `validate_and_submit_workflow_run_output` with workflow_output_json exactly `{"echo":"ECHO:M7_WORKFLOW_ECHO_OK"}`.',
        'If the MCP is unavailable, do not write the marker and set output.message JSON exactly `{"echo":"MCP_UNAVAILABLE"}`.',
        'After the final output tool succeeds, emit one final fenced workflow JSON block with output.message JSON exactly `{"echo":"ECHO:M7_WORKFLOW_ECHO_OK"}` and then stop.',
      ].join('\n\n')
    },
    edgeRequest() {
      return null
    },
    async configureWorkflow(client, sessionId, workflowId, nodeIds) {
      await client.send(setWorkflowNodeCanCompleteRunRequest(sessionId, workflowId, nodeIds[0], true))
    },
    async assertResult(result, { options }) {
      const expected = JSON.stringify({ echo: 'ECHO:M7_WORKFLOW_ECHO_OK' })
      const output = result.finalOutput?.message ?? result.nodeRuns[0]?.completion?.output?.message
      if (output !== expected) {
        throw new Error(`workflow MCP echo output mismatch: expected ${expected}, got ${output}`)
      }
      const markerPath = path.join(options.worktree, 'outputs', markerFile)
      const marker = (await readFile(markerPath, 'utf8')).trim()
      if (marker !== 'ECHO:M7_WORKFLOW_ECHO_OK') {
        throw new Error(`workflow MCP marker mismatch: ${JSON.stringify(marker)}`)
      }
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
  if (options.scenario === 'final-run-output-chain') {
    return buildFinalRunOutputScenario(options.providers, options.model, schemaPath)
  }
  if (options.scenario === 'cyclic-final-run-output-chain') {
    return buildCyclicFinalRunOutputScenario(options.providers, options.model, schemaPath)
  }
  if (options.scenario === 'cyclic-budgeted-final-run-output-chain') {
    return buildCyclicBudgetedFinalRunOutputScenario(options.providers, options.model, schemaPath)
  }
  if (options.scenario === 'cyclic-final-run-with-intermediate-output-chain') {
    return buildCyclicFinalRunWithIntermediateOutputScenario(options.providers, options.model, schemaPath)
  }
  if (options.scenario === 'conditional-branch-subset') {
    return buildConditionalBranchSubsetScenario(options.providers, options.model)
  }
  if (options.scenario === 'immediate-release-downstream') {
    return buildImmediateReleaseDownstreamScenario(options.providers, options.model, schemaPath)
  }
  if (options.scenario === 'mcp-echo-workflow') {
    return buildMcpEchoWorkflowScenario(options.providers, options.model)
  }
  throw new Error(`unsupported scenario: ${options.scenario}`)
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
      path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'),
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
