import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'

export const PROJECT_ENVIRONMENT_SETUP_DRILL_PIN_SCHEMA = 'chariox.project-environment-setup-drill.target-pin.v1'
export const PROJECT_ENVIRONMENT_SETUP_DRILL_OPERATION_TIMEOUT_MS = 15 * 60 * 1_000
export const PROJECT_ENVIRONMENT_SETUP_DRILL_POLL_MS = 1_500
export const PROJECT_ENVIRONMENT_SETUP_DRILL_CONTROL_TIMEOUT_MS = 10_000
export const PROJECT_ENVIRONMENT_SETUP_DRILL_MAX_RETRIES = 1

const OFFICIAL_PROVIDERS = new Set(['codex', 'opencode', 'claude', 'claude-p', 'claude-headless'])
const PIN_FIELDS = new Set([
  'schema', 'reviewed_at', 'reviewed_by', 'review_reference', 'disposable_target', 'target_setup_approved',
  'drill_owned_context', 'project_id', 'project_owner_user_id', 'session_id',
  'workspace_id', 'worktree_id', 'target_machine_id', 'target_kernel_id',
  'target_platform', 'utility_agent_id', 'utility_provider', 'utility_account_profile',
  'utility_provider_run_id', 'launch_agent_id', 'launch_provider', 'launch_account_profile',
  'launch_model', 'launch_effort',
])
const SAFE_SETUP_MESSAGES = new Set([
  'utility agent is preparing the target environment',
  'kernel is reusing the stored project environment definition',
  'kernel is validating the reused definition in the target worker',
  'reused definition passed kernel validation',
])
const TERMINAL_PHASES = new Set(['ready', 'failed', 'cancelled'])

const BASE_VALIDATION_COMMANDS = Object.freeze([
  'rustc --version',
  'cargo --version',
  'cc --version',
  'pkg-config --version',
])
const FIXTURE_RUN_ID = '00000000-0000-4000-8000-000000000000'
export const PROJECT_ENVIRONMENT_SETUP_DRILL_NATIVE_BUILD_PACKAGE = 'chariox-kernel'
export const PROJECT_ENVIRONMENT_SETUP_DRILL_NATIVE_BUILD_BINARY = 'chariox-kernel'
export const PROJECT_ENVIRONMENT_SETUP_DRILL_NATIVE_BUILD_JOBS = 2

export function projectEnvironmentSetupDrillNativeBuildCommand(runId) {
  assert.ok(/^[0-9a-f-]{36}$/.test(runId), 'validation scratch identity must be a UUID')
  // The worker kernel SIGKILLs the command process group on cancellation or
  // timeout, which bypasses EXIT traps. A successful exit therefore verifies
  // cleanup; a hard-killed attempt remains a failed, incomplete validation.
  const buildCommand = [
    'set -eu',
    'build_root="${TMPDIR:-/tmp}"',
    'case "$build_root" in /*) ;; *) build_root=/tmp ;; esac',
    'workspace_root=$(pwd -P)',
    'build_root=$(CDPATH= cd "$build_root" && pwd -P)',
    'case "$build_root/" in "$workspace_root/"*) exit 72 ;; esac',
    `build_dir="$build_root/chariox-project-environment-setup-${runId}"`,
    'owner_file="$build_dir/.chariox-project-setup-owner"',
    `if [ -e "$build_dir" ]; then [ ! -L "$build_dir" ] && [ -d "$build_dir" ] && [ -f "$owner_file" ] && [ "$(cat "$owner_file")" = "${runId}" ] || exit 73; rm -rf "$build_dir"; fi`,
    'mkdir -m 700 "$build_dir"',
    `printf '%s\\n' '${runId}' > "$owner_file"`,
    `cleanup_build_dir() { if [ ! -e "$build_dir" ]; then return 0; fi; [ ! -L "$build_dir" ] && [ -d "$build_dir" ] && [ -f "$owner_file" ] && [ "$(cat "$owner_file")" = "${runId}" ] || return 74; rm -rf "$build_dir"; [ ! -e "$build_dir" ]; }`,
    'finish_build() { build_status=$?; trap - EXIT; if cleanup_build_dir; then exit "$build_status"; else exit 74; fi; }',
    'trap finish_build EXIT',
    `CARGO_TARGET_DIR="$build_dir/target" CARGO_BUILD_JOBS=${PROJECT_ENVIRONMENT_SETUP_DRILL_NATIVE_BUILD_JOBS} CARGO_INCREMENTAL=0 cargo build --locked --jobs ${PROJECT_ENVIRONMENT_SETUP_DRILL_NATIVE_BUILD_JOBS} --package ${PROJECT_ENVIRONMENT_SETUP_DRILL_NATIVE_BUILD_PACKAGE} --bin ${PROJECT_ENVIRONMENT_SETUP_DRILL_NATIVE_BUILD_BINARY}`,
  ].join('; ')
  return buildCommand
}

