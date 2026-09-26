import { providerCapability } from './project-environment-setup-drill.mjs'

const TERMINAL_PHASES = new Set(['ready', 'failed', 'cancelled'])
export const PROJECT_ENVIRONMENT_SETUP_DRILL_CLEANUP_TIMEOUT_MS = 60_000

export async function cleanupProjectEnvironmentSetupDrillContext({
  createClient,
  pin,
  operationIds,
  expectedOwnership,
  evidence,
  cleanupTimeoutMs = PROJECT_ENVIRONMENT_SETUP_DRILL_CLEANUP_TIMEOUT_MS,
  controlTimeoutMs = 10_000,
  pollMs = 500,
  nowMs = Date.now,
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
  const startedAtMs = nowMs()
  const cleanupErrors = []
  evidence.ownershipChecks ??= []
  evidence.destroyedAgentIds ??= []
  const acknowledgedAgentIds = []
  let operationsSettled = true
  let client = null
  let mayDestroyAgents = false

  try {
    client = await createClient()
    for (const operationId of operationIds) {
      try {
        let status = await readSetupStatus(client, operationId, controlTimeoutMs)
        if (!TERMINAL_PHASES.has(status.phase)) {
          await sendWithTimeout(client, {
            CancelProjectEnvironmentSetup: { operationId, sessionId: pin.session_id },
          }, controlTimeoutMs, 'cancel Project setup for cleanup').catch(() => {})
          status = await waitForTerminalSetup(client, operationId, {
            controlTimeoutMs,
            deadline: nowMs() + cleanupTimeoutMs,
            pollMs,
            nowMs,
            delay,
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
        controlTimeoutMs,
        boundary: 'before_agent_destruction',
        agentPresence: 'exact',
        projectSessions: 'only_pinned',
      })
      if (!mayDestroyAgents) evidence.blockedDeletion = true
    }

    if (mayDestroyAgents) {
      for (const agentId of [pin.utility_agent_id, pin.launch_agent_id]) {
        try {
          const response = await sendWithTimeout(client, {
            DestroyAgent: { session_id: pin.session_id, agent_id: agentId },
          }, controlTimeoutMs, 'destroy drill-owned remote agent')
          requireVariant(response, 'AgentDestroyed')
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
          controlTimeoutMs,
          boundary: 'before_session_deletion',
          agentPresence: 'pinned_subset',
          projectSessions: 'only_pinned',
        })

        if (!mayDeleteSession) {
          evidence.blockedDeletion = true
        } else {
          try {
            requireVariant(
              await sendWithTimeout(client, {
                DeleteSession: { session_ref: pin.session_id, workspace_id: pin.workspace_id },
              }, controlTimeoutMs, 'delete drill-owned session'),
              'SessionDeleted',
            )
            evidence.deletedSessionId = pin.session_id
          } catch {
            cleanupErrors.push(`session_delete_failed:${pin.session_id}`)
            evidence.blockedDeletion = true
          }

          if (evidence.deletedSessionId === pin.session_id) {
            const mayDeleteProject = await checkProjectCleanupBoundary(client, pin, evidence, cleanupErrors, controlTimeoutMs)
            if (mayDeleteProject) {
              try {
                requireVariant(
                  await sendWithTimeout(client, { DeleteProject: { project_id: pin.project_id } }, controlTimeoutMs, 'delete drill-owned Project'),
                  'ProjectDeleted',
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
  } finally {
    await client?.close?.().catch(() => {})
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
  ]
  return evidence
}

async function checkOwnership(client, pin, expectedOwnership, evidence, cleanupErrors, {
  controlTimeoutMs,
  boundary,
  agentPresence,
  projectSessions,
}) {
  let snapshot
  try {
    snapshot = await readOwnershipSnapshot(client, pin, controlTimeoutMs)
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

async function readOwnershipSnapshot(client, pin, controlTimeoutMs) {
  const machines = requireVariant(await sendWithTimeout(client, { ListRemoteMachines: null }, controlTimeoutMs, 'refresh enrolled machines'), 'RemoteMachinesListed').machines ?? []
  const kernels = requireVariant(await sendWithTimeout(client, {
    ListRemoteMachineKernels: { machine_ref: pin.target_machine_id },
  }, controlTimeoutMs, 'refresh target kernels'), 'RemoteMachineKernelsListed').kernels ?? []
  const projects = requireVariant(await sendWithTimeout(client, { ListProjects: { include_archived: false } }, controlTimeoutMs, 'refresh Project ownership'), 'ProjectsListed').projects ?? []
  const sessions = requireVariant(await sendWithTimeout(client, { ListSessions: null }, controlTimeoutMs, 'refresh Project sessions'), 'SessionsListed').sessions ?? []
  const state = requireVariant(await sendWithTimeout(client, {
    GetSessionState: { session_id: pin.session_id },
  }, controlTimeoutMs, 'refresh pinned session ownership'), 'SessionState', 'SessionStateLoaded')
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
  else {
    if (machine.trust_status !== 'approved') failures.push('target_machine_not_approved')
    if (machine.online !== true) failures.push('target_machine_offline')
  }
  if (!kernel) failures.push('target_kernel_missing')
  else {
    if (kernel.machine_id !== pin.target_machine_id) failures.push('target_kernel_machine_changed')
    if (kernel.accepting_remote_leases !== true) failures.push('target_kernel_not_accepting_leases')
    const providers = new Set(kernel.available_providers ?? [])
    if (!providers.has(providerCapability(pin.utility_provider)) || !providers.has(providerCapability(pin.launch_provider))) {
      failures.push('target_kernel_provider_capabilities_changed')
    }
  }
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
    if (!agent || !lease || agent.worktree_id !== pin.worktree_id
      || !remote
      || remote.worker_machine_id !== pin.target_machine_id
      || remote.worker_kernel_id !== pin.target_kernel_id
      || remote.execution_lease_id !== lease.executionLeaseId
      || remote.leased_agent_id !== lease.leasedAgentId
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

async function checkProjectCleanupBoundary(client, pin, evidence, cleanupErrors, controlTimeoutMs) {
  let projects
  let sessions
  try {
    projects = requireVariant(await sendWithTimeout(client, { ListProjects: { include_archived: false } }, controlTimeoutMs, 'recheck Project ownership before deletion'), 'ProjectsListed').projects ?? []
    sessions = requireVariant(await sendWithTimeout(client, { ListSessions: null }, controlTimeoutMs, 'recheck Project sessions before deletion'), 'SessionsListed').sessions ?? []
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

async function readSetupStatus(client, operationId, controlTimeoutMs) {
  const response = await sendWithTimeout(client, {
    GetProjectEnvironmentSetupStatus: { operationId },
  }, controlTimeoutMs, 'read Project setup status for cleanup')
  return requireVariant(response, 'ProjectEnvironmentSetupStatus').status
}

async function waitForTerminalSetup(client, operationId, { controlTimeoutMs, deadline, pollMs, nowMs, delay }) {
  let last = null
  while (nowMs() < deadline) {
    last = await readSetupStatus(client, operationId, controlTimeoutMs)
    if (TERMINAL_PHASES.has(last?.phase)) return last
    const remaining = deadline - nowMs()
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

async function sendWithTimeout(client, request, timeoutMs, label) {
  let timer
  try {
    return await Promise.race([
      client.send(request),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out`)), timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}
