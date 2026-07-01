#!/usr/bin/env node
import { execFile, spawn, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { promisify } from 'node:util'

import {
  LocalIpcClient,
  applyWorkflowCodeRequest,
  applyWorkflowCodeArtifactRequest,
  attachToSessionRequest,
  createSessionRequest,
  createWorkflowCodeArtifactRequest,
  deleteWorkflowCodeArtifactRequest,
  endSessionRequest,
  exportWorkflowCodeArtifactRequest,
  exportWorkflowCodePackageRequest,
  exportWorkflowCodeSourceRequest,
  focusAgentRequest,
  getSessionStateRequest,
  getWorkflowCodeArtifactRequest,
  getProviderRunRequest,
  importWorkflowCodeArtifactRequest,
  importWorkflowCodePackageRequest,
  installSkillRequest,
  invokeWorkflowEndpointRequest,
  launchProviderRunRequest,
  listWorkflowCodeArtifactsRequest,
  runWorkflowCodeRequest,
  runWorkflowCodeArtifactRequest,
  spawnAgentRequest,
  uninstallSkillRequest,
  updateWorkflowCodeArtifactRequest,
  validateWorkflowCodeRequest,
} from '@arroba/kernel-client'

import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'
import {
  assertHetznerArrobaBinaries,
  runHetznerCommand,
  shellQuote,
} from './lib/native-tui-remote-execution.mjs'
import { buildWorkflowOutline } from '../dist/workflow-outline/build.js'
import { renderWorkflowOutlineToText } from '../dist/workflow-outline/text.js'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const execFileAsync = promisify(execFile)
const DEFAULT_TIMEOUT_MS = 120_000
const WORKFLOW_CODE_ARTIFACT_SKILL_PREFIX = 'workflow-code-artifact-skill'

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
    secondKernel: false,
    hetznerSecondKernel: false,
    realProviderTopology: null,
    providerModels: {
      codex: process.env.ARROBA_WORKFLOW_CODE_CODEX_MODEL ?? process.env.ARROBA_CODEX_MODEL ?? 'gpt-5.4-mini',
      opencode: process.env.ARROBA_WORKFLOW_CODE_OPENCODE_MODEL ?? process.env.ARROBA_OPENCODE_MODEL ?? 'opencode/gpt-5.2',
      claude: process.env.ARROBA_WORKFLOW_CODE_CLAUDE_MODEL ?? process.env.ARROBA_CLAUDE_MODEL ?? 'sonnet',
    },
    providerIds: {
      codex: process.env.ARROBA_WORKFLOW_CODE_CODEX_PROVIDER ?? 'codex',
      opencode: process.env.ARROBA_WORKFLOW_CODE_OPENCODE_PROVIDER ?? 'opencode',
      claude: process.env.ARROBA_WORKFLOW_CODE_CLAUDE_PROVIDER ?? 'claude-p',
    },
    providerAccounts: {
      codex: process.env.ARROBA_WORKFLOW_CODE_CODEX_ACCOUNT ?? 'default',
      opencode: process.env.ARROBA_WORKFLOW_CODE_OPENCODE_ACCOUNT ?? 'default',
      claude: process.env.ARROBA_WORKFLOW_CODE_CLAUDE_ACCOUNT ?? 'default',
    },
    providerEfforts: {
      codex: process.env.ARROBA_WORKFLOW_CODE_CODEX_EFFORT ?? 'low',
      opencode: process.env.ARROBA_WORKFLOW_CODE_OPENCODE_EFFORT ?? 'low',
      claude: process.env.ARROBA_WORKFLOW_CODE_CLAUDE_EFFORT ?? 'low',
    },
    hetznerHost: process.env.ARROBA_WORKFLOW_CODE_HETZNER_HOST ?? process.env.ARROBA_NATIVE_TUI_HETZNER_HOST ?? 'root@195.201.123.115',
    hetznerKey: process.env.ARROBA_WORKFLOW_CODE_HETZNER_KEY ?? process.env.ARROBA_NATIVE_TUI_HETZNER_KEY ?? path.join(os.homedir(), '.ssh/arroba_hetzner_staging'),
    hetznerRepo: process.env.ARROBA_WORKFLOW_CODE_HETZNER_REPO ?? process.env.ARROBA_NATIVE_TUI_HETZNER_REPO ?? '/tmp/arroba-native-remote-validate',
    hetznerRemoteRoot: process.env.ARROBA_WORKFLOW_CODE_HETZNER_ROOT ?? '/tmp/arroba-workflow-code-second-kernel',
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
    else if (arg === '--second-kernel') options.secondKernel = true
    else if (arg === '--hetzner-second-kernel') options.hetznerSecondKernel = true
    else if (arg === '--real-provider-topology') options.realProviderTopology = argv[++index]
    else if (arg === '--provider-id') applyKeyValueOverride(options.providerIds, argv[++index], '--provider-id')
    else if (arg === '--provider-model') applyKeyValueOverride(options.providerModels, argv[++index], '--provider-model')
    else if (arg === '--provider-account') applyKeyValueOverride(options.providerAccounts, argv[++index], '--provider-account')
    else if (arg === '--provider-effort') applyKeyValueOverride(options.providerEfforts, argv[++index], '--provider-effort')
    else if (arg === '--hetzner-host') options.hetznerHost = argv[++index]
    else if (arg === '--hetzner-key') options.hetznerKey = argv[++index]
    else if (arg === '--hetzner-repo') options.hetznerRepo = argv[++index]
    else if (arg === '--hetzner-remote-root') options.hetznerRemoteRoot = argv[++index]
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

function applyKeyValueOverride(target, value, flag) {
  const separator = String(value ?? '').indexOf('=')
  if (separator <= 0 || separator === String(value).length - 1) {
    throw new Error(`${flag} must use provider=value`)
  }
  const key = String(value).slice(0, separator).trim()
  const entryValue = String(value).slice(separator + 1).trim()
  if (!key || !entryValue) throw new Error(`${flag} must use provider=value`)
  target[key] = entryValue
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
    '  --second-kernel',
    '  --hetzner-second-kernel',
    '  --real-provider-topology NAME.js',
    '  --provider-id SOURCE_PROVIDER=LAUNCH_PROVIDER',
    '  --provider-model PROVIDER=MODEL',
    '  --provider-account PROVIDER=ACCOUNT_PROFILE',
    '  --provider-effort PROVIDER=EFFORT',
    '  --hetzner-host HOST',
    '  --hetzner-key PATH',
    '  --hetzner-repo PATH',
    '  --hetzner-remote-root PATH',
    '  --dry-run',
  ].join('\n'))
}

function assert(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

function stage(message, details = null) {
  const suffix = details ? ` ${JSON.stringify(details)}` : ''
  console.error(`[workflow-code-drill] ${message}${suffix}`)
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

function spawnedKernel(label = 'workflow-code-drill', rootDir = null) {
  const kernelPort = 45400 + Math.floor(Math.random() * 1000)
  const runId = `${label}-${process.pid}-${Date.now()}`
  const socketPath = path.join(os.tmpdir(), `${runId}.sock`)
  const env = {
    ...process.env,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(kernelPort + 1000),
    ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
    ARROBA_CODEX_PORT: String(kernelPort + 2001),
    ARROBA_DAEMON_SOCKET: socketPath,
    ARROBA_DAEMON_ID: runId,
  }
  if (rootDir) {
    env.ARROBA_HOME = path.join(rootDir, 'arroba-home')
  }
  return {
    kernelUrl: `ws://127.0.0.1:${kernelPort}`,
    env,
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function withTimeout(promise, timeoutMs, message) {
  let timer = null
  return Promise.race([
    promise.finally(() => {
      if (timer) clearTimeout(timer)
    }),
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), timeoutMs)
    }),
  ])
}

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

