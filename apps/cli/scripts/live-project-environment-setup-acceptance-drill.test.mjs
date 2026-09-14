import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  DEFAULT_ACCOUNT_PROFILE,
  DEFAULT_EFFORT,
  DEFAULT_MODEL,
  DEFAULT_PROVIDER,
  PROJECT_ENVIRONMENT_SETUP_MINIMUM_PROTOCOL,
  assertProviderLaunchAfterReady,
  buildProjectEnvironmentSetupRequestInput,
  buildProjectEnvironmentSetupRequests,
  classifyProjectEnvironmentSetupCapabilityProbeError,
  isUnknownRequestVariant,
  parseProjectEnvironmentSetupArguments,
  selectApprovedPath1Worker,
  validateCreatedRemoteSession,
  validateManagedPath1Environment,
  validateProjectEnvironmentSetupOptions,
  validateReadyProjectEnvironmentSetup,
  validateSelectedProject,
  validateUtilityGeneratedProjectDefinition,
  withProjectEnvironmentSetupProtocolMinimum,
} from './live-project-environment-setup-acceptance-drill.mjs'

function options(overrides = {}) {
  return {
    kernelUrl: 'wss://home.example/kernel',
    relayUrl: null,
    relayToken: null,
    localAuthToken: null,
    homeDaemonId: null,
    homeDaemonAlias: null,
    workerMachineId: 'machine-path1',
    workerKernelId: 'kernel-path1',
    managedEnvironmentId: 'environment-path1',
    projectId: 'project-selected',
    workspaceId: 'workspace-selected',
    worktreeId: 'worktree-selected',
    targetPlatform: 'linux-x86_64',
    provider: DEFAULT_PROVIDER,
    accountProfile: DEFAULT_ACCOUNT_PROFILE,
    model: DEFAULT_MODEL,
    effort: DEFAULT_EFFORT,
    prompt: 'Reply with exactly OK.',
    validationCommands: ['rustc --version'],
    timeoutMs: 900_000,
    pollMs: 1_000,
    artifactRoot: '/tmp/project-environment-setup-acceptance',
    reportPath: null,
    allowTerminalBeforeCancel: false,
    help: false,
    ...overrides,
  }
}

function workerFixture() {
  return {
    machine: {
      machine_id: 'machine-path1',
      trust_status: 'approved',
      online: true,
      pending: false,
    },
    kernel: {
      kernel_id: 'kernel-path1',
      machine_id: 'machine-path1',
      accepting_remote_leases: true,
      available_providers: ['codex'],
      provider_accounts: [{
        provider: 'codex',
        account_id: 'codex-1',
        alias: 'codex-1',
        state: 'authenticated',
      }],
    },
  }
}

test('strict inputs preserve selected identities and use Luna max defaults', () => {
  const parsed = parseProjectEnvironmentSetupArguments([
    '--kernel-url', 'wss://home.example/kernel',
    '--worker-machine-id', 'machine-path1',
    '--worker-kernel-id', 'kernel-path1',
    '--managed-environment-id', 'environment-path1',
    '--project-id', 'project-selected',
    '--workspace-id', 'workspace-selected',
    '--worktree-id', 'worktree-selected',
    '--target-platform', 'linux-x86_64',
    '--validation-command', 'rustc --version',
  ], {})
  validateProjectEnvironmentSetupOptions(parsed)
  assert.equal(parsed.provider, DEFAULT_PROVIDER)
  assert.equal(parsed.accountProfile, DEFAULT_ACCOUNT_PROFILE)
  assert.equal(parsed.model, DEFAULT_MODEL)
  assert.equal(parsed.effort, DEFAULT_EFFORT)
  assert.deepEqual(parsed.validationCommands, ['rustc --version'])
  assert.throws(
    () => validateProjectEnvironmentSetupOptions({ ...parsed, workerKernelId: null }),
    /--worker-kernel-id is required/,
  )
})

