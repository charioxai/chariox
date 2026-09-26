import { performance } from 'node:perf_hooks'
import { assertBoundSetupStatus, getSetupStatusFromResponse, providerCapability } from './project-environment-setup-drill.mjs'

const TERMINAL_PHASES = new Set(['ready', 'failed', 'cancelled'])
export const PROJECT_ENVIRONMENT_SETUP_DRILL_CLEANUP_TIMEOUT_MS = 60_000

export async function cleanupProjectEnvironmentSetupDrillContext({
  createClient,
  pin,
  operationIds,
  expectedOwnership,
  evidence,
  requestBuilders,
  cleanupTimeoutMs = PROJECT_ENVIRONMENT_SETUP_DRILL_CLEANUP_TIMEOUT_MS,
  controlTimeoutMs = 10_000,
  pollMs = 500,
  nowMs = () => performance.now(),
  delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
}) {
  if (!Number.isInteger(cleanupTimeoutMs) || cleanupTimeoutMs <= 0 || cleanupTimeoutMs > PROJECT_ENVIRONMENT_SETUP_DRILL_CLEANUP_TIMEOUT_MS) {
    throw new Error('Project setup cleanup timeout must fit the fixed 60 second bound')
  }
  if (!Number.isInteger(controlTimeoutMs) || controlTimeoutMs <= 0 || controlTimeoutMs > 10_000) {
    throw new Error('Project setup cleanup control request timeout must fit the fixed 10 second bound')
  }
  if (!Number.isInteger(pollMs) || pollMs < 250 || pollMs > 5_000) {
    throw new Error('Project setup cleanup poll interval is outside the bounded range')
  }
  if (!Array.isArray(operationIds) || operationIds.length > 2 || new Set(operationIds).size !== operationIds.length) {
    throw new Error('Project setup cleanup accepts at most two distinct drill operation identities')
  }
  requireRequestBuilders(requestBuilders)
  const startedAtMs = nowMs()
  const deadline = startedAtMs + cleanupTimeoutMs
  const budget = { deadline, controlTimeoutMs, nowMs }
  const cleanupErrors = []
  evidence.ownershipChecks ??= []
  evidence.destroyedAgentIds ??= []
  const acknowledgedAgentIds = []
  let operationsSettled = true
  let client = null
  let mayDestroyAgents = false

  try {
    client = await createClientWithinBudget(createClient, budget)
    for (const operationId of operationIds) {
      try {
        let status = await readSetupStatus(client, pin, operationId, budget, requestBuilders)
        if (!TERMINAL_PHASES.has(status.phase)) {
          let cancelResponse = null
          try {
            cancelResponse = await sendWithTimeout(client,
              requestBuilders.cancelProjectEnvironmentSetupRequest(operationId, pin.session_id),
              budget,
              'cancel Project setup for cleanup',
            )
          } catch {
            // Status polling remains authoritative if cancellation is unavailable.
          }
          if (cancelResponse?.ProjectEnvironmentSetupCancelled?.status) {
            assertBoundSetupStatus(cancelResponse.ProjectEnvironmentSetupCancelled.status, setupStatusBinding(pin, operationId))
          }
          status = await waitForTerminalSetup(client, operationId, {
            pin,
            budget,
            pollMs,
            delay,
            requestBuilders,
          })
        }
        if (!TERMINAL_PHASES.has(status?.phase)) throw new Error('setup operation did not settle')
      } catch {
        operationsSettled = false
        cleanupErrors.push(`operation_not_confirmed_terminal:${operationId}`)
      }
    }

    evidence.operationsSettled = operationsSettled
    if (!operationsSettled) {
      evidence.blockedDeletion = true
      cleanupErrors.push('ownership_not_checked_because_operations_unsettled')
    } else {
      mayDestroyAgents = await checkOwnership(client, pin, expectedOwnership, evidence, cleanupErrors, {
        budget,
        requestBuilders,
        boundary: 'before_agent_destruction',
        agentPresence: 'exact',
        projectSessions: 'only_pinned',
      })
      if (!mayDestroyAgents) evidence.blockedDeletion = true
    }

    if (mayDestroyAgents) {
      for (const agentId of [pin.utility_agent_id, pin.launch_agent_id]) {
        try {
          const response = await sendWithTimeout(client,
            requestBuilders.destroyAgentRequest(pin.session_id, agentId),
            budget,
            'destroy drill-owned remote agent',
          )
          assertAgentDestroyed(response, pin.session_id, agentId)
          acknowledgedAgentIds.push(agentId)
          evidence.destroyedAgentIds.push(agentId)
        } catch {
          cleanupErrors.push(`agent_destroy_failed:${agentId}`)
        }
      }

      if (acknowledgedAgentIds.length !== 2) {
        evidence.blockedDeletion = true
        cleanupErrors.push('session_delete_blocked_without_two_destroy_agent_acknowledgments')
      } else {
        const mayDeleteSession = await checkOwnership(client, pin, expectedOwnership, evidence, cleanupErrors, {
          budget,
          requestBuilders,
          boundary: 'before_session_deletion',
          agentPresence: 'pinned_subset',
          projectSessions: 'only_pinned',
        })

        if (!mayDeleteSession) {
          evidence.blockedDeletion = true
        } else {
          // The public API has no atomic ownership precondition on DeleteSession.
          // This last fresh snapshot narrows, but cannot eliminate, the join race.
          try {
            assertSessionDeleted(
              await sendWithTimeout(client,
                requestBuilders.deleteSessionRequest(pin.session_id, pin.workspace_id),
                budget,
                'delete drill-owned session',
              ),
              pin,
            )
            evidence.deletedSessionId = pin.session_id
          } catch {
            cleanupErrors.push(`session_delete_failed:${pin.session_id}`)
            evidence.blockedDeletion = true
          }

          if (evidence.deletedSessionId === pin.session_id) {
            const mayDeleteProject = await checkProjectCleanupBoundary(client, pin, evidence, cleanupErrors, budget, requestBuilders)
            if (mayDeleteProject) {
              try {
                assertProjectDeleted(
                  await sendWithTimeout(client, requestBuilders.deleteProjectRequest(pin.project_id), budget, 'delete drill-owned Project'),
                  pin,
                )
                evidence.deletedProjectId = pin.project_id
              } catch {
                cleanupErrors.push(`project_delete_failed:${pin.project_id}`)
                evidence.blockedDeletion = true
              }
            } else {
              evidence.blockedDeletion = true
            }
          }
        }
      }
    }
  } catch {
    cleanupErrors.push('cleanup_transport_or_kernel_failure')
    evidence.blockedDeletion = true
    if (!client) operationsSettled = false
  } finally {
    if (client && !await closeClientWithinBudget(client, budget)) {
      cleanupErrors.push('home_kernel_client_close_failed_or_timed_out')
      evidence.blockedDeletion = true
    }
  }

  evidence.errors = [...new Set([...(evidence.errors ?? []), ...cleanupErrors])]
  evidence.complete = acknowledgedAgentIds.length === 2
    && evidence.deletedSessionId === pin.session_id
    && evidence.deletedProjectId === pin.project_id
    && operationsSettled
    && cleanupErrors.length === 0
  evidence.durationMs = Math.max(0, nowMs() - startedAtMs)
  evidence.limitations = [
    'The public home-kernel API exposes no delete operation for retained setup status records.',
    'The worker machine, enrollment, and materialized worktree are retained for root-owned post-campaign disposition; this drill never provisions or deletes infrastructure.',
    'The final ownership snapshot and DeleteSession are separate home-kernel requests; a foreign agent can still join between them because the public API has no atomic conditional delete.',
  ]
  return evidence
}

