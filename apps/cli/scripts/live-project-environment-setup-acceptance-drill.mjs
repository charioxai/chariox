#!/usr/bin/env node

import { access, mkdir, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { randomUUID } from 'node:crypto'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

export const PROJECT_ENVIRONMENT_SETUP_MINIMUM_PROTOCOL = 326
export const PROJECT_ENVIRONMENT_SETUP_RELAY_PEER_MINIMUM_PROTOCOL = 51
export const DEFAULT_PROVIDER = 'codex'
export const DEFAULT_ACCOUNT_PROFILE = 'codex-1'
export const DEFAULT_MODEL = 'gpt-5.6-luna'
export const DEFAULT_EFFORT = 'max'
export const DEFAULT_PROMPT =
  'Reply with exactly PROJECT_ENVIRONMENT_SETUP_ACCEPTANCE_OK. Do not modify files.'
export const DEFAULT_TIMEOUT_MS = 900_000
export const DEFAULT_POLL_MS = 1_000
export const DEFAULT_ARTIFACT_ROOT = path.join(
  os.homedir(),
  '.chariox',
  'dev',
  'project-environment-setup-acceptance',
)

const TERMINAL_SETUP_PHASES = new Set(['ready', 'failed', 'cancelled'])
const CANCELLABLE_SETUP_PHASES = new Set(['preparing', 'validating'])

function valueAfter(argv, index, flag) {
  const value = argv[index + 1]
  if (value === undefined || value.startsWith('--')) {
    throw new Error(`${flag} requires a value`)
  }
  return value
}

export function parseProjectEnvironmentSetupArguments(argv, environment = process.env) {
  const options = {
    kernelUrl: environment.CHARIOX_KERNEL_URL?.trim() || null,
    relayUrl: environment.CHARIOX_RELAY_URL?.trim() || null,
    relayToken: environment.CHARIOX_RELAY_TOKEN?.trim() || null,
    localAuthToken: environment.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN?.trim() || null,
    homeDaemonId: environment.CHARIOX_HOME_DAEMON_ID?.trim() || null,
    homeDaemonAlias: environment.CHARIOX_HOME_DAEMON_ALIAS?.trim() || null,
    workerMachineId: environment.CHARIOX_PATH1_WORKER_MACHINE_ID?.trim() || null,
    workerKernelId: environment.CHARIOX_PATH1_WORKER_KERNEL_ID?.trim() || null,
    managedEnvironmentId: environment.CHARIOX_PATH1_MANAGED_ENVIRONMENT_ID?.trim() || null,
    projectId: environment.CHARIOX_PROJECT_ID?.trim() || null,
    workspaceId: environment.CHARIOX_WORKSPACE_ID?.trim() || null,
    worktreeId: environment.CHARIOX_WORKTREE_ID?.trim() || null,
    targetPlatform: environment.CHARIOX_TARGET_PLATFORM?.trim() || null,
    provider: environment.CHARIOX_PROVIDER?.trim() || DEFAULT_PROVIDER,
    accountProfile: environment.CHARIOX_ACCOUNT_PROFILE?.trim() || DEFAULT_ACCOUNT_PROFILE,
    model: environment.CHARIOX_MODEL?.trim() || DEFAULT_MODEL,
    effort: environment.CHARIOX_EFFORT?.trim() || DEFAULT_EFFORT,
    prompt: environment.CHARIOX_PROJECT_ENVIRONMENT_PROMPT || DEFAULT_PROMPT,
    validationCommands: [],
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    artifactRoot: environment.CHARIOX_PROJECT_ENVIRONMENT_ARTIFACT_ROOT?.trim()
      || DEFAULT_ARTIFACT_ROOT,
    reportPath: environment.CHARIOX_PROJECT_ENVIRONMENT_REPORT?.trim() || null,
    allowTerminalBeforeCancel: false,
    help: false,
  }

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    if (arg === '--help') options.help = true
    else if (arg === '--kernel-url' || arg === '--kernel') {
      options.kernelUrl = valueAfter(argv, index, arg)
      options.relayUrl = null
      index += 1
    }
    else if (arg === '--relay-url') {
      options.relayUrl = valueAfter(argv, index, arg)
      options.kernelUrl = null
      index += 1
    }
    else if (arg === '--relay-token') options.relayToken = valueAfter(argv, index, arg), index += 1
    else if (arg === '--home-daemon-id' || arg === '--target-daemon-id') options.homeDaemonId = valueAfter(argv, index, arg), index += 1
    else if (arg === '--home-daemon-alias' || arg === '--target-daemon-alias') options.homeDaemonAlias = valueAfter(argv, index, arg), index += 1
    else if (arg === '--worker-machine-id') options.workerMachineId = valueAfter(argv, index, arg), index += 1
    else if (arg === '--worker-kernel-id') options.workerKernelId = valueAfter(argv, index, arg), index += 1
    else if (arg === '--managed-environment-id') options.managedEnvironmentId = valueAfter(argv, index, arg), index += 1
    else if (arg === '--project-id') options.projectId = valueAfter(argv, index, arg), index += 1
    else if (arg === '--workspace-id') options.workspaceId = valueAfter(argv, index, arg), index += 1
    else if (arg === '--worktree-id') options.worktreeId = valueAfter(argv, index, arg), index += 1
    else if (arg === '--target-platform') options.targetPlatform = valueAfter(argv, index, arg), index += 1
    else if (arg === '--provider') options.provider = valueAfter(argv, index, arg), index += 1
    else if (arg === '--account-profile') options.accountProfile = valueAfter(argv, index, arg), index += 1
    else if (arg === '--model') options.model = valueAfter(argv, index, arg), index += 1
    else if (arg === '--effort') options.effort = valueAfter(argv, index, arg), index += 1
    else if (arg === '--prompt') options.prompt = valueAfter(argv, index, arg), index += 1
    else if (arg === '--validation-command') options.validationCommands.push(valueAfter(argv, index, arg)), index += 1
    else if (arg === '--timeout-ms') options.timeoutMs = Number(valueAfter(argv, index, arg)), index += 1
    else if (arg === '--poll-ms') options.pollMs = Number(valueAfter(argv, index, arg)), index += 1
    else if (arg === '--artifact-root') options.artifactRoot = valueAfter(argv, index, arg), index += 1
    else if (arg === '--report') options.reportPath = valueAfter(argv, index, arg), index += 1
    else if (arg === '--allow-terminal-before-cancel') options.allowTerminalBeforeCancel = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

export function validateProjectEnvironmentSetupOptions(options) {
  if (options.help) return
  if (options.kernelUrl && options.relayUrl) {
    throw new Error('--kernel-url and --relay-url are mutually exclusive')
  }
  const endpoint = options.relayUrl || options.kernelUrl
  if (!endpoint) throw new Error('a home kernel WebSocket endpoint is required via --kernel-url or --relay-url')
  if (!/^wss?:\/\//i.test(endpoint)) {
    throw new Error('project environment acceptance requires a WebSocket kernel or relay endpoint')
  }
  if (options.relayUrl && (!options.relayToken || (!options.homeDaemonId && !options.homeDaemonAlias))) {
    throw new Error('--relay-url requires a relay token and --home-daemon-id or --home-daemon-alias')
  }
  const required = [
    ['--worker-machine-id', options.workerMachineId],
    ['--worker-kernel-id', options.workerKernelId],
    ['--managed-environment-id', options.managedEnvironmentId],
    ['--project-id', options.projectId],
    ['--workspace-id', options.workspaceId],
    ['--worktree-id', options.worktreeId],
    ['--target-platform', options.targetPlatform],
  ]
  for (const [flag, value] of required) {
    if (!value?.trim()) throw new Error(`${flag} is required; the drill never discovers or substitutes a target`)
  }
  for (const [flag, value] of [
    ['--provider', options.provider],
    ['--account-profile', options.accountProfile],
    ['--model', options.model],
    ['--effort', options.effort],
    ['--prompt', options.prompt],
  ]) {
    if (!value?.trim()) throw new Error(`${flag} must not be empty`)
  }
  if (options.provider === 'dev-stub') throw new Error('live project setup acceptance requires an official provider, not dev-stub')
  if (options.validationCommands.length === 0) {
    throw new Error('at least one --validation-command is required; validation counts may not be fabricated')
  }
  if (options.validationCommands.length > 32) {
    throw new Error('--validation-command may be repeated at most 32 times')
  }
  if (options.validationCommands.some((command) => !command.trim() || command.length > 8_192)) {
    throw new Error('--validation-command values must be non-empty and at most 8192 characters')
  }
  if (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 10_000) {
    throw new Error('--timeout-ms must be an integer of at least 10000')
  }
  if (!Number.isInteger(options.pollMs) || options.pollMs < 100) {
    throw new Error('--poll-ms must be an integer of at least 100')
  }
}

export function printProjectEnvironmentSetupHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-project-environment-setup-acceptance-drill.mjs [options]',
    '',
    'The drill is strict and live-only: it creates one disposable session, runs no-definition',
    'Project setup on the selected Path1 worker, and launches one provider prompt only after Ready.',
    '',
    'Connection (one is required):',
    '  --kernel-url wss://home-kernel.example',
    '  --relay-url wss://relay.example --home-daemon-id HOME_DAEMON_ID',
    '  CHARIOX_RELAY_TOKEN supplies the relay credential without putting it in shell history',
    '',
    'Target identity (all are required):',
    '  --worker-machine-id MACHINE_ID --worker-kernel-id KERNEL_ID',
    '  --managed-environment-id ENVIRONMENT_ID --project-id PROJECT_ID',
    '  --workspace-id WORKSPACE_ID --worktree-id WORKTREE_ID',
    '  --target-platform linux-x86_64',
    '',
    'Run inputs:',
    '  --validation-command COMMAND (repeat; use real bounded worker commands)',
    `  --provider ${DEFAULT_PROVIDER} --account-profile ${DEFAULT_ACCOUNT_PROFILE}`,
    `  --model ${DEFAULT_MODEL} --effort ${DEFAULT_EFFORT}`,
    `  --prompt ${JSON.stringify(DEFAULT_PROMPT)}`,
    '  --timeout-ms 900000 --poll-ms 1000',
    '  --allow-terminal-before-cancel (explicitly records cancellation as not applicable)',
    '  --artifact-root PATH --report PATH (outside the repository)',
  ].join('\n'))
}