test('setup request plan calls the official builders without supplying a definition', () => {
  const calls = []
  const requests = {
    startProjectEnvironmentSetupRequest: (input) => {
      calls.push(['start', input])
      return { StartProjectEnvironmentSetup: input }
    },
    getProjectEnvironmentSetupStatusRequest: (operationId) => ({
      GetProjectEnvironmentSetupStatus: { operationId },
    }),
    cancelProjectEnvironmentSetupRequest: (operationId, sessionId) => ({
      CancelProjectEnvironmentSetup: { operationId, sessionId },
    }),
    retryProjectEnvironmentSetupRequest: (operationId, sessionId) => ({
      RetryProjectEnvironmentSetup: { operationId, sessionId },
    }),
  }
  const input = buildProjectEnvironmentSetupRequestInput(
    options(),
    'operation-1',
    'agent-1',
    'session-1',
  )
  const plan = buildProjectEnvironmentSetupRequests(requests, input)
  assert.equal(calls.length, 1)
  assert.equal('definition' in plan.start.StartProjectEnvironmentSetup, false)
  assert.deepEqual(plan.start.StartProjectEnvironmentSetup.validationCommands, ['rustc --version'])
  assert.deepEqual(plan.get, {
    GetProjectEnvironmentSetupStatus: { operationId: 'operation-1' },
  })
  assert.deepEqual(plan.cancel, {
    CancelProjectEnvironmentSetup: { operationId: 'operation-1', sessionId: 'session-1' },
  })
  assert.deepEqual(plan.retry, {
    RetryProjectEnvironmentSetup: { operationId: 'operation-1', sessionId: 'session-1' },
  })
})

test('worker and managed environment checks require the exact approved Path1 target and revision', () => {
  const input = options()
  const worker = selectApprovedPath1Worker(
    [workerFixture().machine],
    [workerFixture().kernel],
    input,
  )
  const environment = validateManagedPath1Environment({
    environmentId: 'environment-path1',
    desiredState: 'running',
    observedState: 'ready',
    runtimeMachineId: 'machine-path1',
    runtimeKernelId: 'kernel-path1',
    runtimeReleaseDigest: 'sha256:path1-release',
  }, input, worker)
  assert.equal(environment.runtimeReleaseDigest, 'sha256:path1-release')
  assert.throws(
    () => validateManagedPath1Environment({
      environmentId: 'environment-path1',
      desiredState: 'running',
      observedState: 'ready',
      runtimeMachineId: 'machine-path1',
      runtimeKernelId: 'kernel-other',
      runtimeReleaseDigest: 'sha256:path1-release',
    }, input, worker),
    /does not match kernel-path1/,
  )
  assert.throws(
    () => selectApprovedPath1Worker(
      [workerFixture().machine],
      [{ ...workerFixture().kernel, provider_accounts: [] }],
      input,
    ),
    /does not advertise usable codex\/codex-1 authentication/,
  )
})

test('selected project and created session cannot substitute a project, worktree, or worker', () => {
  const input = options()
  const worker = workerFixture()
  assert.equal(validateSelectedProject({
    id: 'project-selected',
    status: 'active',
    workspace_id: 'workspace-selected',
    workspace_ids: ['workspace-selected'],
    environment_definition: null,
  }, input).id, 'project-selected')
  assert.throws(
    () => validateSelectedProject({
      id: 'project-selected',
      status: 'active',
      workspace_id: 'workspace-selected',
      environment_definition: { origin: 'user_authored' },
    }, input),
    /already has an environment definition/,
  )
  const remote = validateCreatedRemoteSession({
    id: 'session-1',
    project_id: 'project-selected',
    workspace_id: 'workspace-selected',
    worktree_id: 'worktree-selected',
  }, {
    id: 'agent-1',
    session_id: 'session-1',
    worktree_id: 'worktree-selected',
    remote_execution: {
      leased_agent_id: 'lease-1',
      worker_machine_id: 'machine-path1',
      worker_kernel_id: 'kernel-path1',
      relay_peer_protocol_version: 51,
    },
  }, input, worker)
  assert.equal(remote.worker_kernel_id, 'kernel-path1')
  assert.throws(
    () => validateCreatedRemoteSession({
      id: 'session-1',
      project_id: 'project-selected',
      workspace_id: 'workspace-selected',
      worktree_id: 'worktree-selected',
    }, {
      id: 'agent-1',
      session_id: 'session-1',
      worktree_id: 'worktree-selected',
      remote_execution: {
        leased_agent_id: 'lease-1',
        worker_machine_id: 'machine-path1',
        worker_kernel_id: 'kernel-path1',
      },
    }, input, worker),
    /relay peer protocol/,
  )
  assert.throws(
    () => validateCreatedRemoteSession({
      id: 'session-1',
      project_id: 'project-selected',
      workspace_id: 'workspace-selected',
      worktree_id: 'other-worktree',
    }, null, input, worker),
    /selected workspace\/worktree/,
  )
})