function workflowCodeSource(skillName) {
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
  extensions: [{ kind: "skill", name: "${skillName}" }],
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
const entry = workflow.endpoint(planner, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
const urgent = workflow.queue({ handle: "urgent", alias: "urgent", priority: 10, enabled: false });
workflow.schedule(entry, {
  handle: "entry_schedule",
  queue: urgent,
  everySeconds: 300,
  invocationPrompt: "Wake the workflow-code artifact drill.",
  overlapPolicy: "skip",
  maxRuns: 2,
});
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
workflow.endpoint(existingWorker, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
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

workflow.endpoint(worker, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
`.trim()
}

function defaultToyExpectation(skillName = null) {
  return {
    nodes: 3,
    agents: 3,
    edges: 2,
    endpoints: 1,
    queues: 1,
    schedules: 1,
    requiredSchemas: ['handoff', 'final_output'],
    nodeExtensions: skillName
      ? { planner: [{ kind: 'skill', name: skillName }] }
      : {},
  }
}

function expectationFromDefinition(definition) {
  return {
    nodes: definition.nodes?.length ?? 0,
    agents: definition.nodes?.length ?? 0,
    edges: definition.edges?.length ?? 0,
    endpoints: definition.endpoints?.length ?? 0,
    queues: (definition.queues?.length ?? 0) || 1,
    schedules: definition.schedules?.length ?? definition.watchdogs?.length ?? 0,
    requiredSchemas: (definition.schemas ?? []).map((schema) => schema.handle),
    agentModel: 'default',
    nodeExtensions: Object.fromEntries(
      (definition.nodes ?? [])
        .filter((node) => (node.extensions ?? []).length > 0)
        .map((node) => [node.handle, node.extensions]),
    ),
  }
}

function topologyRuntimeExpectation(definition) {
  return {
    ...expectationFromDefinition(definition),
    agentModel: 'workflow-code-topology-node',
  }
}

function rebindingsForDefinition(definition) {
  return (definition.nodes ?? []).map((node) => ({
    node: node.handle,
    provider: 'dev-stub',
    model: 'default',
  }))
}

function topologyRuntimeRebindingsForDefinition(definition) {
  return (definition.nodes ?? []).map((node) => ({
    node: node.handle,
    provider: 'dev-stub',
    model: 'workflow-code-topology-node',
  }))
}

function providerSetForDefinition(definition) {
  return new Set((definition.nodes ?? []).map((node) => node.agent?.provider).filter(Boolean))
}

function realProviderRebindingsForDefinition(definition, options) {
  return (definition.nodes ?? []).map((node) => {
    const sourceProvider = node.agent?.provider
    assert(sourceProvider && sourceProvider !== 'dev-stub', `real-provider topology node ${node.handle} must use a real provider`, node)
    const provider = options.providerIds[sourceProvider] ?? sourceProvider
    assert(provider && provider !== 'dev-stub', `real-provider topology node ${node.handle} must launch a real provider`, {
      node,
      sourceProvider,
      providerIds: options.providerIds,
    })
    const model = options.providerModels[sourceProvider] ?? options.providerModels[provider] ?? node.agent?.model
    assert(model && model !== 'default', `real-provider topology node ${node.handle} requires a concrete model for ${sourceProvider}`, {
      node,
      providerModels: options.providerModels,
    })
    const effort = options.providerEfforts[sourceProvider] ?? options.providerEfforts[provider] ?? node.agent?.effort ?? 'low'
    const accountProfile = options.providerAccounts[sourceProvider] ?? options.providerAccounts[provider] ?? node.agent?.account_profile ?? 'default'
    return {
      node: node.handle,
      provider,
      model,
      effort,
      account_profile: accountProfile,
    }
  })
}

function rebindingByNode(rebindings) {
  return new Map((rebindings ?? []).map((entry) => [entry.node, entry]))
}

function providerFamily(provider) {
  if (provider === 'claude-headless' || provider === 'claude-p') return 'claude'
  return provider
}

function shouldPrelaunchRealProvider(provider) {
  return providerFamily(provider) !== 'claude'
}

const EXAMPLE_TOPOLOGY_EXPECTATIONS = {
  'adversarial-verification.js': {
    alias: 'pattern-adversarial-verification',
    schemas: ['proposal', 'critique', 'final_output'],
    nodes: ['proposer', 'critic', 'judge'],
    edges: ['proposal_to_critic', 'critic_loop', 'critic_to_judge'],
    endpoints: ['entry'],
    completers: ['judge'],
    providers: ['codex', 'claude', 'opencode'],
    hasLoop: true,
  },
  'evaluator-optimizer.js': {
    alias: 'pattern-evaluator-optimizer',
    schemas: ['candidate', 'evaluation', 'final_output'],
    nodes: ['optimizer', 'evaluator'],
    edges: ['candidate_to_evaluator', 'revision_loop'],
    endpoints: ['entry'],
    completers: ['evaluator'],
    providers: ['codex', 'claude'],
    hasLoop: true,
  },
  'fan-out-synthesize.js': {
    alias: 'pattern-fan-out-synthesize',
    schemas: ['assignment', 'finding', 'final_output'],
    nodes: ['planner', 'worker_a', 'worker_b', 'synthesizer'],
    edges: ['planner_to_a', 'planner_to_b', 'a_to_synth', 'b_to_synth'],
    endpoints: ['entry'],
    completers: ['synthesizer'],
    providers: ['codex', 'claude', 'opencode'],
    waitForAll: ['synthesizer'],
  },
  'generate-filter.js': {
    alias: 'pattern-generate-filter',
    schemas: ['candidates', 'filtered', 'final_output'],
    nodes: ['generator', 'filter', 'finisher'],
    edges: ['generated_candidates', 'filtered_candidates'],
    endpoints: ['entry'],
    completers: ['finisher'],
    providers: ['codex', 'claude', 'opencode'],
  },
  'loop-until-done.js': {
    alias: 'pattern-loop-until-done',
    schemas: ['work_product', 'feedback', 'final_output'],
    nodes: ['worker', 'checker'],
    edges: ['work_to_checker', 'revise_loop'],
    endpoints: ['entry'],
    completers: ['checker'],
    providers: ['codex', 'claude'],
    hasLoop: true,
  },
  'orchestrator-workers.js': {
    alias: 'pattern-orchestrator-workers',
    schemas: ['assignment', 'result', 'final_output'],
    nodes: ['orchestrator', 'worker', 'synthesizer'],
    edges: ['orchestrator_to_worker', 'worker_to_synthesizer'],
    endpoints: ['entry'],
    completers: ['synthesizer'],
    providers: ['codex', 'opencode', 'claude'],
  },
  'parallelization.js': {
    alias: 'pattern-parallelization',
    schemas: ['review_task', 'review_result', 'final_output'],
    nodes: ['dispatcher', 'policy_reviewer', 'quality_reviewer', 'aggregator'],
    edges: ['to_policy', 'to_quality', 'policy_to_aggregator', 'quality_to_aggregator'],
    endpoints: ['entry'],
    completers: ['aggregator'],
    providers: ['codex', 'claude', 'opencode'],
    waitForAll: ['aggregator'],
  },
  'prompt-chaining.js': {
    alias: 'pattern-prompt-chaining',
    schemas: ['handoff', 'final_output'],
    nodes: ['drafter', 'refiner'],
    edges: ['draft_to_refiner'],
    endpoints: ['entry'],
    completers: ['refiner'],
    providers: ['codex', 'claude'],
  },
  'routing.js': {
    alias: 'pattern-routing',
    schemas: ['route_task', 'final_output'],
    nodes: ['classifier', 'code_specialist', 'research_specialist'],
    edges: ['to_code', 'to_research'],
    endpoints: ['entry'],
    completers: ['code_specialist', 'research_specialist'],
    providers: ['codex', 'opencode', 'claude'],
    multiEdgeRouter: 'classifier',
  },
  'tournament.js': {
    alias: 'pattern-tournament',
    schemas: ['contest_prompt', 'entry', 'final_output'],
    nodes: ['seeder', 'contestant_a', 'contestant_b', 'judge'],
    edges: ['seed_a', 'seed_b', 'entry_a', 'entry_b'],
    endpoints: ['entry'],
    completers: ['judge'],
    providers: ['codex', 'claude', 'opencode'],
    waitForAll: ['judge'],
  },
}

function assertSameSet(actual, expected, label) {
  const sortedActual = [...actual].sort()
  const sortedExpected = [...expected].sort()
  assert(
    JSON.stringify(sortedActual) === JSON.stringify(sortedExpected),
    `${label} mismatch`,
    { actual: sortedActual, expected: sortedExpected },
  )
}

function validationDiagnostics(validation) {
  return validation?.diagnostics ?? []
}

function validateExampleTopologyDefinition(exampleName, definition, validation) {
  const expectation = EXAMPLE_TOPOLOGY_EXPECTATIONS[exampleName]
  assert(expectation, `missing workflow-code topology expectation for ${exampleName}`)

  assert(definition?.workflow?.alias === expectation.alias, `${exampleName} alias mismatch`, definition?.workflow)
  assert(definition.workflow?.max_concurrent === 32, `${exampleName} should set maxConcurrent to 32`, definition.workflow)
  assert(definition.workflow?.run_output_schema === 'final_output', `${exampleName} should set workflow-level final output schema`, definition.workflow)
  assertSameSet((definition.schemas ?? []).map((schema) => schema.handle), expectation.schemas, `${exampleName} schemas`)
  assertSameSet((definition.nodes ?? []).map((node) => node.handle), expectation.nodes, `${exampleName} nodes`)
  assertSameSet((definition.edges ?? []).map((edge) => edge.handle), expectation.edges, `${exampleName} edges`)
  assertSameSet((definition.endpoints ?? []).map((endpoint) => endpoint.handle), expectation.endpoints, `${exampleName} endpoints`)
  assertSameSet(
    (definition.nodes ?? [])
      .filter((node) => node.can_complete_workflow_run === true)
      .map((node) => node.handle),
    expectation.completers,
    `${exampleName} completion nodes`,
  )

  for (const schema of definition.schemas ?? []) {
    assert(schema.schema?.type === 'object', `${exampleName} schema ${schema.handle} should be an object schema`, schema)
  }
  for (const node of definition.nodes ?? []) {
    assert(node.instructions, `${exampleName} node ${node.handle} should include node-level instructions`, node)
    assert(node.canvas && Number.isFinite(node.canvas.x) && Number.isFinite(node.canvas.y), `${exampleName} node ${node.handle} should include canvas coordinates`, node)
    assert(node.agent?.kind === 'create', `${exampleName} node ${node.handle} should create an agent for portability`, node)
  }
  for (const endpoint of definition.endpoints ?? []) {
    assert(endpoint.canvas && Number.isFinite(endpoint.canvas.x) && Number.isFinite(endpoint.canvas.y), `${exampleName} endpoint ${endpoint.handle} should include canvas coordinates`, endpoint)
    assert(expectation.nodes.includes(endpoint.entry_node), `${exampleName} endpoint ${endpoint.handle} should target a known node`, endpoint)
  }
  for (const edge of definition.edges ?? []) {
    assert(expectation.nodes.includes(edge.from_node), `${exampleName} edge ${edge.handle} should have a known source node`, edge)
    assert(expectation.nodes.includes(edge.to_node), `${exampleName} edge ${edge.handle} should have a known target node`, edge)
    assert(expectation.schemas.includes(edge.handoff_schema), `${exampleName} edge ${edge.handle} should use a known schema`, edge)
  }

  for (const waitNode of expectation.waitForAll ?? []) {
    const node = (definition.nodes ?? []).find((entry) => entry.handle === waitNode)
    assert(node?.wait_for_all_inputs === true, `${exampleName} node ${waitNode} should wait for all inputs`, node)
  }
  if (expectation.hasLoop) {
    assert(
      (definition.edges ?? []).some((edge) => edge.to_node === expectation.nodes[0]),
      `${exampleName} should include a loop edge back to the first node`,
      definition.edges,
    )
  }
  if (expectation.multiEdgeRouter) {
    const outgoing = (definition.edges ?? []).filter((edge) => edge.from_node === expectation.multiEdgeRouter)
    assert(outgoing.length >= 2, `${exampleName} should model conditional routing as multi-edge agent handoff`, outgoing)
  }

  const providers = new Set((definition.nodes ?? []).map((node) => node.agent?.provider).filter(Boolean))
  assertSameSet(providers, expectation.providers, `${exampleName} provider mix`)
  if ((definition.nodes ?? []).length >= 3) {
    assertSameSet(providers, ['claude', 'codex', 'opencode'], `${exampleName} 3+ node provider coverage`)
  }

  const diagnostics = validationDiagnostics(validation)
  assert(!diagnostics.some((diagnostic) => diagnostic.code === 'canvas_overlap'), `${exampleName} should not have canvas overlap diagnostics`, diagnostics)
  return {
    alias: expectation.alias,
    providers: [...providers].sort(),
    completers: expectation.completers,
    waitForAll: expectation.waitForAll ?? [],
    hasLoop: Boolean(expectation.hasLoop),
    multiEdgeRouter: expectation.multiEdgeRouter ?? null,
  }
}

function hasDirectedCycle(definition) {
  const adjacency = new Map()
  for (const node of definition.nodes ?? []) {
    adjacency.set(node.handle, [])
  }
  for (const edge of definition.edges ?? []) {
    const outgoing = adjacency.get(edge.from_node)
    if (outgoing) outgoing.push(edge.to_node)
  }
  const visiting = new Set()
  const visited = new Set()
  const visit = (node) => {
    if (visiting.has(node)) return true
    if (visited.has(node)) return false
    visiting.add(node)
    for (const next of adjacency.get(node) ?? []) {
      if (visit(next)) return true
    }
    visiting.delete(node)
    visited.add(node)
    return false
  }
  return [...adjacency.keys()].some(visit)
}

function validateLiveExportedTopologyDefinition(exampleName, definition, validation) {
  const expectation = EXAMPLE_TOPOLOGY_EXPECTATIONS[exampleName]
  assert(expectation, `missing workflow-code topology expectation for ${exampleName}`)

  assert(definition?.workflow?.alias?.startsWith(expectation.alias), `${exampleName} live export alias should derive from original alias`, definition?.workflow)
  assert(definition.workflow?.max_concurrent === 32, `${exampleName} live export should preserve maxConcurrent`, definition.workflow)
  const schemaHandles = new Set((definition.schemas ?? []).map((schema) => schema.handle))
  const nodeHandles = new Set((definition.nodes ?? []).map((node) => node.handle))
  assert((definition.schemas ?? []).length === expectation.schemas.length, `${exampleName} live export schema count mismatch`, definition.schemas)
  assert((definition.nodes ?? []).length === expectation.nodes.length, `${exampleName} live export node count mismatch`, definition.nodes)
  assert((definition.edges ?? []).length === expectation.edges.length, `${exampleName} live export edge count mismatch`, definition.edges)
  assert((definition.endpoints ?? []).length === expectation.endpoints.length, `${exampleName} live export endpoint count mismatch`, definition.endpoints)
  assert(schemaHandles.has(definition.workflow?.run_output_schema), `${exampleName} live export run output schema should resolve to an exported schema`, definition.workflow)

  for (const schema of definition.schemas ?? []) {
    assert(schema.schema?.type === 'object', `${exampleName} live export schema ${schema.handle} should be an object schema`, schema)
  }
  for (const node of definition.nodes ?? []) {
    assert(node.instructions, `${exampleName} live export node ${node.handle} should preserve instructions`, node)
    assert(node.canvas && Number.isFinite(node.canvas.x) && Number.isFinite(node.canvas.y), `${exampleName} live export node ${node.handle} should include canvas coordinates`, node)
    assert(node.agent?.kind === 'create', `${exampleName} live export node ${node.handle} should create an agent for portability`, node)
    assert(node.agent?.provider === 'dev-stub', `${exampleName} live export node ${node.handle} should preserve provider rebinding`, node.agent)
    assert(node.agent?.model === 'workflow-code-topology-node', `${exampleName} live export node ${node.handle} should preserve model rebinding`, node.agent)
    if (node.intermediate_output_schema != null) {
      assert(schemaHandles.has(node.intermediate_output_schema), `${exampleName} live export node ${node.handle} intermediate schema should resolve`, node)
    }
  }
  for (const endpoint of definition.endpoints ?? []) {
    assert(endpoint.canvas && Number.isFinite(endpoint.canvas.x) && Number.isFinite(endpoint.canvas.y), `${exampleName} live export endpoint ${endpoint.handle} should include canvas coordinates`, endpoint)
    assert(nodeHandles.has(endpoint.entry_node), `${exampleName} live export endpoint ${endpoint.handle} should target an exported node`, endpoint)
  }
  for (const edge of definition.edges ?? []) {
    assert(nodeHandles.has(edge.from_node), `${exampleName} live export edge ${edge.handle} should have an exported source node`, edge)
    assert(nodeHandles.has(edge.to_node), `${exampleName} live export edge ${edge.handle} should have an exported target node`, edge)
    if (edge.handoff_schema != null) {
      assert(schemaHandles.has(edge.handoff_schema), `${exampleName} live export edge ${edge.handle} handoff schema should resolve`, edge)
    }
  }

  const completers = (definition.nodes ?? []).filter((node) => node.can_complete_workflow_run === true)
  assert(completers.length === expectation.completers.length, `${exampleName} live export completion node count mismatch`, completers)
  const waiters = (definition.nodes ?? []).filter((node) => node.wait_for_all_inputs === true)
  assert(waiters.length === (expectation.waitForAll ?? []).length, `${exampleName} live export wait-for-all node count mismatch`, waiters)
  if (expectation.hasLoop) {
    assert(hasDirectedCycle(definition), `${exampleName} live export should preserve a loop`, definition.edges)
  }
  if (expectation.multiEdgeRouter) {
    const outgoingCounts = new Map()
    for (const edge of definition.edges ?? []) {
      outgoingCounts.set(edge.from_node, (outgoingCounts.get(edge.from_node) ?? 0) + 1)
    }
    assert([...outgoingCounts.values()].some((count) => count >= 2), `${exampleName} live export should preserve a multi-edge router`, definition.edges)
  }

  const diagnostics = validationDiagnostics(validation)
  assert(!diagnostics.some((diagnostic) => diagnostic.code === 'canvas_overlap'), `${exampleName} live export should not have canvas overlap diagnostics`, diagnostics)
}

function parseWorkflowOutputMessage(output) {
  const message = output?.message
  if (message == null) return null
  if (typeof message !== 'string') return message
  try {
    return JSON.parse(message)
  } catch {
    return message
  }
}

function compactWorkflowRunSummary(run) {
  if (!run) return null
  return {
    id: run.id,
    status: run.status,
    active_node_run_id: run.active_node_run_id ?? null,
    messages: run.messages?.length ?? 0,
    intermediate_outputs: run.intermediate_outputs?.length ?? 0,
    final_output_valid: run.final_output_valid ?? null,
    node_runs: (run.node_runs ?? []).map((nodeRun) => ({
      id: nodeRun.id,
      node_id: nodeRun.node_id,
      agent_id: nodeRun.agent_id,
      status: nodeRun.status,
      iteration_index: nodeRun.iteration_index,
      failures: nodeRun.failures?.length ?? 0,
      completed: Boolean(nodeRun.completed_at_ms),
      summary: nodeRun.summary ?? nodeRun.completion?.summary ?? null,
      recent_failure_events: (nodeRun.failure_events ?? []).slice(-3).map((event) => ({
        kind: event.kind,
        message: event.message,
      })),
    })),
  }
}

function validateExampleRuntimeResult(exampleName, run) {
  assert(run?.status === 'Completed', `example ${exampleName} runtime should complete`, run)
  assert(run.final_output_valid !== false, `example ${exampleName} final output should be schema-valid`, run)
  const finalOutput = parseWorkflowOutputMessage(run.final_output)
  assert(finalOutput && typeof finalOutput === 'object', `example ${exampleName} should produce structured final output`, run.final_output)
  const completionHandoffCount = (run.node_runs ?? []).reduce((count, nodeRun) => {
    const output = parseWorkflowOutputMessage(nodeRun.completion?.output)
    return count + (Array.isArray(output?.workflow_handoffs) ? output.workflow_handoffs.length : 0)
  }, 0)
  const messageCount = (run.messages?.length ?? 0) || completionHandoffCount
  const nodeRunCount = run.node_runs?.length ?? 0
  const expectations = {
    'adversarial-verification.js': {
      minMessages: 4,
      fields: { decision: 'accept' },
      note: 'proposer/critic loop plus judge handoff',
    },
    'evaluator-optimizer.js': {
      minMessages: 3,
      fields: { accepted: true },
      note: 'optimizer revision loop',
    },
    'fan-out-synthesize.js': {
      minMessages: 4,
      fields: { source_count: 2 },
      note: 'two workers and synthesizer join',
    },
    'generate-filter.js': {
      minMessages: 2,
      fields: { selected_count: 1 },
      note: 'candidate generation and filter',
    },
    'loop-until-done.js': {
      minMessages: 3,
      fields: { iterations: 2 },
      note: 'one revise loop before completion',
    },
    'orchestrator-workers.js': {
      minMessages: 2,
      fields: { delegated: true },
      note: 'orchestrator assignment and synthesis',
    },
    'parallelization.js': {
      minMessages: 4,
      fields: { reviewer_count: 2 },
      note: 'two independent reviewers and aggregation',
    },
    'prompt-chaining.js': {
      minMessages: 1,
      fields: { answer: 'refined draft accepted' },
      note: 'draft handoff to refiner',
    },
    'routing.js': {
      minMessages: 1,
      maxMessages: 1,
      fields: { specialist: 'code' },
      note: 'router chooses exactly one specialist edge',
    },
    'tournament.js': {
      minMessages: 4,
      fields: { winner: 'a' },
      note: 'two contestants and judge',
    },
  }[exampleName]
  assert(expectations, `missing runtime expectation for ${exampleName}`)
  assert(messageCount >= expectations.minMessages, `example ${exampleName} runtime should emit enough handoffs for ${expectations.note}`, { messageCount, run })
  if (expectations.maxMessages != null) {
    assert(messageCount <= expectations.maxMessages, `example ${exampleName} runtime should not emit extra routed handoffs`, { messageCount, run })
  }
  for (const [field, value] of Object.entries(expectations.fields)) {
    assert(finalOutput[field] === value, `example ${exampleName} final output field ${field} mismatch`, { finalOutput, expected: value })
  }
  return {
    status: run.status,
    messages: messageCount,
    nodeRuns: nodeRunCount,
    finalOutput,
  }
}

async function validateTopologyTuiOutlineProjection(client, sessionId, exampleName, apply, completedRun) {
  const stateResponse = await client.send(getSessionStateRequest(sessionId))
  const session = unwrap(stateResponse, 'SessionStateLoaded')?.session
    ?? unwrap(stateResponse, 'SessionState')?.session
  const workflow = (session?.workflows ?? []).find((entry) => entry.id === apply.workflow_id)
  assert(workflow, `example ${exampleName} TUI outline should find workflow in session`, { workflowId: apply.workflow_id })
  const entryNodeId = completedRun.entry_node_id ?? workflow.endpoints?.[0]?.entry_node_id ?? workflow.nodes?.[0]?.id
  const outline = buildWorkflowOutline({
    workflow,
    agents: session?.agents ?? [],
    workflowRuns: session?.workflow_runs ?? [],
    selectedNodeId: entryNodeId ?? null,
  })
  const rendered = renderWorkflowOutlineToText(outline)
  assert(rendered.includes(`workflow: ${workflow.id}`), `example ${exampleName} TUI outline should include workflow id`, rendered)
  assert(rendered.includes(`status ${String(completedRun.status).toLowerCase()}`), `example ${exampleName} TUI outline should include run status`, rendered)
  const finalMessage = completedRun.final_output?.message
  const finalOutputVisible = Boolean(finalMessage && renderedIncludesWorkflowFinalOutput(rendered, finalMessage))
  assert(finalOutputVisible, `example ${exampleName} TUI outline should include final output`, {
    rendered,
    finalMessage,
  })
  for (const node of workflow.nodes ?? []) {
    assert(rendered.includes(`node ${node.id}`), `example ${exampleName} TUI outline should include node ${node.id}`, rendered)
  }
  for (const edge of workflow.edges ?? []) {
    assert(rendered.includes(edge.id), `example ${exampleName} TUI outline should include edge ${edge.id}`, rendered)
    assert(rendered.includes(edge.from_node_id), `example ${exampleName} TUI outline should include edge source ${edge.from_node_id}`, rendered)
    assert(rendered.includes(edge.to_node_id), `example ${exampleName} TUI outline should include edge target ${edge.to_node_id}`, rendered)
  }
  for (const endpoint of workflow.endpoints ?? []) {
    assert(rendered.includes(endpoint.id), `example ${exampleName} TUI outline should include endpoint ${endpoint.id}`, rendered)
  }
  return {
    workflowId: workflow.id,
    workflowRunId: completedRun.id,
    statusVisible: rendered.includes(`status ${String(completedRun.status).toLowerCase()}`),
    finalOutputVisible,
    nodes: workflow.nodes?.length ?? 0,
    edges: workflow.edges?.length ?? 0,
    endpoints: workflow.endpoints?.length ?? 0,
  }
}

function renderedIncludesWorkflowFinalOutput(rendered, message) {
  const singleLine = String(message).replace(/\s+/g, ' ').trim()
  if (singleLine.length <= 180) {
    return rendered.includes(`final output: ${singleLine}`)
  }
  return rendered.includes(`final output: ${singleLine.slice(0, 177)}...`)
}

function validateApplyResult(result, label, expected = defaultToyExpectation()) {
  assert(result?.compile?.validation?.ok, `${label} compile validation failed`, result?.compile?.validation)
  const apply = result.apply
  assert(apply?.workflow_id, `${label} did not return workflow id`, result)
  assert(Object.keys(apply.node_ids ?? {}).length === expected.nodes, `${label} should create ${expected.nodes} nodes`, apply)
  assert(Object.keys(apply.agent_ids ?? {}).length === expected.agents, `${label} should resolve ${expected.agents} node agents`, apply)
  assert(Object.keys(apply.edge_ids ?? {}).length === expected.edges, `${label} should create ${expected.edges} edges`, apply)
  assert(Object.keys(apply.endpoint_ids ?? {}).length === expected.endpoints, `${label} should create ${expected.endpoints} endpoints`, apply)
  assert(Object.keys(apply.queue_ids ?? {}).length === (expected.queues ?? 1), `${label} should create ${expected.queues ?? 1} queues`, apply)
  assert(Object.keys(apply.schedule_ids ?? apply.watchdog_ids ?? {}).length === (expected.schedules ?? 0), `${label} should create ${expected.schedules ?? 0} schedules`, apply)
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
  for (const [handle, queueId] of Object.entries(apply.queue_ids ?? {})) {
    const queue = (session.workflow_prompt_queues ?? []).find((entry) => entry.id === queueId)
    assert(queue, `${label} prompt queue ${handle} should appear in session`, { queueId })
    assert(queue.workflow_id === apply.workflow_id, `${label} prompt queue ${handle} should belong to workflow`, { queue, apply })
    if (handle === 'urgent') {
      assert(queue.alias === 'urgent', `${label} urgent queue alias should be projected`, queue)
      assert(queue.priority === 10, `${label} urgent queue priority should be projected`, queue)
      assert(queue.enabled === false, `${label} urgent queue enabled state should be projected`, queue)
    }
  }
  for (const [handle, scheduleId] of Object.entries(apply.schedule_ids ?? apply.watchdog_ids ?? {})) {
    const schedule = (session.workflow_schedules ?? session.workflow_watchdogs ?? []).find((entry) => entry.id === scheduleId)
    assert(schedule, `${label} schedule ${handle} should appear in session`, { scheduleId })
    assert(schedule.workflow_id === apply.workflow_id, `${label} schedule ${handle} should belong to workflow`, { schedule, apply })
    if (handle === 'entry_schedule') {
      assert(schedule.queue_id === apply.queue_ids?.urgent, `${label} schedule should target the scripted urgent queue`, { schedule, apply })
      assert(schedule.trigger?.kind === 'interval' && schedule.trigger.every_seconds === 300, `${label} schedule interval should be projected`, schedule)
      assert(schedule.invocation_prompt === 'Wake the workflow-code artifact drill.', `${label} schedule prompt should be projected`, schedule)
    }
  }
  for (const [handle, agentId] of Object.entries(apply.agent_ids ?? {})) {
    const agent = (session.agents ?? []).find((entry) => entry.id === agentId)
    assert(agent, `${label} node agent ${handle} should appear in session`, { agentId })
    const expectedProvider = expected.providerByNode?.[handle] ?? 'dev-stub'
    const expectedModel = expected.modelByNode?.[handle] ?? expected.agentModel ?? 'default'
    assert(agent.provider === expectedProvider, `${label} node agent ${handle} should use ${expectedProvider}`, agent)
    assert(agent.model === expectedModel, `${label} node agent ${handle} should use ${expectedModel} model`, agent)
    for (const extension of expected.nodeExtensions?.[handle] ?? []) {
      assert(
        (agent.extension_grants ?? []).some((grant) => (
          grant.kind === extension.kind
          && grant.name === extension.name
          && (extension.environment === undefined || grant.environment === extension.environment)
          && (extension.credential === undefined || grant.credential === extension.credential)
          && (extension.max_safety === undefined || grant.max_safety === extension.max_safety)
        )),
        `${label} node agent ${handle} should receive extension ${extension.kind}:${extension.name}`,
        { agent, expectedExtension: extension },
      )
    }
  }
}

async function writeWorkflowCodeArtifactSkillSource(skillSourceRoot, skillName) {
  const skillDir = path.join(skillSourceRoot, skillName)
  await mkdir(skillDir, { recursive: true })
  await writeFile(
    path.join(skillDir, 'SKILL.md'),
    [
      '---',
      `name: ${skillName}`,
      'description: Skill used by the workflow-code artifact drill.',
      '---',
      'Use this skill only for workflow-code artifact drill validation.',
      '',
    ].join('\n'),
    'utf8',
  )
  return skillDir
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

async function completeAppliedTopologyWorkflow(client, sessionId, exampleName, definition, apply, timeoutMs) {
  const endpoint = (definition.endpoints ?? []).find((entry) => entry.handle === 'entry' || entry.alias === 'entry')
    ?? definition.endpoints?.[0]
  const entryNodeHandle = endpoint?.entry_node
  assert(entryNodeHandle, `example ${exampleName} runtime should resolve entry node`, { definition, apply })
  const runtimeAgentEntries = Object.entries(apply.agent_ids ?? {})
    .sort(([left], [right]) => {
      if (left === entryNodeHandle) return 1
      if (right === entryNodeHandle) return -1
      return left.localeCompare(right)
    })
  for (const [handle, agentId] of runtimeAgentEntries) {
    const launchResponse = unwrap(
      await client.send(launchProviderRunRequest(
        sessionId,
        'dev-stub',
        'default',
        'workflow-code-topology-node',
        'low',
        agentId,
      )),
      'ProviderRunLaunchAccepted',
    )
    assert(launchResponse?.provider_run?.id, `example ${exampleName} runtime should launch provider for node ${handle}`, launchResponse)
    await waitForProviderRunReady(client, launchResponse.provider_run.id, timeoutMs)
  }
  const endpointId = apply.endpoint_ids?.[endpoint.handle]
  assert(endpointId, `example ${exampleName} runtime should resolve entry endpoint`, { endpoint, apply })
  const entryAgentId = apply.agent_ids?.[entryNodeHandle]
  assert(entryAgentId, `example ${exampleName} runtime should resolve entry agent`, { entryNodeHandle, apply })
  await client.send(focusAgentRequest(sessionId, entryAgentId))
  const invokeResponse = unwrap(
    await client.send(invokeWorkflowEndpointRequest(
      sessionId,
      apply.workflow_id,
      endpointId,
      `Run ${exampleName} topology runtime validation.`,
    )),
    'WorkflowRunInvoked',
  )
  const runtimeRun = invokeResponse?.workflow_run
  assert(runtimeRun?.id, `example ${exampleName} runtime should start a workflow run`, invokeResponse)
  const completedRun = await waitForCompletedWorkflowRun(client, sessionId, runtimeRun.id, timeoutMs)
  const tuiOutline = await validateTopologyTuiOutlineProjection(
    client,
    sessionId,
    exampleName,
    apply,
    completedRun,
  )
  return {
    workflowRunId: completedRun.id,
    runtime: validateExampleRuntimeResult(exampleName, completedRun),
    tuiOutline,
  }
}

function validateRealProviderTopologyRuntimeResult(exampleName, run) {
  assert(run?.status === 'Completed', `real-provider example ${exampleName} runtime should complete`, run)
  assert(run.final_output_valid === true, `real-provider example ${exampleName} final output should be schema-valid`, run)
  const finalOutput = parseWorkflowOutputMessage(run.final_output)
  assert(
    finalOutput && typeof finalOutput === 'object' && !Array.isArray(finalOutput),
    `real-provider example ${exampleName} should produce structured final output`,
    run.final_output,
  )
  assert(
    (run.node_runs?.length ?? 0) >= 3,
    `real-provider example ${exampleName} should execute at least three workflow nodes`,
    run,
  )
  return {
    status: run.status,
    nodeRuns: run.node_runs?.length ?? 0,
    finalOutput,
  }
}

async function completeAppliedTopologyWorkflowWithRealProviders(client, sessionId, exampleName, definition, apply, rebindings, timeoutMs) {
  const endpoint = (definition.endpoints ?? []).find((entry) => entry.handle === 'entry' || entry.alias === 'entry')
    ?? definition.endpoints?.[0]
  const entryNodeHandle = endpoint?.entry_node
  assert(entryNodeHandle, `real-provider example ${exampleName} runtime should resolve entry node`, { definition, apply })
  const rebindingsByNode = rebindingByNode(rebindings)
  const runtimeAgentEntries = Object.entries(apply.agent_ids ?? {})
    .sort(([left], [right]) => {
      if (left === entryNodeHandle) return 1
      if (right === entryNodeHandle) return -1
      return left.localeCompare(right)
    })
  const prelaunchedProviders = new Set()
  const prelaunchedProviderFamilies = new Set()
  const configuredProviderFamilies = new Set((rebindings ?? []).map((entry) => providerFamily(entry.provider)))
  for (const [handle, agentId] of runtimeAgentEntries) {
    const rebinding = rebindingsByNode.get(handle)
    assert(rebinding, `real-provider example ${exampleName} missing rebinding for node ${handle}`, { handle, rebindings })
    assert(rebinding.provider !== 'dev-stub', `real-provider example ${exampleName} must not launch dev-stub for node ${handle}`, rebinding)
    if (!shouldPrelaunchRealProvider(rebinding.provider)) {
      stage(`real-provider ${exampleName}: deferring node ${handle} launch to workflow dispatch`, {
        provider: rebinding.provider,
        model: rebinding.model,
      })
      continue
    }
    stage(`real-provider ${exampleName}: launching node ${handle}`, {
      provider: rebinding.provider,
      model: rebinding.model,
      account_profile: rebinding.account_profile ?? 'default',
      effort: rebinding.effort ?? '',
    })
    const launchResponse = unwrap(
      await client.send(launchProviderRunRequest(
        sessionId,
        rebinding.provider,
        rebinding.account_profile ?? 'default',
        rebinding.model,
        rebinding.effort ?? 'low',
        agentId,
      )),
      'ProviderRunLaunchAccepted',
    )
    assert(launchResponse?.provider_run?.id, `real-provider example ${exampleName} should launch provider for node ${handle}`, launchResponse)
    try {
      await waitForProviderRunReady(client, launchResponse.provider_run.id, timeoutMs)
    } catch (error) {
      throw new Error([
        `real-provider example ${exampleName} provider launch failed for node ${handle}`,
        `provider=${rebinding.provider}`,
        `model=${rebinding.model}`,
        `account_profile=${rebinding.account_profile ?? 'default'}`,
        `effort=${rebinding.effort ?? 'low'}`,
        `provider_run_id=${launchResponse.provider_run.id}`,
        error?.message ?? String(error),
      ].join('\n'))
    }
    stage(`real-provider ${exampleName}: provider ready for node ${handle}`, {
      provider: rebinding.provider,
      provider_run_id: launchResponse.provider_run.id,
    })
    prelaunchedProviders.add(rebinding.provider)
    prelaunchedProviderFamilies.add(providerFamily(rebinding.provider))
  }
  assertSameSet(configuredProviderFamilies, ['claude', 'codex', 'opencode'], `real-provider example ${exampleName} configured provider families`)

  const endpointId = apply.endpoint_ids?.[endpoint.handle]
  assert(endpointId, `real-provider example ${exampleName} runtime should resolve entry endpoint`, { endpoint, apply })
  const entryAgentId = apply.agent_ids?.[entryNodeHandle]
  assert(entryAgentId, `real-provider example ${exampleName} runtime should resolve entry agent`, { entryNodeHandle, apply })
  await client.send(focusAgentRequest(sessionId, entryAgentId))
  stage(`real-provider ${exampleName}: invoking workflow endpoint`, {
    workflow_id: apply.workflow_id,
    endpoint_id: endpointId,
    entry_node: entryNodeHandle,
  })
  const invokeResponse = unwrap(
    await client.send(invokeWorkflowEndpointRequest(
      sessionId,
      apply.workflow_id,
      endpointId,
      [
        `Run ${exampleName} as a short release-validation workflow.`,
        'Use concise handoffs.',
        'When you complete the workflow, submit final output that strictly matches the workflow final-output schema.',
      ].join(' '),
    )),
    'WorkflowRunInvoked',
  )
  const runtimeRun = invokeResponse?.workflow_run
  assert(runtimeRun?.id, `real-provider example ${exampleName} runtime should start a workflow run`, invokeResponse)
  stage(`real-provider ${exampleName}: workflow run started`, { workflow_run_id: runtimeRun.id })
  const completedRun = await waitForCompletedWorkflowRun(
    client,
    sessionId,
    runtimeRun.id,
    timeoutMs,
    `real-provider ${exampleName}`,
  )
  const runtime = validateRealProviderTopologyRuntimeResult(exampleName, completedRun)
  const tuiOutline = await validateTopologyTuiOutlineProjection(
    client,
    sessionId,
    exampleName,
    apply,
    completedRun,
  )
  return {
    workflowRunId: completedRun.id,
    runtime,
    tuiOutline,
    configuredProviderFamilies: [...configuredProviderFamilies].sort(),
    prelaunchedProviders: [...prelaunchedProviders].sort(),
    prelaunchedProviderFamilies: [...prelaunchedProviderFamilies].sort(),
  }
}

async function applyRealProviderTopology(client, sessionId, nodePath, exampleName, source, options) {
  const validated = unwrap(
    await client.send(validateWorkflowCodeRequest(sessionId, nodePath, source)),
    'WorkflowCodeValidated',
  ).result
  assert(validated?.validation?.ok, `real-provider example ${exampleName} should validate`, validated?.validation)
  const topology = validateExampleTopologyDefinition(exampleName, validated.definition, validated.validation)
  const providers = providerSetForDefinition(validated.definition)
  assertSameSet(providers, ['claude', 'codex', 'opencode'], `real-provider example ${exampleName} provider mix`)
  assert((validated.definition.nodes?.length ?? 0) >= 3, `real-provider example ${exampleName} must contain at least three nodes`, validated.definition)

  const rebindings = realProviderRebindingsForDefinition(validated.definition, options)
  const providerByNode = Object.fromEntries(rebindings.map((entry) => [entry.node, entry.provider]))
  const modelByNode = Object.fromEntries(rebindings.map((entry) => [entry.node, entry.model]))
  const appliedResponse = unwrap(
    await client.send(applyWorkflowCodeRequest(sessionId, nodePath, source, rebindings)),
    'WorkflowCodeApplied',
  )
  const expected = expectationFromDefinition(validated.definition)
  const apply = validateApplyResult(appliedResponse.result, `real-provider example ${exampleName}`, expected)
  validateSessionProjection(appliedResponse.session, apply, `real-provider example ${exampleName}`, {
    ...expected,
    providerByNode,
    modelByNode,
  })
  const completed = await completeAppliedTopologyWorkflowWithRealProviders(
    client,
    sessionId,
    exampleName,
    validated.definition,
    apply,
    rebindings,
    options.timeoutMs,
  )
  return {
    example: exampleName,
    topology,
    workflowId: apply.workflow_id,
    workflowRunId: completed.workflowRunId,
    rebindings,
    runtime: completed.runtime,
    tuiOutline: completed.tuiOutline,
    configuredProviderFamilies: completed.configuredProviderFamilies,
    prelaunchedProviders: completed.prelaunchedProviders,
    prelaunchedProviderFamilies: completed.prelaunchedProviderFamilies,
  }
}

async function runRealProviderTopologyDrill(client, sessionId, nodePath, options) {
  const examples = await workflowCodeExamples()
  const requested = options.realProviderTopology
  const exampleName = requested.endsWith('.js') ? requested : `${requested}.js`
  const example = examples.find((entry) => entry.name === exampleName)
  assert(example, `real-provider topology example not found: ${requested}`, {
    requested,
    available: examples.map((entry) => entry.name),
  })
  return await applyRealProviderTopology(client, sessionId, nodePath, example.name, example.source, options)
}

async function applyExampleSuite(client, sessionId, nodePath, workspace, timeoutMs) {
  const examples = await workflowCodeExamples()
  const results = []
  const liveExports = []
  for (const [index, example] of examples.entries()) {
    const slug = example.name.replace(/\.js$/, '')
    const artifactName = `pattern-${index + 1}-${slug}-${Date.now()}`
    const created = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, artifactName, nodePath, example.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(created?.metadata?.validation?.ok, `example ${example.name} should validate`, created?.metadata?.validation)

    const exported = unwrap(
      await client.send(exportWorkflowCodeArtifactRequest(sessionId, artifactName)),
      'WorkflowCodeArtifactExported',
    ).package
    assert(exported?.source_sha256, `example ${example.name} package export should include source hash`, exported)
    assert(exported?.definition_sha256, `example ${example.name} package export should include definition hash`, exported)
    const topology = validateExampleTopologyDefinition(
      example.name,
      exported.definition,
      created.metadata.validation,
    )

    const importedArtifactName = `${artifactName}-package-imported`
    const imported = unwrap(
      await client.send(importWorkflowCodeArtifactRequest(sessionId, exported, nodePath, {
        name: importedArtifactName,
        overwrite: false,
      })),
      'WorkflowCodeArtifactImported',
    ).artifact
    assert(imported?.metadata?.validation?.ok, `example ${example.name} package import should validate`, imported?.metadata?.validation)

    const expected = expectationFromDefinition(exported.definition)
    const rebindings = rebindingsForDefinition(exported.definition)
    const appliedResponse = unwrap(
      await client.send(applyWorkflowCodeArtifactRequest(sessionId, importedArtifactName, rebindings)),
      'WorkflowCodeApplied',
    )
    const apply = validateApplyResult(appliedResponse.result, `example ${example.name}`, expected)
    validateSessionProjection(appliedResponse.session, apply, `example ${example.name}`, expected)

    const inlineSource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        sessionId,
        { kind: 'artifact', name: artifactName },
        'inline',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(inlineSource?.source, `example ${example.name} inline source export should include source`, inlineSource)
    assert(
      sha256Hex(inlineSource.source) === inlineSource.source_sha256,
      `example ${example.name} inline source hash should match contents`,
      inlineSource,
    )
    const inlineRoundTripName = `${artifactName}-inline-source`
    const inlineRoundTrip = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, inlineRoundTripName, nodePath, inlineSource.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(inlineRoundTrip?.metadata?.validation?.ok, `example ${example.name} inline source should recompile`, inlineRoundTrip?.metadata?.validation)
    validateExampleTopologyDefinition(
      example.name,
      inlineRoundTrip.definition,
      inlineRoundTrip.metadata.validation,
    )

    const directorySource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        sessionId,
        { kind: 'artifact', name: artifactName },
        'directory',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(directorySource?.source_path === 'workflow.js', `example ${example.name} source directory should export workflow.js`, directorySource)
    const directoryManifest = await writeSourceDirectoryExport(workspace, directorySource, `example ${example.name}`)
    assert(directoryManifest?.schema_paths, `example ${example.name} source directory manifest should include schema_paths`, directoryManifest)
    const directoryRoundTripName = `${artifactName}-directory-source`
    const directoryRoundTrip = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, directoryRoundTripName, nodePath, directorySource.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(directoryRoundTrip?.metadata?.validation?.ok, `example ${example.name} source directory should recompile`, directoryRoundTrip?.metadata?.validation)
    validateExampleTopologyDefinition(
      example.name,
      directoryRoundTrip.definition,
      directoryRoundTrip.metadata.validation,
    )

    const runtimeRebindings = topologyRuntimeRebindingsForDefinition(exported.definition)
    const runtimeExpected = topologyRuntimeExpectation(exported.definition)
    const runtimeAppliedResponse = unwrap(
      await client.send(applyWorkflowCodeArtifactRequest(sessionId, importedArtifactName, runtimeRebindings)),
      'WorkflowCodeApplied',
    )
    const runtimeApply = validateApplyResult(runtimeAppliedResponse.result, `example ${example.name} runtime`, runtimeExpected)
    validateSessionProjection(runtimeAppliedResponse.session, runtimeApply, `example ${example.name} runtime`, runtimeExpected)

    const liveInlineSource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        sessionId,
        { kind: 'workflow', workflow_ref: runtimeApply.workflow_id },
        'inline',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(liveInlineSource?.source, `example ${example.name} live inline source export should include source`, liveInlineSource)
    assert(
      sha256Hex(liveInlineSource.source) === liveInlineSource.source_sha256,
      `example ${example.name} live inline source hash should match contents`,
      liveInlineSource,
    )
    const liveInlineRoundTripName = `${artifactName}-live-inline-source`
    const liveInlineRoundTrip = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, liveInlineRoundTripName, nodePath, liveInlineSource.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(liveInlineRoundTrip?.metadata?.validation?.ok, `example ${example.name} live inline source should recompile`, liveInlineRoundTrip?.metadata?.validation)
    validateLiveExportedTopologyDefinition(
      example.name,
      liveInlineRoundTrip.definition,
      liveInlineRoundTrip.metadata.validation,
    )

    const liveDirectorySource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        sessionId,
        { kind: 'workflow', workflow_ref: runtimeApply.workflow_id },
        'directory',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(liveDirectorySource?.source_path === 'workflow.js', `example ${example.name} live source directory should export workflow.js`, liveDirectorySource)
    const liveDirectoryManifest = await writeSourceDirectoryExport(workspace, liveDirectorySource, `example ${example.name} live source directory`)
    assert(liveDirectoryManifest?.schema_paths, `example ${example.name} live source directory manifest should include schema_paths`, liveDirectoryManifest)
    const liveDirectoryRoundTripName = `${artifactName}-live-directory-source`
    const liveDirectoryRoundTrip = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, liveDirectoryRoundTripName, nodePath, liveDirectorySource.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(liveDirectoryRoundTrip?.metadata?.validation?.ok, `example ${example.name} live source directory should recompile`, liveDirectoryRoundTrip?.metadata?.validation)
    validateLiveExportedTopologyDefinition(
      example.name,
      liveDirectoryRoundTrip.definition,
      liveDirectoryRoundTrip.metadata.validation,
    )

    const completed = await completeAppliedTopologyWorkflow(
      client,
      sessionId,
      example.name,
      exported.definition,
      runtimeApply,
      timeoutMs,
    )

    liveExports.push({
      exampleName: example.name,
      sourceWorkflowId: runtimeApply.workflow_id,
      inlineSource: liveInlineSource,
      directorySource: liveDirectorySource,
    })

    results.push({
      example: example.name,
      artifactName,
      topology,
      runtime: completed.runtime,
      tuiOutline: completed.tuiOutline,
      packageImportedArtifactName: importedArtifactName,
      workflowId: apply.workflow_id,
      runtimeWorkflowId: runtimeApply.workflow_id,
      runtimeWorkflowRunId: completed.workflowRunId,
      nodes: expected.nodes,
      edges: expected.edges,
      endpoints: expected.endpoints,
      schemas: Object.keys(apply.schema_refs ?? {}),
      packageSha256: exported.source_sha256,
      inlineSourceSha256: inlineSource.source_sha256,
      directorySourceSha256: directorySource.source_sha256,
      liveInlineSourceSha256: liveInlineSource.source_sha256,
      liveDirectorySourceSha256: liveDirectorySource.source_sha256,
      directoryFiles: (directorySource.files ?? []).map((file) => file.path).sort(),
      liveDirectoryFiles: (liveDirectorySource.files ?? []).map((file) => file.path).sort(),
    })
  }
  return { results, liveExports }
}

async function writeSourceDirectoryExport(workspace, exportResult, label) {
  const files = exportResult.files ?? []
  assert(files.some((file) => file.path === 'workflow.js'), `${label} source directory should include workflow.js`, exportResult)
  const manifestFile = files.find((file) => file.path === 'manifest.json')
  assert(manifestFile, `${label} source directory should include manifest.json`, exportResult)
  for (const file of files) {
    assert(sha256Hex(file.contents) === file.sha256, `${label} source directory file hash mismatch for ${file.path}`, file)
    const target = path.join(workspace, file.path)
    await mkdir(path.dirname(target), { recursive: true })
    await writeFile(target, file.contents, 'utf8')
  }
  const manifest = JSON.parse(manifestFile.contents)
  assert(manifest.source_path === 'workflow.js', `${label} manifest source_path should be workflow.js`, manifest)
  assert(manifest.source_sha256 === exportResult.source_sha256, `${label} manifest source hash should match export`, {
    manifest,
    exportResult,
  })
  assert(manifest.definition_sha256 === exportResult.definition_sha256, `${label} manifest definition hash should match export`, {
    manifest,
    exportResult,
  })
  return manifest
}

function sha256Hex(value) {
  return createHash('sha256').update(value).digest('hex')
}

function validateArtifactHistory(artifact, expectedActions) {
  const actions = (artifact?.metadata?.history ?? []).map((entry) => entry.action)
  for (const action of expectedActions) {
    assert(actions.includes(action), `artifact history should include ${action}`, actions)
  }
}

async function startIsolatedKernel(label, rootDir, workspace, worktree, timeoutMs) {
  await mkdir(workspace, { recursive: true })
  await mkdir(worktree, { recursive: true })
  const spawned = spawnedKernel(label, rootDir)
  const daemonChild = spawn(buildKernel(), [], {
    cwd: repoRoot,
    env: spawned.env,
    stdio: ['ignore', 'ignore', 'inherit'],
  })
  const client = new LocalIpcClient(spawned.kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    await waitForKernel(client, workspace, worktree, timeoutMs)
    return { client, daemonChild, kernelUrl: spawned.kernelUrl }
  } catch (error) {
    daemonChild.kill('SIGTERM')
    await client.close().catch(() => {})
    throw error
  }
}

async function validateSourceExportOnKernel(client, session, nodePath, exportResult, label, timeoutMs) {
  assert(exportResult?.source, `${label} should include source`, exportResult)
  assert(
    sha256Hex(exportResult.source) === exportResult.source_sha256,
    `${label} source hash should match contents`,
    exportResult,
  )
  const validated = unwrap(
    await client.send(validateWorkflowCodeRequest(session.id, nodePath, exportResult.source)),
    'WorkflowCodeValidated',
  ).result
  assert(validated?.validation?.ok, `${label} should validate on second kernel`, validated?.validation)
  const expected = expectationFromDefinition(validated.definition)
  const endpointHandle = validated.definition.endpoints?.[0]?.handle
  assert(endpointHandle, `${label} should define an endpoint handle`, validated.definition)
  const appliedResponse = unwrap(
    await client.send(applyWorkflowCodeRequest(session.id, nodePath, exportResult.source)),
    'WorkflowCodeApplied',
  )
  const apply = validateApplyResult(appliedResponse.result, `${label} apply`, expected)
  validateSessionProjection(appliedResponse.session, apply, `${label} apply`, expected)
  const runResponse = unwrap(
    await client.send(runWorkflowCodeRequest(session.id, nodePath, exportResult.source, `Run ${label}.`, {
      endpoint: endpointHandle,
    })),
    'WorkflowCodeRun',
  )
  const runApply = validateApplyResult(runResponse.result.apply, `${label} run`, expected)
  validateSessionProjection(runResponse.session, runApply, `${label} run`, expected)
  assert(runResponse.result.invocation?.workflow_run || runResponse.result.invocation?.queued_prompt, `${label} should invoke or enqueue`, runResponse.result.invocation)
  assert(apply.workflow_id !== runApply.workflow_id, `${label} run should apply a fresh workflow`, { apply, runApply })
  return {
    validatedAlias: validated.definition.workflow?.alias ?? null,
    applyWorkflowId: apply.workflow_id,
    runWorkflowId: runApply.workflow_id,
    runInvocation: runResponse.result.invocation.workflow_run ? 'started' : 'enqueued',
    timeoutMs,
  }
}

async function validateSecondKernelDistribution({
  packageExport,
  inlineSource,
  directorySource,
  sourceWorkflowId,
  skillSourceDir,
  skillName,
  generatedRoot,
  nodePath,
  timeoutMs,
}) {
  const secondRoot = path.join(generatedRoot, 'second-kernel')
  const secondWorkspace = path.join(secondRoot, 'workspace')
  const secondWorktree = path.join(secondRoot, 'worktree')
  const { client, daemonChild, kernelUrl } = await startIsolatedKernel(
    'workflow-code-second-kernel-drill',
    secondRoot,
    secondWorkspace,
    secondWorktree,
    timeoutMs,
  )
  let sessionId = null
  let installedSkill = false
  try {
    const session = unwrap(
      await client.send(createSessionRequest(secondWorkspace, secondWorktree, 'workflow-code-second-kernel-drill', undefined, null, 'off')),
      'SessionCreated',
    ).session
    sessionId = session.id
    const skillInstall = unwrap(
      await client.send(installSkillRequest(secondWorkspace, skillSourceDir)),
      'SkillInstalled',
    )
    assert(skillInstall?.skill?.name === skillName, 'second kernel should install workflow-code drill skill', skillInstall)
    installedSkill = true

    const importedName = `second-kernel-package-${Date.now()}`
    const imported = unwrap(
      await client.send(importWorkflowCodePackageRequest(session.id, packageExport, nodePath, {
        name: importedName,
        overwrite: false,
      })),
      'WorkflowCodePackageImported',
    ).artifact
    assert(imported?.metadata?.validation?.ok, 'second kernel package import should validate', imported?.metadata?.validation)
    const packageExpected = expectationFromDefinition(packageExport.definition)
    const packageRebindings = rebindingsForDefinition(packageExport.definition)
    const packageApplyResponse = unwrap(
      await client.send(applyWorkflowCodeArtifactRequest(session.id, importedName, packageRebindings)),
      'WorkflowCodeApplied',
    )
    const packageApply = validateApplyResult(packageApplyResponse.result, 'second kernel package', packageExpected)
    validateSessionProjection(packageApplyResponse.session, packageApply, 'second kernel package', packageExpected)
    const packageRun = unwrap(
      await client.send(runWorkflowCodeArtifactRequest(session.id, importedName, 'Run the second-kernel package workflow.', {
        endpoint: 'entry',
        providerRebindings: packageRebindings,
      })),
      'WorkflowCodeRun',
    )
    const packageRunApply = validateApplyResult(packageRun.result.apply, 'second kernel package run', packageExpected)
    validateSessionProjection(packageRun.session, packageRunApply, 'second kernel package run', packageExpected)
    assert(packageApply.workflow_id !== sourceWorkflowId, 'second kernel package workflow id must be fresh', { packageApply, sourceWorkflowId })
    assert(packageRunApply.workflow_id !== sourceWorkflowId, 'second kernel package run workflow id must be fresh', { packageRunApply, sourceWorkflowId })

    const inline = await validateSourceExportOnKernel(
      client,
      session,
      nodePath,
      inlineSource,
      'second kernel inline source',
      timeoutMs,
    )

    const directoryManifest = await writeSourceDirectoryExport(
      secondWorkspace,
      directorySource,
      'second kernel source directory',
    )
    const directory = await validateSourceExportOnKernel(
      client,
      session,
      nodePath,
      directorySource,
      'second kernel source directory',
      timeoutMs,
    )

    return {
      kernelUrl,
      sessionId: session.id,
      packageImportedArtifact: importedName,
      packageApplyWorkflowId: packageApply.workflow_id,
      packageRunWorkflowId: packageRunApply.workflow_id,
      inline,
      directory,
      directoryManifest,
    }
  } finally {
    if (client && sessionId) {
      if (installedSkill) {
        await client.send(uninstallSkillRequest(secondWorkspace, skillName)).catch(() => {})
      }
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
    daemonChild.kill('SIGTERM')
    await sleep(1000)
  }
}

async function validateTopologySourceExportOnKernel(client, session, nodePath, workspace, exportResult, exampleName, label, timeoutMs) {
  assert(exportResult?.source, `${label} should include source`, exportResult)
  assert(
    sha256Hex(exportResult.source) === exportResult.source_sha256,
    `${label} source hash should match contents`,
    exportResult,
  )
  const validated = unwrap(
    await client.send(validateWorkflowCodeRequest(session.id, nodePath, exportResult.source)),
    'WorkflowCodeValidated',
  ).result
  assert(validated?.validation?.ok, `${label} should validate on second kernel`, validated?.validation)
  validateLiveExportedTopologyDefinition(exampleName, validated.definition, validated.validation)
  const appliedResponse = unwrap(
    await client.send(applyWorkflowCodeRequest(session.id, nodePath, exportResult.source)),
    'WorkflowCodeApplied',
  )
  const expected = topologyRuntimeExpectation(validated.definition)
  const apply = validateApplyResult(appliedResponse.result, `${label} apply`, expected)
  validateSessionProjection(appliedResponse.session, apply, `${label} apply`, expected)
  assert(apply.workflow_id !== exportResult.source_workflow_id, `${label} apply workflow id must be fresh`, apply)
  const completed = await completeAppliedTopologyWorkflow(
    client,
    session.id,
    exampleName,
    validated.definition,
    apply,
    timeoutMs,
  )
  return {
    applyWorkflowId: apply.workflow_id,
    workflowRunId: completed.workflowRunId,
    runtime: completed.runtime,
    tuiOutline: completed.tuiOutline,
    sourceSha256: exportResult.source_sha256,
    directoryManifest: exportResult.format === 'directory'
      ? await writeSourceDirectoryExport(workspace, exportResult, label)
      : null,
  }
}

async function validateSecondKernelExampleSuite({
  liveExports,
  generatedRoot,
  nodePath,
  timeoutMs,
}) {
  if ((liveExports ?? []).length === 0) return null
  const secondRoot = path.join(generatedRoot, 'second-kernel-example-suite')
  const secondWorkspace = path.join(secondRoot, 'workspace')
  const secondWorktree = path.join(secondRoot, 'worktree')
  const { client, daemonChild, kernelUrl } = await startIsolatedKernel(
    'workflow-code-second-kernel-example-suite-drill',
    secondRoot,
    secondWorkspace,
    secondWorktree,
    timeoutMs,
  )
  let sessionId = null
  try {
    const session = unwrap(
      await client.send(createSessionRequest(secondWorkspace, secondWorktree, 'workflow-code-second-kernel-example-suite-drill', undefined, null, 'off')),
      'SessionCreated',
    ).session
    sessionId = session.id
    const examples = []
    for (const liveExport of liveExports) {
      const inlineSource = {
        ...liveExport.inlineSource,
        source_workflow_id: liveExport.sourceWorkflowId,
      }
      const directorySource = {
        ...liveExport.directorySource,
        source_workflow_id: liveExport.sourceWorkflowId,
      }
      const inline = await validateTopologySourceExportOnKernel(
        client,
        session,
        nodePath,
        secondWorkspace,
        inlineSource,
        liveExport.exampleName,
        `second kernel ${liveExport.exampleName} live inline source`,
        timeoutMs,
      )
      await writeSourceDirectoryExport(
        secondWorkspace,
        directorySource,
        `second kernel ${liveExport.exampleName} live source directory`,
      )
      const directory = await validateTopologySourceExportOnKernel(
        client,
        session,
        nodePath,
        secondWorkspace,
        directorySource,
        liveExport.exampleName,
        `second kernel ${liveExport.exampleName} live source directory`,
        timeoutMs,
      )
      examples.push({
        example: liveExport.exampleName,
        sourceWorkflowId: liveExport.sourceWorkflowId,
        inline,
        directory,
      })
    }
    return {
      kernelUrl,
      sessionId: session.id,
      examples,
    }
  } finally {
    if (client && sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
    daemonChild.kill('SIGTERM')
    await sleep(1000)
  }
}

async function validateHetznerSecondKernelDistribution({
  packageExport,
  inlineSource,
  directorySource,
  sourceWorkflowId,
  liveExports,
  skillSourceDir,
  skillName,
  generatedRoot,
  timeoutMs,
  options,
}) {
  const localBundle = path.join(generatedRoot, 'hetzner-second-kernel-bundle')
  const remoteBundle = path.posix.join(
    options.hetznerRemoteRoot,
    `workflow-code-second-kernel-${process.pid}-${Date.now()}`,
  )
  await writeHetznerWorkflowCodeBundle(localBundle, {
    packageExport,
    inlineSource,
    directorySource,
    sourceWorkflowId,
    liveExports,
    skillName,
    skillSourceDir,
  })
  await assertHetznerCheckoutMatchesLocal(options)
  await assertHetznerArrobaBinaries({
    hetznerHost: options.hetznerHost,
    hetznerKey: options.hetznerKey,
    hetznerRepo: options.hetznerRepo,
  })
  await runHetznerCommand(options, [
    `rm -rf ${shellQuote(remoteBundle)}`,
    `mkdir -p ${shellQuote(remoteBundle)}`,
  ].join(' && '))
  await execFileAsync('scp', [
    '-i',
    options.hetznerKey,
    '-o',
    'BatchMode=yes',
    '-o',
    'StrictHostKeyChecking=accept-new',
    '-r',
    `${localBundle}/.`,
    `${options.hetznerHost}:${remoteBundle}/`,
  ], { maxBuffer: 4 * 1024 * 1024 })
  try {
    const command = [
      `cd ${shellQuote(options.hetznerRepo)}`,
      `runner_cmd=${shellQuote(`node ${shellQuote(path.posix.join(remoteBundle, 'remote-runner.mjs'))} --repo ${shellQuote(options.hetznerRepo)} --bundle ${shellQuote(remoteBundle)} --timeout-ms ${Number(timeoutMs)}`)}`,
      `if command -v timeout >/dev/null 2>&1; then timeout --kill-after=5s ${Math.ceil(Number(timeoutMs) / 1000) + 90}s bash -lc "$runner_cmd"; else bash -lc "$runner_cmd"; fi`,
    ].join(' && ')
    const stdout = await runHetznerCommand(options, command)
    const jsonStart = stdout.lastIndexOf('\n{')
    const summaryText = (jsonStart >= 0 ? stdout.slice(jsonStart + 1) : stdout).trim()
    const summary = JSON.parse(summaryText)
    assert(summary?.packageApplyWorkflowId !== sourceWorkflowId, 'Hetzner package apply workflow id must be fresh', summary)
    assert(summary?.inline?.applyWorkflowId !== sourceWorkflowId, 'Hetzner inline source workflow id must be fresh', summary)
    assert(summary?.directory?.applyWorkflowId !== sourceWorkflowId, 'Hetzner directory source workflow id must be fresh', summary)
    return {
      remoteHost: options.hetznerHost,
      remoteRepo: options.hetznerRepo,
      remoteBundle,
      ...summary,
    }
  } finally {
    await runHetznerCommand(options, `rm -rf ${shellQuote(remoteBundle)}`).catch(() => {})
  }
}

async function assertHetznerCheckoutMatchesLocal(options) {
  const [{ stdout: localHeadRaw }, { stdout: localCommitEpochRaw }] = await Promise.all([
    execFileAsync('git', ['rev-parse', 'HEAD'], { cwd: repoRoot }),
    execFileAsync('git', ['show', '-s', '--format=%ct', 'HEAD'], { cwd: repoRoot }),
  ])
  const localHead = localHeadRaw.trim()
  const localCommitEpoch = Number(localCommitEpochRaw.trim())
  const remoteInfo = await runHetznerCommand(options, [
    `cd ${shellQuote(options.hetznerRepo)}`,
    `printf 'head=%s\\n' "$(git rev-parse HEAD)"`,
    `printf 'kernel_mtime=%s\\n' "$(stat -c %Y apps/kernel/target/debug/arroba-kernel 2>/dev/null || printf 0)"`,
  ].join(' && '))
  const info = Object.fromEntries(remoteInfo
    .trim()
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      const separator = line.indexOf('=')
      return [line.slice(0, separator), line.slice(separator + 1)]
    }))
  if (info.head !== localHead) {
    throw new Error([
      `Hetzner checkout ${options.hetznerRepo} is not on the local workflow-code validation commit.`,
      `local HEAD: ${localHead}`,
      `remote HEAD: ${info.head ?? 'unknown'}`,
      'Update/build the remote checkout before running --hetzner-second-kernel.',
    ].join('\n'))
  }
  const kernelMtime = Number(info.kernel_mtime ?? 0)
  if (!Number.isFinite(kernelMtime) || kernelMtime < localCommitEpoch) {
    throw new Error([
      `Hetzner kernel binary is older than local HEAD at ${options.hetznerRepo}.`,
      `local HEAD epoch: ${localCommitEpoch}`,
      `remote kernel mtime: ${info.kernel_mtime ?? 'unknown'}`,
      'Rebuild apps/kernel/target/debug/arroba-kernel on Hetzner before running --hetzner-second-kernel.',
    ].join('\n'))
  }
}

async function writeHetznerWorkflowCodeBundle(localBundle, {
  packageExport,
  inlineSource,
  directorySource,
  sourceWorkflowId,
  liveExports,
  skillName,
  skillSourceDir,
}) {
  await rm(localBundle, { recursive: true, force: true })
  await mkdir(localBundle, { recursive: true })
  await writeFile(path.join(localBundle, 'package-export.json'), JSON.stringify(packageExport, null, 2), 'utf8')
  await writeFile(path.join(localBundle, 'package.json'), JSON.stringify({ type: 'module' }, null, 2), 'utf8')
  await writeFile(path.join(localBundle, 'inline-source.json'), JSON.stringify(inlineSource, null, 2), 'utf8')
  await writeFile(path.join(localBundle, 'directory-source.json'), JSON.stringify(directorySource, null, 2), 'utf8')
  await writeFile(path.join(localBundle, 'topology-live-exports.json'), JSON.stringify(liveExports ?? [], null, 2), 'utf8')
  await writeFile(path.join(localBundle, 'metadata.json'), JSON.stringify({ sourceWorkflowId, skillName }, null, 2), 'utf8')
  await copyDirectory(skillSourceDir, path.join(localBundle, 'skill', path.basename(skillSourceDir)))
  await cp(path.join(repoRoot, 'packages/kernel-client/dist'), path.join(localBundle, 'kernel-client-dist'), {
    recursive: true,
    force: true,
  })
  await mkdir(path.join(localBundle, 'node_modules'), { recursive: true })
  await cp(path.join(repoRoot, 'packages/kernel-client/node_modules/ws'), path.join(localBundle, 'node_modules/ws'), {
    recursive: true,
    dereference: true,
    force: true,
  })
  await writeFile(path.join(localBundle, 'remote-runner.mjs'), remoteWorkflowCodeRunnerSource(), 'utf8')
}

async function copyDirectory(sourceDir, targetDir) {
  await mkdir(targetDir, { recursive: true })
  for (const entry of await readdir(sourceDir, { withFileTypes: true })) {
    const sourcePath = path.join(sourceDir, entry.name)
    const targetPath = path.join(targetDir, entry.name)
    if (entry.isDirectory()) {
      await copyDirectory(sourcePath, targetPath)
    } else if (entry.isFile()) {
      await writeFile(targetPath, await readFile(sourcePath), 'utf8')
    }
  }
}

function remoteWorkflowCodeRunnerSource() {
  return String.raw`#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { closeSync, openSync, readFileSync } from 'node:fs'
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

function parseArgs(argv) {
  const options = { repo: null, bundle: null, timeoutMs: 120_000 }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--repo') options.repo = argv[++index]
    else if (arg === '--bundle') options.bundle = argv[++index]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else throw new Error('unknown option: ' + arg)
  }
  if (!options.repo || !options.bundle) throw new Error('--repo and --bundle are required')
  return options
}

function assert(condition, message, details) {
  if (!condition) throw new Error(message + (details ? '\n' + JSON.stringify(details, null, 2) : ''))
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const sha256Hex = (value) => createHash('sha256').update(value).digest('hex')

function stage(message) {
  console.error('[workflow-code-hetzner] ' + message)
}

function expectationFromDefinition(definition) {
  return {
    nodes: definition.nodes?.length ?? 0,
    agents: definition.nodes?.length ?? 0,
    edges: definition.edges?.length ?? 0,
    endpoints: definition.endpoints?.length ?? 0,
    queues: (definition.queues?.length ?? 0) || 1,
    schedules: definition.schedules?.length ?? definition.watchdogs?.length ?? 0,
    requiredSchemas: (definition.schemas ?? []).map((schema) => schema.handle),
  }
}

function rebindingsForDefinition(definition) {
  return (definition.nodes ?? []).map((node) => ({ node: node.handle, provider: 'dev-stub', model: 'default' }))
}

function validateApplyResult(result, label, expected) {
  assert(result?.compile?.validation?.ok, label + ' compile validation failed', result?.compile?.validation)
  const apply = result.apply
  assert(apply?.workflow_id, label + ' did not return workflow id', result)
  assert(Object.keys(apply.node_ids ?? {}).length === expected.nodes, label + ' node count mismatch', apply)
  assert(Object.keys(apply.agent_ids ?? {}).length === expected.agents, label + ' agent count mismatch', apply)
  assert(Object.keys(apply.edge_ids ?? {}).length === expected.edges, label + ' edge count mismatch', apply)
  assert(Object.keys(apply.endpoint_ids ?? {}).length === expected.endpoints, label + ' endpoint count mismatch', apply)
  assert(Object.keys(apply.queue_ids ?? {}).length === expected.queues, label + ' queue count mismatch', apply)
  assert(Object.keys(apply.schedule_ids ?? apply.watchdog_ids ?? {}).length === expected.schedules, label + ' schedule count mismatch', apply)
  for (const schemaHandle of expected.requiredSchemas) {
    assert(apply.schema_refs?.[schemaHandle], label + ' missing schema ' + schemaHandle, apply)
  }
  assert(apply.canvas_layout_applied === true, label + ' should apply canvas layout', apply)
  return apply
}

function validateSessionProjection(session, apply, label) {
  const workflow = (session.workflows ?? []).find((entry) => entry.id === apply.workflow_id)
  assert(workflow, label + ' workflow should appear in session projection', { workflowId: apply.workflow_id })
  assert(workflow.canvas_layout, label + ' workflow should include canvas layout', workflow)
  return workflow
}

async function waitForKernel(client, requests, workspace, worktree, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const session = unwrap(
        await withTimeout(
          client.send(requests.createSessionRequest(workspace, worktree, 'workflow-code-hetzner-ready-probe', undefined, null, 'off')),
          3_000,
          'kernel readiness request timed out',
        ),
        'SessionCreated',
      ).session
      await client.send(requests.endSessionRequest(session.id)).catch(() => {})
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error('kernel did not become ready: ' + (lastError?.message ?? 'unknown error'))
}

function withTimeout(promise, timeoutMs, message) {
  let timer = null
  return Promise.race([
    promise.finally(() => {
      if (timer) clearTimeout(timer)
    }),
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), timeoutMs)
    }),
  ])
}