export function requestVariant(request) {
  const keys = Object.keys(request || {})
  return keys.length === 1 ? keys[0] : keys.join(',')
}

export function isUnknownRequestVariant(error, variant) {
  const message = error instanceof Error ? error.message : String(error)
  const escapedVariant = variant.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  return new RegExp(
    "unknown variant\\s+[`'\\\"]?" + escapedVariant + "(?:[`'\\\"]|\\b)",
    'i',
  ).test(message)
}

export function withProjectEnvironmentSetupProtocolMinimum(operation, diagnostic = {}) {
  return Promise.resolve()
    .then(operation)
    .catch((error) => {
      const message = error instanceof Error ? error.message : String(error)
      const variants = [diagnostic.requestVariant, diagnostic.nestedVariant].filter(Boolean)
      if (!variants.some((variant) => isUnknownRequestVariant(error, variant))) throw error
      throw new Error(
        `${diagnostic.capability || 'Project environment setup'} requires kernel protocol ${diagnostic.minimumProtocolVersion || PROJECT_ENVIRONMENT_SETUP_MINIMUM_PROTOCOL} or newer: ${message}`,
      )
    })
}

export function classifyProjectEnvironmentSetupCapabilityProbeError(error) {
  const message = error instanceof Error ? error.message : String(error)
  if (isUnknownRequestVariant(error, 'GetProjectEnvironmentSetupStatus')) {
    return { status: 'blocked', reason: 'setup request variant is unavailable', message }
  }
  if (/setup operation was not found/i.test(message)) {
    return { status: 'verified', reason: 'status request reached the setup store', message }
  }
  return { status: 'unverified', reason: 'status probe did not produce the expected not-found result', message }
}

