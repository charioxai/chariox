import assert from 'node:assert/strict'
import test from 'node:test'
import { cleanupProjectEnvironmentSetupDrillContext as cleanupWithRequestBuilders } from './project-environment-setup-drill-cleanup.mjs'

const targetPin = Object.freeze({
  project_id: 'project-drill',
  project_owner_user_id: 'owner-drill',
  session_id: 'session-drill',
  workspace_id: '/external/workspace',
  worktree_id: '/external/worktree',
  target_machine_id: 'machine-drill',
  target_kernel_id: 'kernel-drill',
  target_platform: 'linux-x86_64',
  utility_agent_id: 'agent-utility',
  utility_provider: 'codex',
  utility_account_profile: 'utility-profile',
  utility_provider_run_id: 'utility-run',
  launch_agent_id: 'agent-launch',
  launch_provider: 'opencode',
  launch_account_profile: 'launch-profile',
})

const expectedOwnership = Object.freeze({
  machineId: targetPin.target_machine_id,
  kernelId: targetPin.target_kernel_id,
  projectId: targetPin.project_id,
  sessionId: targetPin.session_id,
  workspaceId: targetPin.workspace_id,
  worktreeId: targetPin.worktree_id,
  workerLeases: [
    { agentId: targetPin.utility_agent_id, executionLeaseId: 'lease-utility', leasedAgentId: 'leased-utility' },
    { agentId: targetPin.launch_agent_id, executionLeaseId: 'lease-launch', leasedAgentId: 'leased-launch' },
  ],
})

const requestBuilders = {
  cancelProjectEnvironmentSetupRequest: (operationId, sessionId) => ({ CancelProjectEnvironmentSetup: { operationId, sessionId } }),
  deleteProjectRequest: (projectId) => ({ DeleteProject: { project_id: projectId } }),
  deleteSessionRequest: (sessionRef, workspaceId) => ({ DeleteSession: { session_ref: sessionRef, workspace_id: workspaceId } }),
  destroyAgentRequest: (sessionId, agentId) => ({ DestroyAgent: { session_id: sessionId, agent_id: agentId } }),
  getProjectEnvironmentSetupStatusRequest: (operationId) => ({ GetProjectEnvironmentSetupStatus: { operationId } }),
  getSessionStateRequest: (sessionId) => ({ GetSessionState: { session_id: sessionId } }),
  listProjectsRequest: (includeArchived = false) => ({ ListProjects: { include_archived: includeArchived } }),
  listRemoteMachineKernelsRequest: (machineRef) => ({ ListRemoteMachineKernels: { machine_ref: machineRef } }),
  listRemoteMachinesRequest: () => ({ ListRemoteMachines: null }),
  listSessionsRequest: () => ({ ListSessions: null }),
}

function cleanupProjectEnvironmentSetupDrillContext(options) {
  return cleanupWithRequestBuilders({ ...options, requestBuilders })
}

function setupStatus(phase = 'ready', overrides = {}) {
  return {
    operation_id: 'setup-operation-1',
    project_id: targetPin.project_id,
    session_id: targetPin.session_id,
    agent_id: targetPin.utility_agent_id,
    worker_id: targetPin.target_machine_id,
    platform: targetPin.target_platform,
    phase,
    attempt: 1,
    progress_percent: ['ready', 'failed', 'cancelled'].includes(phase) ? 100 : 20,
    created_at_ms: 1_000,
    updated_at_ms: 1_000,
    ...overrides,
  }
}

function cleanupEvidence() {
  return {
    complete: false,
    operationsSettled: false,
    blockedDeletion: false,
    destroyedAgentIds: [],
    deletedSessionId: null,
    deletedProjectId: null,
    errors: [],
  }
}