async function checkOwnership(client, pin, expectedOwnership, evidence, cleanupErrors, {
  budget,
  requestBuilders,
  boundary,
  agentPresence,
  projectSessions,
}) {
  let snapshot
  try {
    snapshot = await readOwnershipSnapshot(client, pin, budget, requestBuilders)
  } catch {
    evidence.ownershipChecks.push({ boundary, verified: false, failures: ['home_kernel_snapshot_failed'] })
    cleanupErrors.push(`ownership_check_failed:${boundary}`)
    return false
  }

  const report = inspectOwnershipSnapshot(snapshot, pin, expectedOwnership, { boundary, agentPresence, projectSessions })
  evidence.ownershipChecks.push(report)
  if (!report.verified) {
    cleanupErrors.push(`ownership_changed:${boundary}:${report.failures.join(',')}`)
    return false
  }
  return true
}

async function readOwnershipSnapshot(client, pin, budget, requestBuilders) {
  const machines = requireVariant(await sendWithTimeout(client, requestBuilders.listRemoteMachinesRequest(), budget, 'refresh enrolled machines'), 'RemoteMachinesListed').machines ?? []
  const kernels = requireVariant(await sendWithTimeout(client, requestBuilders.listRemoteMachineKernelsRequest(pin.target_machine_id), budget, 'refresh target kernels'), 'RemoteMachineKernelsListed').kernels ?? []
  const projects = requireVariant(await sendWithTimeout(client, requestBuilders.listProjectsRequest(false), budget, 'refresh Project ownership'), 'ProjectsListed').projects ?? []
  const sessions = requireVariant(await sendWithTimeout(client, requestBuilders.listSessionsRequest(), budget, 'refresh Project sessions'), 'SessionsListed').sessions ?? []
  const state = requireVariant(await sendWithTimeout(client, requestBuilders.getSessionStateRequest(pin.session_id), budget, 'refresh pinned session ownership'), 'SessionState', 'SessionStateLoaded')
  return { machines, kernels, projects, sessions, session: state.session }
}