function requireResponseVariant(response, variants, label) {
  for (const variant of variants) {
    if (response?.[variant] !== undefined) return response[variant]
  }
  throw new Error(`${label} returned an unexpected response variant: ${requestVariant(response)}`)
}

export function buildProjectEnvironmentSetupRequestInput(options, operationId, agentId, sessionId) {
  return {
    operationId,
    projectId: options.projectId,
    sessionId,
    agentId,
    targetWorkerId: options.workerMachineId,
    targetPlatform: options.targetPlatform,
    validationCommands: [...options.validationCommands],
  }
}

export function buildProjectEnvironmentSetupRequests(requests, input) {
  return {
    start: requests.startProjectEnvironmentSetupRequest(input),
    get: requests.getProjectEnvironmentSetupStatusRequest(input.operationId),
    cancel: requests.cancelProjectEnvironmentSetupRequest(input.operationId, input.sessionId),
    retry: requests.retryProjectEnvironmentSetupRequest(input.operationId, input.sessionId),
  }
}

export function selectApprovedPath1Worker(machines, kernels, options) {
  const machine = (machines || []).find((candidate) => candidate.machine_id === options.workerMachineId)
  if (!machine) throw new Error(`selected worker machine ${options.workerMachineId} is absent from home inventory`)
  if (machine.trust_status !== 'approved' || !machine.online || machine.pending) {
    throw new Error(`selected worker machine ${options.workerMachineId} is not approved and online`)
  }
  const kernel = (kernels || []).find((candidate) => candidate.kernel_id === options.workerKernelId)
  if (!kernel) throw new Error(`selected worker kernel ${options.workerKernelId} is absent from worker inventory`)
  if (kernel.machine_id !== options.workerMachineId) {
    throw new Error(`selected kernel ${options.workerKernelId} belongs to ${kernel.machine_id}, not selected worker ${options.workerMachineId}`)
  }
  if (!kernel.accepting_remote_leases || !(kernel.available_providers || []).includes(options.provider)) {
    throw new Error(`selected worker kernel ${options.workerKernelId} does not advertise remote leases and provider ${options.provider}`)
  }
  const account = (kernel.provider_accounts || []).find((candidate) => (
    candidate.provider === options.provider
      && (candidate.alias === options.accountProfile || candidate.account_id === options.accountProfile)
      && ['authenticated', 'configured'].includes(candidate.state)
  ))
  if (!account) {
    throw new Error(`selected worker kernel does not advertise usable ${options.provider}/${options.accountProfile} authentication`)
  }
  return { machine, kernel, account }
}

export function validateManagedPath1Environment(environment, options, worker) {
  if (!environment || environment.environmentId !== options.managedEnvironmentId) {
    throw new Error(`managed environment ${options.managedEnvironmentId} was not returned by the selected home kernel`)
  }
  if (environment.observedState !== 'ready' || environment.desiredState !== 'running') {
    throw new Error(`managed environment ${options.managedEnvironmentId} is not ready/running`)
  }
  if (environment.runtimeMachineId !== worker.machine.machine_id) {
    throw new Error(`managed environment runtime machine ${environment.runtimeMachineId} does not match ${worker.machine.machine_id}`)
  }
  if (environment.runtimeKernelId !== worker.kernel.kernel_id) {
    throw new Error(`managed environment runtime kernel ${environment.runtimeKernelId} does not match ${worker.kernel.kernel_id}`)
  }
  if (!environment.runtimeReleaseDigest?.trim()) {
    throw new Error('managed environment has no trusted runtime release digest; worker revision cannot be proven')
  }
  return environment
}

export function validateSelectedProject(project, options) {
  if (!project || project.id !== options.projectId) {
    throw new Error(`selected project ${options.projectId} is absent from the home project inventory`)
  }
  if (project.status !== 'active') throw new Error(`selected project ${options.projectId} is not active`)
  const workspaceMatch = project.workspace_id === options.workspaceId
    || (project.workspace_ids || []).includes(options.workspaceId)
  if (!workspaceMatch) throw new Error(`project ${options.projectId} is not associated with selected workspace ${options.workspaceId}`)
  if (project.environment_definition != null) {
    throw new Error(`project ${options.projectId} already has an environment definition; the no-definition utility path was not selected`)
  }
  return project
}

export function validateCreatedRemoteSession(session, agent, options, worker) {
  if (!session || session.project_id !== options.projectId) throw new Error('created session did not preserve the selected project')
  if (session.workspace_id !== options.workspaceId || session.worktree_id !== options.worktreeId) {
    throw new Error('created session did not preserve the selected workspace/worktree')
  }
  if (!agent || agent.session_id !== session.id || agent.worktree_id !== options.worktreeId) {
    throw new Error('created worker agent did not preserve the selected worktree')
  }
  const remote = agent.remote_execution
  if (!remote?.leased_agent_id) throw new Error('created session agent has no remote worker lease')
  if (remote.worker_machine_id !== worker.machine.machine_id || remote.worker_kernel_id !== worker.kernel.kernel_id) {
    throw new Error('created session agent was not leased to the selected Path1 worker')
  }
  if (!Number.isInteger(remote.relay_peer_protocol_version)
    || remote.relay_peer_protocol_version < PROJECT_ENVIRONMENT_SETUP_RELAY_PEER_MINIMUM_PROTOCOL) {
    throw new Error(`remote agent does not expose relay peer protocol ${PROJECT_ENVIRONMENT_SETUP_RELAY_PEER_MINIMUM_PROTOCOL} or newer`)
  }
  return remote
}