async function send(client, request, label, timeoutMs = 30_000) {
  stage(label)
  return await withTimeout(client.send(request), timeoutMs, label + ' timed out')
}

async function startKernel(repo, bundle, timeoutMs, LocalIpcClient, requests) {
  stage('starting isolated kernel')
  const port = 52000 + Math.floor(Math.random() * 1000)
  const runId = 'workflow-code-hetzner-second-kernel-' + process.pid + '-' + Date.now()
  const root = path.join(bundle, 'remote-kernel-root')
  const workspace = path.join(bundle, 'workspace')
  const worktree = path.join(bundle, 'worktree')
  const stdoutPath = path.join(bundle, 'kernel.stdout.log')
  const stderrPath = path.join(bundle, 'kernel.stderr.log')
  await mkdir(workspace, { recursive: true })
  await mkdir(worktree, { recursive: true })
  const stdoutFd = openSync(stdoutPath, 'a')
  const stderrFd = openSync(stderrPath, 'a')
  const child = spawn(path.join(repo, 'apps/kernel/target/debug/arroba-kernel'), [], {
    cwd: repo,
    env: {
      ...process.env,
      HOME: path.join(root, 'home'),
      XDG_CONFIG_HOME: path.join(root, 'xdg-config'),
      XDG_STATE_HOME: path.join(root, 'xdg-state'),
      XDG_CACHE_HOME: path.join(root, 'xdg-cache'),
      ARROBA_HOME: path.join(root, 'arroba-home'),
      ARROBA_KERNEL_PORT: String(port),
      ARROBA_MCP_PORT: String(port + 1000),
      ARROBA_OPENCODE_PORT: String(port + 2000),
      ARROBA_CODEX_PORT: String(port + 2001),
      ARROBA_DAEMON_SOCKET: path.join(os.tmpdir(), runId + '.sock'),
      ARROBA_DAEMON_ID: runId,
    },
    stdio: ['ignore', stdoutFd, stderrFd],
  })
  closeSync(stdoutFd)
  closeSync(stderrFd)
  const client = new LocalIpcClient('ws://127.0.0.1:' + port, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    await waitForKernel(client, requests, workspace, worktree, timeoutMs)
    stage('isolated kernel ready on ws://127.0.0.1:' + port)
    return { child, client, kernelUrl: 'ws://127.0.0.1:' + port, workspace, worktree, stdoutPath, stderrPath }
  } catch (error) {
    child.kill('SIGTERM')
    await client.close().catch(() => {})
    throw error
  }
}