function fakeHome(options = {}) {
  const calls = []
  const destroyed = new Set()
  let ownershipSnapshots = 0
  let projectSessionReadsAfterDelete = 0
  let statusReads = 0
  let sessionDeleted = false
  const project = {
    id: targetPin.project_id,
    owner_user_id: targetPin.project_owner_user_id,
    status: 'active',
    workspace_id: targetPin.workspace_id,
    environment_definition: { origin: 'utility_generated' },
    ...(options.projectOverrides ?? {}),
  }
  const defaultAgents = [
    {
      id: targetPin.utility_agent_id,
      provider: targetPin.utility_provider,
      account_profile: targetPin.utility_account_profile,
      worktree_id: targetPin.worktree_id,
      remote_execution: {
        worker_machine_id: targetPin.target_machine_id,
        worker_kernel_id: targetPin.target_kernel_id,
        execution_lease_id: 'lease-utility',
        leased_agent_id: 'leased-utility',
        active_worker_provider_run_id: targetPin.utility_provider_run_id,
      },
    },
    {
      id: targetPin.launch_agent_id,
      provider: targetPin.launch_provider,
      account_profile: targetPin.launch_account_profile,
      worktree_id: targetPin.worktree_id,
      remote_execution: {
        worker_machine_id: targetPin.target_machine_id,
        worker_kernel_id: targetPin.target_kernel_id,
        execution_lease_id: 'lease-launch',
        leased_agent_id: 'leased-launch',
        active_worker_provider_run_id: 'official-provider-run',
      },
    },
  ]
  const agents = defaultAgents.map((agent) => {
    const override = options.agentOverrides?.[agent.id] ?? {}
    return {
      ...agent,
      ...override,
      remote_execution: { ...agent.remote_execution, ...(override.remote_execution ?? {}) },
    }
  })
  const session = {
    id: targetPin.session_id,
    project_id: targetPin.project_id,
    workspace_id: targetPin.workspace_id,
    worktree_id: targetPin.worktree_id,
    status: 'Active',
    agents,
    ...(options.sessionOverrides ?? {}),
  }

  const client = {
    async send(request) {
      const name = Object.keys(request)[0]
      calls.push(name)
      if (name === 'GetProjectEnvironmentSetupStatus') {
        const sequenceEntry = options.statusSequence?.[statusReads]
        statusReads += 1
        const phase = sequenceEntry?.phase ?? options.operationPhase ?? 'ready'
        const overrides = sequenceEntry?.overrides ?? options.statusOverrides ?? {}
        return { ProjectEnvironmentSetupStatus: { status: setupStatus(phase, overrides) } }
      }
      if (name === 'CancelProjectEnvironmentSetup' && options.cancelHangs) return await new Promise(() => {})
      if (name === 'CancelProjectEnvironmentSetup') return { ProjectEnvironmentSetupCancelled: { status: setupStatus('cancelled', options.cancelStatusOverrides ?? {}) } }
      if (name === 'ListRemoteMachines') return { RemoteMachinesListed: { machines: [{
        machine_id: targetPin.target_machine_id,
        trust_status: 'approved',
        online: true,
        ...(options.machineOverrides ?? {}),
      }] } }
      if (name === 'ListRemoteMachineKernels') {
        if (options.transportFailureAt === name) throw new Error('injected transport failure')
        return { RemoteMachineKernelsListed: { kernels: [{
          kernel_id: targetPin.target_kernel_id,
          machine_id: targetPin.target_machine_id,
          accepting_remote_leases: true,
          available_providers: ['codex', 'opencode'],
          ...(options.kernelOverrides ?? {}),
        }] } }
      }
      if (name === 'ListProjects') return { ProjectsListed: { projects: [project] } }
      if (name === 'ListSessions') {
        if (sessionDeleted) {
          projectSessionReadsAfterDelete += 1
          if (options.additionalProjectSessionAfterDelete && projectSessionReadsAfterDelete === 1) {
            return { SessionsListed: { sessions: [{ id: 'foreign-session', project_id: targetPin.project_id }] } }
          }
          return { SessionsListed: { sessions: [] } }
        }
        if (options.foreignProjectSessionAtSnapshot === ownershipSnapshots + 1) {
          return { SessionsListed: { sessions: [
            { id: targetPin.session_id, project_id: targetPin.project_id },
            { id: 'foreign-session', project_id: targetPin.project_id },
          ] } }
        }
        return { SessionsListed: { sessions: [{ id: targetPin.session_id, project_id: targetPin.project_id }] } }
      }
      if (name === 'GetSessionState') {
        ownershipSnapshots += 1
        if (options.transportFailureAt === name) throw new Error('injected transport failure')
        const liveAgents = session.agents.filter((agent) => !destroyed.has(agent.id))
        const withForeign = options.foreignAgentAtSnapshot === ownershipSnapshots
          ? [...liveAgents, {
              id: 'foreign-agent',
              provider: 'codex',
              account_profile: 'other-profile',
              worktree_id: '/foreign/worktree',
              remote_execution: { worker_machine_id: 'foreign-machine', worker_kernel_id: 'foreign-kernel' },
            }]
          : liveAgents
        return { SessionStateLoaded: { session: { ...session, agents: withForeign } } }
      }
      if (name === 'DestroyAgent') {
        const agentId = request.DestroyAgent.agent_id
        if (options.failDestroyAgent === agentId) return { Error: { code: 'injected_destroy_failure' } }
        destroyed.add(agentId)
        return { AgentDestroyed: { agent: {
          id: options.wrongAgentAckFor === agentId ? 'foreign-agent' : agentId,
          session_id: options.wrongAgentSessionAckFor === agentId ? 'foreign-session' : targetPin.session_id,
        } } }
      }
      if (name === 'DeleteSession') {
        sessionDeleted = true
        return { SessionDeleted: { session: {
          id: options.wrongSessionDeleted ? 'foreign-session' : targetPin.session_id,
          project_id: targetPin.project_id,
          workspace_id: targetPin.workspace_id,
          worktree_id: targetPin.worktree_id,
          agents: options.sessionDeletedAgents ?? [],
          ...(options.sessionDeletedOverrides ?? {}),
        } } }
      }
      if (name === 'DeleteProject') {
        if (options.transportFailureAt === name) throw new Error('injected transport failure')
        return { ProjectDeleted: {
          project: {
            id: options.wrongProjectDeleted ? 'foreign-project' : targetPin.project_id,
            owner_user_id: targetPin.project_owner_user_id,
            workspace_id: targetPin.workspace_id,
            ...(options.projectDeletedOverrides ?? {}),
          },
          sessions: options.projectDeletedSessions ?? [],
        } }
      }
      throw new Error(`unexpected request ${name}`)
    },
    async close() {
      calls.push('Close')
      if (options.closeHangs) return await new Promise(() => {})
    },
  }
  return { client, calls }
}