export function summarizeProjectEnvironmentSetupStatus(status) {
  const validation = status?.validation
  const commands = validation?.commands || []
  return {
    operationId: status?.operation_id ?? null,
    projectId: status?.project_id ?? null,
    sessionId: status?.session_id ?? null,
    agentId: status?.agent_id ?? null,
    workerId: status?.worker_id ?? null,
    platform: status?.platform ?? null,
    phase: status?.phase ?? null,
    attempt: status?.attempt ?? null,
    progressPercent: status?.progress_percent ?? null,
    definitionDigest: status?.definition_digest ?? null,
    validationCommandCount: commands.length,
    validationPassedCount: commands.filter((command) => command.exit_code === 0).length,
    validationFailedCount: commands.filter((command) => command.exit_code !== 0).length,
    validationOutputBytes: commands.reduce(
      (total, command) => total + (command.stdout_bytes || 0) + (command.stderr_bytes || 0),
      0,
    ),
    failureCode: status?.failure_code ?? null,
    retryable: status?.retryable ?? null,
    updatedAtMs: status?.updated_at_ms ?? null,
  }
}

export function validateSetupStatusIdentity(status, options, operationId, sessionId, agentId) {
  const expected = {
    operation_id: operationId,
    project_id: options.projectId,
    session_id: sessionId,
    agent_id: agentId,
    worker_id: options.workerMachineId,
    platform: options.targetPlatform,
  }
  for (const [field, value] of Object.entries(expected)) {
    if (status?.[field] !== value) throw new Error(`setup status ${field}=${status?.[field]} does not match selected ${value}`)
  }
  return status
}

export function validateReadyProjectEnvironmentSetup(status, options, operationId, sessionId, agentId) {
  validateSetupStatusIdentity(status, options, operationId, sessionId, agentId)
  if (status.phase !== 'ready') throw new Error(`setup did not reach authoritative ready (phase=${status.phase})`)
  if (!status.definition_digest) throw new Error('ready setup status has no definition digest')
  const validation = status.validation
  const commands = validation?.commands || []
  if (!validation || validation.worker_id !== options.workerMachineId || validation.platform !== options.targetPlatform) {
    throw new Error('ready setup status did not identify the selected worker/platform')
  }
  if (commands.length === 0 || commands.some((command) => command.exit_code !== 0)) {
    throw new Error('ready setup status did not contain a nonzero all-passing validation result set')
  }
  return {
    commandCount: commands.length,
    passedCount: commands.filter((command) => command.exit_code === 0).length,
    failedCount: commands.filter((command) => command.exit_code !== 0).length,
    outputBytes: commands.reduce(
      (total, command) => total + (command.stdout_bytes || 0) + (command.stderr_bytes || 0),
      0,
    ),
  }
}

export function validateUtilityGeneratedProjectDefinition(project, options, validation) {
  const definition = project?.environment_definition
  if (!definition || definition.origin !== 'utility_generated') {
    throw new Error('ready project does not contain a utility-generated environment definition')
  }
  if (definition.target_platform !== options.targetPlatform) {
    throw new Error(`persisted definition targets ${definition.target_platform}, not ${options.targetPlatform}`)
  }
  if (!Array.isArray(definition.setup_steps) || definition.setup_steps.length === 0) {
    throw new Error('utility-generated definition contains no measured install/setup steps')
  }
  if (!Array.isArray(definition.validation_commands) || definition.validation_commands.length === 0) {
    throw new Error('utility-generated definition contains no validation commands')
  }
  for (const command of options.validationCommands) {
    if (!definition.validation_commands.includes(command)) {
      throw new Error('persisted utility definition omitted a requested validation command')
    }
  }
  if (validation.commandCount !== definition.validation_commands.length) {
    throw new Error('persisted definition and worker validation command counts differ')
  }
  return {
    origin: definition.origin,
    source: definition.source,
    setupStepCount: definition.setup_steps.length,
    validationCommandCount: definition.validation_commands.length,
    setupStepKinds: [...new Set(definition.setup_steps.map((step) => step.kind))].sort(),
  }
}

export function assertProviderLaunchAfterReady(requestLog, readySequence) {
  const launches = requestLog.filter((entry) => entry.variant === 'LaunchProviderRun')
  if (launches.length !== 1) throw new Error(`expected exactly one explicit provider launch, observed ${launches.length}`)
  if (launches[0].sequence <= readySequence) throw new Error('provider launch was sent before authoritative setup Ready')
  const submissions = requestLog.filter((entry) => entry.variant === 'SubmitPrompt')
  if (submissions.length !== 1 || submissions[0].sequence <= launches[0].sequence) {
    throw new Error('provider prompt was not submitted exactly once after provider launch')
  }
  return { launchSequence: launches[0].sequence, submitSequence: submissions[0].sequence }
}

function redactEndpoint(endpoint) {
  try {
    const url = new URL(endpoint)
    url.username = ''
    url.password = ''
    url.search = ''
    url.hash = ''
    return url.href
  } catch {
    return '<invalid-endpoint>'
  }
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds))
}