export function projectEnvironmentSetupDrillValidationCommands(runId) {
  assert.ok(/^[0-9a-f-]{36}$/.test(runId), 'validation scratch identity must be a UUID')
  return Object.freeze([...BASE_VALIDATION_COMMANDS, projectEnvironmentSetupDrillNativeBuildCommand(runId)])
}

export const PROJECT_ENVIRONMENT_SETUP_DRILL_VALIDATION_COMMANDS = projectEnvironmentSetupDrillValidationCommands(FIXTURE_RUN_ID)

export function parseProjectEnvironmentSetupDrillArgs(argv) {
  const options = {}
  const valueFlags = new Set([
    '--home-kernel', '--target-pin', '--scratch-dir', '--evidence-dir', '--confirm-pin-sha256', '--confirm-target',
    '--confirm-delete-session', '--confirm-delete-project',
  ])
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index]
    if (flag === '--help' || flag === '-h') {
      options.help = true
    } else if (flag === '--allow-provider-launch') {
      if (options.allowProviderLaunch) throw new Error('--allow-provider-launch may be supplied once')
      options.allowProviderLaunch = true
    } else if (flag === '--allow-target-setup') {
      if (options.allowTargetSetup) throw new Error('--allow-target-setup may be supplied once')
      options.allowTargetSetup = true
    } else if (flag === '--allow-one-retry') {
      if (options.allowOneRetry) throw new Error('--allow-one-retry may be supplied once')
      options.allowOneRetry = true
    } else if (valueFlags.has(flag)) {
      const value = argv[index + 1]
      if (!value || value.startsWith('--')) throw new Error(`${flag} requires a value`)
      if (options[flag]) throw new Error(`${flag} may be supplied once`)
      options[flag] = value
      index += 1
    } else {
      throw new Error(`unknown option: ${flag}`)
    }
  }
  if (options.help) return options
  for (const flag of valueFlags) {
    if (!options[flag]) throw new Error(`${flag} is required`)
  }
  if (!options.allowProviderLaunch) throw new Error('--allow-provider-launch is required')
  if (!options.allowTargetSetup) throw new Error('--allow-target-setup is required')
  return options
}

export function assertLoopbackHomeKernelUrl(value) {
  let url
  try {
    url = new URL(value)
  } catch {
    throw new Error('home kernel URL must be a loopback WebSocket URL')
  }
  const host = url.hostname.toLowerCase()
  assert.ok(url.protocol === 'ws:', 'home kernel URL must use ws:// on loopback')
  assert.ok(['127.0.0.1', '[::1]'].includes(host), 'home kernel endpoint must be numeric loopback')
  assert.equal(url.username, '', 'home kernel URL cannot contain a username')
  assert.equal(url.password, '', 'home kernel URL cannot contain a password')
  assert.equal(url.search, '', 'home kernel URL cannot contain query parameters')
  assert.equal(url.hash, '', 'home kernel URL cannot contain a fragment')
  assert.ok(url.pathname === '/' || url.pathname === '/kernel', 'home kernel URL path is unsupported')
  return url.toString()
}

