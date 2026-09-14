import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

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
  runProjectEnvironmentSetupAcceptance,
  selectApprovedPath1Worker,
  validateCreatedRemoteSession,
  validateManagedPath1Environment,
  validateProjectEnvironmentSetupOptions,
  validateReadyProjectEnvironmentSetup,
  validateSelectedProject,
  validateUtilityGeneratedProjectDefinition,
  withProjectEnvironmentSetupProtocolMinimum,
} from './live-project-environment-setup-acceptance-drill.mjs'

// These tests exercise orchestration only. The injected client exposes the same
// public request/response and event shapes, but never starts a provider or VM;
// their reports are not live-acceptance evidence.

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

function request(variant, payload) {
  return { [variant]: payload }
}

function setupStatus(options, phase, attempt, overrides = {}) {
  return {
    operation_id: 'operation-live-test',
    project_id: options.projectId,
    session_id: 'session-live-test',
    agent_id: 'agent-live-test',
    worker_id: options.workerMachineId,
    platform: options.targetPlatform,
    phase,
    attempt,
    progress_percent: phase === 'ready' ? 100 : 45,
    definition_digest: phase === 'ready' ? 'sha256:definition-live-test' : null,
    validation: phase === 'ready'
      ? {
          worker_id: options.workerMachineId,
          platform: options.targetPlatform,
          commands: [{
            command_digest: 'sha256:validation-command',
            exit_code: 0,
            stdout_bytes: 12,
            stderr_bytes: 0,
          }],
        }
      : null,
    failure_code: null,
    retryable: phase === 'cancelled',
    ...overrides,
  }
}