async function loadKernelClientModules() {
  const modulePaths = {
    ipc: path.join(repoRoot, 'packages', 'kernel-client', 'dist', 'ipc.js'),
    requests: path.join(repoRoot, 'packages', 'kernel-client', 'dist', 'ipc-requests.js'),
  }
  for (const [name, modulePath] of Object.entries(modulePaths)) {
    try {
      await access(modulePath)
    } catch {
      throw new Error(`live acceptance prerequisites are absent: built kernel-client ${name} module ${modulePath}`)
    }
  }
  const [{ LocalIpcClient }, requests] = await Promise.all([
    import(pathToFileURL(modulePaths.ipc).href),
    import(pathToFileURL(modulePaths.requests).href),
  ])
  return { LocalIpcClient, requests }
}

function setupCapabilityDiagnostic() {
  return {
    capability: 'Project environment setup',
    requestVariant: 'GetProjectEnvironmentSetupStatus',
    minimumProtocolVersion: PROJECT_ENVIRONMENT_SETUP_MINIMUM_PROTOCOL,
  }
}

async function sendTracked(context, request, diagnostic = null) {
  const entry = {
    sequence: context.sequence + 1,
    atMs: Date.now(),
    variant: requestVariant(request),
    phase: context.lastStatus?.phase ?? null,
  }
  context.sequence = entry.sequence
  context.requestLog.push(entry)
  if (diagnostic) {
    return withProjectEnvironmentSetupProtocolMinimum(
      () => context.client.send(request),
      diagnostic,
    )
  }
  return context.client.send(request)
}

function observeSetupStatus(context, status) {
  validateSetupStatusIdentity(
    status,
    context.options,
    context.operationId,
    context.sessionId,
    context.agentId,
  )
  context.lastStatus = status
  context.statuses.push(summarizeProjectEnvironmentSetupStatus(status))
  if (status.phase === 'ready' && context.readySequence == null) {
    context.readySequence = context.sequence
    context.readyAtMs = Date.now()
  }
  return status
}

async function getSetupStatus(context, requests) {
  const response = await sendTracked(
    context,
    requests.getProjectEnvironmentSetupStatusRequest(context.operationId),
    setupCapabilityDiagnostic(),
  )
  return observeSetupStatus(
    context,
    requireResponseVariant(response, ['ProjectEnvironmentSetupStatus'], 'GetProjectEnvironmentSetupStatus').status,
  )
}

async function waitForCancellableOrTerminal(context, requests) {
  const deadline = Date.now() + context.options.timeoutMs
  while (Date.now() < deadline) {
    const status = await getSetupStatus(context, requests)
    if (CANCELLABLE_SETUP_PHASES.has(status.phase)) return status
    if (TERMINAL_SETUP_PHASES.has(status.phase)) return status
    await sleep(context.options.pollMs)
  }
  throw new Error('timed out waiting for a cancellable project setup phase')
}

async function waitForReady(context, requests) {
  const deadline = Date.now() + context.options.timeoutMs
  while (Date.now() < deadline) {
    const status = await getSetupStatus(context, requests)
    if (status.phase === 'ready') return status
    if (status.phase === 'failed' || status.phase === 'cancelled') {
      throw new Error(`project setup stopped in ${status.phase} (${status.failure_code || 'no failure code'})`)
    }
    await sleep(context.options.pollMs)
  }
  throw new Error('timed out waiting for authoritative project environment Ready')
}

async function waitForProviderRun(context, requests, providerRunId) {
  const deadline = Date.now() + context.options.timeoutMs
  while (Date.now() < deadline) {
    const response = await sendTracked(context, requests.getProviderRunRequest(providerRunId))
    const providerRun = requireResponseVariant(response, ['ProviderRun'], 'GetProviderRun').provider_run
    if (['Running', 'Parked', 'running', 'parked'].includes(providerRun?.state)) return providerRun
    if (['Failed', 'Exited', 'failed', 'exited'].includes(providerRun?.state)) {
      throw new Error(`provider run ${providerRunId} did not become runnable (state=${providerRun.state})`)
    }
    await sleep(context.options.pollMs)
  }
  throw new Error(`timed out waiting for provider run ${providerRunId}`)
}

function matchingCompletionEvents(context, providerRunId, afterMs = 0) {
  return context.events.filter((event) => (
    event.event === 'assistant_message_completed'
      && event.sessionId === context.sessionId
      && event.agentId === context.agentId
      && event.providerRunId === providerRunId
      && (event.completedAtMs || event.observedAtMs || 0) >= afterMs
  ))
}

async function waitForProviderCompletion(context, providerRunId, baseline, submittedAtMs) {
  const deadline = Date.now() + context.options.timeoutMs
  while (Date.now() < deadline) {
    const completions = matchingCompletionEvents(context, providerRunId)
    const freshCompletions = matchingCompletionEvents(context, providerRunId, submittedAtMs)
    if (completions.length > baseline && freshCompletions.length > 0) {
      return freshCompletions[freshCompletions.length - 1]
    }
    await sleep(context.options.pollMs)
  }
  throw new Error(`timed out waiting for one provider completion on run ${providerRunId}`)
}

function eventMetadata(event) {
  return {
    event: event?.event ?? null,
    sessionId: event?.session_id ?? null,
    agentId: event?.agent_id ?? null,
    providerRunId: event?.provider_run_id ?? null,
    completedAtMs: event?.completed_at_ms ?? null,
    observedAtMs: Date.now(),
  }
}