function printKernelLogTail(kernel) {
  for (const [label, filePath] of [['stdout', kernel.stdoutPath], ['stderr', kernel.stderrPath]]) {
    try {
      const contents = readFileSync(filePath, 'utf8')
      if (contents.trim()) {
        console.error('[workflow-code-hetzner] kernel ' + label + ' tail:')
        console.error(contents.slice(-8000))
      }
    } catch {
      // Best-effort diagnostics only.
    }
  }
}

async function writeSourceDirectoryExport(workspace, exportResult, label) {
  stage(label + ': writing source directory export')
  const files = exportResult.files ?? []
  assert(files.some((file) => file.path === 'workflow.js'), label + ' source directory should include workflow.js', exportResult)
  assert(files.some((file) => file.path === 'manifest.json'), label + ' source directory should include manifest.json', exportResult)
  for (const file of files) {
    assert(sha256Hex(file.contents) === file.sha256, label + ' file hash mismatch for ' + file.path, file)
    const target = path.join(workspace, file.path)
    await mkdir(path.dirname(target), { recursive: true })
    await writeFile(target, file.contents, 'utf8')
  }
}

async function validateSourceExportOnKernel(client, requests, session, nodePath, exportResult, label) {
  assert(exportResult?.source, label + ' should include source', exportResult)
  assert(sha256Hex(exportResult.source) === exportResult.source_sha256, label + ' source hash mismatch', exportResult)
  const validated = unwrap(
    await send(client, requests.validateWorkflowCodeRequest(session.id, nodePath, exportResult.source), label + ': validate source'),
    'WorkflowCodeValidated',
  ).result
  assert(validated?.validation?.ok, label + ' should validate', validated?.validation)
  const expected = expectationFromDefinition(validated.definition)
  const endpointHandle = validated.definition.endpoints?.[0]?.handle
  assert(endpointHandle, label + ' should define an endpoint handle', validated.definition)
  const appliedResponse = unwrap(
    await send(client, requests.applyWorkflowCodeRequest(session.id, nodePath, exportResult.source), label + ': apply source'),
    'WorkflowCodeApplied',
  )
  const apply = validateApplyResult(appliedResponse.result, label + ' apply', expected)
  validateSessionProjection(appliedResponse.session, apply, label + ' apply')
  const runResponse = unwrap(
    await send(
      client,
      requests.runWorkflowCodeRequest(session.id, nodePath, exportResult.source, 'Run ' + label + '.', { endpoint: endpointHandle }),
      label + ': run source',
      60_000,
    ),
    'WorkflowCodeRun',
  )
  const runApply = validateApplyResult(runResponse.result.apply, label + ' run', expected)
  validateSessionProjection(runResponse.session, runApply, label + ' run')
  assert(runResponse.result.invocation?.workflow_run || runResponse.result.invocation?.queued_prompt, label + ' should invoke or enqueue', runResponse.result.invocation)
  assert(apply.workflow_id !== runApply.workflow_id, label + ' run should apply a fresh workflow', { apply, runApply })
  return {
    validatedAlias: validated.definition.workflow?.alias ?? null,
    applyWorkflowId: apply.workflow_id,
    runWorkflowId: runApply.workflow_id,
    runInvocation: runResponse.result.invocation.workflow_run ? 'started' : 'enqueued',
  }
}

