import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'

const DEFAULT_NUMBERS = ['1842', '7315', '4068', '5921', '8473', '2604']

let workflowRuntimeRequests = null

function useWorkflowRuntimeRequests(requests) {
  workflowRuntimeRequests = requests
}

function workflowRuntimeRequest(name) {
  const request = workflowRuntimeRequests?.[name]
  if (typeof request !== 'function') {
    throw new Error(`workflow runtime scenario request builder is missing ${name}`)
  }
  return request
}

function addWorkflowEdgeRequest(...args) {
  return workflowRuntimeRequest('addWorkflowEdgeRequest')(...args)
}

function updateWorkflowNodeInstructionsRequest(...args) {
  return workflowRuntimeRequest('updateWorkflowNodeInstructionsRequest')(...args)
}

function setWorkflowNodeCanCompleteRunRequest(...args) {
  return workflowRuntimeRequest('setWorkflowNodeCanCompleteRunRequest')(...args)
}

function setWorkflowNodeCanEmitIntermediateOutputRequest(...args) {
  return workflowRuntimeRequest('setWorkflowNodeCanEmitIntermediateOutputRequest')(...args)
}

function setWorkflowNodeIntermediateOutputSchemaRequest(...args) {
  return workflowRuntimeRequest('setWorkflowNodeIntermediateOutputSchemaRequest')(...args)
}

function setWorkflowRunOutputSchemaRequest(...args) {
  return workflowRuntimeRequest('setWorkflowRunOutputSchemaRequest')(...args)
}

function setWorkflowNodeMaxTurnsRequest(...args) {
  return workflowRuntimeRequest('setWorkflowNodeMaxTurnsRequest')(...args)
}

function installMcpServerRequest(...args) {
  return workflowRuntimeRequest('installMcpServerRequest')(...args)
}

function grantAgentExtensionRequest(...args) {
  return workflowRuntimeRequest('grantAgentExtensionRequest')(...args)
}

export function workflowOutput(summary, messageJson) {
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
      await client.send(setWorkflowNodeMaxTurnsRequest(sessionId, workflowId, nodeIds[0], aMaxTurns))
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

export function createWorkflowRuntimeScenario(options, schemaPath, requests) {
  useWorkflowRuntimeRequests(requests)
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