function makeOrchestrationModules(
  options,
  {
    failEndSession = false,
    statusSequence = [
      ['requested', 1],
      ['preparing', 1],
      ['preparing', 2],
      ['ready', 2],
    ],
  } = {},
) {
  let clientInstance = null
  const calls = []
  const statuses = [...statusSequence]
  let operationId = 'operation-live-test'
  let projectListCount = 0
  const providerRun = {
    id: 'provider-run-live-test',
    session_id: 'session-live-test',
    agent_instance_id: 'agent-live-test',
    provider: options.provider,
    account_profile: options.accountProfile,
    model: options.model,
    variant: options.effort,
    state: 'Running',
  }
  const session = {
    id: 'session-live-test',
    project_id: options.projectId,
    workspace_id: options.workspaceId,
    worktree_id: options.worktreeId,
  }
  const agent = {
    id: 'agent-live-test',
    session_id: session.id,
    worktree_id: options.worktreeId,
    remote_execution: {
      leased_agent_id: 'lease-live-test',
      execution_lease_id: 'execution-lease-live-test',
      worker_machine_id: options.workerMachineId,
      worker_kernel_id: options.workerKernelId,
      relay_peer_protocol_version: 51,
    },
  }
  const project = {
    id: options.projectId,
    status: 'active',
    workspace_id: options.workspaceId,
    workspace_ids: [options.workspaceId],
    environment_definition: {
      origin: 'utility_generated',
      source: 'commands',
      target_platform: options.targetPlatform,
      setup_steps: [{ kind: 'compiler', command: 'rustc --version' }],
      validation_commands: [...options.validationCommands],
    },
  }
  const requests = {
    listRemoteMachinesRequest: () => request('ListRemoteMachines', null),
    listRemoteMachineKernelsRequest: (machineRef) => request('ListRemoteMachineKernels', { machine_ref: machineRef }),
    getManagedEnvironmentRequest: (environmentId) => request('GetManagedEnvironment', { environmentId }),
    listProjectsRequest: (includeArchived = false) => request('ListProjects', { include_archived: includeArchived }),
    createSessionRequest: (workspaceId, worktreeId, alias, defaults, sliceRef, liveSync, kernelRef, placement, projectSelection) => request('CreateSession', {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      alias,
      agent_defaults: defaults,
      slice_ref: sliceRef,
      workspace_live_sync_mode: liveSync,
      kernel_ref: kernelRef,
      worktree_placement: placement,
      project_selection: projectSelection,
    }),
    attachToSessionRequest: (sessionId, clientId) => request('AttachToSession', {
      session_id: sessionId,
      client_id: clientId,
      capability_level: 'FullTerminal',
    }),
    startProjectEnvironmentSetupRequest: (input) => request('StartProjectEnvironmentSetup', {
      operationId: input.operationId,
      projectId: input.projectId,
      sessionId: input.sessionId,
      agentId: input.agentId,
      targetWorkerId: input.targetWorkerId,
      targetPlatform: input.targetPlatform,
      ...(input.definition ? { definition: input.definition } : {}),
      ...(input.validationCommands.length > 0 ? { validationCommands: [...input.validationCommands] } : {}),
    }),
    getProjectEnvironmentSetupStatusRequest: (operationId) => request('GetProjectEnvironmentSetupStatus', { operationId }),
    cancelProjectEnvironmentSetupRequest: (operationId, sessionId) => request('CancelProjectEnvironmentSetup', { operationId, sessionId }),
    retryProjectEnvironmentSetupRequest: (operationId, sessionId) => request('RetryProjectEnvironmentSetup', { operationId, sessionId }),
    getSessionStateRequest: (sessionId) => request('GetSessionState', { session_id: sessionId }),
    launchProviderRunRequest: (sessionId, provider, accountProfile, model, effort, agentId) => request('LaunchProviderRun', {
      session_id: sessionId,
      agent_id: agentId,
      adapter_key: provider,
      provider,
      account_profile: accountProfile,
      model,
      variant: effort,
      structured_endpoint: null,
      provider_session_id: null,
      native_tui: false,
    }),
    getProviderRunRequest: (providerRunId) => request('GetProviderRun', { provider_run_id: providerRunId }),
    submitPromptRequest: (sessionId, attachmentId, targetAgentId, prompt, attachments) => request('SubmitPrompt', {
      session_id: sessionId,
      attachment_id: attachmentId,
      target_agent_id: targetAgentId,
      prompt,
      attachments,
    }),
    detachFromSessionRequest: (attachmentId) => request('DetachFromSession', { attachment_id: attachmentId }),
    endSessionRequest: (sessionId) => request('EndSession', { session_id: sessionId }),
  }

  class InjectedPublicKernelClient {
    constructor() {
      clientInstance = this
      this.handlers = new Set()
      this.events = []
      this.subscribeCount = 0
      this.unsubscribeCount = 0
    }

    supportsKernelEvents() {
      return true
    }

    onKernelEvent(handler) {
      this.handlers.add(handler)
      return () => this.handlers.delete(handler)
    }

    async subscribeToKernelEvents() {
      this.subscribeCount += 1
    }

    async unsubscribeFromKernelEvents() {
      this.unsubscribeCount += 1
    }

    async send(message) {
      const variant = Object.keys(message)[0]
      calls.push({ variant, message })
      const payload = message[variant]
      if (variant === 'GetProjectEnvironmentSetupStatus') {
        if (payload.operationId.startsWith('setup-capability-probe-')) {
          throw new Error('setup operation was not found')
        }
        const [phase, attempt] = statuses.shift() || []
        if (!phase) throw new Error('test setup status queue exhausted')
        return {
          ProjectEnvironmentSetupStatus: {
            status: setupStatus(options, phase, attempt, { operation_id: operationId }),
          },
        }
      }
      if (variant === 'ListRemoteMachines') {
        return { RemoteMachinesListed: { machines: [workerFixture().machine] } }
      }
      if (variant === 'ListRemoteMachineKernels') {
        return { RemoteMachineKernelsListed: { machine_ref: options.workerMachineId, kernels: [workerFixture().kernel] } }
      }
      if (variant === 'GetManagedEnvironment') {
        return { ManagedEnvironment: { environment: {
          environmentId: options.managedEnvironmentId,
          desiredState: 'running',
          observedState: 'ready',
          runtimeMachineId: options.workerMachineId,
          runtimeKernelId: options.workerKernelId,
          runtimeReleaseDigest: 'sha256:path1-runtime-live-test',
        } } }
      }
      if (variant === 'ListProjects') {
        projectListCount += 1
        return { ProjectsListed: { projects: [{
          ...project,
          environment_definition: projectListCount === 1 ? null : project.environment_definition,
        }] } }
      }
      if (variant === 'CreateSession') return { SessionCreated: { session, agent } }
      if (variant === 'AttachToSession') return { SessionAttached: { attachment: { id: 'attachment-live-test' } } }
      if (variant === 'StartProjectEnvironmentSetup') {
        operationId = payload.operationId
        return {
          ProjectEnvironmentSetupStarted: {
            status: setupStatus(options, 'requested', 1, { operation_id: operationId }),
          },
        }
      }
      if (variant === 'CancelProjectEnvironmentSetup') {
        return {
          ProjectEnvironmentSetupCancelled: {
            status: setupStatus(options, 'cancelled', 1, { operation_id: operationId }),
          },
        }
      }
      if (variant === 'RetryProjectEnvironmentSetup') {
        return {
          ProjectEnvironmentSetupRetried: {
            status: setupStatus(options, 'requested', 2, { operation_id: operationId }),
          },
        }
      }
      if (variant === 'GetSessionState') return { SessionState: { session: { ...session, agents: [agent] } } }
      if (variant === 'LaunchProviderRun') return { ProviderRunLaunched: { provider_run: providerRun } }
      if (variant === 'GetProviderRun') return { ProviderRun: { provider_run: providerRun } }
      if (variant === 'SubmitPrompt') {
        const event = {
          event: 'assistant_message_completed',
          session_id: session.id,
          provider_run_id: providerRun.id,
          agent_id: agent.id,
          message_id: 'message-live-test',
          completed_at_ms: Date.now(),
        }
        this.events.push(event)
        for (const handler of this.handlers) handler(event)
        return { PromptSubmitted: { outcome: { Started: { prompt: { id: 'prompt-live-test' } } } } }
      }
      if (variant === 'DetachFromSession') return { SessionDetached: { attachment: { id: payload.attachment_id } } }
      if (variant === 'EndSession') {
        if (failEndSession) throw new Error('synthetic cleanup failure')
        return { SessionEnded: { session } }
      }
      throw new Error(`unhandled injected public request ${variant}`)
    }

    async close() {}
  }

  return {
    modules: { LocalIpcClient: InjectedPublicKernelClient, requests },
    inspect: () => ({ calls, client: clientInstance }),
  }
}

