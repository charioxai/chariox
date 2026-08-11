import { execFile, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

import { createSessionRequest, endSessionRequest } from '@arroba/kernel-client'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
export const cliRoot = path.resolve(scriptDir, '..', '..')
export const repoRoot = path.resolve(cliRoot, '..', '..')

const execFileAsync = promisify(execFile)
export const DEFAULT_TIMEOUT_MS = 120_000
export const WORKFLOW_CODE_ARTIFACT_SKILL_PREFIX = 'workflow-code-artifact-skill'

export function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

export function parseArgs(argv) {
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

export function applyKeyValueOverride(target, value, flag) {
  const separator = String(value ?? '').indexOf('=')
  if (separator <= 0 || separator === String(value).length - 1) {
    throw new Error(`${flag} must use provider=value`)
  }
  const key = String(value).slice(0, separator).trim()
  const entryValue = String(value).slice(separator + 1).trim()
  if (!key || !entryValue) throw new Error(`${flag} must use provider=value`)
  target[key] = entryValue
}

export function printHelp() {
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

export function assert(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

export function stage(message, details = null) {
  const suffix = details ? ` ${JSON.stringify(details)}` : ''
  console.error(`[workflow-code-drill] ${message}${suffix}`)
}

export function unwrap(response, key) {
  return response?.[key] ?? response
}

export function runChecked(command, args, options = {}) {
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

export function buildKernel() {
  runChecked('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  const cargoTargetDir = process.env.CARGO_TARGET_DIR?.trim()
  const targetDir = cargoTargetDir
    ? path.resolve(repoRoot, cargoTargetDir)
    : path.join(repoRoot, 'target')
  return path.join(targetDir, 'debug', 'arroba-kernel')
}

export function sha256Hex(value) {
  return createHash('sha256').update(value).digest('hex')
}

export async function writeSourceDirectoryExport(workspace, exportResult, label) {
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

export function spawnedKernel(label = 'workflow-code-drill', rootDir = null) {
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

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export function withTimeout(promise, timeoutMs, message) {
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

export async function waitForKernel(client, workspace, worktree, timeoutMs) {
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

export function workflowCodeSource(skillName) {
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

export function providerRebindings() {
  return ['planner', 'worker', 'reviewer'].map((node) => ({
    node,
    provider: 'dev-stub',
    model: 'default',
  }))
}

export function existingAgentWorkflowCodeSource(agentId) {
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

export function existingAgentRebindings() {
  return [{
    node: 'generated_finisher',
    provider: 'dev-stub',
    model: 'default',
  }]
}

export function outputSchemaWorkflowCodeSource() {
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

const progressOutput = workflow.schema({
  handle: "progress_output",
  alias: "Progress output",
  description: "User-visible intermediate progress event for this node.",
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
  intermediateOutputSchema: progressOutput,
  canvas: { x: 0, y: 120 },
});

workflow.endpoint(worker, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
`.trim()
}

export function defaultToyExpectation(skillName = null) {
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

export function expectationFromDefinition(definition) {
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

export function topologyRuntimeExpectation(definition) {
  return {
    ...expectationFromDefinition(definition),
    agentModel: 'workflow-code-topology-node',
  }
}

export function rebindingsForDefinition(definition) {
  return (definition.nodes ?? []).map((node) => ({
    node: node.handle,
    provider: 'dev-stub',
    model: 'default',
  }))
}

export function topologyRuntimeRebindingsForDefinition(definition) {
  return (definition.nodes ?? []).map((node) => ({
    node: node.handle,
    provider: 'dev-stub',
    model: 'workflow-code-topology-node',
  }))
}

export function providerSetForDefinition(definition) {
  return new Set((definition.nodes ?? []).map((node) => node.agent?.provider).filter(Boolean))
}

export function realProviderRebindingsForDefinition(definition, options) {
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

export function rebindingByNode(rebindings) {
  return new Map((rebindings ?? []).map((entry) => [entry.node, entry]))
}

export function providerFamily(provider) {
  if (provider === 'claude-headless' || provider === 'claude-p') return 'claude'
  return provider
}

export function shouldPrelaunchRealProvider(provider) {
  return providerFamily(provider) !== 'claude'
}