async function verifySetupCapability(context, requests) {
  const operationId = `setup-capability-probe-${randomUUID()}`
  try {
    const response = await sendTracked(
      context,
      requests.getProjectEnvironmentSetupStatusRequest(operationId),
      setupCapabilityDiagnostic(),
    )
    requireResponseVariant(response, ['ProjectEnvironmentSetupStatus'], 'Project setup capability probe')
    return { status: 'verified', reason: 'status request was accepted', operationId }
  } catch (error) {
    const classification = classifyProjectEnvironmentSetupCapabilityProbeError(error)
    if (classification.status === 'blocked') {
      throw new Error(
        `selected home kernel does not support project environment setup: ${classification.message}`,
      )
    }
    if (classification.status !== 'verified') {
      throw new Error(`selected home kernel setup capability is unverified: ${classification.message}`)
    }
    return { ...classification, operationId }
  }
}

function reportSafeOptions(options) {
  return {
    endpoint: redactEndpoint(options.relayUrl || options.kernelUrl),
    connection: options.relayUrl ? 'relay-to-home-kernel' : 'home-kernel',
    homeDaemonId: options.homeDaemonId,
    homeDaemonAlias: options.homeDaemonAlias,
    workerMachineId: options.workerMachineId,
    workerKernelId: options.workerKernelId,
    managedEnvironmentId: options.managedEnvironmentId,
    projectId: options.projectId,
    workspaceId: options.workspaceId,
    worktreeId: options.worktreeId,
    targetPlatform: options.targetPlatform,
    provider: options.provider,
    accountProfile: options.accountProfile,
    model: options.model,
    effort: options.effort,
    validationCommandCountRequested: options.validationCommands.length,
    promptSubmitted: false,
  }
}

function safeErrorMessage(error, options) {
  let message = error instanceof Error ? error.message : String(error)
  for (const secret of [options.relayToken, options.localAuthToken]) {
    if (secret) message = message.split(secret).join('<redacted>')
  }
  return message
}

function outsideRepository(candidatePath) {
  const resolved = path.resolve(candidatePath)
  return resolved !== repoRoot && !resolved.startsWith(`${repoRoot}${path.sep}`)
}

function reportPathFor(options) {
  const artifactRoot = path.resolve(options.artifactRoot)
  const reportPath = path.resolve(options.reportPath || path.join(artifactRoot, 'acceptance-report.json'))
  if (!outsideRepository(artifactRoot) || !outsideRepository(reportPath)) {
    throw new Error('drill artifacts and reports must be outside the repository')
  }
  return { artifactRoot, reportPath }
}

async function writeDrillReport(reportPath, report) {
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { encoding: 'utf8', mode: 0o600 })
}

async function cleanupDrillResources(context, requests) {
  const cleanup = {
    cancelledActiveSetup: false,
    unsubscribed: false,
    detached: false,
    endedSession: false,
    errors: [],
  }
  if (context.client && context.operationId && context.sessionId && context.lastStatus
    && !TERMINAL_SETUP_PHASES.has(context.lastStatus.phase)) {
    try {
      await sendTracked(
        context,
        requests.cancelProjectEnvironmentSetupRequest(context.operationId, context.sessionId),
        setupCapabilityDiagnostic(),
      )
      cleanup.cancelledActiveSetup = true
    } catch (error) {
      cleanup.errors.push(`setup cleanup: ${safeErrorMessage(error, context.options)}`)
    }
  }
  if (context.eventUnsubscriber) {
    context.eventUnsubscriber()
    context.eventUnsubscriber = null
  }
  if (context.subscribed) {
    try {
      await context.client.unsubscribeFromKernelEvents()
      cleanup.unsubscribed = true
    } catch (error) {
      cleanup.errors.push(`event unsubscribe: ${safeErrorMessage(error, context.options)}`)
    }
  }
  if (context.attachmentId) {
    try {
      await sendTracked(context, requests.detachFromSessionRequest(context.attachmentId))
      cleanup.detached = true
    } catch (error) {
      cleanup.errors.push(`session detach: ${safeErrorMessage(error, context.options)}`)
    }
  }
  if (context.sessionId) {
    try {
      await sendTracked(context, requests.endSessionRequest(context.sessionId))
      cleanup.endedSession = true
    } catch (error) {
      cleanup.errors.push(`session end: ${safeErrorMessage(error, context.options)}`)
    }
  }
  if (context.client) await context.client.close().catch((error) => {
    cleanup.errors.push(`client close: ${safeErrorMessage(error, context.options)}`)
  })
  return cleanup
}