export function validateProjectEnvironmentSetupDrillPin(pin, { nowMs = Date.now() } = {}) {
  assert.ok(pin && typeof pin === 'object' && !Array.isArray(pin), 'target pin must be a JSON object')
  for (const key of Object.keys(pin)) assert.ok(PIN_FIELDS.has(key), `target pin has unsupported field ${key}`)
  assert.equal(pin.schema, PROJECT_ENVIRONMENT_SETUP_DRILL_PIN_SCHEMA, 'target pin schema is not reviewed for this drill')
  for (const key of PIN_FIELDS) {
    if (['schema', 'disposable_target', 'target_setup_approved', 'drill_owned_context'].includes(key)) continue
    assert.equal(typeof pin[key], 'string', `target pin ${key} must be a string`)
    assert.ok(pin[key].trim(), `target pin ${key} must not be empty`)
  }
  assert.equal(pin.disposable_target, true, 'target pin does not mark the worker disposable')
  assert.equal(pin.target_setup_approved, true, 'target pin does not authorize target Project setup mutations')
  assert.equal(pin.drill_owned_context, true, 'target pin does not mark the context and agents as drill-owned')
  assert.ok(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,3})?Z$/.test(pin.reviewed_at), 'target pin reviewed_at must be an ISO-8601 UTC timestamp')
  const reviewedAt = Date.parse(pin.reviewed_at)
  assert.ok(Number.isFinite(reviewedAt), 'target pin reviewed_at must be an ISO timestamp')
  assert.ok(reviewedAt <= nowMs, 'target pin review timestamp is in the future')
  assert.ok(nowMs - reviewedAt <= 30 * 24 * 60 * 60 * 1_000, 'target pin review is older than 30 days')
  assert.ok(pin.utility_agent_id !== pin.launch_agent_id, 'utility and launch agents must be distinct')
  assert.ok(OFFICIAL_PROVIDERS.has(pin.utility_provider), 'target pin must select an official utility provider')
  assert.ok(OFFICIAL_PROVIDERS.has(pin.launch_provider), 'target pin must select an official provider, not a fixture or shell substitute')
  assert.ok(/^(linux|macos)-(x86_64|aarch64)$/.test(pin.target_platform), 'target platform must be a supported Unix Rust platform')
  return pin
}

export function validateSetupDrillConfirmations(options, pin, pinSha256) {
  assert.equal(options['--confirm-pin-sha256'], pinSha256, 'pin digest confirmation does not match the reviewed target pin bytes')
  assert.equal(options['--confirm-target'], pin.target_machine_id, 'target confirmation does not match the reviewed machine')
  assert.equal(options['--confirm-delete-session'], pin.session_id, 'session deletion confirmation does not match the reviewed context')
  assert.equal(options['--confirm-delete-project'], pin.project_id, 'project deletion confirmation does not match the reviewed context')
}

export function setupDrillOperationId(runId, branch) {
  assert.ok(/^[0-9a-f-]{36}$/.test(runId), 'run id must be a UUID')
  assert.ok(branch === 'cold' || branch === 'stored', 'setup branch is unsupported')
  return `project-setup-live-${branch}-${runId}`
}

export function startSetupParameters(pin, operationId, branch, validationCommands = PROJECT_ENVIRONMENT_SETUP_DRILL_VALIDATION_COMMANDS) {
  const payload = {
    operationId,
    projectId: pin.project_id,
    sessionId: pin.session_id,
    agentId: pin.utility_agent_id,
    targetWorkerId: pin.target_machine_id,
    targetPlatform: pin.target_platform,
  }
  if (branch === 'cold') payload.validationCommands = [...validationCommands]
  return payload
}