function inspectOwnershipSnapshot(snapshot, pin, expectedOwnership, { boundary, agentPresence, projectSessions }) {
  const machine = snapshot.machines.find((entry) => entry.machine_id === pin.target_machine_id) ?? null
  const kernel = snapshot.kernels.find((entry) => entry.kernel_id === pin.target_kernel_id) ?? null
  const project = snapshot.projects.find((entry) => entry.id === pin.project_id) ?? null
  const session = snapshot.session ?? null
  const projectSessionEntries = snapshot.sessions.filter((entry) => entry.project_id === pin.project_id)
  const sessionAgents = session?.agents ?? []
  const pinnedAgentIds = [pin.utility_agent_id, pin.launch_agent_id]
  const agentIds = sessionAgents.map((agent) => agent.id)
  const observed = {
    machine: machine && { id: machine.machine_id, trustStatus: machine.trust_status, online: machine.online },
    kernel: kernel && {
      id: kernel.kernel_id,
      machineId: kernel.machine_id,
      acceptingRemoteLeases: kernel.accepting_remote_leases,
      availableProviders: [...(kernel.available_providers ?? [])].sort(),
    },
    project: project && {
      id: project.id,
      ownerUserId: project.owner_user_id,
      status: project.status,
      workspaceId: project.workspace_id ?? null,
      workspaceIds: [...(project.workspace_ids ?? [])].sort(),
    },
    projectSessionIds: projectSessionEntries.map((entry) => entry.id).sort(),
    session: session && {
      id: session.id,
      projectId: session.project_id,
      workspaceId: session.workspace_id,
      worktreeId: session.worktree_id,
      status: session.status,
      agentIds: [...agentIds].sort(),
    },
  }
  const failures = []
  if (!machine) failures.push('target_machine_missing')
  else if (machine.trust_status !== 'approved') failures.push('target_machine_not_approved')
  if (!kernel) failures.push('target_kernel_missing')
  else {
    if (kernel.machine_id !== pin.target_machine_id) failures.push('target_kernel_machine_changed')
  }
  // These mutations are sent to the home kernel. Worker liveness and lease capacity are launch/preflight health, not ownership.
  if (!project) failures.push('pinned_project_missing')
  else {
    if (project.status !== 'active') failures.push('pinned_project_not_active')
    if (project.owner_user_id !== pin.project_owner_user_id) failures.push('project_owner_changed')
    if (!projectOwnsWorkspace(project, pin.workspace_id)) failures.push('project_workspace_binding_changed')
  }
  if (!session) failures.push('pinned_session_missing')
  else {
    if (session.id !== pin.session_id) failures.push('pinned_session_identity_changed')
    if (session.project_id !== pin.project_id) failures.push('session_project_binding_changed')
    if (session.workspace_id !== pin.workspace_id) failures.push('session_workspace_binding_changed')
    if (session.worktree_id !== pin.worktree_id) failures.push('session_worktree_binding_changed')
    if (!String(session.status ?? '').trim() || ['ended', 'deleted', 'archived', 'closed'].includes(String(session.status).toLowerCase())) {
      failures.push('pinned_session_not_active')
    }
  }
  if (projectSessions === 'only_pinned') {
    if (projectSessionEntries.length !== 1 || projectSessionEntries[0]?.id !== pin.session_id) failures.push('project_session_set_changed')
  } else if (projectSessionEntries.length !== 0) {
    failures.push('project_acquired_another_session')
  }

  const presentPinnedIds = agentIds.filter((id) => pinnedAgentIds.includes(id))
  if (new Set(agentIds).size !== agentIds.length) failures.push('duplicate_session_agent_identity')
  if (agentIds.some((id) => !pinnedAgentIds.includes(id))) failures.push('unowned_session_agent_present')
  if (agentPresence === 'exact' && (agentIds.length !== 2 || new Set(presentPinnedIds).size !== 2)) {
    failures.push('pinned_agent_set_changed_before_destruction')
  }
  if (agentPresence === 'pinned_subset' && presentPinnedIds.length !== agentIds.length) {
    failures.push('unowned_session_agent_present')
  }
  for (const agentId of presentPinnedIds) {
    const agent = sessionAgents.find((entry) => entry.id === agentId)
    const role = agentId === pin.utility_agent_id ? 'utility' : 'launch'
    const lease = expectedOwnership?.workerLeases?.find((entry) => entry.agentId === agentId)
    const remote = agent?.remote_execution
    // Lease identifiers are generation-scoped and may rotate during the normal single binding refresh.
    if (!agent || !lease || agent.worktree_id !== pin.worktree_id
      || !remote
      || remote.worker_machine_id !== pin.target_machine_id
      || remote.worker_kernel_id !== pin.target_kernel_id
      || (role === 'utility' && (agent.provider !== pin.utility_provider || agent.account_profile !== pin.utility_account_profile))
      || (role === 'launch' && (providerCapability(agent.provider) !== providerCapability(pin.launch_provider)
        || agent.account_profile !== pin.launch_account_profile))) {
      failures.push(`pinned_${role}_agent_binding_changed`)
    }
  }

  if (expectedOwnership?.machineId !== pin.target_machine_id || expectedOwnership?.kernelId !== pin.target_kernel_id
    || expectedOwnership?.projectId !== pin.project_id || expectedOwnership?.sessionId !== pin.session_id
    || expectedOwnership?.workspaceId !== pin.workspace_id || expectedOwnership?.worktreeId !== pin.worktree_id) {
    failures.push('preflight_ownership_record_does_not_match_pin')
  }
  return { boundary, verified: failures.length === 0, observed, failures }
}