async function runCleanup(options = {}) {
  const home = fakeHome(options)
  const evidence = cleanupEvidence()
  let clock = 0
  const result = await cleanupProjectEnvironmentSetupDrillContext({
    createClient: async () => home.client,
    pin: targetPin,
    operationIds: ['setup-operation-1'],
    expectedOwnership,
    evidence,
    cleanupTimeoutMs: options.cleanupTimeoutMs ?? 60_000,
    controlTimeoutMs: options.controlTimeoutMs ?? 100,
    pollMs: options.pollMs ?? 500,
    nowMs: () => clock,
    delay: async (ms) => { clock += ms },
  })
  return { result, calls: home.calls }
}

test('unsettled setup and Cancel timeout block every destructive context request', async () => {
  const { result, calls } = await runCleanup({
    operationPhase: 'preparing', cancelHangs: true, cleanupTimeoutMs: 1_000, controlTimeoutMs: 5,
  })
  assert.equal(result.operationsSettled, false)
  assert.equal(result.blockedDeletion, true)
  assert.equal(result.complete, false)
  assert.ok(result.errors.some((entry) => entry.startsWith('operation_not_confirmed_terminal:')))
  assert.equal(calls.includes('DestroyAgent'), false)
  assert.equal(calls.includes('DeleteSession'), false)
  assert.equal(calls.includes('DeleteProject'), false)
})

test('terminal cleanup statuses must match every pinned operation binding', async (t) => {
  const mismatches = [
    ['operation_id', 'foreign-operation'],
    ['project_id', 'foreign-project'],
    ['session_id', 'foreign-session'],
    ['agent_id', 'foreign-agent'],
    ['worker_id', 'foreign-worker'],
    ['platform', 'macos-aarch64'],
  ]
  for (const [field, value] of mismatches) {
    await t.test(field, async () => {
      const { result, calls } = await runCleanup({ statusOverrides: { [field]: value } })
      assert.equal(result.operationsSettled, false)
      assert.equal(result.complete, false)
      assert.equal(calls.includes('CancelProjectEnvironmentSetup'), false)
      assert.equal(calls.includes('DestroyAgent'), false)
      assert.equal(calls.includes('DeleteSession'), false)
    })
  }
})