export function assertProjectEnvironmentSetupDrillPreflight({ pin, machines, kernels, projects, sessions, session }) {
  const machine = (machines ?? []).find((entry) => entry.machine_id === pin.target_machine_id)
  assert.ok(machine, 'reviewed target machine is not enrolled in the home kernel')
  assert.equal(machine.trust_status, 'approved', 'reviewed target machine is not approved')
  assert.equal(machine.online, true, 'reviewed target machine is offline')

  const kernel = (kernels ?? []).find((entry) => entry.kernel_id === pin.target_kernel_id)
  assert.ok(kernel, 'reviewed target kernel is not registered on the machine')
  assert.equal(kernel.machine_id, pin.target_machine_id, 'reviewed target kernel moved to another machine')
  assert.equal(kernel.accepting_remote_leases, true, 'reviewed target kernel is not accepting remote leases')
  const available = new Set(kernel.available_providers ?? [])
  assert.ok(available.has(providerCapability(pin.utility_provider)), 'target kernel does not advertise the pinned utility provider')
  assert.ok(available.has(providerCapability(pin.launch_provider)), 'target kernel does not advertise the pinned launch provider')

  const project = (projects ?? []).find((entry) => entry.id === pin.project_id)
  assert.ok(project, 'pinned Project is not visible in the home kernel')
  assert.equal(project.status, 'active', 'pinned Project is not active')
  assert.equal(project.owner_user_id, pin.project_owner_user_id, 'pinned Project owner changed')
  assert.ok(project.environment_definition == null, 'cold setup requires the drill-owned Project to have no environment definition')
  assert.ok(project.workspace_id === pin.workspace_id || (project.workspace_ids ?? []).includes(pin.workspace_id), 'pinned workspace is not attached to the Project')

  assert.ok(session, 'pinned session is not visible in the home kernel')
  assert.equal(session.id, pin.session_id, 'home kernel returned a different session')
  assert.equal(session.project_id, pin.project_id, 'session is not attached to the pinned Project')
  assert.equal(session.workspace_id, pin.workspace_id, 'session workspace changed')
  assert.equal(session.worktree_id, pin.worktree_id, 'session worktree changed')
  const sessionStatus = String(session.status ?? '').toLowerCase()
  assert.ok(sessionStatus && !['ended', 'deleted', 'archived', 'closed'].includes(sessionStatus), 'pinned session is not active')
  const projectSessions = (sessions ?? []).filter((entry) => entry.project_id === pin.project_id)
  assert.equal(projectSessions.length, 1, 'pinned Project is shared with another session and cannot be safely cleaned up')
  assert.equal(projectSessions[0]?.id, pin.session_id, 'another session owns the pinned Project cleanup boundary')

  const agents = session.agents ?? []
  assert.equal(agents.length, 2, 'drill-owned context must contain exactly the pinned utility and launch agents')
  const utilityAgent = agents.find((entry) => entry.id === pin.utility_agent_id)
  const launchAgent = agents.find((entry) => entry.id === pin.launch_agent_id)
  assert.ok(utilityAgent && launchAgent, 'pinned utility or launch agent is missing')
  assertRemoteAgentBinding(utilityAgent, pin, 'utility')
  assertRemoteAgentBinding(launchAgent, pin, 'launch')
  assert.equal(utilityAgent.provider, pin.utility_provider, 'utility provider changed from the reviewed pin')
  assert.equal(utilityAgent.account_profile, pin.utility_account_profile, 'utility provider account profile changed from the reviewed pin')
  assert.equal(utilityAgent.remote_execution.active_worker_provider_run_id, pin.utility_provider_run_id, 'pinned utility provider run is not active on the target worker')
  assert.equal(providerCapability(launchAgent.provider), providerCapability(pin.launch_provider), 'launch agent is configured for a different provider')
  assert.equal(launchAgent.account_profile, pin.launch_account_profile, 'launch provider account profile changed from the reviewed pin')
  assert.equal(launchAgent.remote_execution.active_worker_provider_run_id ?? null, null, 'official launch agent already has a provider run before Ready')
  assert.notEqual(launchAgent.is_processing, true, 'official launch agent is already processing before Ready')

  return {
    machineId: machine.machine_id,
    trustStatus: machine.trust_status,
    online: machine.online,
    kernelId: kernel.kernel_id,
    acceptingRemoteLeases: kernel.accepting_remote_leases,
    availableProviders: [...available].sort(),
    projectId: project.id,
    sessionId: session.id,
    workspaceId: session.workspace_id,
    worktreeId: session.worktree_id,
    utilityAgentId: utilityAgent.id,
    utilityProvider: utilityAgent.provider,
    utilityAccountProfile: utilityAgent.account_profile,
    utilityProviderRunId: utilityAgent.remote_execution.active_worker_provider_run_id,
    launchAgentId: launchAgent.id,
    launchProvider: pin.launch_provider,
    launchAccountProfile: pin.launch_account_profile,
    workerLeases: [utilityAgent, launchAgent].map((agent) => ({
      agentId: agent.id,
      machineId: agent.remote_execution.worker_machine_id,
      kernelId: agent.remote_execution.worker_kernel_id,
      executionLeaseId: agent.remote_execution.execution_lease_id,
      leasedAgentId: agent.remote_execution.leased_agent_id,
    })),
  }
}