async function withOrchestrationRun(run) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'chariox-project-environment-setup-test-'))
  const runOptions = options({ artifactRoot: root, reportPath: path.join(root, 'report.json'), timeoutMs: 10_000, pollMs: 100 })
  try {
    return await run(runOptions)
  } finally {
    await rm(root, { recursive: true, force: true })
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
  assert.equal(DEFAULT_MODEL, 'gpt-5.6-luna')
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

test('injected orchestration polls requested, exercises cancel/retry, and launches only after Ready', async () => {
  let inspect = null
  const report = await withOrchestrationRun(async (runOptions) => {
    const fixture = makeOrchestrationModules(runOptions)
    inspect = fixture.inspect
    return runProjectEnvironmentSetupAcceptance(runOptions, fixture.modules)
  })
  const { calls, client } = inspect()
  const variants = calls.map((entry) => entry.variant)
  const startIndex = variants.indexOf('StartProjectEnvironmentSetup')
  const cancelIndex = variants.indexOf('CancelProjectEnvironmentSetup')
  const retryIndex = variants.indexOf('RetryProjectEnvironmentSetup')
  const launchIndex = variants.indexOf('LaunchProviderRun')
  const submitIndex = variants.indexOf('SubmitPrompt')
  const setupGets = calls.filter((entry) => entry.variant === 'GetProjectEnvironmentSetupStatus')
  const create = calls.find((entry) => entry.variant === 'CreateSession')
  const launch = calls[launchIndex].message.LaunchProviderRun
  const start = calls[startIndex].message.StartProjectEnvironmentSetup
  const cancel = calls[cancelIndex].message.CancelProjectEnvironmentSetup
  const retry = calls[retryIndex].message.RetryProjectEnvironmentSetup
  const submit = calls[submitIndex].message.SubmitPrompt

  assert.equal(report.status, 'passed')
  assert.equal(report.liveAcceptance, false)
  assert.equal(report.executionMode, 'injected-orchestration-test')
  assert.deepEqual(report.setup.phases, ['requested', 'preparing', 'cancelled', 'ready'])
  assert.deepEqual(report.setup.cancellation, { status: 'exercised', phase: 'cancelled', attempt: 1 })
  assert.deepEqual(report.setup.retry, { status: 'exercised', phase: 'requested', attempt: 2 })
  assert.equal(report.setup.validation.commandCount, 1)
  assert.equal(report.setup.validation.passedCount, 1)
  assert.equal(report.setup.definition.origin, 'utility_generated')
  assert.equal(report.worker.machineId, 'machine-path1')
  assert.equal(report.worker.kernelId, 'kernel-path1')
  assert.equal(report.worker.runtimeReleaseDigest, 'sha256:path1-runtime-live-test')
  assert.equal(report.options.provider, 'codex')
  assert.equal(report.options.accountProfile, 'codex-1')
  assert.equal(report.options.model, 'gpt-5.6-luna')
  assert.equal(report.options.effort, 'max')
  assert.equal(report.options.promptSubmitted, true)
  assert.equal(report.provider.completionCount, 1)
  assert.equal(report.events.length, 1)
  assert.equal(report.events[0].event, 'assistant_message_completed')
  assert.equal(report.events[0].sessionId, 'session-live-test')
  assert.equal(report.events[0].agentId, 'agent-live-test')
  assert.equal(report.events[0].providerRunId, 'provider-run-live-test')
  assert.equal(setupGets.length, 5)
  assert.equal(create.message.CreateSession.kernel_ref, 'kernel-path1')
  assert.deepEqual(create.message.CreateSession.project_selection, {
    kind: 'existing',
    project_id: 'project-selected',
  })
  assert.equal('definition' in start, false)
  assert.deepEqual(start.validationCommands, ['rustc --version'])
  assert.equal(start.targetWorkerId, 'machine-path1')
  assert.equal(start.targetPlatform, 'linux-x86_64')
  assert.equal(cancel.operationId, start.operationId)
  assert.equal(cancel.sessionId, 'session-live-test')
  assert.equal(retry.operationId, start.operationId)
  assert.equal(retry.sessionId, 'session-live-test')
  assert.equal(launch.session_id, 'session-live-test')
  assert.equal(launch.agent_id, 'agent-live-test')
  assert.equal(launch.account_profile, 'codex-1')
  assert.equal(launch.model, 'gpt-5.6-luna')
  assert.equal(launch.variant, 'max')
  assert.equal(submit.session_id, 'session-live-test')
  assert.equal(submit.attachment_id, 'attachment-live-test')
  assert.equal(submit.target_agent_id, 'agent-live-test')
  assert.equal(submit.prompt, 'Reply with exactly OK.')
  assert.deepEqual(submit.attachments, [])
  assert.ok(startIndex < cancelIndex)
  assert.ok(cancelIndex < retryIndex)
  assert.ok(retryIndex < launchIndex)
  assert.ok(launchIndex < submitIndex)
  assert.equal(client.subscribeCount, 1)
  assert.equal(client.unsubscribeCount, 1)
  assert.equal(client.handlers.size, 0)
})

test('cleanup failure makes the injected orchestration report failed and non-live', async () => {
  await withOrchestrationRun(async (runOptions) => {
    const fixture = makeOrchestrationModules(runOptions, {
      failEndSession: true,
      statusSequence: [
        ['preparing', 1],
        ['preparing', 2],
        ['ready', 2],
      ],
    })
    await assert.rejects(
      () => runProjectEnvironmentSetupAcceptance(runOptions, fixture.modules),
      /synthetic cleanup failure; report=/,
    )
    const report = JSON.parse(await readFile(runOptions.reportPath, 'utf8'))
    const { client } = fixture.inspect()
    assert.equal(report.status, 'failed')
    assert.equal(report.liveAcceptance, false)
    assert.equal(report.executionMode, 'injected-orchestration-test')
    assert.match(report.failure, /cleanup failed: session end: synthetic cleanup failure/)
    assert.deepEqual(report.cleanup.errors, ['session end: synthetic cleanup failure'])
    assert.equal(report.cleanup.unsubscribed, true)
    assert.equal(report.cleanup.detached, true)
    assert.equal(report.cleanup.endedSession, false)
    assert.equal(client.subscribeCount, 1)
    assert.equal(client.unsubscribeCount, 1)
    assert.equal(client.handlers.size, 0)
  })
})
