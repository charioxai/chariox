import assert from 'node:assert/strict'
import test from 'node:test'
import { cleanupProjectEnvironmentSetupDrillContext } from './project-environment-setup-drill-cleanup.mjs'

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

function setupStatus(phase = 'ready') {
  return {
    operation_id: 'setup-operation-1',
    project_id: targetPin.project_id,
    session_id: targetPin.session_id,
    agent_id: targetPin.utility_agent_id,
    worker_id: targetPin.target_machine_id,
    platform: targetPin.target_platform,
    phase,
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
  let sessionDeleted = false
  const project = {
    id: targetPin.project_id,
    owner_user_id: targetPin.project_owner_user_id,
    status: 'active',
    workspace_id: targetPin.workspace_id,
    environment_definition: { origin: 'utility_generated' },
    ...(options.projectOverrides ?? {}),
  }
  const agents = [
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
      if (name === 'GetProjectEnvironmentSetupStatus') return { ProjectEnvironmentSetupStatus: { status: setupStatus(options.operationPhase ?? 'ready') } }
      if (name === 'CancelProjectEnvironmentSetup' && options.cancelHangs) return await new Promise(() => {})
      if (name === 'CancelProjectEnvironmentSetup') return { ProjectEnvironmentSetupCancelled: { status: setupStatus('cancelled') } }
      if (name === 'ListRemoteMachines') return { RemoteMachinesListed: { machines: [{ machine_id: targetPin.target_machine_id, trust_status: 'approved', online: true }] } }
      if (name === 'ListRemoteMachineKernels') {
        if (options.transportFailureAt === name) throw new Error('injected transport failure')
        return { RemoteMachineKernelsListed: { kernels: [{
          kernel_id: targetPin.target_kernel_id,
          machine_id: targetPin.target_machine_id,
          accepting_remote_leases: true,
          available_providers: ['codex', 'opencode'],
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
        return { AgentDestroyed: { agent: { id: agentId } } }
      }
      if (name === 'DeleteSession') {
        sessionDeleted = true
        return { SessionDeleted: { session: { id: targetPin.session_id } } }
      }
      if (name === 'DeleteProject') {
        if (options.transportFailureAt === name) throw new Error('injected transport failure')
        return { ProjectDeleted: { project: { id: targetPin.project_id } } }
      }
      throw new Error(`unexpected request ${name}`)
    },
    async close() { calls.push('Close') },
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

test('both exact DestroyAgent acknowledgments are required before DeleteSession', async () => {
  const { result, calls } = await runCleanup({ failDestroyAgent: targetPin.launch_agent_id })
  assert.equal(result.complete, false)
  assert.deepEqual(result.destroyedAgentIds, [targetPin.utility_agent_id])
  assert.equal(calls.includes('DeleteSession'), false)
  assert.equal(calls.includes('DeleteProject'), false)
  assert.ok(result.errors.includes(`agent_destroy_failed:${targetPin.launch_agent_id}`))
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