export async function runProjectEnvironmentSetupAcceptance(options, modules = null) {
  validateProjectEnvironmentSetupOptions(options)
  const { artifactRoot, reportPath } = reportPathFor(options)
  await mkdir(artifactRoot, { recursive: true, mode: 0o700 })
  const startedAtMs = Date.now()
  const injectedOrchestration = modules != null
  const report = {
    schema: 'chariox.live.project_environment_setup_acceptance.v1',
    status: 'running',
    liveAcceptance: false,
    executionMode: injectedOrchestration ? 'injected-orchestration-test' : 'live',
    startedAtMs,
    options: reportSafeOptions(options),
    protocol: {
      minimumLocalDaemon: PROJECT_ENVIRONMENT_SETUP_MINIMUM_PROTOCOL,
      minimumRelayPeer: PROJECT_ENVIRONMENT_SETUP_RELAY_PEER_MINIMUM_PROTOCOL,
      capability: null,
    },
    worker: null,
    setup: null,
    provider: null,
    requests: [],
    statuses: [],
    events: [],
    cleanup: null,
    reportPath,
  }
  const context = {
    options,
    client: null,
    sequence: 0,
    requestLog: report.requests,
    statuses: report.statuses,
    events: report.events,
    operationId: null,
    sessionId: null,
    attachmentId: null,
    agentId: null,
    lastStatus: null,
    readySequence: null,
    readyAtMs: null,
    subscribed: false,
    eventUnsubscriber: null,
  }
  let failure = null
  let requests = null
  try {
    const loaded = modules || await loadKernelClientModules()
    requests = loaded.requests
    const endpoint = options.relayUrl || options.kernelUrl
    context.client = new loaded.LocalIpcClient(endpoint, options.relayUrl
      ? {
          relayAuthToken: options.relayToken,
          targetDaemonId: options.homeDaemonId,
          targetDaemonAlias: options.homeDaemonAlias,
        }
      : { localAuthToken: options.localAuthToken })
    if (typeof context.client.supportsKernelEvents !== 'function' || !context.client.supportsKernelEvents()) {
      throw new Error('live acceptance requires the public WebSocket kernel event transport')
    }

    report.protocol.capability = await verifySetupCapability(context, requests)
    const machineResponse = await sendTracked(context, requests.listRemoteMachinesRequest())
    const machines = requireResponseVariant(machineResponse, ['RemoteMachinesListed'], 'ListRemoteMachines').machines
    const kernelResponse = await sendTracked(
      context,
      requests.listRemoteMachineKernelsRequest(options.workerMachineId),
    )
    const kernels = requireResponseVariant(kernelResponse, ['RemoteMachineKernelsListed'], 'ListRemoteMachineKernels').kernels
    const worker = selectApprovedPath1Worker(machines, kernels, options)
    const managedResponse = await sendTracked(
      context,
      requests.getManagedEnvironmentRequest(options.managedEnvironmentId),
    )
    const managedEnvironment = requireResponseVariant(managedResponse, ['ManagedEnvironment'], 'GetManagedEnvironment').environment
    validateManagedPath1Environment(managedEnvironment, options, worker)
    report.worker = {
      machineId: worker.machine.machine_id,
      kernelId: worker.kernel.kernel_id,
      platform: options.targetPlatform,
      runtimeReleaseDigest: managedEnvironment.runtimeReleaseDigest,
      managedEnvironmentId: managedEnvironment.environmentId,
    }

    const initialProjectsResponse = await sendTracked(context, requests.listProjectsRequest(false))
    const initialProjects = requireResponseVariant(initialProjectsResponse, ['ProjectsListed'], 'ListProjects').projects
    validateSelectedProject(
      initialProjects.find((project) => project.id === options.projectId),
      options,
    )

    const alias = `project-environment-setup-acceptance-${Date.now()}-${process.pid}`
    const createdResponse = await sendTracked(
      context,
      requests.createSessionRequest(
        options.workspaceId,
        options.worktreeId,
        alias,
        {
          provider: options.provider,
          account_profile: options.accountProfile,
          model: options.model,
          effort: options.effort,
          execution_mode: 'build',
          permission_level: 'required',
        },
        null,
        null,
        options.workerKernelId,
        null,
        { kind: 'existing', project_id: options.projectId },
      ),
    )
    const created = requireResponseVariant(createdResponse, ['SessionCreated'], 'CreateSession')
    context.sessionId = created.session.id
    context.agentId = created.agent.id
    validateCreatedRemoteSession(created.session, created.agent, options, worker)

    const attachmentResponse = await sendTracked(
      context,
      requests.attachToSessionRequest(context.sessionId, alias),
    )
    context.attachmentId = requireResponseVariant(attachmentResponse, ['SessionAttached'], 'AttachToSession').attachment.id
    context.eventUnsubscriber = context.client.onKernelEvent((event) => {
      context.events.push(eventMetadata(event))
    })
    await context.client.subscribeToKernelEvents(context.sessionId, context.attachmentId)
    context.subscribed = true

    context.operationId = `project-environment-setup-acceptance-${randomUUID()}`
    const setupInput = buildProjectEnvironmentSetupRequestInput(
      options,
      context.operationId,
      context.agentId,
      context.sessionId,
    )
    const setupRequests = buildProjectEnvironmentSetupRequests(requests, setupInput)
    const startedResponse = await sendTracked(context, setupRequests.start, setupCapabilityDiagnostic())
    const startedStatus = observeSetupStatus(
      context,
      requireResponseVariant(startedResponse, ['ProjectEnvironmentSetupStarted'], 'StartProjectEnvironmentSetup').status,
    )
    if (startedStatus.attempt !== 1) throw new Error(`initial setup attempt was ${startedStatus.attempt}, expected 1`)

    const cancellableStatus = await waitForCancellableOrTerminal(context, requests)
    let cancellation = { status: 'not_applicable', reason: null }
    let retry = { status: 'not_applicable', attempt: null }
    if (CANCELLABLE_SETUP_PHASES.has(cancellableStatus.phase)) {
      const cancelledResponse = await sendTracked(context, setupRequests.cancel, setupCapabilityDiagnostic())
      const cancelledStatus = observeSetupStatus(
        context,
        requireResponseVariant(cancelledResponse, ['ProjectEnvironmentSetupCancelled'], 'CancelProjectEnvironmentSetup').status,
      )
      if (cancelledStatus.phase !== 'cancelled') throw new Error(`cancel did not settle as cancelled (phase=${cancelledStatus.phase})`)
      cancellation = { status: 'exercised', phase: cancelledStatus.phase, attempt: cancelledStatus.attempt }
      const retriedResponse = await sendTracked(context, setupRequests.retry, setupCapabilityDiagnostic())
      const retriedStatus = observeSetupStatus(
        context,
        requireResponseVariant(retriedResponse, ['ProjectEnvironmentSetupRetried'], 'RetryProjectEnvironmentSetup').status,
      )
      if (retriedStatus.phase !== 'requested' || retriedStatus.attempt !== cancelledStatus.attempt + 1) {
        throw new Error('retry did not begin a new setup attempt after cancellation')
      }
      retry = { status: 'exercised', phase: retriedStatus.phase, attempt: retriedStatus.attempt }
    } else if (!options.allowTerminalBeforeCancel) {
      throw new Error(`setup reached terminal ${cancellableStatus.phase} before cancellation; rerun with a bounded real validation delay or explicitly allow this gap`)
    } else {
      cancellation.reason = `setup reached ${cancellableStatus.phase} before a cancellation request was applicable`
    }

    const readyStatus = context.lastStatus?.phase === 'ready'
      ? context.lastStatus
      : await waitForReady(context, requests)
    const validation = validateReadyProjectEnvironmentSetup(
      readyStatus,
      options,
      context.operationId,
      context.sessionId,
      context.agentId,
    )
    const readyProjectsResponse = await sendTracked(context, requests.listProjectsRequest(false))
    const readyProjects = requireResponseVariant(readyProjectsResponse, ['ProjectsListed'], 'ListProjects after Ready').projects
    const readyProject = readyProjects.find((project) => project.id === options.projectId)
    const definition = validateUtilityGeneratedProjectDefinition(readyProject, options, validation)
    report.setup = {
      operationId: context.operationId,
      attempt: readyStatus.attempt,
      phases: [...new Set(context.statuses.map((status) => status.phase))],
      cancellation,
      retry,
      readyObservedAtMs: context.readyAtMs,
      definitionDigest: readyStatus.definition_digest,
      validation,
      definition,
    }

    const stateResponse = await sendTracked(context, requests.getSessionStateRequest(context.sessionId))
    const sessionState = requireResponseVariant(stateResponse, ['SessionState'], 'GetSessionState')
    const finalAgent = (sessionState.session?.agents || []).find((agent) => agent.id === context.agentId)
    validateCreatedRemoteSession(sessionState.session, finalAgent, options, worker)

    const launchResponse = await sendTracked(
      context,
      requests.launchProviderRunRequest(
        context.sessionId,
        options.provider,
        options.accountProfile,
        options.model,
        options.effort,
        context.agentId,
      ),
    )
    const providerRun = requireResponseVariant(
      launchResponse,
      ['ProviderRunLaunched', 'ProviderRunLaunchAccepted'],
      'LaunchProviderRun',
    ).provider_run
    if (!providerRun?.id || providerRun.session_id !== context.sessionId || providerRun.agent_instance_id !== context.agentId) {
      throw new Error('provider launch did not return the selected session/remote agent binding')
    }
    const runnableProviderRun = await waitForProviderRun(context, requests, providerRun.id)
    const completionBaseline = matchingCompletionEvents(context, runnableProviderRun.id).length
    const promptSubmittedAtMs = Date.now()
    const promptResponse = await sendTracked(
      context,
      requests.submitPromptRequest(
        context.sessionId,
        context.attachmentId,
        context.agentId,
        options.prompt,
        [],
      ),
    )
    if (!promptResponse?.PromptSubmitted && !promptResponse?.PromptsSubmitted) {
      throw new Error(`SubmitPrompt returned an unexpected response variant: ${requestVariant(promptResponse)}`)
    }
    if (promptResponse.PromptsSubmitted && (promptResponse.PromptsSubmitted.failures || []).length > 0) {
      throw new Error('SubmitPrompt returned a failed batch result')
    }
    report.options.promptSubmitted = true
    const completion = await waitForProviderCompletion(
      context,
      runnableProviderRun.id,
      completionBaseline,
      promptSubmittedAtMs,
    )
    const completionCount = matchingCompletionEvents(
      context,
      runnableProviderRun.id,
      promptSubmittedAtMs,
    ).length
    if (completionCount !== 1) throw new Error(`expected one provider completion after SubmitPrompt, observed ${completionCount}`)
    const providerEvidence = assertProviderLaunchAfterReady(context.requestLog, context.readySequence)
    report.provider = {
      launchAfterReady: true,
      launchSequence: providerEvidence.launchSequence,
      submitSequence: providerEvidence.submitSequence,
      providerRunId: runnableProviderRun.id,
      providerRunState: runnableProviderRun.state,
      completionCount,
      completionEventAtMs: completion.completedAtMs,
    }
    report.status = 'passed'
    report.liveAcceptance = !injectedOrchestration
  } catch (error) {
    failure = safeErrorMessage(error, options)
    report.status = 'failed'
    report.failure = failure
  } finally {
    report.cleanup = requests ? await cleanupDrillResources(context, requests) : null
    if (report.cleanup?.errors.length) {
      const cleanupFailure = `cleanup failed: ${report.cleanup.errors.join('; ')}`
      failure = failure ? `${failure}; ${cleanupFailure}` : cleanupFailure
      report.status = 'failed'
      report.liveAcceptance = false
      report.failure = failure
    }
    report.finishedAtMs = Date.now()
    report.durationMs = report.finishedAtMs - startedAtMs
    await writeDrillReport(reportPath, report)
  }
  if (failure) {
    throw new Error(`${failure}; report=${reportPath}`)
  }
  return report
}

export async function main(argv = process.argv.slice(2)) {
  const options = parseProjectEnvironmentSetupArguments(argv)
  if (options.help) {
    printProjectEnvironmentSetupHelp()
    return
  }
  validateProjectEnvironmentSetupOptions(options)
  const report = await runProjectEnvironmentSetupAcceptance(options)
  console.log(JSON.stringify({
    status: report.status,
    worker: report.worker,
    setup: report.setup,
    provider: report.provider,
    reportPath: report.reportPath,
  }, null, 2))
}

const invokedAsScript = process.argv[1]
  && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
if (invokedAsScript) {
  main().catch((error) => {
    console.error(`[project-environment-setup-acceptance] ${error instanceof Error ? error.message : String(error)}`)
    process.exitCode = 1
  })
}