async function checkProjectCleanupBoundary(client, pin, evidence, cleanupErrors, budget, requestBuilders) {
  let projects
  let sessions
  try {
    projects = requireVariant(await sendWithTimeout(client, requestBuilders.listProjectsRequest(false), budget, 'recheck Project ownership before deletion'), 'ProjectsListed').projects ?? []
    sessions = requireVariant(await sendWithTimeout(client, requestBuilders.listSessionsRequest(), budget, 'recheck Project sessions before deletion'), 'SessionsListed').sessions ?? []
  } catch {
    evidence.ownershipChecks.push({ boundary: 'before_project_deletion', verified: false, failures: ['home_kernel_snapshot_failed'] })
    cleanupErrors.push('ownership_check_failed:before_project_deletion')
    return false
  }
  const project = projects.find((entry) => entry.id === pin.project_id) ?? null
  const projectSessions = sessions.filter((entry) => entry.project_id === pin.project_id)
  const failures = []
  if (!project) failures.push('pinned_project_missing')
  else {
    if (project.status !== 'active') failures.push('pinned_project_not_active')
    if (project.owner_user_id !== pin.project_owner_user_id) failures.push('project_owner_changed')
    if (!projectOwnsWorkspace(project, pin.workspace_id)) failures.push('project_workspace_binding_changed')
  }
  if (projectSessions.length !== 0) failures.push('project_acquired_another_session')
  evidence.ownershipChecks.push({
    boundary: 'before_project_deletion',
    verified: failures.length === 0,
    observed: {
      project: project && {
        id: project.id,
        ownerUserId: project.owner_user_id,
        status: project.status,
        workspaceId: project.workspace_id ?? null,
        workspaceIds: [...(project.workspace_ids ?? [])].sort(),
      },
      projectSessionIds: projectSessions.map((entry) => entry.id).sort(),
    },
    failures,
  })
  if (failures.length > 0) cleanupErrors.push(`ownership_changed:before_project_deletion:${failures.join(',')}`)
  return failures.length === 0
}

function projectOwnsWorkspace(project, workspaceId) {
  return project.workspace_id === workspaceId || (project.workspace_ids ?? []).includes(workspaceId)
}

async function readSetupStatus(client, pin, operationId, budget, requestBuilders) {
  const response = await sendWithTimeout(client,
    requestBuilders.getProjectEnvironmentSetupStatusRequest(operationId),
    budget,
    'read Project setup status for cleanup',
  )
  const status = getSetupStatusFromResponse(response)
  return assertBoundSetupStatus(status, setupStatusBinding(pin, operationId))
}

async function waitForTerminalSetup(client, operationId, { pin, budget, pollMs, delay, requestBuilders }) {
  let last = null
  while (budget.nowMs() < budget.deadline) {
    last = await readSetupStatus(client, pin, operationId, budget, requestBuilders)
    if (TERMINAL_PHASES.has(last?.phase)) return last
    const remaining = budget.deadline - budget.nowMs()
    if (remaining <= 0) break
    await delay(Math.min(pollMs, remaining))
  }
  return last
}