test('wrong status binding during cancellation polling remains incomplete', async () => {
  const { result, calls } = await runCleanup({
    statusSequence: [
      { phase: 'preparing' },
      { phase: 'cancelled', overrides: { project_id: 'foreign-project' } },
    ],
  })
  assert.equal(result.operationsSettled, false)
  assert.equal(calls.includes('CancelProjectEnvironmentSetup'), true)
  assert.equal(calls.includes('DestroyAgent'), false)
  assert.equal(calls.includes('DeleteSession'), false)
})

test('a wrong binding in the Cancel acknowledgment is not trusted as settlement', async () => {
  const { result, calls } = await runCleanup({
    operationPhase: 'preparing',
    cancelStatusOverrides: { session_id: 'foreign-session' },
  })
  assert.equal(result.operationsSettled, false)
  assert.equal(calls.includes('CancelProjectEnvironmentSetup'), true)
  assert.equal(calls.includes('DestroyAgent'), false)
  assert.equal(calls.includes('DeleteSession'), false)
})

test('wrong workspace ownership and a newly joined foreign agent refuse cleanup', async (t) => {
  await t.test('wrong workspace binding blocks before agent destruction', async () => {
    const { result, calls } = await runCleanup({ projectOverrides: { workspace_id: '/foreign/workspace' } })
    assert.equal(result.complete, false)
    assert.ok(result.ownershipChecks[0].failures.includes('project_workspace_binding_changed'))
    assert.equal(calls.includes('DestroyAgent'), false)
    assert.equal(calls.includes('DeleteSession'), false)
  })

  await t.test('foreign agent in fresh session snapshot blocks deletion', async () => {
    const { result, calls } = await runCleanup({ foreignAgentAtSnapshot: 1 })
    assert.equal(result.complete, false)
    assert.equal(result.ownershipChecks[0].verified, false)
    assert.ok(result.ownershipChecks[0].failures.includes('unowned_session_agent_present'))
    assert.equal(calls.includes('DestroyAgent'), false)
    assert.equal(calls.includes('DeleteSession'), false)
  })

  await t.test('foreign agent joining after the first fresh check blocks session deletion', async () => {
    const { result, calls } = await runCleanup({ foreignAgentAtSnapshot: 2 })
    assert.equal(result.complete, false)
    assert.equal(result.destroyedAgentIds.length, 2)
    assert.equal(result.ownershipChecks[0].verified, true)
    assert.equal(result.ownershipChecks[1].verified, false)
    assert.ok(result.ownershipChecks[1].failures.includes('unowned_session_agent_present'))
    assert.equal(calls.includes('DeleteSession'), false)
    assert.equal(calls.includes('DeleteProject'), false)
  })
})

test('changed Project owner and session bindings refuse cleanup', async (t) => {
  const cases = [
    {
      name: 'Project owner',
      options: { projectOverrides: { owner_user_id: 'foreign-owner' } },
      failure: 'project_owner_changed',
    },
    {
      name: 'session Project',
      options: { sessionOverrides: { project_id: 'foreign-project' } },
      failure: 'session_project_binding_changed',
    },
    {
      name: 'session workspace',
      options: { sessionOverrides: { workspace_id: '/foreign/workspace' } },
      failure: 'session_workspace_binding_changed',
    },
    {
      name: 'session worktree',
      options: { sessionOverrides: { worktree_id: '/foreign/worktree' } },
      failure: 'session_worktree_binding_changed',
    },
  ]
  for (const entry of cases) {
    await t.test(entry.name, async () => {
      const { result, calls } = await runCleanup(entry.options)
      assert.equal(result.complete, false)
      assert.ok(result.ownershipChecks[0].failures.includes(entry.failure))
      assert.equal(calls.includes('DestroyAgent'), false)
      assert.equal(calls.includes('DeleteSession'), false)
    })
  }
})