function topologyRuntimeExpectation(definition) {
  return {
    ...expectationFromDefinition(definition),
  }
}

function parseWorkflowOutputMessage(output) {
  const message = output?.message
  if (message == null) return null
  if (typeof message !== 'string') return message
  try {
    return JSON.parse(message)
  } catch {
    return message
  }
}

function validateTopologyRuntimeResult(exampleName, run) {
  assert(run?.status === 'Completed', 'topology ' + exampleName + ' should complete', run)
  assert(run.final_output_valid !== false, 'topology ' + exampleName + ' final output should be schema-valid', run)
  const finalOutput = parseWorkflowOutputMessage(run.final_output)
  assert(finalOutput && typeof finalOutput === 'object', 'topology ' + exampleName + ' should produce structured final output', run.final_output)
  const expectations = {
    'adversarial-verification.js': { decision: 'accept' },
    'evaluator-optimizer.js': { accepted: true },
    'fan-out-synthesize.js': { source_count: 2 },
    'generate-filter.js': { selected_count: 1 },
    'loop-until-done.js': { iterations: 2 },
    'orchestrator-workers.js': { delegated: true },
    'parallelization.js': { reviewer_count: 2 },
    'prompt-chaining.js': { answer: 'refined draft accepted' },
    'routing.js': { specialist: 'code' },
    'tournament.js': { winner: 'a' },
  }[exampleName]
  assert(expectations, 'missing topology runtime expectation for ' + exampleName)
  for (const [field, value] of Object.entries(expectations)) {
    assert(finalOutput[field] === value, 'topology ' + exampleName + ' final output field ' + field + ' mismatch', {
      finalOutput,
      expected: value,
    })
  }
  return finalOutput
}