export function assertStoredDefinition(project, expectedPlatform, requiredCommands = PROJECT_ENVIRONMENT_SETUP_DRILL_VALIDATION_COMMANDS) {
  const definition = project?.environment_definition
  assert.ok(definition, 'stored definition was not persisted after cold setup')
  assert.equal(definition.origin, 'utility_generated', 'stored definition did not originate from the utility provider path')
  assert.equal(definition.target_platform, expectedPlatform, 'stored definition target platform changed')
  assert.ok(Array.isArray(definition.validation_commands) && definition.validation_commands.length > 0, 'stored definition has no validation commands')
  for (const requiredCommand of requiredCommands) {
    assert.ok(
      definition.validation_commands.includes(requiredCommand),
      `utility-generated definition omitted the required validation command ${commandDigest(requiredCommand)}`,
    )
  }
  return {
    origin: definition.origin,
    targetPlatform: definition.target_platform,
    source: definition.source,
    validationCommandCount: definition.validation_commands.length,
    validationCommandDigests: definition.validation_commands.map(commandDigest),
  }
}

export function providerCapability(provider) {
  return provider === 'claude-p' || provider === 'claude-headless' ? 'claude' : provider
}

export async function runBoundedProjectEnvironmentSetupOperation({
  createClient,
  pin,
  request,
  requestBuilders,
  timeoutMs = PROJECT_ENVIRONMENT_SETUP_DRILL_OPERATION_TIMEOUT_MS,
  pollMs = PROJECT_ENVIRONMENT_SETUP_DRILL_POLL_MS,
  controlTimeoutMs = PROJECT_ENVIRONMENT_SETUP_DRILL_CONTROL_TIMEOUT_MS,
  idempotentStartReplay = false,
  allowOneRetry = false,
  onStatus = () => {},
  nowMs = Date.now,
  delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
}) {
  const startPayload = request.StartProjectEnvironmentSetup
  assert.ok(startPayload?.operationId, 'setup operation request is missing operationId')
  assert.ok(typeof requestBuilders?.getProjectEnvironmentSetupStatusRequest === 'function', 'setup status request builder is required')
  assert.ok(typeof requestBuilders?.retryProjectEnvironmentSetupRequest === 'function', 'setup retry request builder is required')
  assert.ok(timeoutMs > 0 && timeoutMs <= PROJECT_ENVIRONMENT_SETUP_DRILL_OPERATION_TIMEOUT_MS, 'setup timeout must fit the fixed 15 minute bound')
  assert.ok(pollMs >= 250 && pollMs <= 5_000, 'setup poll interval is outside the bounded range')
  const deadline = nowMs() + timeoutMs
  const expected = {
    operation_id: startPayload.operationId,
    project_id: pin.project_id,
    session_id: pin.session_id,
    agent_id: pin.utility_agent_id,
    worker_id: pin.target_machine_id,
    platform: pin.target_platform,
  }
  let client = null
  let latest = null
  let retries = 0
  let expectedAttempt = null
  let recoveredAttempt = null
  let replayedAttempt = null
  const attemptHistory = []
  const onCandidate = (candidate, source) => {
    assertBoundSetupStatus(candidate, expected, expectedAttempt)
    if (expectedAttempt == null) expectedAttempt = candidate.attempt
    if (!attemptHistory.includes(candidate.attempt)) attemptHistory.push(candidate.attempt)
    latest = candidate
    onStatus(candidate, source)
  }

  try {
    client = await createClient()
    const startResponse = await sendWithin(client, request, controlTimeoutMs, 'start Project setup')
    latest = setupStatusFromResponse(startResponse, ['ProjectEnvironmentSetupStarted', 'ProjectEnvironmentSetupStatus'])
    onCandidate(latest, 'start')

    if (idempotentStartReplay) {
      const replayResponse = await sendWithin(client, request, controlTimeoutMs, 'replay Project setup start')
      const replayed = setupStatusFromResponse(replayResponse, ['ProjectEnvironmentSetupStarted', 'ProjectEnvironmentSetupStatus'])
      assertBoundSetupStatus(replayed, expected, expectedAttempt)
      assert.equal(replayed.attempt, latest.attempt, 'idempotent Start replay changed the attempt')
      assert.equal(replayed.created_at_ms, latest.created_at_ms, 'idempotent Start replay changed the creation timestamp')
      replayedAttempt = replayed.attempt
      onCandidate(replayed, 'idempotent_start_replay')
    }

    await closeClient(client)
    client = await createClient()
    const recovered = await getSetupStatus(client, startPayload.operationId, controlTimeoutMs, requestBuilders)
    assertBoundSetupStatus(recovered, expected, expectedAttempt)
    assert.equal(recovered.attempt, latest.attempt, 'reconnect changed the setup attempt')
    recoveredAttempt = recovered.attempt
    onCandidate(recovered, 'reconnect')
    latest = recovered

    while (true) {
      if (latest.phase === 'ready') return { status: latest, retries, attemptHistory, replayedAttempt, recoveredAttempt }
      if (latest.phase === 'cancelled') throw setupFailure('setup_cancelled')
      if (latest.phase === 'failed') {
        if (latest.retryable && allowOneRetry && retries < PROJECT_ENVIRONMENT_SETUP_DRILL_MAX_RETRIES) {
          const priorAttempt = latest.attempt
          const response = await sendWithin(client,
            requestBuilders.retryProjectEnvironmentSetupRequest(startPayload.operationId, pin.session_id),
            controlTimeoutMs,
            'retry Project setup',
          )
          const retried = setupStatusFromResponse(response, ['ProjectEnvironmentSetupRetried', 'ProjectEnvironmentSetupStatus'])
          assertBoundSetupStatus(retried, expected, priorAttempt + 1)
          assert.equal(retried.attempt, priorAttempt + 1, 'retry did not create exactly one new attempt')
          retries += 1
          expectedAttempt = priorAttempt + 1
          onCandidate(retried, 'retry')
          latest = retried
          continue
        }
        throw setupFailure(latest.retryable ? 'retryable_setup_failure_without_retry_authorization' : 'setup_failed_non_retryable')
      }
      if (!['requested', 'preparing', 'validating'].includes(latest.phase)) {
        throw setupFailure('setup_status_phase_unknown')
      }
      if (nowMs() >= deadline) throw setupFailure('setup_operation_timeout')
      await delay(Math.min(pollMs, Math.max(1, deadline - nowMs())))
      const next = await getSetupStatus(client, startPayload.operationId, controlTimeoutMs, requestBuilders)
      onCandidate(next, 'poll')
      latest = next
    }
  } finally {
    await closeClient(client)
  }
}