test('same-owner lease refresh is accepted for cleanup', async () => {
  const { result, calls } = await runCleanup({
    agentOverrides: {
      [targetPin.launch_agent_id]: {
        remote_execution: {
          execution_lease_id: 'lease-launch-refreshed',
          leased_agent_id: 'leased-launch-refreshed',
        },
      },
    },
  })
  assert.equal(result.ownershipChecks[0].verified, true)
  assert.equal(result.complete, true)
  assert.deepEqual(result.destroyedAgentIds, [targetPin.utility_agent_id, targetPin.launch_agent_id])
  assert.equal(calls.includes('DeleteSession'), true)
})

test('offline worker health does not block home-kernel cleanup', async () => {
  const { result, calls } = await runCleanup({
    machineOverrides: { online: false },
    kernelOverrides: { accepting_remote_leases: false, available_providers: [] },
  })
  assert.equal(result.ownershipChecks[0].verified, true)
  assert.equal(result.complete, true)
  assert.equal(calls.includes('DestroyAgent'), true)
  assert.equal(calls.includes('DeleteSession'), true)
  assert.equal(calls.includes('DeleteProject'), true)
})

test('changed remote owner bindings and provider profile still refuse cleanup', async (t) => {
  const cases = [
    {
      name: 'worker machine',
      override: { remote_execution: { worker_machine_id: 'foreign-machine' } },
    },
    {
      name: 'worker kernel',
      override: { remote_execution: { worker_kernel_id: 'foreign-kernel' } },
    },
    { name: 'provider', override: { provider: 'claude' } },
    { name: 'provider profile', override: { account_profile: 'foreign-profile' } },
    { name: 'worktree', override: { worktree_id: '/foreign/worktree' } },
  ]
  for (const entry of cases) {
    await t.test(entry.name, async () => {
      const { result, calls } = await runCleanup({
        agentOverrides: { [targetPin.launch_agent_id]: entry.override },
      })
      assert.equal(result.complete, false)
      assert.ok(result.ownershipChecks[0].failures.includes('pinned_launch_agent_binding_changed'))
      assert.equal(calls.includes('DestroyAgent'), false)
      assert.equal(calls.includes('DeleteSession'), false)
    })
  }
})

test('both exact DestroyAgent acknowledgments are required before DeleteSession', async () => {
  const { result, calls } = await runCleanup({ failDestroyAgent: targetPin.launch_agent_id })
  assert.equal(result.complete, false)
  assert.deepEqual(result.destroyedAgentIds, [targetPin.utility_agent_id])
  assert.equal(calls.includes('DeleteSession'), false)
  assert.equal(calls.includes('DeleteProject'), false)
  assert.ok(result.errors.includes(`agent_destroy_failed:${targetPin.launch_agent_id}`))
})