async function waitForProviderRunReady(client, requests, providerRunId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const providerRun = unwrap(
      await send(client, requests.getProviderRunRequest(providerRunId), 'get provider run ' + providerRunId, 10_000),
      'ProviderRun',
    )?.provider_run
    if (providerRun?.state && providerRun.state !== 'Starting') {
      if (providerRun.state !== 'Running' && providerRun.state !== 'Parked') {
        throw new Error('provider run ' + providerRunId + ' reached unexpected state ' + providerRun.state)
      }
      return providerRun
    }
    await sleep(250)
  }
  throw new Error('provider run ' + providerRunId + ' did not become ready')
}

async function waitForCompletedWorkflowRun(client, requests, sessionId, workflowRunId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastRun = null
  while (Date.now() < deadline) {
    const stateResponse = await send(client, requests.getSessionStateRequest(sessionId), 'load session state for workflow run ' + workflowRunId, 10_000)
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
  throw new Error('workflow run ' + workflowRunId + ' did not complete before timeout' + (lastRun ? '\n' + JSON.stringify(lastRun, null, 2) : ''))
}

async function completeAppliedTopologyWorkflow(client, requests, sessionId, exampleName, definition, apply, timeoutMs) {
  const endpoint = (definition.endpoints ?? []).find((entry) => entry.handle === 'entry' || entry.alias === 'entry')
    ?? definition.endpoints?.[0]
  const entryNodeHandle = endpoint?.entry_node
  assert(entryNodeHandle, 'topology ' + exampleName + ' should resolve entry node', { definition, apply })
  const runtimeAgentEntries = Object.entries(apply.agent_ids ?? {})
    .sort(([left], [right]) => {
      if (left === entryNodeHandle) return 1
      if (right === entryNodeHandle) return -1
      return left.localeCompare(right)
    })
  for (const [handle, agentId] of runtimeAgentEntries) {
    const launchResponse = unwrap(
      await send(
        client,
        requests.launchProviderRunRequest(sessionId, 'dev-stub', 'default', 'workflow-code-topology-node', 'low', agentId),
        'launch topology provider ' + exampleName + ':' + handle,
        30_000,
      ),
      'ProviderRunLaunchAccepted',
    )
    assert(launchResponse?.provider_run?.id, 'topology ' + exampleName + ' should launch provider for node ' + handle, launchResponse)
    await waitForProviderRunReady(client, requests, launchResponse.provider_run.id, timeoutMs)
  }
  const endpointId = apply.endpoint_ids?.[endpoint.handle]
  assert(endpointId, 'topology ' + exampleName + ' should resolve entry endpoint', { endpoint, apply })
  const entryAgentId = apply.agent_ids?.[entryNodeHandle]
  assert(entryAgentId, 'topology ' + exampleName + ' should resolve entry agent', { entryNodeHandle, apply })
  await send(client, requests.focusAgentRequest(sessionId, entryAgentId), 'focus topology entry agent ' + exampleName, 10_000)
  const invokeResponse = unwrap(
    await send(
      client,
      requests.invokeWorkflowEndpointRequest(sessionId, apply.workflow_id, endpointId, 'Run ' + exampleName + ' Hetzner topology validation.'),
      'invoke topology workflow ' + exampleName,
      30_000,
    ),
    'WorkflowRunInvoked',
  )
  const workflowRun = invokeResponse?.workflow_run
  assert(workflowRun?.id, 'topology ' + exampleName + ' should start a workflow run', invokeResponse)
  const completed = await waitForCompletedWorkflowRun(client, requests, sessionId, workflowRun.id, timeoutMs)
  return {
    workflowRunId: completed.id,
    finalOutput: validateTopologyRuntimeResult(exampleName, completed),
  }
}

async function validateTopologySourceExportOnKernel(client, requests, session, nodePath, workspace, exportResult, exampleName, label, timeoutMs) {
  assert(exportResult?.source, label + ' should include source', exportResult)
  assert(sha256Hex(exportResult.source) === exportResult.source_sha256, label + ' source hash mismatch', exportResult)
  if (exportResult.format === 'directory') {
    await writeSourceDirectoryExport(workspace, exportResult, label)
  }
  const validated = unwrap(
    await send(client, requests.validateWorkflowCodeRequest(session.id, nodePath, exportResult.source), label + ': validate topology source', 60_000),
    'WorkflowCodeValidated',
  ).result
  assert(validated?.validation?.ok, label + ' should validate', validated?.validation)
  const appliedResponse = unwrap(
    await send(client, requests.applyWorkflowCodeRequest(session.id, nodePath, exportResult.source), label + ': apply topology source', 60_000),
    'WorkflowCodeApplied',
  )
  const expected = topologyRuntimeExpectation(validated.definition)
  const apply = validateApplyResult(appliedResponse.result, label + ' apply', expected)
  validateSessionProjection(appliedResponse.session, apply, label + ' apply')
  assert(apply.workflow_id !== exportResult.source_workflow_id, label + ' apply workflow id must be fresh', apply)
  const completed = await completeAppliedTopologyWorkflow(client, requests, session.id, exampleName, validated.definition, apply, timeoutMs)
  return {
    applyWorkflowId: apply.workflow_id,
    workflowRunId: completed.workflowRunId,
    finalOutput: completed.finalOutput,
    sourceSha256: exportResult.source_sha256,
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  stage('loading bundled kernel client and workflow-code exports')
  const ipcModule = await import(pathToFileURL(path.join(options.bundle, 'kernel-client-dist/ipc.js')))
  const requests = await import(pathToFileURL(path.join(options.bundle, 'kernel-client-dist/ipc-requests.js')))
  const packageExport = JSON.parse(await readFile(path.join(options.bundle, 'package-export.json'), 'utf8'))
  const inlineSource = JSON.parse(await readFile(path.join(options.bundle, 'inline-source.json'), 'utf8'))
  const directorySource = JSON.parse(await readFile(path.join(options.bundle, 'directory-source.json'), 'utf8'))
  const topologyLiveExports = JSON.parse(await readFile(path.join(options.bundle, 'topology-live-exports.json'), 'utf8'))
  const metadata = JSON.parse(await readFile(path.join(options.bundle, 'metadata.json'), 'utf8'))
  const kernel = await startKernel(options.repo, options.bundle, options.timeoutMs, ipcModule.LocalIpcClient, requests)
  const sessionIds = []
  let installedSkill = false
  try {
    const session = unwrap(
      await send(
        kernel.client,
        requests.createSessionRequest(kernel.workspace, kernel.worktree, 'workflow-code-hetzner-second-kernel', undefined, null, 'off'),
        'create validation session',
      ),
      'SessionCreated',
    ).session
    sessionIds.push(session.id)
    const skillRoot = path.join(options.bundle, 'skill')
    const skillDirs = await readdir(skillRoot)
    const skillSourceDir = path.join(skillRoot, skillDirs[0])
    const skillInstall = unwrap(
      await send(kernel.client, requests.installSkillRequest(kernel.workspace, skillSourceDir), 'install bundled drill skill'),
      'SkillInstalled',
    )
    assert(skillInstall?.skill?.name === metadata.skillName, 'Hetzner kernel should install workflow-code drill skill', skillInstall)
    installedSkill = true
    const nodePath = process.execPath
    const importedName = 'hetzner-package-' + Date.now()
    const imported = unwrap(
      await send(
        kernel.client,
        requests.importWorkflowCodePackageRequest(session.id, packageExport, nodePath, { name: importedName, overwrite: false }),
        'import workflow-code package',
        120_000,
      ),
      'WorkflowCodePackageImported',
    ).artifact
    assert(imported?.metadata?.validation?.ok, 'Hetzner package import should validate', imported?.metadata?.validation)
    const packageExpected = expectationFromDefinition(packageExport.definition)
    const packageRebindings = rebindingsForDefinition(packageExport.definition)
    const packageApplyResponse = unwrap(
      await send(
        kernel.client,
        requests.applyWorkflowCodeArtifactRequest(session.id, importedName, packageRebindings),
        'apply workflow-code package',
      ),
      'WorkflowCodeApplied',
    )
    const packageApply = validateApplyResult(packageApplyResponse.result, 'Hetzner package', packageExpected)
    validateSessionProjection(packageApplyResponse.session, packageApply, 'Hetzner package')
    const packageRun = unwrap(
      await send(
        kernel.client,
        requests.runWorkflowCodeArtifactRequest(session.id, importedName, 'Run the Hetzner package workflow.', { endpoint: 'entry', providerRebindings: packageRebindings }),
        'run workflow-code package',
        60_000,
      ),
      'WorkflowCodeRun',
    )
    const packageRunApply = validateApplyResult(packageRun.result.apply, 'Hetzner package run', packageExpected)
    validateSessionProjection(packageRun.session, packageRunApply, 'Hetzner package run')
    assert(packageApply.workflow_id !== metadata.sourceWorkflowId, 'Hetzner package workflow id must be fresh', { packageApply, metadata })
    assert(packageRunApply.workflow_id !== metadata.sourceWorkflowId, 'Hetzner package run workflow id must be fresh', { packageRunApply, metadata })

    const inline = await validateSourceExportOnKernel(kernel.client, requests, session, nodePath, inlineSource, 'Hetzner inline source')
    await writeSourceDirectoryExport(kernel.workspace, directorySource, 'Hetzner source directory')
    const directory = await validateSourceExportOnKernel(kernel.client, requests, session, nodePath, directorySource, 'Hetzner source directory')
    await send(kernel.client, requests.endSessionRequest(session.id), 'end package/source validation session', 10_000)
    const endedSessionIndex = sessionIds.indexOf(session.id)
    if (endedSessionIndex !== -1) {
      sessionIds.splice(endedSessionIndex, 1)
    }
    const topologySession = unwrap(
      await send(
        kernel.client,
        requests.createSessionRequest(kernel.workspace, kernel.worktree, 'workflow-code-hetzner-topology-suite', undefined, null, 'off'),
        'create topology validation session',
      ),
      'SessionCreated',
    ).session
    sessionIds.push(topologySession.id)
    const topologySourceRuns = []
    for (const liveExport of topologyLiveExports ?? []) {
      const inlineTopology = await validateTopologySourceExportOnKernel(
        kernel.client,
        requests,
        topologySession,
        nodePath,
        kernel.workspace,
        { ...liveExport.inlineSource, source_workflow_id: liveExport.sourceWorkflowId },
        liveExport.exampleName,
        'Hetzner ' + liveExport.exampleName + ' live inline source',
        options.timeoutMs,
      )
      const directoryTopology = await validateTopologySourceExportOnKernel(
        kernel.client,
        requests,
        topologySession,
        nodePath,
        kernel.workspace,
        { ...liveExport.directorySource, source_workflow_id: liveExport.sourceWorkflowId },
        liveExport.exampleName,
        'Hetzner ' + liveExport.exampleName + ' live source directory',
        options.timeoutMs,
      )
      topologySourceRuns.push({
        example: liveExport.exampleName,
        sourceWorkflowId: liveExport.sourceWorkflowId,
        inline: inlineTopology,
        directory: directoryTopology,
      })
    }
    console.log(JSON.stringify({
      kernelUrl: kernel.kernelUrl,
      packageSourceSessionId: session.id,
      topologySessionId: topologySession.id,
      packageImportedArtifact: importedName,
      packageApplyWorkflowId: packageApply.workflow_id,
      packageRunWorkflowId: packageRunApply.workflow_id,
      inline,
      directory,
      topologySourceRuns,
    }, null, 2))
  } catch (error) {
    printKernelLogTail(kernel)
    throw error
  } finally {
    for (const sessionId of sessionIds) {
      if (installedSkill) {
        await send(
          kernel.client,
          requests.uninstallSkillRequest(kernel.workspace, metadata.skillName),
          'uninstall bundled drill skill',
          10_000,
        ).catch(() => {})
        installedSkill = false
      }
      await send(kernel.client, requests.endSessionRequest(sessionId), 'end validation session', 10_000).catch(() => {})
    }
    await kernel.client.close().catch(() => {})
    kernel.child.kill('SIGTERM')
    await sleep(1000)
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
`
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

async function waitForCompletedWorkflowRun(client, sessionId, workflowRunId, timeoutMs, progressLabel = null) {
  const deadline = Date.now() + timeoutMs
  let lastRun = null
  let lastProgressAt = 0
  let lastProgressKey = null
  while (Date.now() < deadline) {
    const stateResponse = await withTimeout(
      client.send(getSessionStateRequest(sessionId)),
      Math.min(30_000, timeoutMs),
      `${progressLabel ?? 'workflow run'} status poll timed out for ${workflowRunId}`,
    )
    const state = unwrap(stateResponse, 'SessionStateLoaded')?.session
      ?? unwrap(stateResponse, 'SessionState')?.session
    const run = (state?.workflow_runs ?? []).find((entry) => entry.id === workflowRunId)
    if (run) {
      lastRun = run
      if (progressLabel) {
        const nodeStatuses = (run.node_runs ?? [])
          .map((nodeRun) => `${nodeRun.node_id}:${nodeRun.status}`)
          .join(',')
        const progressKey = `${run.status}|${nodeStatuses}|${run.messages?.length ?? 0}|${run.intermediate_outputs?.length ?? 0}`
        const now = Date.now()
        if (progressKey !== lastProgressKey || now - lastProgressAt >= 10_000) {
          stage(`${progressLabel}: workflow run status`, {
            workflow_run_id: workflowRunId,
            status: run.status,
            node_runs: (run.node_runs ?? []).map((nodeRun) => ({
              node_id: nodeRun.node_id,
              status: nodeRun.status,
              failures: nodeRun.failures?.length ?? 0,
            })),
            messages: run.messages?.length ?? 0,
            intermediate_outputs: run.intermediate_outputs?.length ?? 0,
          })
          lastProgressKey = progressKey
          lastProgressAt = now
        }
      }
      if (['Completed', 'Failed', 'Stopped'].includes(run.status)) {
        return run
      }
    }
    await sleep(500)
  }
  throw new Error(`workflow run ${workflowRunId} did not complete before timeout${lastRun ? `\n${JSON.stringify(compactWorkflowRunSummary(lastRun), null, 2)}` : ''}`)
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
  const skillName = `${WORKFLOW_CODE_ARTIFACT_SKILL_PREFIX}-${process.pid}-${Date.now()}`
  const source = workflowCodeSource(skillName)
  if (options.dryRun) {
    console.log(JSON.stringify({
      artifactRoot: options.artifactRoot,
      spawnDaemon: options.spawnDaemon,
      kernel: options.kernel,
      skillName,
      source,
      providerRebindings: providerRebindings(),
      exampleSuite: options.exampleSuite,
      secondKernel: options.secondKernel,
      hetznerSecondKernel: options.hetznerSecondKernel,
      realProviderTopology: options.realProviderTopology,
      providerIds: options.providerIds,
      providerModels: options.providerModels,
      providerAccounts: options.providerAccounts,
      providerEfforts: options.providerEfforts,
      hetznerHost: options.hetznerHost,
      hetznerRepo: options.hetznerRepo,
      hetznerRemoteRoot: options.hetznerRemoteRoot,
    }, null, 2))
    return
  }

  await prepareDrillArtifacts(options.artifactRoot)
  const generatedRoot = path.join(repoRoot, 'target', 'workflow-code-artifact-drill', `${process.pid}-${Date.now()}`)
  const workspace = options.workspace ?? path.join(generatedRoot, 'workspace')
  const worktree = options.worktree ?? path.join(generatedRoot, 'worktree')
  const skillSourceRoot = path.join(generatedRoot, 'skills')
  await mkdir(workspace, { recursive: true })
  await mkdir(worktree, { recursive: true })
  const skillSourceDir = await writeWorkflowCodeArtifactSkillSource(skillSourceRoot, skillName)

  let passed = false
  let failure = null
  let daemonChild = null
  let sessionId = null
  let attachmentId = null
  let kernelUrl = options.kernel ?? 'ws://127.0.0.1:43284'
  let summary = {}
  let client = null
  let installedSkill = false

  try {
    if (options.spawnDaemon) {
      const spawned = spawnedKernel('workflow-code-drill', generatedRoot)
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

    const skillInstall = unwrap(
      await client.send(installSkillRequest(workspace, skillSourceDir)),
      'SkillInstalled',
    )
    assert(skillInstall?.skill?.name === skillName, 'workflow-code artifact drill skill should install', skillInstall)
    installedSkill = true

    const artifactName = `artifact-drill-${Date.now()}`
    const importedName = `${artifactName}-imported`
    const nodePath = process.execPath

    const created = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(session.id, artifactName, nodePath, source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(created?.metadata?.validation?.ok, 'created artifact should validate', created?.metadata?.validation)
    validateArtifactHistory(created, ['created'])

    const listedAfterCreate = unwrap(
      await client.send(listWorkflowCodeArtifactsRequest(session.id)),
      'WorkflowCodeArtifactsListed',
    ).artifacts
    assert(
      (listedAfterCreate ?? []).some((artifact) => artifact.name === artifactName),
      'created artifact should appear in workflow-code artifact list',
      listedAfterCreate,
    )

    const updatedSource = source.replace('workflow_code_artifact_drill', 'workflow_code_artifact_drill_updated')
    const updated = unwrap(
      await client.send(updateWorkflowCodeArtifactRequest(session.id, artifactName, nodePath, updatedSource)),
      'WorkflowCodeArtifactUpdated',
    ).artifact
    assert(updated?.metadata?.validation?.ok, 'updated artifact should validate', updated?.metadata?.validation)
    assert(
      updated?.definition?.workflow?.alias === 'workflow_code_artifact_drill_updated',
      'updated artifact should persist revised workflow-code source',
      updated,
    )
    validateArtifactHistory(updated, ['created', 'updated'])

    const appliedResponse = unwrap(
      await client.send(applyWorkflowCodeArtifactRequest(session.id, artifactName, providerRebindings())),
      'WorkflowCodeApplied',
    )
    const defaultExpected = defaultToyExpectation(skillName)
    const firstApply = validateApplyResult(appliedResponse.result, 'artifact apply', defaultExpected)
    validateSessionProjection(appliedResponse.session, firstApply, 'artifact apply', defaultExpected)

    const exported = unwrap(
      await client.send(exportWorkflowCodePackageRequest(session.id, artifactName)),
      'WorkflowCodePackageExported',
    ).package
    assert(exported?.source_sha256, 'exported package should include source hash', exported)
    assert(exported?.definition_sha256, 'exported package should include compiled definition hash', exported)

    const liveInlineSource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        session.id,
        { kind: 'workflow', workflow_ref: firstApply.workflow_id },
        'inline',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(liveInlineSource?.source, 'live workflow inline source export should include source', liveInlineSource)
    assert(liveInlineSource.source_path === 'workflow.js', 'live workflow inline source path should be workflow.js', liveInlineSource)

    const liveDirectorySource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        session.id,
        { kind: 'workflow', workflow_ref: firstApply.workflow_id },
        'directory',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(liveDirectorySource?.source_path === 'workflow.js', 'live workflow directory source export should include workflow.js', liveDirectorySource)
    assert(
      (liveDirectorySource.files ?? []).some((file) => file.path === 'manifest.json'),
      'live workflow directory source export should include manifest.json',
      liveDirectorySource,
    )

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
    const runApply = validateApplyResult(runResponse.result.apply, 'artifact run', defaultExpected)
    validateSessionProjection(runResponse.session, runApply, 'artifact run', defaultExpected)
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
    let exampleSuite = []
    let exampleSuiteLiveExports = []
    if (options.exampleSuite) {
      const exampleWorkspace = path.join(generatedRoot, 'example-suite', 'workspace')
      const exampleWorktree = path.join(generatedRoot, 'example-suite', 'worktree')
      const exampleSession = unwrap(
        await client.send(createSessionRequest(exampleWorkspace, exampleWorktree, 'workflow-code-example-suite-drill', undefined, null, 'off')),
        'SessionCreated',
      ).session
      try {
        const exampleSuiteResult = await applyExampleSuite(client, exampleSession.id, nodePath, exampleWorkspace, options.timeoutMs)
        exampleSuite = exampleSuiteResult.results
        exampleSuiteLiveExports = exampleSuiteResult.liveExports
      } finally {
        await client.send(endSessionRequest(exampleSession.id)).catch(() => {})
      }
    }
    let realProviderTopology = null
    if (options.realProviderTopology) {
      const realProviderWorkspace = path.join(generatedRoot, 'real-provider-topology', 'workspace')
      const realProviderWorktree = path.join(generatedRoot, 'real-provider-topology', 'worktree')
      const realProviderSession = unwrap(
        await client.send(createSessionRequest(realProviderWorkspace, realProviderWorktree, 'workflow-code-real-provider-topology-drill', undefined, null, 'off')),
        'SessionCreated',
      ).session
      try {
        realProviderTopology = await runRealProviderTopologyDrill(
          client,
          realProviderSession.id,
          nodePath,
          options,
        )
      } finally {
        await client.send(endSessionRequest(realProviderSession.id)).catch(() => {})
      }
    }
    if ((options.secondKernel || options.hetznerSecondKernel) && installedSkill) {
      await client.send(uninstallSkillRequest(workspace, skillName)).catch(() => {})
      installedSkill = false
    }
    const secondKernel = options.secondKernel
      ? await validateSecondKernelDistribution({
        packageExport: exported,
        inlineSource: liveInlineSource,
        directorySource: liveDirectorySource,
        sourceWorkflowId: firstApply.workflow_id,
        skillSourceDir,
        skillName,
        generatedRoot,
        nodePath,
        timeoutMs: options.timeoutMs,
      })
      : null
    const exampleSuiteSecondKernel = options.secondKernel && exampleSuiteLiveExports.length > 0
      ? await validateSecondKernelExampleSuite({
        liveExports: exampleSuiteLiveExports,
        generatedRoot,
        nodePath,
        timeoutMs: options.timeoutMs,
      })
      : null
    const hetznerSecondKernel = options.hetznerSecondKernel
      ? await validateHetznerSecondKernelDistribution({
        packageExport: exported,
        inlineSource: liveInlineSource,
        directorySource: liveDirectorySource,
        sourceWorkflowId: firstApply.workflow_id,
        liveExports: exampleSuiteLiveExports,
        skillSourceDir,
        skillName,
        generatedRoot,
        timeoutMs: options.timeoutMs,
        options,
      })
      : null

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
      realProviderTopology,
      secondKernel,
      exampleSuiteSecondKernel,
      hetznerSecondKernel,
    }
    console.log(JSON.stringify(summary, null, 2))

    if (installedSkill) {
      await client.send(uninstallSkillRequest(workspace, skillName)).catch(() => {})
      installedSkill = false
    }
    await client.send(endSessionRequest(session.id)).catch(() => {})
    await client.close()
    passed = true
  } catch (error) {
    failure = error
    console.error(error)
    if (client && sessionId) {
      if (installedSkill) {
        await client.send(uninstallSkillRequest(workspace, skillName)).catch(() => {})
      }
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