export function assertBoundSetupStatus(status, expected, expectedAttempt = null) {
  assert.ok(status && typeof status === 'object', 'Project setup response has no status')
  for (const [field, value] of Object.entries(expected)) {
    assert.equal(status[field], value, `Project setup ${field} changed from the reviewed binding`)
  }
  assert.ok(Number.isInteger(status.attempt) && status.attempt > 0, 'Project setup attempt is missing')
  assert.ok(Number.isInteger(status.progress_percent) && status.progress_percent >= 0 && status.progress_percent <= 100, 'Project setup progress is outside its bounded range')
  if (expectedAttempt != null) assert.equal(status.attempt, expectedAttempt, 'Project setup attempt changed unexpectedly')
  assert.ok(Number.isInteger(status.created_at_ms) && Number.isInteger(status.updated_at_ms), 'Project setup timestamps are missing')
  assert.ok(status.updated_at_ms >= status.created_at_ms, 'Project setup timestamps are out of order')
  assert.ok(TERMINAL_PHASES.has(status.phase) || ['requested', 'preparing', 'validating'].includes(status.phase), 'Project setup phase is unknown')
  return status
}

export function setupStatusBinding(pin, operationId) {
  return {
    operation_id: operationId,
    project_id: pin.project_id,
    session_id: pin.session_id,
    agent_id: pin.utility_agent_id,
    worker_id: pin.target_machine_id,
    platform: pin.target_platform,
  }
}

export function requireVariant(response, ...variants) {
  for (const variant of variants) {
    if (response?.[variant] != null) return response[variant]
  }
  throw new Error(`home kernel returned none of the expected response variants: ${variants.join(', ')}`)
}