test('deletion acknowledgments must identify the pinned agent, session, and Project', async (t) => {
  await t.test('DestroyAgent identifies the requested agent and session', async () => {
    const { result, calls } = await runCleanup({ wrongAgentAckFor: targetPin.utility_agent_id })
    assert.equal(result.complete, false)
    assert.equal(result.deletedSessionId, null)
    assert.equal(calls.includes('DeleteSession'), false)
    assert.ok(result.errors.includes(`agent_destroy_failed:${targetPin.utility_agent_id}`))
  })

  await t.test('DestroyAgent rejects a mismatched session identity', async () => {
    const { result, calls } = await runCleanup({ wrongAgentSessionAckFor: targetPin.utility_agent_id })
    assert.equal(result.complete, false)
    assert.equal(calls.includes('DeleteSession'), false)
    assert.ok(result.errors.includes(`agent_destroy_failed:${targetPin.utility_agent_id}`))
  })

  await t.test('SessionDeleted identifies the pinned context', async () => {
    const { result, calls } = await runCleanup({ wrongSessionDeleted: true })
    assert.equal(result.complete, false)
    assert.equal(result.deletedSessionId, null)
    assert.equal(calls.includes('DeleteProject'), false)
    assert.ok(result.errors.includes(`session_delete_failed:${targetPin.session_id}`))
  })

  await t.test('SessionDeleted rejects a mismatched workspace binding', async () => {
    const { result, calls } = await runCleanup({ sessionDeletedOverrides: { workspace_id: '/foreign/workspace' } })
    assert.equal(result.complete, false)
    assert.equal(result.deletedSessionId, null)
    assert.equal(calls.includes('DeleteProject'), false)
    assert.ok(result.errors.includes(`session_delete_failed:${targetPin.session_id}`))
  })

  await t.test('SessionDeleted rejects an unpinned agent exposed by the serialized session', async () => {
    const { result } = await runCleanup({ sessionDeletedAgents: [{ id: 'foreign-agent', session_id: targetPin.session_id }] })
    assert.equal(result.complete, false)
    assert.equal(result.deletedSessionId, null)
  })

  await t.test('ProjectDeleted identifies the pinned owner and workspace', async () => {
    const { result, calls } = await runCleanup({ wrongProjectDeleted: true })
    assert.equal(result.complete, false)
    assert.equal(result.deletedSessionId, targetPin.session_id)
    assert.equal(result.deletedProjectId, null)
    assert.equal(calls.includes('DeleteProject'), true)
    assert.ok(result.errors.includes(`project_delete_failed:${targetPin.project_id}`))
  })

  await t.test('ProjectDeleted rejects a changed owner', async () => {
    const { result } = await runCleanup({ projectDeletedOverrides: { owner_user_id: 'foreign-owner' } })
    assert.equal(result.complete, false)
    assert.equal(result.deletedProjectId, null)
  })

  await t.test('ProjectDeleted rejects sessions outside the verified empty boundary', async () => {
    const { result } = await runCleanup({
      projectDeletedSessions: [{ id: 'foreign-session', project_id: targetPin.project_id }],
    })
    assert.equal(result.complete, false)
    assert.equal(result.deletedProjectId, null)
  })
})

test('a new Project session after session deletion blocks DeleteProject', async () => {
  const { result, calls } = await runCleanup({ additionalProjectSessionAfterDelete: true })
  assert.equal(result.deletedSessionId, targetPin.session_id)
  assert.equal(result.deletedProjectId, null)
  assert.equal(result.complete, false)
  assert.equal(calls.includes('DeleteProject'), false)
  const projectCheck = result.ownershipChecks.find((entry) => entry.boundary === 'before_project_deletion')
  assert.equal(projectCheck.verified, false)
  assert.ok(projectCheck.failures.includes('project_acquired_another_session'))
})

test('successful cleanup follows settlement, fresh ownership checks, two agent ACKs, then session and Project deletion', async () => {
  const { result, calls } = await runCleanup()
  assert.equal(result.complete, true)
  assert.deepEqual(result.destroyedAgentIds, [targetPin.utility_agent_id, targetPin.launch_agent_id])
  assert.equal(result.deletedSessionId, targetPin.session_id)
  assert.equal(result.deletedProjectId, targetPin.project_id)
  assert.deepEqual(result.ownershipChecks.map((entry) => entry.boundary), [
    'before_agent_destruction', 'before_session_deletion', 'before_project_deletion',
  ])
  assert.deepEqual(calls, [
    'GetProjectEnvironmentSetupStatus',
    'ListRemoteMachines', 'ListRemoteMachineKernels', 'ListProjects', 'ListSessions', 'GetSessionState',
    'DestroyAgent', 'DestroyAgent',
    'ListRemoteMachines', 'ListRemoteMachineKernels', 'ListProjects', 'ListSessions', 'GetSessionState',
    'DeleteSession', 'ListProjects', 'ListSessions', 'DeleteProject', 'Close',
  ])
})