function requireVariant(response, ...variants) {
  for (const variant of variants) {
    if (response?.[variant] != null) return response[variant]
  }
  throw new Error(`home kernel returned none of the expected response variants: ${variants.join(', ')}`)
}

function requireRequestBuilders(requestBuilders) {
  const required = [
    'cancelProjectEnvironmentSetupRequest',
    'deleteProjectRequest',
    'deleteSessionRequest',
    'destroyAgentRequest',
    'getProjectEnvironmentSetupStatusRequest',
    'getSessionStateRequest',
    'listProjectsRequest',
    'listRemoteMachineKernelsRequest',
    'listRemoteMachinesRequest',
    'listSessionsRequest',
  ]
  for (const name of required) {
    if (typeof requestBuilders?.[name] !== 'function') throw new Error(`kernel-client request builder ${name} is required`)
  }
}

function setupStatusBinding(pin, operationId) {
  return {
    operation_id: operationId,
    project_id: pin.project_id,
    session_id: pin.session_id,
    agent_id: pin.utility_agent_id,
    worker_id: pin.target_machine_id,
    platform: pin.target_platform,
  }
}

function assertAgentDestroyed(response, sessionId, agentId) {
  const { agent } = requireVariant(response, 'AgentDestroyed')
  assertIdentity(agent, { id: agentId, session_id: sessionId }, 'AgentDestroyed')
}

function assertSessionDeleted(response, pin) {
  const { session } = requireVariant(response, 'SessionDeleted')
  assertIdentity(session, {
    id: pin.session_id,
    project_id: pin.project_id,
    workspace_id: pin.workspace_id,
    worktree_id: pin.worktree_id,
  }, 'SessionDeleted')
  if (!Array.isArray(session.agents)) throw new Error('SessionDeleted omitted its serialized agent identities')
  const ownedAgentIds = new Set([pin.utility_agent_id, pin.launch_agent_id])
  for (const agent of session.agents) {
    if (!ownedAgentIds.has(agent?.id)) throw new Error('SessionDeleted returned an unpinned session agent')
    assertIdentity(agent, { session_id: pin.session_id }, 'SessionDeleted agent')
  }
}

function assertProjectDeleted(response, pin) {
  const { project, sessions } = requireVariant(response, 'ProjectDeleted')
  assertIdentity(project, { id: pin.project_id, owner_user_id: pin.project_owner_user_id }, 'ProjectDeleted')
  if (!projectOwnsWorkspace(project, pin.workspace_id)) throw new Error('ProjectDeleted returned a Project with a different workspace binding')
  if (!Array.isArray(sessions) || sessions.length !== 0) throw new Error('ProjectDeleted returned unexpected session identities')
}

function assertIdentity(record, expected, responseName) {
  if (!record || typeof record !== 'object') throw new Error(`${responseName} omitted its serialized identity`)
  for (const [key, value] of Object.entries(expected)) {
    if (record[key] !== value) throw new Error(`${responseName} returned a different ${key}`)
  }
}

async function createClientWithinBudget(createClient, budget) {
  const timeoutMs = remainingTimeout(budget, 'connect to home kernel')
  const creation = Promise.resolve().then(createClient)
  try {
    return await withTimeout(creation, timeoutMs, 'connect to home kernel')
  } catch (error) {
    void creation.then((client) => {
      try {
        Promise.resolve(client?.close?.()).catch(() => {})
      } catch {
        // A late transport connection is closed best effort without extending cleanup.
      }
    }, () => {})
    throw error
  }
}

async function closeClientWithinBudget(client, budget) {
  let closing
  try {
    closing = Promise.resolve().then(() => client.close?.())
  } catch {
    return false
  }
  closing.catch(() => {})
  let timeoutMs
  try {
    timeoutMs = remainingTimeout(budget, 'close home kernel connection')
  } catch {
    return false
  }
  try {
    await withTimeout(closing, timeoutMs, 'close home kernel connection')
    return true
  } catch {
    return false
  }
}

function remainingTimeout(budget, label) {
  const remainingMs = budget.deadline - budget.nowMs()
  if (!Number.isFinite(remainingMs) || remainingMs <= 0) throw new Error(`${label} exceeded cleanup deadline`)
  return Math.max(1, Math.min(budget.controlTimeoutMs, remainingMs))
}

async function sendWithTimeout(client, request, budget, label) {
  const timeoutMs = remainingTimeout(budget, label)
  return await withTimeout(client.send(request), timeoutMs, label)
}

async function withTimeout(promise, timeoutMs, label) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out`)), timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}