export function assertReadySetupValidation(status, { expected, commands }) {
  assertBoundSetupStatus(status, expected)
  assert.equal(status.phase, 'ready', 'Project environment setup did not reach Ready')
  assert.equal(status.progress_percent, 100, 'Ready setup did not report completed progress')
  assert.ok(typeof status.definition_digest === 'string' && status.definition_digest.startsWith('sha256:'), 'Ready setup did not include a definition digest')
  const validation = status.validation
  assert.ok(validation, 'Ready setup has no target validation record')
  assert.ok(Array.isArray(validation.commands), 'Ready setup validation has no command results')
  assert.equal(validation.worker_id, expected.worker_id, 'validation worker changed from the reviewed target')
  assert.equal(validation.platform, expected.platform, 'validation platform changed from the reviewed target')
  assert.equal(validation.commands.length, commands.length, 'target validation result count changed')
  const expectedDigests = commands.map(commandDigest)
  assert.deepEqual(validation.commands.map((entry) => entry.command_digest), expectedDigests, 'target validation commands changed or were reordered')
  for (const result of validation.commands) {
    assert.equal(result.exit_code, 0, `target validation command ${result.command_digest} failed`)
    assert.ok(Number.isInteger(result.stdout_bytes) && result.stdout_bytes >= 0, 'target validation stdout byte count is invalid')
    assert.ok(Number.isInteger(result.stderr_bytes) && result.stderr_bytes >= 0, 'target validation stderr byte count is invalid')
  }
  return status
}

export function safeSetupStatusEvidence(status, source = null) {
  return {
    ...(source ? { source } : {}),
    operationId: status.operation_id,
    attempt: status.attempt,
    phase: status.phase,
    progressPercent: status.progress_percent,
    workerId: status.worker_id,
    platform: status.platform,
    definitionDigest: status.definition_digest ?? null,
    createdAtMs: status.created_at_ms,
    updatedAtMs: status.updated_at_ms,
    safeMessage: SAFE_SETUP_MESSAGES.has(status.message) ? status.message : null,
    failureCode: status.failure_code ?? null,
    retryable: Boolean(status.retryable),
    validation: status.validation ? {
      workerId: status.validation.worker_id,
      platform: status.validation.platform,
      commands: status.validation.commands.map((entry) => ({
        commandDigest: entry.command_digest,
        exitCode: entry.exit_code,
        stdoutBytes: entry.stdout_bytes,
        stderrBytes: entry.stderr_bytes,
      })),
    } : null,
  }
}

export function commandDigest(command) {
  return `sha256:${createHash('sha256').update(command, 'utf8').digest('hex')}`
}

export function setupStatusFromResponse(response, variants) {
  for (const name of variants) {
    if (response?.[name]?.status) return response[name].status
  }
  throw setupFailure(`expected_${variants.join('_or_')}_response`)
}

export function getSetupStatusFromResponse(response) {
  return setupStatusFromResponse(response, ['ProjectEnvironmentSetupStatus'])
}

export function setupFailure(code) {
  const error = new Error(code)
  error.code = code
  return error
}

async function getSetupStatus(client, operationId, timeoutMs, requestBuilders) {
  const response = await sendWithin(client, requestBuilders.getProjectEnvironmentSetupStatusRequest(operationId), timeoutMs, 'get Project setup status')
  return getSetupStatusFromResponse(response)
}

async function sendWithin(client, request, timeoutMs, label) {
  let timer
  try {
    return await Promise.race([
      client.send(request),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(setupFailure(`${label.replaceAll(' ', '_')}_timeout`)), timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function closeClient(client) {
  if (!client) return
  try {
    await client.close?.()
  } catch {
    // Transport close is cleanup only; the home kernel operation is durable.
  }
}

function assertRemoteAgentBinding(agent, pin, role) {
  assert.equal(agent.worktree_id, pin.worktree_id, `${role} agent worktree changed from the reviewed pin`)
  const remote = agent.remote_execution
  assert.ok(remote, `${role} agent is not placed on the reviewed remote worker`)
  assert.equal(remote.worker_machine_id, pin.target_machine_id, `${role} agent moved to a different worker machine`)
  assert.equal(remote.worker_kernel_id, pin.target_kernel_id, `${role} agent moved to a different worker kernel`)
  assert.ok(typeof remote.execution_lease_id === 'string' && remote.execution_lease_id, `${role} agent has no execution lease`)
  assert.ok(typeof remote.leased_agent_id === 'string' && remote.leased_agent_id, `${role} agent has no leased-agent identity`)
}