test('two operation settlements share one cleanup deadline', async () => {
  const calls = []
  const now = { value: 0 }
  const operationIds = ['operation-one', 'operation-two']
  const reads = new Map(operationIds.map((operationId) => [operationId, 0]))
  const statusFor = (operationId, phase) => ({
    operation_id: operationId,
    project_id: targetPin.project_id,
    session_id: targetPin.session_id,
    agent_id: targetPin.utility_agent_id,
    worker_id: targetPin.target_machine_id,
    platform: targetPin.target_platform,
    phase,
    attempt: 1,
    progress_percent: phase === 'ready' ? 100 : 20,
    created_at_ms: 1_000,
    updated_at_ms: 1_000,
  })
  const client = {
    async send(request) {
      const [name] = Object.keys(request)
      calls.push(name)
      if (name === 'GetProjectEnvironmentSetupStatus') {
        const operationId = request[name].operationId
        const read = reads.get(operationId) + 1
        reads.set(operationId, read)
        const phase = operationId === operationIds[0] && read >= 4 ? 'ready' : 'preparing'
        return { ProjectEnvironmentSetupStatus: { status: statusFor(operationId, phase) } }
      }
      if (name === 'CancelProjectEnvironmentSetup') {
        return { ProjectEnvironmentSetupCancelled: { status: statusFor(request[name].operationId, 'cancelled') } }
      }
      throw new Error(`unexpected request ${name}`)
    },
    async close() { calls.push('Close') },
  }
  const evidence = cleanupEvidence()
  const result = await cleanupProjectEnvironmentSetupDrillContext({
    createClient: async () => client,
    pin: targetPin,
    operationIds,
    expectedOwnership,
    evidence,
    cleanupTimeoutMs: 1_000,
    controlTimeoutMs: 100,
    pollMs: 250,
    nowMs: () => now.value,
    delay: async (ms) => { now.value += ms },
  })
  assert.equal(now.value, 1_000)
  assert.equal(reads.get(operationIds[0]), 4)
  assert.equal(reads.get(operationIds[1]), 3)
  assert.equal(result.operationsSettled, false)
  assert.equal(result.complete, false)
  assert.equal(calls.some((name) => name === 'ListRemoteMachines' || name === 'DestroyAgent' || name === 'DeleteSession'), false)
})

test('home-kernel connection establishment is bounded by the cleanup budget', async () => {
  const evidence = cleanupEvidence()
  const startedAtMs = Date.now()
  const result = await cleanupProjectEnvironmentSetupDrillContext({
    createClient: () => new Promise(() => {}),
    pin: targetPin,
    operationIds: ['setup-operation-1'],
    expectedOwnership,
    evidence,
    cleanupTimeoutMs: 50,
    controlTimeoutMs: 20,
  })
  assert.equal(result.operationsSettled, false)
  assert.equal(result.complete, false)
  assert.ok(Date.now() - startedAtMs < 250)
  assert.ok(result.errors.includes('cleanup_transport_or_kernel_failure'))
})

test('a hanging home-kernel close is bounded and prevents complete cleanup evidence', async () => {
  const startedAtMs = Date.now()
  const { result, calls } = await runCleanup({ closeHangs: true, cleanupTimeoutMs: 250, controlTimeoutMs: 40 })
  assert.equal(calls.at(-1), 'Close')
  assert.equal(result.complete, false)
  assert.equal(result.deletedSessionId, targetPin.session_id)
  assert.equal(result.deletedProjectId, targetPin.project_id)
  assert.ok(result.errors.includes('home_kernel_client_close_failed_or_timed_out'))
  assert.ok(Date.now() - startedAtMs < 300)
})

test('transport failure records incomplete ownership evidence and prevents destructive cleanup', async () => {
  const { result, calls } = await runCleanup({ transportFailureAt: 'ListRemoteMachineKernels' })
  assert.equal(result.complete, false)
  assert.equal(result.blockedDeletion, true)
  assert.deepEqual(result.ownershipChecks, [{
    boundary: 'before_agent_destruction',
    verified: false,
    failures: ['home_kernel_snapshot_failed'],
  }])
  assert.equal(calls.includes('DestroyAgent'), false)
  assert.equal(calls.includes('DeleteSession'), false)
  assert.equal(calls.includes('DeleteProject'), false)
  assert.ok(result.errors.includes('ownership_check_failed:before_agent_destruction'))
})

test('transport failure after deleting the session remains explicitly incomplete', async () => {
  const { result, calls } = await runCleanup({ transportFailureAt: 'DeleteProject' })
  assert.equal(result.deletedSessionId, targetPin.session_id)
  assert.equal(result.deletedProjectId, null)
  assert.equal(result.complete, false)
  assert.equal(calls.includes('DeleteProject'), true)
  assert.ok(result.errors.includes(`project_delete_failed:${targetPin.project_id}`))
})