test('Ready requires nonzero all-passing worker validation and utility-generated install steps', () => {
  const input = options()
  const status = {
    operation_id: 'operation-1',
    project_id: 'project-selected',
    session_id: 'session-1',
    agent_id: 'agent-1',
    worker_id: 'machine-path1',
    platform: 'linux-x86_64',
    phase: 'ready',
    attempt: 2,
    definition_digest: 'sha256:definition',
    validation: {
      worker_id: 'machine-path1',
      platform: 'linux-x86_64',
      commands: [{ command_digest: 'sha256:command', exit_code: 0, stdout_bytes: 12, stderr_bytes: 0 }],
    },
  }
  const validation = validateReadyProjectEnvironmentSetup(status, input, 'operation-1', 'session-1', 'agent-1')
  assert.deepEqual(validation, { commandCount: 1, passedCount: 1, failedCount: 0, outputBytes: 12 })
  const definition = validateUtilityGeneratedProjectDefinition({
    environment_definition: {
      origin: 'utility_generated',
      source: 'commands',
      target_platform: 'linux-x86_64',
      setup_steps: [{ kind: 'compiler', command: 'rustc --version' }],
      validation_commands: ['rustc --version'],
    },
  }, input, validation)
  assert.equal(definition.setupStepCount, 1)
  assert.equal(definition.validationCommandCount, 1)
  assert.throws(
    () => validateReadyProjectEnvironmentSetup({
      ...status,
      validation: { ...status.validation, commands: [{ ...status.validation.commands[0], exit_code: 1 }] },
    }, input, 'operation-1', 'session-1', 'agent-1'),
    /all-passing validation/,
  )
  assert.throws(
    () => validateUtilityGeneratedProjectDefinition({
      environment_definition: {
        origin: 'user_authored',
        target_platform: 'linux-x86_64',
        setup_steps: [{ kind: 'compiler', command: 'rustc --version' }],
        validation_commands: ['rustc --version'],
      },
    }, input, validation),
    /utility-generated/,
  )
})

test('provider launch and SubmitPrompt are ordered after authoritative Ready', () => {
  const log = [
    { sequence: 1, variant: 'StartProjectEnvironmentSetup' },
    { sequence: 2, variant: 'GetProjectEnvironmentSetupStatus' },
    { sequence: 3, variant: 'CancelProjectEnvironmentSetup' },
    { sequence: 4, variant: 'RetryProjectEnvironmentSetup' },
    { sequence: 5, variant: 'GetProjectEnvironmentSetupStatus' },
    { sequence: 6, variant: 'LaunchProviderRun' },
    { sequence: 7, variant: 'GetProviderRun' },
    { sequence: 8, variant: 'SubmitPrompt' },
  ]
  assert.deepEqual(assertProviderLaunchAfterReady(log, 5), {
    launchSequence: 6,
    submitSequence: 8,
  })
  assert.throws(
    () => assertProviderLaunchAfterReady(log.map((entry) => (
      entry.variant === 'LaunchProviderRun' ? { ...entry, sequence: 4 } : entry
    )), 5),
    /before authoritative setup Ready/,
  )
})

test('protocol guard blocks old kernels and fails closed for unrelated probe errors', async () => {
  assert.equal(PROJECT_ENVIRONMENT_SETUP_MINIMUM_PROTOCOL, 326)
  assert.equal(isUnknownRequestVariant(new Error('unknown variant `GetProjectEnvironmentSetupStatus`'), 'GetProjectEnvironmentSetupStatus'), true)
  assert.deepEqual(
    classifyProjectEnvironmentSetupCapabilityProbeError(new Error('unknown variant `GetProjectEnvironmentSetupStatus`')).status,
    'blocked',
  )
  assert.deepEqual(
    classifyProjectEnvironmentSetupCapabilityProbeError(new Error('setup operation was not found')).status,
    'verified',
  )
  assert.deepEqual(
    classifyProjectEnvironmentSetupCapabilityProbeError(new Error('permission denied')).status,
    'unverified',
  )
  await assert.rejects(
    () => withProjectEnvironmentSetupProtocolMinimum(
      async () => { throw new Error('unknown variant `GetProjectEnvironmentSetupStatus`') },
      { capability: 'Project environment setup', requestVariant: 'GetProjectEnvironmentSetupStatus', minimumProtocolVersion: 326 },
    ),
    /requires kernel protocol 326 or newer/,
  )
})
