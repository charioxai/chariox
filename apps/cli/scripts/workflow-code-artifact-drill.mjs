#!/usr/bin/env node
import { spawn, spawnSync } from 'node:child_process'
import { mkdir, readFile, readdir, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import {
  LocalIpcClient,
  applyWorkflowCodeArtifactRequest,
  attachToSessionRequest,
  createSessionRequest,
  createWorkflowCodeArtifactRequest,
  deleteWorkflowCodeArtifactRequest,
  endSessionRequest,
  exportWorkflowCodeArtifactRequest,
  getSessionStateRequest,
  getWorkflowCodeArtifactRequest,
  getProviderRunRequest,
  importWorkflowCodeArtifactRequest,
  invokeWorkflowEndpointRequest,
  launchProviderRunRequest,
  runWorkflowCodeArtifactRequest,
  spawnAgentRequest,
} from '@arroba/kernel-client'

import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 120_000

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

function parseArgs(argv) {
  const options = {
    kernel: null,
    spawnDaemon: true,
    workspace: null,
    worktree: null,
    artifactRoot: path.join(repoRoot, '.artifacts', 'workflow-code-artifact-drill', nowStamp()),
    timeoutMs: DEFAULT_TIMEOUT_MS,
    keepArtifactsOnFailure: true,
    preserveOnSuccess: false,
    dryRun: false,
    exampleSuite: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    else if (arg === '--kernel') {
      options.kernel = argv[++index]
      options.spawnDaemon = false
    } else if (arg === '--spawn-daemon') options.spawnDaemon = true
    else if (arg === '--no-spawn-daemon') options.spawnDaemon = false
    else if (arg === '--workspace') options.workspace = argv[++index]
    else if (arg === '--worktree') options.worktree = argv[++index]
    else if (arg === '--artifact-root') options.artifactRoot = argv[++index]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--discard-artifacts-on-failure') options.keepArtifactsOnFailure = false
    else if (arg === '--preserve-on-success') options.preserveOnSuccess = true
    else if (arg === '--discard-artifacts-on-success') options.preserveOnSuccess = false
    else if (arg === '--example-suite') options.exampleSuite = true
    else if (arg === '--dry-run') options.dryRun = true
    else if (arg === '--help' || arg === '-h') {
      printHelp()
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
    throw new Error('--timeout-ms must be positive')
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/workflow-code-artifact-drill.mjs [options]',
    '',
    'Creates a workflow-code artifact through public IPC, applies it, exports/imports it,',
    'runs the imported artifact, and verifies generated agents, schemas, canvas layout,',
    'artifact history, and provider/model rebinding.',
    '',
    'Options:',
    '  --kernel ws://127.0.0.1:43284',
    '  --spawn-daemon',
    '  --no-spawn-daemon',
    '  --workspace PATH',
    '  --worktree PATH',
    '  --artifact-root PATH',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    '  --keep-artifacts-on-failure',
    '  --discard-artifacts-on-failure',
    '  --preserve-on-success',
    '  --discard-artifacts-on-success',
    '  --example-suite',
    '  --dry-run',
  ].join('\n'))
}

function assert(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? repoRoot,
    env: options.env ?? process.env,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

function buildKernel() {
  runChecked('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  return path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
}

function spawnedKernel() {
  const kernelPort = 45400 + Math.floor(Math.random() * 1000)
  const socketPath = path.join(os.tmpdir(), `arroba-workflow-code-drill-${process.pid}-${Date.now()}.sock`)
  return {
    kernelUrl: `ws://127.0.0.1:${kernelPort}`,
    env: {
      ...process.env,
      ARROBA_KERNEL_PORT: String(kernelPort),
      ARROBA_MCP_PORT: String(kernelPort + 1000),
      ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
      ARROBA_CODEX_PORT: String(kernelPort + 2001),
      ARROBA_DAEMON_SOCKET: socketPath,
      ARROBA_DAEMON_ID: `workflow-code-drill-${process.pid}-${Date.now()}`,
    },
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function waitForKernel(client, workspace, worktree, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const session = unwrap(
        await client.send(createSessionRequest(workspace, worktree, 'workflow-code-ready-probe', undefined, null, 'off')),
        'SessionCreated',
      ).session
      await client.send(endSessionRequest(session.id)).catch(() => {})
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? 'unknown error'}`)
}

function workflowCodeSource() {
  return `
workflow.define({
  alias: "workflow_code_artifact_drill",
  prompt: "Coordinate the toy workflow and preserve structured output.",
  maxConcurrent: 3,
  flushAgentContextBeforeRun: true,
});

const handoff = workflow.schema({
  handle: "handoff",
  alias: "Handoff payload",
  schema: {
    type: "object",
    required: ["value", "note"],
    properties: {
      value: { type: "number" },
      note: { type: "string" },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Final output",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "artifact-planner", provider: "codex", model: "gpt-5" }),
  publicLabel: "Planner",
  instructions: "Read the endpoint prompt and hand a numbered task to the worker.",
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: 120 },
});

const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "artifact-worker", provider: "opencode", model: "opencode/gpt-5" }),
  publicLabel: "Worker",
  instructions: "Transform the planner handoff and pass it to the reviewer.",
  canCompleteWorkflowRun: false,
  canvas: { x: 280, y: 120 },
});

const reviewer = workflow.node({
  handle: "reviewer",
  agent: workflow.newAgent({ alias: "artifact-reviewer", provider: "claude", model: "sonnet" }),
  publicLabel: "Reviewer",
  instructions: "Review the worker result and submit final output that matches final_output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 560, y: 120 },
});

workflow.edge(planner, worker, { handle: "planner_to_worker", handoffSchema: handoff, validationPolicy: "warn" });
workflow.edge(worker, reviewer, { handle: "worker_to_reviewer", handoffSchema: handoff, validationPolicy: "warn" });
workflow.endpoint(planner, { handle: "entry", alias: "entry", canvas: { x: -180, y: 120 } });
`.trim()
}

function providerRebindings() {
  return ['planner', 'worker', 'reviewer'].map((node) => ({
    node,
    provider: 'dev-stub',
    model: 'default',
  }))
}

function existingAgentWorkflowCodeSource(agentId) {
  return `
workflow.define({
  alias: "workflow_code_existing_agent_artifact_drill",
  prompt: "Use one pre-existing worker and one generated finisher.",
  maxConcurrent: 2,
});

const handoff = workflow.schema({
  handle: "existing_handoff",
  alias: "Existing-agent handoff",
  schema: {
    type: "object",
    required: ["summary"],
    properties: {
      summary: { type: "string" },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Existing-agent final output",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const existingWorker = workflow.node({
  handle: "existing_worker",
  agent: workflow.existingAgent("${agentId}"),
  publicLabel: "Existing worker",
  instructions: "Use the endpoint prompt to produce an existing_handoff payload.",
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: 120 },
});

const generatedFinisher = workflow.node({
  handle: "generated_finisher",
  agent: workflow.newAgent({ alias: "artifact-generated-finisher", provider: "codex", model: "gpt-5" }),
  publicLabel: "Generated finisher",
  instructions: "Finish the workflow and submit final output matching final_output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 280, y: 120 },
});

workflow.edge(existingWorker, generatedFinisher, {
  handle: "existing_to_generated",
  handoffSchema: handoff,
  validationPolicy: "warn",
});
workflow.endpoint(existingWorker, { handle: "entry", alias: "entry", canvas: { x: -180, y: 120 } });
`.trim()
}

function existingAgentRebindings() {
  return [{
    node: 'generated_finisher',
    provider: 'dev-stub',
    model: 'default',
  }]
}

function outputSchemaWorkflowCodeSource() {
  return `
workflow.define({
  alias: "workflow_code_output_schema_artifact_drill",
  prompt: "Validate schema-backed intermediate and final workflow outputs from workflow-code.",
  maxConcurrent: 1,
});

const valueOutput = workflow.schema({
  handle: "value_output",
  alias: "Value output",
  schema: {
    type: "object",
    required: ["value"],
    properties: {
      value: { type: "number" },
    },
    additionalProperties: false,
  },
});

workflow.define({
  runOutputSchema: valueOutput,
  intermediateOutputSchema: valueOutput,
});

const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({
    alias: "artifact-output-worker",
    provider: "dev-stub",
    model: "workflow-intermediate-node",
  }),
  publicLabel: "Output worker",
  instructions: "Acknowledge the workflow turn, submit one intermediate output, then submit the final output.",
  canCompleteWorkflowRun: true,
  canEmitIntermediateRunOutput: true,
  intermediateOutputSchema: valueOutput,
  canvas: { x: 0, y: 120 },
});

workflow.endpoint(worker, { handle: "entry", alias: "entry", canvas: { x: -180, y: 120 } });
`.trim()
}

function defaultToyExpectation() {
  return {
    nodes: 3,
    agents: 3,
    edges: 2,
    endpoints: 1,
    requiredSchemas: ['handoff', 'final_output'],
  }
}

function expectationFromDefinition(definition) {
  return {
    nodes: definition.nodes?.length ?? 0,
    agents: definition.nodes?.length ?? 0,
    edges: definition.edges?.length ?? 0,
    endpoints: definition.endpoints?.length ?? 0,
    requiredSchemas: (definition.schemas ?? []).map((schema) => schema.handle),
  }
}

function rebindingsForDefinition(definition) {
  return (definition.nodes ?? []).map((node) => ({
    node: node.handle,
    provider: 'dev-stub',
    model: 'default',
  }))
}

function validateApplyResult(result, label, expected = defaultToyExpectation()) {
  assert(result?.compile?.validation?.ok, `${label} compile validation failed`, result?.compile?.validation)
  const apply = result.apply
  assert(apply?.workflow_id, `${label} did not return workflow id`, result)
  assert(Object.keys(apply.node_ids ?? {}).length === expected.nodes, `${label} should create ${expected.nodes} nodes`, apply)
  assert(Object.keys(apply.agent_ids ?? {}).length === expected.agents, `${label} should resolve ${expected.agents} node agents`, apply)
  assert(Object.keys(apply.edge_ids ?? {}).length === expected.edges, `${label} should create ${expected.edges} edges`, apply)
  assert(Object.keys(apply.endpoint_ids ?? {}).length === expected.endpoints, `${label} should create ${expected.endpoints} endpoints`, apply)
  for (const schemaHandle of expected.requiredSchemas) {
    assert(apply.schema_refs?.[schemaHandle], `${label} should report schema ${schemaHandle}`, apply)
  }
  assert(apply.canvas_layout_applied === true, `${label} should create a canvas layout`, apply)
  return apply
}

function validateSessionProjection(session, apply, label, expected = defaultToyExpectation()) {
  const workflow = (session.workflows ?? []).find((entry) => entry.id === apply.workflow_id)
  assert(workflow, `${label} workflow should appear in session projection`, { workflowId: apply.workflow_id })
  assert(workflow.canvas_layout, `${label} workflow should include canvas layout`, workflow)
  if (expected.requiredSchemas.includes('final_output')) {
    assert(workflow.run_output_schema_ref === apply.schema_refs.final_output, `${label} final schema ref should be assigned`, {
      workflow,
      schemaRefs: apply.schema_refs,
    })
  }
  for (const [handle, agentId] of Object.entries(apply.agent_ids ?? {})) {
    const agent = (session.agents ?? []).find((entry) => entry.id === agentId)
    assert(agent, `${label} node agent ${handle} should appear in session`, { agentId })
    assert(agent.provider === 'dev-stub', `${label} node agent ${handle} should use dev-stub`, agent)
    assert(agent.model === 'default', `${label} node agent ${handle} should use default model`, agent)
  }
}

async function workflowCodeExamples() {
  const examplesDir = path.join(repoRoot, 'examples', 'workflow-code')
  const names = (await readdir(examplesDir))
    .filter((name) => name.endsWith('.js'))
    .sort()
  return await Promise.all(names.map(async (name) => ({
    name,
    source: await readFile(path.join(examplesDir, name), 'utf8'),
  })))
}

async function applyExampleSuite(client, sessionId, nodePath) {
  const examples = await workflowCodeExamples()
  const results = []
  for (const [index, example] of examples.entries()) {
    const artifactName = `pattern-${index + 1}-${example.name.replace(/\.js$/, '')}-${Date.now()}`
    const created = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, artifactName, nodePath, example.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(created?.metadata?.validation?.ok, `example ${example.name} should validate`, created?.metadata?.validation)

    const exported = unwrap(
      await client.send(exportWorkflowCodeArtifactRequest(sessionId, artifactName)),
      'WorkflowCodeArtifactExported',
    ).package
    const expected = expectationFromDefinition(exported.definition)
    const rebindings = rebindingsForDefinition(exported.definition)
    const appliedResponse = unwrap(
      await client.send(applyWorkflowCodeArtifactRequest(sessionId, artifactName, rebindings)),
      'WorkflowCodeApplied',
    )
    const apply = validateApplyResult(appliedResponse.result, `example ${example.name}`, expected)
    validateSessionProjection(appliedResponse.session, apply, `example ${example.name}`, expected)
    results.push({
      example: example.name,
      artifactName,
      workflowId: apply.workflow_id,
      nodes: expected.nodes,
      edges: expected.edges,
      endpoints: expected.endpoints,
      schemas: Object.keys(apply.schema_refs ?? {}),
    })
  }
  return results
}

function validateArtifactHistory(artifact, expectedActions) {
  const actions = (artifact?.metadata?.history ?? []).map((entry) => entry.action)
  for (const action of expectedActions) {
    assert(actions.includes(action), `artifact history should include ${action}`, actions)
  }
}

async function applyExistingAgentArtifact(client, session, nodePath, workspace) {
  const existingAgent = unwrap(
    await client.send(spawnAgentRequest(
      session.id,
      'dev-stub',
      'artifact-existing-worker',
      'default',
      workspace,
      'low',
    )),
    'AgentSpawned',
  ).agent
  const artifactName = `existing-agent-artifact-${Date.now()}`
  const source = existingAgentWorkflowCodeSource(existingAgent.id)
  const created = unwrap(
    await client.send(createWorkflowCodeArtifactRequest(session.id, artifactName, nodePath, source)),
    'WorkflowCodeArtifactCreated',
  ).artifact
  assert(created?.metadata?.validation?.ok, 'existing-agent artifact should validate', created?.metadata?.validation)

  const expected = {
    nodes: 2,
    agents: 2,
    edges: 1,
    endpoints: 1,
    requiredSchemas: ['existing_handoff', 'final_output'],
  }
  const appliedResponse = unwrap(
    await client.send(applyWorkflowCodeArtifactRequest(session.id, artifactName, existingAgentRebindings())),
    'WorkflowCodeApplied',
  )
  const apply = validateApplyResult(appliedResponse.result, 'existing-agent artifact apply', expected)
  validateSessionProjection(appliedResponse.session, apply, 'existing-agent artifact apply', expected)
  assert(
    apply.agent_ids?.existing_worker === existingAgent.id,
    'existing-agent artifact should preserve the pre-existing agent id for its node',
    { apply, existingAgent },
  )
  const generatedAgentId = apply.agent_ids?.generated_finisher
  assert(
    generatedAgentId && generatedAgentId !== existingAgent.id,
    'existing-agent artifact should create a distinct generated node agent',
    { apply, existingAgent },
  )
  return {
    artifactName,
    workflowId: apply.workflow_id,
    existingAgentId: existingAgent.id,
    generatedAgentId,
  }
}

async function waitForCompletedWorkflowRun(client, sessionId, workflowRunId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastRun = null
  while (Date.now() < deadline) {
    const stateResponse = await client.send(getSessionStateRequest(sessionId))
    const state = unwrap(stateResponse, 'SessionStateLoaded')?.session
      ?? unwrap(stateResponse, 'SessionState')?.session
    const run = (state?.workflow_runs ?? []).find((entry) => entry.id === workflowRunId)
    if (run) {
      lastRun = run
      if (['Completed', 'Failed', 'Stopped'].includes(run.status)) {
        return run
      }
    }
    await sleep(500)
  }
  throw new Error(`workflow run ${workflowRunId} did not complete before timeout${lastRun ? `\n${JSON.stringify(lastRun, null, 2)}` : ''}`)
}

async function waitForProviderRunReady(client, providerRunId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const providerRun = unwrap(await client.send(getProviderRunRequest(providerRunId)), 'ProviderRun')?.provider_run
    if (providerRun?.state && providerRun.state !== 'Starting') {
      if (providerRun.state !== 'Running' && providerRun.state !== 'Parked') {
        throw new Error(`provider run ${providerRunId} reached unexpected state ${providerRun.state}`)
      }
      return providerRun
    }
    await sleep(250)
  }
  throw new Error(`provider run ${providerRunId} did not become ready`)
}

async function applyOutputSchemaArtifact(client, session, nodePath, timeoutMs) {
  const artifactName = `output-schema-artifact-${Date.now()}`
  const source = outputSchemaWorkflowCodeSource()
  const created = unwrap(
    await client.send(createWorkflowCodeArtifactRequest(session.id, artifactName, nodePath, source)),
    'WorkflowCodeArtifactCreated',
  ).artifact
  assert(created?.metadata?.validation?.ok, 'output-schema artifact should validate', created?.metadata?.validation)

  const expected = {
    nodes: 1,
    agents: 1,
    edges: 0,
    endpoints: 1,
    requiredSchemas: ['value_output'],
  }
  const appliedResponse = unwrap(
    await client.send(applyWorkflowCodeArtifactRequest(session.id, artifactName)),
    'WorkflowCodeApplied',
  )
  const apply = validateApplyResult(appliedResponse.result, 'output-schema artifact apply', expected)
  const workflow = (appliedResponse.session?.workflows ?? []).find((entry) => entry.id === apply.workflow_id)
  assert(workflow, 'output-schema artifact workflow should appear in session projection', { workflowId: apply.workflow_id })
  assert(
    workflow.run_output_schema_ref === apply.schema_refs.value_output,
    'output-schema artifact should assign workflow final output schema',
    { workflow, schemaRefs: apply.schema_refs },
  )
  assert(
    workflow.intermediate_output_schema_ref === apply.schema_refs.value_output,
    'output-schema artifact should assign workflow intermediate output schema',
    { workflow, schemaRefs: apply.schema_refs },
  )
  const nodeId = apply.node_ids?.worker
  const agentId = apply.agent_ids?.worker
  const node = (workflow.nodes ?? []).find((entry) => entry.id === nodeId)
  assert(node?.intermediate_output_schema_ref === apply.schema_refs.value_output, 'output-schema artifact should assign node intermediate schema', {
    node,
    schemaRefs: apply.schema_refs,
  })
  assert(agentId, 'output-schema artifact should resolve worker agent id', apply)

  const launchResponse = unwrap(
    await client.send(launchProviderRunRequest(
      session.id,
      'dev-stub',
      'default',
      'workflow-intermediate-node',
      'low',
      agentId,
    )),
    'ProviderRunLaunchAccepted',
  )
  assert(launchResponse?.provider_run?.id, 'output-schema artifact should launch generated worker provider run', launchResponse)
  await waitForProviderRunReady(client, launchResponse.provider_run.id, timeoutMs)

  const endpointId = apply.endpoint_ids?.entry
  assert(endpointId, 'output-schema artifact should resolve entry endpoint id', apply)
  const invokeResponse = await client.send(invokeWorkflowEndpointRequest(
    session.id,
    apply.workflow_id,
    endpointId,
    'Run the workflow-code output schema drill.',
  ))
  const workflowRun = unwrap(invokeResponse, 'WorkflowRunInvoked')?.workflow_run
  assert(workflowRun?.id, 'output-schema artifact should start a workflow run', invokeResponse)
  const completed = await waitForCompletedWorkflowRun(client, session.id, workflowRun.id, timeoutMs)
  assert(completed.status === 'Completed', 'output-schema artifact workflow run should complete', completed)
  assert(completed.final_output?.message === JSON.stringify({ value: 1842 }), 'output-schema artifact final output mismatch', completed)
  assert(completed.final_output_valid === true, 'output-schema artifact final output should validate', completed)
  const intermediate = (completed.intermediate_outputs ?? []).find(
    (entry) => entry.output?.message === JSON.stringify({ value: 1841 }),
  )
  assert(intermediate, 'output-schema artifact should record the intermediate output', completed)
  assert(intermediate.valid === true, 'output-schema artifact intermediate output should validate', intermediate)

  return {
    artifactName,
    workflowId: apply.workflow_id,
    workflowRunId: workflowRun.id,
    finalOutput: completed.final_output?.message,
    intermediateOutputs: completed.intermediate_outputs?.map((entry) => entry.output?.message) ?? [],
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const source = workflowCodeSource()
  if (options.dryRun) {
    console.log(JSON.stringify({
      artifactRoot: options.artifactRoot,
      spawnDaemon: options.spawnDaemon,
      kernel: options.kernel,
      source,
      providerRebindings: providerRebindings(),
      exampleSuite: options.exampleSuite,
    }, null, 2))
    return
  }

  await prepareDrillArtifacts(options.artifactRoot)
  const generatedRoot = path.join(repoRoot, 'target', 'workflow-code-artifact-drill', `${process.pid}-${Date.now()}`)
  const workspace = options.workspace ?? path.join(generatedRoot, 'workspace')
  const worktree = options.worktree ?? path.join(generatedRoot, 'worktree')
  await mkdir(workspace, { recursive: true })
  await mkdir(worktree, { recursive: true })

  let passed = false
  let failure = null
  let daemonChild = null
  let sessionId = null
  let attachmentId = null
  let kernelUrl = options.kernel ?? 'ws://127.0.0.1:43284'
  let summary = {}
  let client = null

  try {
    if (options.spawnDaemon) {
      const spawned = spawnedKernel()
      kernelUrl = spawned.kernelUrl
      daemonChild = spawn(buildKernel(), [], {
        cwd: repoRoot,
        env: spawned.env,
        stdio: ['ignore', 'ignore', 'inherit'],
      })
    }

    client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await waitForKernel(client, workspace, worktree, options.timeoutMs)

    const session = unwrap(
      await client.send(createSessionRequest(workspace, worktree, 'workflow-code-artifact-drill', undefined, null, 'off')),
      'SessionCreated',
    ).session
    sessionId = session.id
    const attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `workflow-code-artifact-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    attachmentId = attachment.id

    const artifactName = `artifact-drill-${Date.now()}`
    const importedName = `${artifactName}-imported`
    const nodePath = process.execPath

    const created = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(session.id, artifactName, nodePath, source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(created?.metadata?.validation?.ok, 'created artifact should validate', created?.metadata?.validation)
    validateArtifactHistory(created, ['created'])

    const appliedResponse = unwrap(
      await client.send(applyWorkflowCodeArtifactRequest(session.id, artifactName, providerRebindings())),
      'WorkflowCodeApplied',
    )
    const firstApply = validateApplyResult(appliedResponse.result, 'artifact apply')
    validateSessionProjection(appliedResponse.session, firstApply, 'artifact apply')

    const exported = unwrap(
      await client.send(exportWorkflowCodeArtifactRequest(session.id, artifactName)),
      'WorkflowCodeArtifactExported',
    ).package
    assert(exported?.source_sha256, 'exported package should include source hash', exported)
    assert(exported?.definition_sha256, 'exported package should include compiled definition hash', exported)

    await client.send(deleteWorkflowCodeArtifactRequest(session.id, artifactName))

    const imported = unwrap(
      await client.send(importWorkflowCodeArtifactRequest(session.id, exported, nodePath, {
        name: importedName,
        overwrite: false,
      })),
      'WorkflowCodeArtifactImported',
    ).artifact
    assert(imported?.metadata?.validation?.ok, 'imported artifact should validate', imported?.metadata?.validation)

    const outputSchemaArtifact = await applyOutputSchemaArtifact(client, session, nodePath, options.timeoutMs)

    const runResponse = unwrap(
      await client.send(runWorkflowCodeArtifactRequest(session.id, importedName, 'Run the imported workflow-code artifact.', {
        endpoint: 'entry',
        providerRebindings: providerRebindings(),
      })),
      'WorkflowCodeRun',
    )
    const runApply = validateApplyResult(runResponse.result.apply, 'artifact run')
    validateSessionProjection(runResponse.session, runApply, 'artifact run')
    const invocation = runResponse.result.invocation
    assert(invocation?.workflow_run || invocation?.queued_prompt, 'artifact run should invoke or enqueue a workflow run', invocation)

    const readBack = unwrap(
      await client.send(getWorkflowCodeArtifactRequest(session.id, importedName)),
      'WorkflowCodeArtifact',
    ).artifact
    validateArtifactHistory(readBack, ['imported', 'run'])

    const stateResponse = await client.send(getSessionStateRequest(session.id))
    const state = unwrap(stateResponse, 'SessionStateLoaded')?.session
      ?? unwrap(stateResponse, 'SessionState')?.session
    assert((state?.workflows ?? []).some((workflow) => workflow.id === runApply.workflow_id), 'run workflow should be in loaded session state')
    const existingAgentArtifact = await applyExistingAgentArtifact(client, session, nodePath, workspace)
    const exampleSuite = options.exampleSuite
      ? await applyExampleSuite(client, session.id, nodePath)
      : []

    summary = {
      sessionId: session.id,
      attachmentId,
      createdArtifact: artifactName,
      importedArtifact: importedName,
      appliedWorkflowId: firstApply.workflow_id,
      runWorkflowId: runApply.workflow_id,
      runInvocation: invocation.workflow_run ? 'started' : 'enqueued',
      generatedAgents: Object.values(runApply.agent_ids ?? {}),
      schemaRefs: runApply.schema_refs,
      existingAgentArtifact,
      outputSchemaArtifact,
      exampleSuite,
    }
    console.log(JSON.stringify(summary, null, 2))

    await client.send(endSessionRequest(session.id)).catch(() => {})
    await client.close()
    passed = true
  } catch (error) {
    failure = error
    console.error(error)
    if (client && sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    if (client) {
      await client.close().catch(() => {})
    }
    process.exitCode = 1
  } finally {
    if (daemonChild) {
      daemonChild.kill('SIGTERM')
      await sleep(1000)
    }
    if (passed || !options.keepArtifactsOnFailure) {
      await rm(generatedRoot, { recursive: true, force: true }).catch(() => {})
    }
    await finalizeDrillArtifacts({
      rootDir: options.artifactRoot,
      passed,
      preserveOnFailure: options.keepArtifactsOnFailure,
      preserveOnSuccess: options.preserveOnSuccess,
      failure,
      metadata: {
        drill: 'workflow-code-artifact',
        kernelUrl,
        workspace,
        worktree,
        sessionId,
        attachmentId,
        summary,
      },
    })
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  await main()
}
