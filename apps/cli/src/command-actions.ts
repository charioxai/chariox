import type {
  AgentInstance,
  RuntimeAttachment,
  QueuedWorkflowLaunch,
  RuntimeProviderRun,
  RuntimeSession,
  SessionConfigState,
  WorkflowEdgeDefinition,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
  WorkflowRun,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { RelayCloudProfile, MultiAgentResponseLayout } from "./preferences.js"
import { type RelayStatus } from "./cloud-command-lifecycle.js"
import { handleCloudSlashCommand, handleRelaySlashCommand } from "./cloud-command-handlers.js"
import {
  handleProviderSlashCommand,
  type ProviderCommandHandlerDeps,
} from "./provider-command-handlers.js"
import {
  handleModelSlashCommand,
  handleVariantSlashCommand,
  handleViewSlashCommand,
  type SelectionCommandHandlerDeps,
} from "./selection-command-handlers.js"
import {
  handleConfigSlashCommand,
  type ConfigCommandHandlerDeps,
} from "./config-command-handlers.js"
import {
  handleRemoteMachineSlashCommand,
  type RemoteMachineCommandHandlerDeps,
} from "./remote-machine-command-handlers.js"
import {
  handleMcpSlashCommand,
  handleSkillSlashCommand,
  type CapabilityCommandHandlerDeps,
} from "./capability-command-handlers.js"
import {
  handleSliceSlashCommand,
  type SliceCommandHandlerDeps,
} from "./slice-command-handlers.js"
import {
  handleWorkspaceSlashCommand,
  handleWorktreeSlashCommand,
  type WorkspaceCommandHandlerDeps,
} from "./workspace-command-handlers.js"
import {
  handleSessionSlashCommand,
  type SessionCommandHandlerDeps,
} from "./session-command-handlers.js"
import {
  handleAgentSlashCommand,
  handleCycleAgentFocus as cycleAgentFocusCommand,
  type AgentCommandHandlerDeps,
} from "./agent-command-handlers.js"
import {
  handleKernelSlashCommand,
  type KernelCommandHandlerDeps,
} from "./kernel-command-handlers.js"
import {
  prepareLocalGitWorktree,
  type LocalGitWorktreeOptions,
  type RemoteGitWorktreePlacement,
} from "./command-worktree-placement.js"
import { readFile } from "node:fs/promises"
import { resolve as resolvePath } from "node:path"

const WORKFLOW_MAX_TURNS_CONFIG_KEY = "workflow.max_turns"
const WORKFLOW_LAUNCH_POLICY_CONFIG_KEY = "workflow.launch_policy"

type FooterTone = "info" | "error"

type ResolvedAgentReference = {
  agent: AgentInstance | null
  error?: string
}

type AgentCyclePayload = {
  agent: AgentInstance | null
  session: RuntimeSession
}

type AgentFocusPayload = {
  agent: AgentInstance
  session: RuntimeSession
}

type AgentSpawnPayload = {
  agent: AgentInstance
  session: RuntimeSession
}

type AgentConfigUpdatePayload = {
  agent: AgentInstance
  session: RuntimeSession
}

type WorkflowCreatePayload = {
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowResolvePayload = {
  workflow: WorkflowDefinition
}

type WorkflowEndpointPayload = {
  endpoint: WorkflowEndpointDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowNodePayload = {
  node: WorkflowNodeDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowEdgePayload = {
  edge: WorkflowEdgeDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowRunInvokePayload = {
  workflow: WorkflowDefinition
  endpoint: WorkflowEndpointDefinition
  session: RuntimeSession
} & ({ workflow_run: WorkflowRun } | { queued_launch: QueuedWorkflowLaunch })

type QueuedWorkflowLaunchPayload = {
  queued_launch: QueuedWorkflowLaunch
  session: RuntimeSession
}

type WorkflowRunCancelPayload = {
  workflow_run: WorkflowRun
  session: RuntimeSession
}

type WorkflowRunResumePayload = {
  workflow_run: WorkflowRun
  session: RuntimeSession
}

type WorkflowWatchdogPayload = {
  watchdog: WorkflowWatchdogDefinition
  workflow?: WorkflowDefinition
  endpoint?: WorkflowEndpointDefinition
  session: RuntimeSession
}

export { parseRequestedViewLayout } from "./selection-command-handlers.js"
export {
  formatAgentListSummary,
  formatAgentSubstituteSummary,
} from "./agent-command-handlers.js"
export {
  formatAgentCapabilityGrants,
  parseMcpInstallConfig,
} from "./capability-command-handlers.js"

type CommandActionDeps =
  & ProviderCommandHandlerDeps
  & SelectionCommandHandlerDeps
  & ConfigCommandHandlerDeps
  & RemoteMachineCommandHandlerDeps
  & CapabilityCommandHandlerDeps
  & Omit<SliceCommandHandlerDeps, "currentWorktreeTarget">
  & Omit<WorkspaceCommandHandlerDeps, "currentWorkspaceTarget" | "currentWorktreeTarget" | "setWorkspaceTarget" | "setWorktreeTarget" | "baseWorktree" | "hasDynamicWorktreeTarget">
  & Omit<SessionCommandHandlerDeps, "currentWorkspaceTarget" | "currentWorktreeTarget">
  & KernelCommandHandlerDeps
  & {
  workspace: string
  worktree: string
  getWorkspaceTarget?: () => string
  getWorktreeTarget?: () => string
  setWorkspaceTarget?: (workspace: string) => void
  setWorktreeTarget?: (worktree: string) => void
  accountProfile?: string | null
  clientId?: string | null
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  attachmentState: () => RuntimeAttachment | null
  providerRunState: () => RuntimeProviderRun | null
  currentModelId: () => string
  currentVariantId: () => string
  currentProviderId: () => string
  focusedAgentId: () => string | null
  multiAgentResponseLayout: () => MultiAgentResponseLayout
  maxAgentsPerScreen: () => number
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  appendCloudNotice?: (message: string) => void
  formatError: (error: unknown) => string
  createSessionInvite?: (
    sessionId: string,
    expiresInMs: number | null,
    maxUses: number | null,
  ) => Promise<{ invite: { invite_token: string; invite: { invite_id: string } }; session: RuntimeSession }>
  joinSessionInvite?: (
    inviteToken: string,
    userId: string,
  ) => Promise<{ member: { user_id: string }; session: RuntimeSession }>
  prepareLocalGitWorktree?: (options: LocalGitWorktreeOptions) => Promise<string>
  attachBinding: (
    session: Pick<RuntimeSession, "id">,
    createdSession: boolean,
  ) => Promise<void>
  deleteKernel?: () => Promise<{ kernelId: string; deletedSessions: RuntimeSession[] }>
  transitionToNoSession: (message: string) => void
  getRelayStatus?: () => Promise<RelayStatus>
  configureRelay?: (relayUrl: string | null, relayToken: string | null) => Promise<RelayStatus>
  cloudRelayConnectTimeoutMs?: number
  cloudRelayConnectPollMs?: number
  refreshWaitingRoomData?: () => Promise<void>
  getCloudRelayProfile?: () => RelayCloudProfile | null
  saveCloudRelayProfile?: (profile: RelayCloudProfile | null) => Promise<void>
  cloudRelayApiUrl?: string | undefined
  bootstrapCloudRelay?: (
    apiUrl: string,
    email: string,
    accountSlug?: string,
  ) => Promise<RelayCloudProfile>
  startCloudDeviceLogin?: (
    apiUrl: string,
    input: { clientId?: string; machineId?: string; clientAlias?: string; machineAlias?: string },
  ) => Promise<{
    apiUrl: string
    deviceCode: string
    userCode: string
    verificationUrl: string
    expiresAtMs: number
    intervalSeconds: number
  }>
  pollCloudDeviceLogin?: (
    apiUrl: string,
    deviceCode: string,
  ) => Promise<
    | { status: "authorization_pending"; intervalSeconds: number; expiresAtMs: number }
    | { status: "expired_token" }
    | { status: "approved"; profile: RelayCloudProfile }
  >
  openExternalUrl?: (url: string) => Promise<boolean>
  logoutCloudRelay?: (profile: RelayCloudProfile, options?: { revokeClient?: boolean; revokeMachine?: boolean }) => Promise<void>
  pairCloudRelayClient?: (
    profile: RelayCloudProfile,
    clientId: string,
    alias?: string,
  ) => Promise<RelayCloudProfile>
  pairCloudRelayMachine?: (
    profile: RelayCloudProfile,
    machineId: string,
    alias?: string,
  ) => Promise<RelayCloudProfile>
  issueCloudKernelRelayToken?: (
    profile: RelayCloudProfile,
    daemonId: string,
  ) => Promise<{ relayUrl: string; relayToken: string; tokenExpiresAtMs: number; profile?: RelayCloudProfile }>
  issueCloudMachineRelayToken?: (
    profile: RelayCloudProfile,
    daemonId: string,
    machineId: string,
  ) => Promise<{ relayUrl: string; relayToken: string; tokenExpiresAtMs: number; profile?: RelayCloudProfile }>
  issueCloudClientRelayToken?: (
    profile: RelayCloudProfile,
    targetDaemonAlias: string,
    options?: { sessionId?: string | null },
  ) => Promise<{ relayUrl: string; relayToken: string; tokenExpiresAtMs: number; profile?: RelayCloudProfile }>
  createCloudSessionInvite?: (
    sessionId: string,
    options: { displayName?: string | null; expiresInMs?: number | null; maxUses?: number | null },
  ) => Promise<Record<string, unknown>>
  showCloudSessionInvite?: (inviteToken: string) => Promise<Record<string, unknown>>
  acceptCloudSessionInvite?: (inviteToken: string) => Promise<Record<string, unknown>>
  listCloudSessionMembers?: (sessionId: string) => Promise<Record<string, unknown>>
  listCloudCollaborators?: () => Promise<Record<string, unknown>[]>
  updateSessionConfig: (
    sessionId: string,
    attachmentId: string,
    values: Record<string, string>,
    requiresIdle: boolean,
  ) => Promise<{ session: RuntimeSession; config: SessionConfigState }>
  updateAgentConfig?: (
    sessionId: string,
    agentId: string,
    options: {
      executionMode?: "build" | "plan" | null
      clearExecutionMode?: boolean
      permissionLevel?: "required" | "yolo" | null
      clearPermissionLevel?: boolean
    },
  ) => Promise<AgentConfigUpdatePayload>
  updateAgentProfile?: (
    sessionId: string,
    agentId: string,
    options: {
      provider?: string | null
      model?: string | null
      effort?: string | null
      clearEffort?: boolean
    },
  ) => Promise<AgentConfigUpdatePayload>
  aliasAgent?: (
    sessionId: string,
    agentId: string,
    alias: string,
  ) => Promise<AgentConfigUpdatePayload>
  updateAgentSubstitutes?: (
    sessionId: string,
    agentId: string,
    action: Record<string, unknown>,
  ) => Promise<AgentConfigUpdatePayload>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  rebuildTranscript: () => void
  requestRender: () => void
  cycleAgentFocus: () => Promise<AgentCyclePayload>
  launchAgentProviderRun: (
    provider: string,
    model: string,
    variant: string,
    agentId: string,
  ) => Promise<RuntimeProviderRun>
  setProviderRunState: (run: RuntimeProviderRun | null) => void
  refreshSessionState: (sessionId: string) => Promise<RuntimeSession>
  spawnAgent: (
    provider?: string | null,
    alias?: string,
    model?: string | null,
    effort?: string | null,
    worktreeId?: string,
    machineRef?: string,
    worktreePlacement?: RemoteGitWorktreePlacement | undefined,
    sliceRef?: string,
  ) => Promise<AgentSpawnPayload>
  destroyAgent: (agentId: string) => Promise<RuntimeSession>
  focusAgent: (agentId: string) => Promise<AgentFocusPayload>
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  workflowScreenActive: () => boolean
  showWorkflowScreen: () => void
  selectedWorkflowId?: () => string | null
  selectWorkflowCanvas: (workflowId: string | null) => void
  replaceWorkflowDefinitions: (workflows: WorkflowDefinition[]) => void
  upsertWorkflowDefinition: (workflow: WorkflowDefinition) => void
  createWorkflow: (alias?: string | null) => Promise<WorkflowCreatePayload>
  listWorkflows: () => Promise<WorkflowDefinition[]>
  resolveWorkflow: (workflowRef: string) => Promise<WorkflowResolvePayload>
  assignWorkflowAlias: (workflowId: string, alias: string) => Promise<WorkflowDefinition | null>
  createWorkflowEndpoint: (
    workflowRef: string,
    entryNodeId: string,
    alias?: string | null,
  ) => Promise<WorkflowEndpointPayload>
  assignWorkflowEndpointAlias: (
    workflowRef: string,
    endpointRef: string,
    alias: string,
  ) => Promise<WorkflowEndpointPayload>
  bindWorkflowEndpoint: (
    workflowRef: string,
    endpointRef: string,
    entryNodeId: string,
  ) => Promise<WorkflowEndpointPayload>
  addWorkflowNode: (workflowRef: string, agentId: string) => Promise<WorkflowNodePayload>
  removeWorkflowNode: (workflowRef: string, nodeId: string) => Promise<WorkflowNodePayload>
  addWorkflowEdge: (
    workflowRef: string,
    fromNodeId: string,
    toNodeId: string,
  ) => Promise<WorkflowEdgePayload>
  removeWorkflowEdge: (workflowRef: string, edgeId: string) => Promise<WorkflowEdgePayload>
  invokeWorkflowEndpoint?: (
    workflowRef: string,
    endpointRef: string,
    prompt?: string | null,
  ) => Promise<WorkflowRunInvokePayload>
  createWorkflowWatchdog?: (
    workflowRef: string,
    endpointRef: string,
    intervalSeconds: number,
    invocationPrompt: string,
    policy: "skip" | "queue",
    maxWakeups?: number | null,
  ) => Promise<WorkflowWatchdogPayload>
  listWorkflowWatchdogs?: (workflowRef?: string | null) => Promise<{ watchdogs: WorkflowWatchdogDefinition[] }>
  setWorkflowWatchdogEnabled?: (watchdogRef: string, enabled: boolean) => Promise<WorkflowWatchdogPayload>
  removeWorkflowWatchdog?: (watchdogRef: string) => Promise<WorkflowWatchdogPayload>
  setWorkflowFlushContext?: (
    workflowRef: string,
    flushAgentContextBeforeRun: boolean,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  setWorkflowLaunchPolicy?: (policy: "reject" | "queue") => Promise<{ session: RuntimeSession }>
  listQueuedWorkflowLaunches?: () => Promise<QueuedWorkflowLaunch[]>
  removeQueuedWorkflowLaunch?: (queueItemRef: string) => Promise<QueuedWorkflowLaunchPayload>
  clearQueuedWorkflowLaunches?: () => Promise<{ queued_launches: QueuedWorkflowLaunch[]; session: RuntimeSession }>
  listWorkflowRuns?: (workflowRef?: string | null) => Promise<WorkflowRun[]>
  cancelWorkflowRun?: (workflowRunRef: string) => Promise<WorkflowRunCancelPayload>
  resumeWorkflowRun?: (workflowRunRef: string) => Promise<WorkflowRunResumePayload>
  updateWorkflowNodeInstructions?: (
    workflowRef: string,
    nodeId: string,
    instructions: string | null,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeCanCompleteRun?: (
    workflowRef: string,
    nodeId: string,
    canCompleteWorkflowRun: boolean,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeCanEmitIntermediateOutput?: (
    workflowRef: string,
    nodeId: string,
    canEmitIntermediateWorkflowRunOutput: boolean,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeIntermediateOutputSchema?: (
    workflowRef: string,
    nodeId: string,
    intermediateOutputSchemaRef: string | null,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeMaxTurns?: (
    workflowRef: string,
    nodeId: string,
    maxTurns: number | null,
  ) => Promise<WorkflowNodePayload>
  setWorkflowRunOutputSchema?: (
    workflowRef: string,
    runOutputSchemaRef: string | null,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  setWorkflowIntermediateOutputSchema?: (
    workflowRef: string,
    intermediateOutputSchemaRef: string | null,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  openWorkflowNodeInstructionsEditor?: (workflowId: string, nodeId: string, draft: string) => void
  closeWorkflowNodeInstructionsEditor?: () => void
  getWorkflowNodeInstructionsDraft?: () => string
  getWorkflowNodeInstructionsContext?: () => { workflowId: string; nodeId: string } | null
  openWorkflowTerminalPanel?: (workflowId: string) => void
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  refreshSplitPaneFocusRepaint: () => void
}

export function createCommandActionHandlers(deps: CommandActionDeps) {
  const currentWorkflowLaunchPolicy = (): "reject" | "queue" => {
    const policy =
      deps.sessionState().workflow_launch_policy ??
      deps.sessionState().config_state?.values?.[WORKFLOW_LAUNCH_POLICY_CONFIG_KEY] ??
      "reject"
    return policy === "queue" ? "queue" : "reject"
  }

  const parseWatchdogIntervalSeconds = (value: string | undefined): number | null => {
    if (!value) return null
    const match = value.trim().toLowerCase().match(/^(\d+)(s|m|h|d)$/)
    if (!match) return null
    const amount = Number(match[1])
    const unit = match[2]
    if (!Number.isFinite(amount) || amount <= 0) return null
    const multiplier = unit === "s" ? 1 : unit === "m" ? 60 : unit === "h" ? 3600 : 86400
    return amount * multiplier
  }

  const formatQueuedWorkflowLaunch = (queuedLaunch: QueuedWorkflowLaunch): string =>
    [
      queuedLaunch.id,
      `[${queuedLaunch.source}]`,
      `workflow=${queuedLaunch.workflow_id}`,
      `endpoint=${queuedLaunch.endpoint_id}`,
      queuedLaunch.watchdog_id ? `watchdog=${queuedLaunch.watchdog_id}` : null,
      `queued_at=${queuedLaunch.queued_at_ms}`,
      queuedLaunch.invocation_prompt && queuedLaunch.invocation_prompt.trim() !== ""
        ? `prompt=${JSON.stringify(
            queuedLaunch.invocation_prompt.length > 50
              ? `${queuedLaunch.invocation_prompt.slice(0, 50)}...`
              : queuedLaunch.invocation_prompt,
          )}`
        : null,
    ]
      .filter((value): value is string => Boolean(value))
      .join(" ")
  const parseWatchdogMaxWakeups = (value: string | undefined): number | null | undefined => {
    if (value == null) return undefined
    const normalized = value.trim().toLowerCase()
    if (!normalized) return undefined
    if (normalized === "null" || normalized === "unbounded") return null
    const numeric = Number(normalized)
    if (!Number.isFinite(numeric) || numeric <= 0 || !Number.isInteger(numeric)) {
      return undefined
    }
    return numeric
  }
  const hasDuplicateWorkflowEdge = (
    workflow: WorkflowDefinition,
    fromNodeId: string,
    toNodeId: string,
  ) => {
    return (workflow.edges ?? []).some((edge) => (
      edge.from_node_id === fromNodeId && edge.to_node_id === toNodeId
    ))
  }
  const workflowEdgeAddUsage = "usage: /workflow edge add [workflow-ref] <from-node-id|from-agent-ref> <to-node-id|to-agent-ref>"
  const selectedWorkflowRef = () => deps.selectedWorkflowId?.() ?? null
  const workflowRefOrSelected = (workflowRef: string | null | undefined) => workflowRef ?? selectedWorkflowRef()
  const firstWorkflowArgIsExplicit = (workflowRef: string | undefined) => (
    !selectedWorkflowRef() || isKnownWorkflowReference(workflowRef)
  )
  const isKnownWorkflowReference = (reference: string | undefined) => {
    if (!reference) {
      return false
    }
    if (reference === selectedWorkflowRef()) {
      return true
    }
    return (deps.sessionState().workflows ?? []).some((workflow) => (
      workflow.id === reference || workflow.alias === reference
    ))
  }

  const resolveWorkflowNodeReference = (
    workflow: WorkflowDefinition,
    workflowRef: string,
    reference: string,
  ): { nodeId: string } | { error: string } => {
    const nodes = workflow.nodes ?? []
    const nodeMatch = nodes.find((node) => node.id === reference)
    if (nodeMatch) {
      return { nodeId: nodeMatch.id }
    }

    const resolvedAgent = deps.resolveSessionAgent(reference)
    if (!resolvedAgent.agent) {
      if (resolvedAgent.error?.startsWith("multiple agents match")) {
        return { error: resolvedAgent.error }
      }
      return { nodeId: reference }
    }

    const matches = nodes.filter((node) => node.agent_id === resolvedAgent.agent?.id)
    if (matches.length === 1) {
      const [node] = matches
      return { nodeId: node?.id ?? reference }
    }
    if (matches.length > 1) {
      return {
        error: `agent '${reference}' maps to multiple nodes in workflow '${workflow.id}'; use explicit node ids`,
      }
    }
    return {
      error: `agent '${reference}' is not a node in workflow '${workflowRef}'; add it first with /workflow node add <workflow-ref> <agent-id>`,
    }
  }

  const addAllRemainingWorkflowNodes = async (workflowRef: string) => {
    const resolved = await deps.resolveWorkflow(workflowRef)
    deps.upsertWorkflowDefinition(resolved.workflow)

    const existingAgentIds = new Set((resolved.workflow.nodes ?? []).map((node) => node.agent_id))
    const agentsToAdd = deps.sessionState().agents.filter((agent) => !existingAgentIds.has(agent.id))
    if (agentsToAdd.length === 0) {
      deps.selectWorkflowCanvas(resolved.workflow.id)
      deps.flashFooter(`workflow ${resolved.workflow.id} already has nodes for all session agents`, "info")
      return
    }

    let latestWorkflow = resolved.workflow
    for (const agent of agentsToAdd) {
      const payload = await deps.addWorkflowNode(latestWorkflow.id, agent.id)
      latestWorkflow = payload.workflow
      deps.applySessionState(payload.session)
      deps.upsertWorkflowDefinition(payload.workflow)
    }

    deps.selectWorkflowCanvas(latestWorkflow.id)
    deps.flashFooter(
      `added ${agentsToAdd.length} workflow node${agentsToAdd.length === 1 ? "" : "s"} for ${agentsToAdd.map((agent) => deps.formatAgentLabel(agent)).join(", ")}`,
      "info",
    )
  }

  const formatWorkflowRunSummary = (workflowRun: WorkflowRun) => {
    const failureSummary = (workflowRun.failure_events?.length ?? 0) > 0
      ? `, failures ${workflowRun.failure_events?.length ?? 0}`
      : ""
    return `${workflowRun.id} [${String(workflowRun.status).toLowerCase()}${failureSummary}]`
  }

  const currentWorkspaceTarget = () => deps.getWorkspaceTarget?.() ?? deps.workspace
  const currentWorktreeTarget = () => deps.getWorktreeTarget?.() ?? deps.worktree
  const setWorkspaceTarget = (workspace: string) => deps.setWorkspaceTarget?.(workspace)
  const setWorktreeTarget = (worktree: string) => deps.setWorktreeTarget?.(worktree)
  const workspaceCommandDeps = (): WorkspaceCommandHandlerDeps => ({
    ...deps,
    currentWorkspaceTarget,
    currentWorktreeTarget,
    setWorkspaceTarget,
    setWorktreeTarget,
    baseWorktree: deps.worktree,
    hasDynamicWorktreeTarget: Boolean(deps.getWorktreeTarget),
  })
  const agentCommandDeps = (): AgentCommandHandlerDeps => ({
    ...deps,
    currentWorktreeTarget,
  })

  const handleSessionCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "session" }>,
  ): Promise<boolean> => {
    return handleSessionSlashCommand({ ...deps, currentWorkspaceTarget, currentWorktreeTarget }, command)
  }

  const handleProviderCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "provider" }>,
  ): Promise<void> => {
    await handleProviderSlashCommand(deps, command)
  }

  const handleModelCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "model" }>,
  ): Promise<void> => {
    await handleModelSlashCommand(deps, command)
  }

  const handleVariantCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "variant" }>,
  ): Promise<void> => {
    await handleVariantSlashCommand(deps, command)
  }

  const handleViewCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "view" }>,
  ): Promise<void> => {
    await handleViewSlashCommand(deps, command)
  }

  const handleCycleAgentFocus = async (): Promise<void> => {
    await cycleAgentFocusCommand(agentCommandDeps())
  }

  const handleAgentCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "agent" }>,
  ): Promise<void> => {
    await handleAgentSlashCommand(agentCommandDeps(), command)
  }


  const handleRelayCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "relay" }>,
  ): Promise<void> => handleRelaySlashCommand(deps, command)

  const handleCloudCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "cloud" }>,
  ): Promise<void> => handleCloudSlashCommand(deps, command)

  const handleConfigCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "config" }>,
  ): Promise<void> => {
    await handleConfigSlashCommand(deps, command)
  }

  const handleMachineCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "machine" }>,
  ): Promise<void> => {
    await handleRemoteMachineSlashCommand(deps, command)
  }

  const handleSliceCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "slice" }>,
  ): Promise<void> => {
    await handleSliceSlashCommand({ ...deps, currentWorktreeTarget }, command)
  }

  const handleKernelCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "kernel" }>,
  ): Promise<void> => {
    await handleKernelSlashCommand(deps, command)
  }

  const handleMcpCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "mcp" }>,
  ): Promise<void> => {
    await handleMcpSlashCommand(deps, command)
  }

  const handleSkillCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "skill" }>,
  ): Promise<void> => {
    await handleSkillSlashCommand(deps, command)
  }

  const handleWorkspaceCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "workspace" }>,
  ): Promise<void> => {
    await handleWorkspaceSlashCommand(workspaceCommandDeps(), command)
  }

  const handleWorktreeCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "worktree" }>,
  ): Promise<void> => {
    await handleWorktreeSlashCommand(workspaceCommandDeps(), command)
  }

  const handleWorkflowCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "workflow" }>,
  ): Promise<void> => {
    if (!deps.isAttached()) {
      deps.flashFooter("must be attached to a session to manage workflows", "error")
      return
    }

    const args = command.args
    const subcommand = args[0]

    if (!subcommand) {
      const knownWorkflows = deps.sessionState().workflows ?? []
      if (knownWorkflows.length > 0) {
        if (!deps.workflowScreenActive()) {
          deps.selectWorkflowCanvas(knownWorkflows[0]?.id ?? null)
          deps.showWorkflowScreen()
        }
        return
      }

      if (!deps.workflowScreenActive()) {
        deps.showWorkflowScreen()
        return
      }

      const workflows = await deps.listWorkflows()
      if (workflows.length > 0) {
        deps.replaceWorkflowDefinitions(workflows)
        deps.selectWorkflowCanvas(workflows[0]?.id ?? null)
      } else {
        // If already on workflow screen but no workflows exist, create one
        const payload = await deps.createWorkflow(null)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.applySessionState(payload.session)
        deps.flashFooter(`created workflow ${payload.workflow.id}`, "info")
      }
      return
    }

    if (subcommand === "list") {
      const workflows = await deps.listWorkflows()
      deps.replaceWorkflowDefinitions(workflows)
      deps.flashFooter(
        workflows.length === 0
          ? "no workflows in workspace"
          : `workflows: ${workflows.map((workflow) => workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id).join(", ")}`,
        "info",
      )
      return
    }

    if (subcommand === "show") {
      const workflowRef = workflowRefOrSelected(args[1])
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow show [workflow-ref]", "error")
        return
      }
      const payload = await deps.resolveWorkflow(workflowRef)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.showWorkflowScreen()
      deps.flashFooter(
        `workflow ${payload.workflow.id}${payload.workflow.alias ? ` (${payload.workflow.alias})` : ""}`,
        "info",
      )
      return
    }

    if (subcommand === "new") {
      const payload = await deps.createWorkflow(args[1] ?? null)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.showWorkflowScreen()
      deps.applySessionState(payload.session)
      deps.flashFooter(
        `created workflow ${payload.workflow.id}${payload.workflow.alias ? ` (${payload.workflow.alias})` : ""}`,
        "info",
      )
      return
    }

    if (subcommand === "run" || subcommand === "start") {
      const firstArg = args[1]
      const explicitWorkflowRef = firstWorkflowArgIsExplicit(firstArg) ? firstArg : null
      const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
      const endpointRef = explicitWorkflowRef ? args[2] : firstArg
      const prompt = args.slice(explicitWorkflowRef ? 3 : 2).join(" ").trim()
      if (!workflowRef || !endpointRef) {
        deps.flashFooter("usage: /workflow run|start [workflow-ref] <endpoint-ref> [prompt]", "error")
        return
      }
      if (!deps.invokeWorkflowEndpoint) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const payload = await deps.invokeWorkflowEndpoint(workflowRef, endpointRef, prompt || null)
      deps.applySessionState(payload.session)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.showWorkflowScreen()
      if ("workflow_run" in payload) {
        deps.flashFooter(
          `started workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
          "info",
        )
      } else {
        deps.flashFooter(
          `queued workflow launch ${payload.queued_launch.id}; active workflow run in session`,
          "info",
        )
      }
      return
    }

    if (subcommand === "launch-policy") {
      const value = args[1]?.trim().toLowerCase()
      if (!value) {
        deps.flashFooter(`workflow launch policy: ${currentWorkflowLaunchPolicy()}`, "info")
        return
      }
      if (value !== "reject" && value !== "queue") {
        deps.flashFooter("usage: /workflow launch-policy <reject|queue>", "error")
        return
      }
      if (!deps.setWorkflowLaunchPolicy) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const payload = await deps.setWorkflowLaunchPolicy(value)
      deps.applySessionState(payload.session)
      deps.flashFooter(`workflow launch policy set to ${value}`, "info")
      return
    }

    if (subcommand === "flush-context") {
      const firstArg = args[1]?.trim().toLowerCase()
      const selectedRef = selectedWorkflowRef()
      const firstArgIsValue = firstArg === "true" || firstArg === "false"
      const workflowRef = workflowRefOrSelected(firstArgIsValue ? null : args[1])
      const value = (firstArgIsValue ? args[1] : args[2])?.trim().toLowerCase()
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow flush-context [workflow-ref] [true|false]", "error")
        return
      }
      if (firstArgIsValue && !selectedRef) {
        deps.flashFooter("usage: /workflow flush-context [workflow-ref] [true|false]", "error")
        return
      }
      const resolved = await deps.resolveWorkflow(workflowRef)
      deps.upsertWorkflowDefinition(resolved.workflow)
      if (!value) {
        deps.flashFooter(
          `workflow ${resolved.workflow.id} flush-context: ${(resolved.workflow.flush_agent_context_before_run ?? true) ? "true" : "false"}`,
          "info",
        )
        return
      }
      if (value !== "true" && value !== "false") {
        deps.flashFooter("usage: /workflow flush-context [workflow-ref] [true|false]", "error")
        return
      }
      if (!deps.setWorkflowFlushContext) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const payload = await deps.setWorkflowFlushContext(
        resolved.workflow.id,
        value === "true",
      )
      deps.applySessionState(payload.session)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.flashFooter(
        `workflow ${payload.workflow.id} flush-context set to ${payload.workflow.flush_agent_context_before_run ? "true" : "false"}`,
        "info",
      )
      return
    }

    if (subcommand === "run-output-schema") {
      const explicitWorkflowRef = firstWorkflowArgIsExplicit(args[1]) ? args[1] : null
      const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
      const value = explicitWorkflowRef ? args[2] : args[1]
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow run-output-schema [workflow-ref] [schema-ref|none]", "error")
        return
      }
      const resolved = await deps.resolveWorkflow(workflowRef)
      deps.upsertWorkflowDefinition(resolved.workflow)
      if (value === undefined) {
        deps.flashFooter(
          `workflow ${resolved.workflow.id} run-output-schema: ${resolved.workflow.run_output_schema_ref ?? "none"}`,
          "info",
        )
        return
      }
      if (!deps.setWorkflowRunOutputSchema) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const schemaRef = value.trim().toLowerCase() === "none" ? null : value
      const payload = await deps.setWorkflowRunOutputSchema(resolved.workflow.id, schemaRef)
      deps.applySessionState(payload.session)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.flashFooter(
        `workflow ${payload.workflow.id} run-output-schema set to ${payload.workflow.run_output_schema_ref ?? "none"}`,
        "info",
      )
      return
    }

    if (subcommand === "intermediate-output-schema") {
      const explicitWorkflowRef = firstWorkflowArgIsExplicit(args[1]) ? args[1] : null
      const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
      const value = explicitWorkflowRef ? args[2] : args[1]
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow intermediate-output-schema [workflow-ref] [schema-ref|none]", "error")
        return
      }
      const resolved = await deps.resolveWorkflow(workflowRef)
      deps.upsertWorkflowDefinition(resolved.workflow)
      if (value === undefined) {
        deps.flashFooter(
          `workflow ${resolved.workflow.id} intermediate-output-schema: ${resolved.workflow.intermediate_output_schema_ref ?? "none"}`,
          "info",
        )
        return
      }
      if (!deps.setWorkflowIntermediateOutputSchema) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const schemaRef = value.trim().toLowerCase() === "none" ? null : value
      const payload = await deps.setWorkflowIntermediateOutputSchema(resolved.workflow.id, schemaRef)
      deps.applySessionState(payload.session)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.flashFooter(
        `workflow ${payload.workflow.id} intermediate-output-schema set to ${payload.workflow.intermediate_output_schema_ref ?? "none"}`,
        "info",
      )
      return
    }

    if (subcommand === "queue") {
      const action = args[1]?.trim().toLowerCase() ?? "list"
      if (action === "list") {
        if (!deps.listQueuedWorkflowLaunches) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const queuedLaunches = await deps.listQueuedWorkflowLaunches()
        deps.flashFooter(
          queuedLaunches.length === 0
            ? "workflow queue is empty"
            : `workflow queue: ${queuedLaunches.map(formatQueuedWorkflowLaunch).join(", ")}`,
          "info",
        )
        return
      }
      if (action === "flush") {
        if (!deps.clearQueuedWorkflowLaunches) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const payload = await deps.clearQueuedWorkflowLaunches()
        deps.applySessionState(payload.session)
        deps.flashFooter(
          payload.queued_launches.length === 0
            ? "workflow queue already empty"
            : `cleared ${payload.queued_launches.length} queued workflow launch${payload.queued_launches.length === 1 ? "" : "es"}`,
          "info",
        )
        return
      }
      if (action === "remove") {
        const queueItemRef = args[2]
        if (!queueItemRef) {
          deps.flashFooter("usage: /workflow queue remove <queue-item-ref>", "error")
          return
        }
        if (!deps.removeQueuedWorkflowLaunch) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const payload = await deps.removeQueuedWorkflowLaunch(queueItemRef)
        deps.applySessionState(payload.session)
        deps.flashFooter(`removed queued workflow launch ${payload.queued_launch.id}`, "info")
        return
      }
      deps.flashFooter("usage: /workflow queue [list|flush|remove <queue-item-ref>]", "error")
      return
    }

    if (subcommand === "max-turns") {
      const value = args[1]
      if (!value) {
        const current = deps.sessionState().config_state?.values?.[WORKFLOW_MAX_TURNS_CONFIG_KEY]
        const label = current && current.trim() !== "" ? current : "unset"
        deps.flashFooter(`workflow max turns: ${label}`, "info")
        return
      }
      if (!deps.attachmentState()) {
        deps.flashFooter("must be attached to set workflow max turns", "error")
        return
      }
      const normalized = value.trim().toLowerCase()
      const nextValue =
        normalized === "off" || normalized === "0"
          ? "0"
          : Number.isFinite(Number(normalized))
            ? String(Math.max(1, Math.floor(Number(normalized))))
            : null
      if (!nextValue) {
        deps.flashFooter("usage: /workflow max-turns <count|off>", "error")
        return
      }
      const payload = await deps.updateSessionConfig(
        deps.sessionState().id,
        deps.attachmentState()!.id,
        { [WORKFLOW_MAX_TURNS_CONFIG_KEY]: nextValue },
        false,
      )
      deps.applySessionState(payload.session)
      deps.flashFooter(
        nextValue === "0"
          ? "workflow max turns disabled"
          : `workflow max turns set to ${nextValue}`,
        "info",
      )
      return
    }

    if (subcommand === "runs") {
      if (!deps.listWorkflowRuns) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const workflowRef = args[1] ?? null
      const workflowRuns = await deps.listWorkflowRuns(workflowRef)
      deps.flashFooter(
        workflowRuns.length === 0
          ? (workflowRef ? `no workflow runs for ${workflowRef}` : "no workflow runs in session")
          : `workflow runs: ${workflowRuns.map(formatWorkflowRunSummary).join(", ")}`,
        "info",
      )
      return
    }

    if (subcommand === "cancel") {
      const workflowRunRef = args[1]
      if (!workflowRunRef) {
        deps.flashFooter("usage: /workflow cancel <run-ref>", "error")
        return
      }
      if (!deps.cancelWorkflowRun) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const payload = await deps.cancelWorkflowRun(workflowRunRef)
      deps.applySessionState(payload.session)
      deps.flashFooter(
        `cancelled workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
        "info",
      )
      return
    }

    if (subcommand === "resume") {
      const workflowRunRef = args[1]
      if (!workflowRunRef) {
        deps.flashFooter("usage: /workflow resume <run-ref>", "error")
        return
      }
      if (!deps.resumeWorkflowRun) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const payload = await deps.resumeWorkflowRun(workflowRunRef)
      deps.applySessionState(payload.session)
      deps.flashFooter(
        `resumed workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
        "info",
      )
      return
    }

    if (subcommand === "terminal") {
      const workflowRef = workflowRefOrSelected(args[1]) ?? deps.sessionState().workflows?.[0]?.id ?? null
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow terminal [workflow-ref]", "error")
        return
      }
      const payload = await deps.resolveWorkflow(workflowRef)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.showWorkflowScreen()
      deps.openWorkflowTerminalPanel?.(payload.workflow.id)
      deps.flashFooter(`opened workflow terminal for ${payload.workflow.id}`, "info")
      return
    }

    if (subcommand === "add" && args[1] === "node") {
      const explicitWorkflowRef = args.length >= 4 ? args[2] : null
      const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
      const target = explicitWorkflowRef ? args[3] : args[2]
      if (!workflowRef || target !== "all") {
        deps.flashFooter("usage: /workflow add node [workflow-ref] all", "error")
        return
      }
      await addAllRemainingWorkflowNodes(workflowRef)
      return
    }

    if (subcommand === "node") {
      const action = args[1]
      if (action === "add") {
        const explicitWorkflowRef = args.length >= 4 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const agentRef = explicitWorkflowRef ? args[3] : args[2]
        if (!workflowRef || !agentRef) {
          deps.flashFooter("usage: /workflow node add [workflow-ref] <agent-id|all>", "error")
          return
        }
        if (agentRef === "all") {
          await addAllRemainingWorkflowNodes(workflowRef)
          return
        }
        const resolvedAgent = deps.resolveSessionAgent(agentRef)
        if (!resolvedAgent.agent || resolvedAgent.error) {
          deps.flashFooter(resolvedAgent.error ?? `agent '${agentRef}' not found`, "error")
          return
        }
        const payload = await deps.addWorkflowNode(workflowRef, resolvedAgent.agent.id)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(`added workflow node ${payload.node.id} for agent ${deps.formatAgentLabel(resolvedAgent.agent)}`, "info")
        return
      }
      if (action === "remove") {
        const explicitWorkflowRef = args.length >= 4 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[3] : args[2]
        if (!workflowRef || !nodeId) {
          deps.flashFooter("usage: /workflow node remove [workflow-ref] <node-id>", "error")
          return
        }
        const payload = await deps.removeWorkflowNode(workflowRef, nodeId)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(`removed workflow node ${payload.node.id}`, "info")
        return
      }
      if (action === "instructions") {
        const instructionsAction = args[2]
        if (!instructionsAction) {
          deps.flashFooter(
            "usage: /workflow node instructions show|set|save|close [workflow-ref] <node-id> [file]",
            "error",
          )
          return
        }
        if (instructionsAction === "close") {
          deps.closeWorkflowNodeInstructionsEditor?.()
          deps.flashFooter("closed node instructions editor", "info")
          return
        }
        if (instructionsAction === "save") {
          const context = deps.getWorkflowNodeInstructionsContext?.()
          if (!context || !deps.updateWorkflowNodeInstructions || !deps.getWorkflowNodeInstructionsDraft) {
            deps.flashFooter("no workflow node instructions editor is open", "error")
            return
          }
          const payload = await deps.updateWorkflowNodeInstructions(
            context.workflowId,
            context.nodeId,
            deps.getWorkflowNodeInstructionsDraft(),
          )
          deps.applySessionState(payload.session)
          deps.upsertWorkflowDefinition(payload.workflow)
          deps.closeWorkflowNodeInstructionsEditor?.()
          deps.flashFooter(`saved node instructions for ${payload.node.id}`, "info")
          return
        }
        const explicitWorkflowRef = firstWorkflowArgIsExplicit(args[3]) ? args[3] : null
        const instructionsWorkflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[4] : args[3]
        const fileRef = explicitWorkflowRef ? args[5] : args[4]
        if (!instructionsWorkflowRef || !nodeId) {
          deps.flashFooter(
            "usage: /workflow node instructions show|set [workflow-ref] <node-id> [file]",
            "error",
          )
          return
        }
        const resolved = await deps.resolveWorkflow(instructionsWorkflowRef)
        deps.upsertWorkflowDefinition(resolved.workflow)
        const node = resolved.workflow.nodes?.find((entry) => entry.id === nodeId)
        if (!node) {
          deps.flashFooter(`workflow node ${nodeId} not found`, "error")
          return
        }
        if (instructionsAction === "show") {
          deps.openWorkflowNodeInstructionsEditor?.(
            resolved.workflow.id,
            node.id,
            node.instructions ?? "",
          )
          deps.selectWorkflowCanvas(resolved.workflow.id)
          deps.flashFooter(`opened node ${node.id} instructions in the I/O panel`, "info")
          return
        }
        if (instructionsAction !== "set") {
          deps.flashFooter(
            "usage: /workflow node instructions show|set|save|close [workflow-ref] <node-id> [file]",
            "error",
          )
          return
        }
        if (fileRef) {
          if (!deps.updateWorkflowNodeInstructions) {
            deps.flashFooter("workflow instructions unavailable", "error")
            return
          }
          const content = await readFile(resolvePath(currentWorkspaceTarget(), fileRef), "utf8")
          const payload = await deps.updateWorkflowNodeInstructions(resolved.workflow.id, node.id, content)
          deps.applySessionState(payload.session)
          deps.upsertWorkflowDefinition(payload.workflow)
          deps.flashFooter(`updated node instructions for ${payload.node.id}`, "info")
          return
        }
        deps.openWorkflowNodeInstructionsEditor?.(resolved.workflow.id, node.id, node.instructions ?? "")
        deps.selectWorkflowCanvas(resolved.workflow.id)
        deps.flashFooter("editing node instructions in the I/O panel; submit text then /workflow node instructions save", "info")
        return
      }
      if (action === "can-complete-run") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[3] : args[2]
        const value = (explicitWorkflowRef ? args[4] : args[3])?.trim().toLowerCase()
        if (!workflowRef || !nodeId || (value !== "true" && value !== "false")) {
          deps.flashFooter("usage: /workflow node can-complete-run [workflow-ref] <node-id> <true|false>", "error")
          return
        }
        if (!deps.setWorkflowNodeCanCompleteRun) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const payload = await deps.setWorkflowNodeCanCompleteRun(workflowRef, nodeId, value === "true")
        deps.applySessionState(payload.session)
        deps.upsertWorkflowDefinition(payload.workflow)
        deps.flashFooter(
          `workflow node ${payload.node.id} can-complete-run set to ${payload.node.can_complete_workflow_run ? "true" : "false"}`,
          "info",
        )
        return
      }
      if (action === "can-emit-intermediate-output") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[3] : args[2]
        const value = (explicitWorkflowRef ? args[4] : args[3])?.trim().toLowerCase()
        if (!workflowRef || !nodeId || (value !== "true" && value !== "false")) {
          deps.flashFooter("usage: /workflow node can-emit-intermediate-output [workflow-ref] <node-id> <true|false>", "error")
          return
        }
        if (!deps.setWorkflowNodeCanEmitIntermediateOutput) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const payload = await deps.setWorkflowNodeCanEmitIntermediateOutput(workflowRef, nodeId, value === "true")
        deps.applySessionState(payload.session)
        deps.upsertWorkflowDefinition(payload.workflow)
        deps.flashFooter(
          `workflow node ${payload.node.id} can-emit-intermediate-output set to ${payload.node.can_emit_intermediate_run_output ? "true" : "false"}`,
          "info",
        )
        return
      }
      if (action === "intermediate-output-schema") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[3] : args[2]
        const value = explicitWorkflowRef ? args[4] : args[3]
        if (!workflowRef || !nodeId || value === undefined) {
          deps.flashFooter("usage: /workflow node intermediate-output-schema [workflow-ref] <node-id> <schema-ref|none>", "error")
          return
        }
        if (!deps.setWorkflowNodeIntermediateOutputSchema) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const schemaRef = value.trim().toLowerCase() === "none" ? null : value
        const payload = await deps.setWorkflowNodeIntermediateOutputSchema(workflowRef, nodeId, schemaRef)
        deps.applySessionState(payload.session)
        deps.upsertWorkflowDefinition(payload.workflow)
        deps.flashFooter(
          `workflow node ${payload.node.id} intermediate-output-schema set to ${payload.node.intermediate_output_schema_ref ?? "none"}`,
          "info",
        )
        return
      }
      if (action === "max-turns") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[3] : args[2]
        const value = (explicitWorkflowRef ? args[4] : args[3])?.trim().toLowerCase()
        if (!workflowRef || !nodeId || !value) {
          deps.flashFooter("usage: /workflow node max-turns [workflow-ref] <node-id> <count|none>", "error")
          return
        }
        if (!deps.setWorkflowNodeMaxTurns) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        let maxTurns: number | null
        if (value === "none") {
          maxTurns = null
        } else {
          const parsed = Number.parseInt(value, 10)
          if (!Number.isFinite(parsed) || parsed <= 0) {
            deps.flashFooter("usage: /workflow node max-turns [workflow-ref] <node-id> <count|none>", "error")
            return
          }
          maxTurns = parsed
        }
        const payload = await deps.setWorkflowNodeMaxTurns(workflowRef, nodeId, maxTurns)
        deps.applySessionState(payload.session)
        deps.upsertWorkflowDefinition(payload.workflow)
        deps.flashFooter(
          `workflow node ${payload.node.id} max-turns set to ${payload.node.max_turns ?? "none"}`,
          "info",
        )
        return
      }
      deps.flashFooter(
        "usage: /workflow node add [workflow-ref] <agent-id|all> | remove [workflow-ref] <node-id> | instructions ... | can-complete-run [workflow-ref] <node-id> <true|false> | can-emit-intermediate-output [workflow-ref] <node-id> <true|false> | intermediate-output-schema [workflow-ref] <node-id> <schema-ref|none> | max-turns [workflow-ref] <node-id> <count|none>",
        "error",
      )
      return
    }

    if (subcommand === "edge") {
      const action = args[1]
      if (action === "add") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = explicitWorkflowRef ?? selectedWorkflowRef()
        const fromRef = explicitWorkflowRef ? args[3] : args[2]
        const toRef = explicitWorkflowRef ? args[4] : args[3]
        if (!workflowRef || !fromRef || !toRef) {
          deps.flashFooter(workflowEdgeAddUsage, "error")
          return
        }
        if (!explicitWorkflowRef && isKnownWorkflowReference(fromRef)) {
          deps.flashFooter(workflowEdgeAddUsage, "error")
          return
        }
        const resolvedWorkflow = await deps.resolveWorkflow(workflowRef)
        deps.upsertWorkflowDefinition(resolvedWorkflow.workflow)
        const fromNode = resolveWorkflowNodeReference(resolvedWorkflow.workflow, workflowRef, fromRef)
        if ("error" in fromNode) {
          deps.flashFooter(fromNode.error, "error")
          return
        }
        const toNode = resolveWorkflowNodeReference(resolvedWorkflow.workflow, workflowRef, toRef)
        if ("error" in toNode) {
          deps.flashFooter(toNode.error, "error")
          return
        }
        if (fromNode.nodeId === toNode.nodeId) {
          deps.flashFooter("workflow edges must connect two different nodes", "error")
          return
        }
        if (hasDuplicateWorkflowEdge(resolvedWorkflow.workflow, fromNode.nodeId, toNode.nodeId)) {
          deps.flashFooter("workflow edge already exists between those nodes", "error")
          return
        }
        const payload = await deps.addWorkflowEdge(workflowRef, fromNode.nodeId, toNode.nodeId)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(`added workflow edge ${payload.edge.id}`, "info")
        return
      }
      if (action === "remove") {
        const explicitWorkflowRef = args.length >= 4 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const edgeId = explicitWorkflowRef ? args[3] : args[2]
        if (!workflowRef || !edgeId) {
          deps.flashFooter("usage: /workflow edge remove [workflow-ref] <edge-id>", "error")
          return
        }
        const payload = await deps.removeWorkflowEdge(workflowRef, edgeId)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(`removed workflow edge ${payload.edge.id}`, "info")
        return
      }
      deps.flashFooter(
        `${workflowEdgeAddUsage} | remove [workflow-ref] <edge-id>`,
        "error",
      )
      return
    }

    if (subcommand === "endpoint") {
      const action = args[1]
      if (action === "new") {
        const explicitWorkflowRef = firstWorkflowArgIsExplicit(args[2]) ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const entryNodeId = explicitWorkflowRef ? args[3] : args[2]
        const alias = (explicitWorkflowRef ? args[4] : args[3]) ?? null
        if (!workflowRef || !entryNodeId) {
          deps.flashFooter(
            "usage: /workflow endpoint new [workflow-ref] <entry-node-id> [alias]",
            "error",
          )
          return
        }
        const payload = await deps.createWorkflowEndpoint(workflowRef, entryNodeId, alias)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(`created workflow endpoint ${payload.endpoint.id}`, "info")
        return
      }
      if (action === "alias") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const endpointRef = explicitWorkflowRef ? args[3] : args[2]
        const alias = explicitWorkflowRef ? args[4] : args[3]
        if (!workflowRef || !endpointRef || !alias) {
          deps.flashFooter(
            "usage: /workflow endpoint alias [workflow-ref] <endpoint-ref> <alias>",
            "error",
          )
          return
        }
        const payload = await deps.assignWorkflowEndpointAlias(workflowRef, endpointRef, alias)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(
          `workflow endpoint ${payload.endpoint.id} aliased as ${payload.endpoint.alias}`,
          "info",
        )
        return
      }
      if (action === "bind") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const endpointRef = explicitWorkflowRef ? args[3] : args[2]
        const entryNodeId = explicitWorkflowRef ? args[4] : args[3]
        if (!workflowRef || !endpointRef || !entryNodeId) {
          deps.flashFooter(
            "usage: /workflow endpoint bind [workflow-ref] <endpoint-ref> <entry-node-id>",
            "error",
          )
          return
        }
        const payload = await deps.bindWorkflowEndpoint(workflowRef, endpointRef, entryNodeId)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(
          `workflow endpoint ${payload.endpoint.id} bound to node ${payload.endpoint.entry_node_id}`,
          "info",
        )
        return
      }
      deps.flashFooter(
        "usage: /workflow endpoint new [workflow-ref] <entry-node-id> [alias] | alias [workflow-ref] <endpoint-ref> <alias> | bind [workflow-ref] <endpoint-ref> <entry-node-id>",
        "error",
      )
      return
    }

    if (subcommand === "watchdog") {
      const action = args[1]
      if (action === "add") {
        if (!deps.createWorkflowWatchdog) {
          deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
          return
        }
        const explicitWorkflowRef = args[4] === "every" ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const endpointRef = explicitWorkflowRef ? args[3] : args[2]
        const everyLiteral = explicitWorkflowRef ? args[4] : args[3]
        const intervalLiteral = explicitWorkflowRef ? args[5] : args[4]
        const optionStartIndex = explicitWorkflowRef ? 6 : 5
        const hasPolicyArg = args[optionStartIndex] === "skip" || args[optionStartIndex] === "queue"
        const policy = (hasPolicyArg ? args[optionStartIndex] : "skip") as "skip" | "queue"
        const maxWakeupsKeyword = args[optionStartIndex + (hasPolicyArg ? 1 : 0)]
        const hasMaxWakeupsArg = maxWakeupsKeyword === "max-wakeups"
        const maxWakeupsLiteral = hasMaxWakeupsArg ? args[optionStartIndex + (hasPolicyArg ? 2 : 1)] : undefined
        const maxWakeups = hasMaxWakeupsArg ? parseWatchdogMaxWakeups(maxWakeupsLiteral) : undefined
        const prompt = args
          .slice(optionStartIndex + (hasPolicyArg ? 1 : 0) + (hasMaxWakeupsArg ? 2 : 0))
          .join(" ")
          .trim() || "Run the workflow exactly as instructed."
        if (!workflowRef || !endpointRef || everyLiteral !== "every") {
          deps.flashFooter(
            "usage: /workflow watchdog add [workflow-ref] <endpoint-ref> every <Ns|Nm|Nh|Nd> [skip|queue] [max-wakeups <n|null>] [prompt]",
            "error",
          )
          return
        }
        const intervalSeconds = parseWatchdogIntervalSeconds(intervalLiteral)
        if (!intervalSeconds) {
          deps.flashFooter("watchdog interval must be like 30s, 5m, 1h, or 1d", "error")
          return
        }
        if (hasMaxWakeupsArg && maxWakeups === undefined) {
          deps.flashFooter("max-wakeups must be a positive integer or `null`", "error")
          return
        }
        const payload = await deps.createWorkflowWatchdog(
          workflowRef,
          endpointRef,
          intervalSeconds,
          prompt,
          policy,
          maxWakeups,
        )
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow?.id ?? workflowRef)
        deps.flashFooter(`created workflow watchdog ${payload.watchdog.id}`, "info")
        return
      }
      if (action === "list") {
        if (!deps.listWorkflowWatchdogs) {
          deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
          return
        }
        const workflowRef = args[2] ?? null
        const payload = await deps.listWorkflowWatchdogs(workflowRef)
        if (payload.watchdogs.length === 0) {
          deps.flashFooter("no workflow watchdogs configured", "info")
          return
        }
        deps.appendNotice(payload.watchdogs.map((watchdog) =>
          `${watchdog.id} workflow=${watchdog.workflow_id} endpoint=${watchdog.endpoint_id} every=${watchdog.interval_seconds}s policy=${watchdog.policy} enabled=${String(watchdog.enabled)} wakeups=${watchdog.wakeups_executed}/${watchdog.max_wakeups ?? "unbounded"} next=${new Date(watchdog.next_run_at_ms).toISOString()}${watchdog.pending_run ? " pending=true" : ""}`
        ).join("\n"))
        deps.flashFooter(`listed ${payload.watchdogs.length} workflow watchdog(s)`, "info")
        return
      }
      if (action === "enable" || action === "disable") {
        if (!deps.setWorkflowWatchdogEnabled) {
          deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
          return
        }
        const watchdogRef = args[2]
        if (!watchdogRef) {
          deps.flashFooter(`usage: /workflow watchdog ${action} <watchdog-ref>`, "error")
          return
        }
        const payload = await deps.setWorkflowWatchdogEnabled(watchdogRef, action === "enable")
        deps.applySessionState(payload.session)
        deps.flashFooter(
          `${action === "enable" ? "enabled" : "disabled"} workflow watchdog ${payload.watchdog.id}`,
          "info",
        )
        return
      }
      if (action === "remove") {
        if (!deps.removeWorkflowWatchdog) {
          deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
          return
        }
        const watchdogRef = args[2]
        if (!watchdogRef) {
          deps.flashFooter("usage: /workflow watchdog remove <watchdog-ref>", "error")
          return
        }
        const payload = await deps.removeWorkflowWatchdog(watchdogRef)
        deps.applySessionState(payload.session)
        deps.flashFooter(`removed workflow watchdog ${payload.watchdog.id}`, "info")
        return
      }
      deps.flashFooter(
        "usage: /workflow watchdog add [workflow-ref] <endpoint-ref> every <Ns|Nm|Nh|Nd> [skip|queue] [max-wakeups <n|null>] [prompt] | list [workflow-ref] | enable <watchdog-ref> | disable <watchdog-ref> | remove <watchdog-ref>",
        "error",
      )
      return
    }

    const edgeFromRef = args[1]
    const edgeToRef = args[2]
    if (edgeFromRef && edgeToRef) {
      const resolvedWorkflow = await deps.resolveWorkflow(subcommand)
      deps.upsertWorkflowDefinition(resolvedWorkflow.workflow)
      const fromNode = resolveWorkflowNodeReference(resolvedWorkflow.workflow, subcommand, edgeFromRef)
      if ("error" in fromNode) {
        deps.flashFooter(fromNode.error, "error")
        return
      }
      const toNode = resolveWorkflowNodeReference(resolvedWorkflow.workflow, subcommand, edgeToRef)
      if ("error" in toNode) {
        deps.flashFooter(toNode.error, "error")
        return
      }
      if (fromNode.nodeId === toNode.nodeId) {
        deps.flashFooter("workflow edges must connect two different nodes", "error")
        return
      }
      if (hasDuplicateWorkflowEdge(resolvedWorkflow.workflow, fromNode.nodeId, toNode.nodeId)) {
        deps.flashFooter("workflow edge already exists between those nodes", "error")
        return
      }
      const payload = await deps.addWorkflowEdge(subcommand, fromNode.nodeId, toNode.nodeId)
      deps.applySessionState(payload.session)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.showWorkflowScreen()
      deps.flashFooter(`added workflow edge ${payload.edge.id}`, "info")
      return
    }

    const alias = args[1]
    if (!alias) {
      deps.flashFooter(
        "usage: /workflow | /workflow list | /workflow show [workflow-ref] | /workflow new [alias] | /workflow run|start [workflow-ref] <endpoint-ref> [prompt] | /workflow max-turns <count|off> | /workflow run-output-schema [workflow-ref] [schema-ref|none] | /workflow intermediate-output-schema [workflow-ref] [schema-ref|none] | /workflow runs [workflow-ref] | /workflow cancel <run-ref> | /workflow resume <run-ref> | /workflow terminal [workflow-ref] | /workflow <workflow-ref> <alias> | /workflow <workflow-ref> <from-node-or-agent-ref> <to-node-or-agent-ref> | /workflow node ... | /workflow edge ... | /workflow endpoint ...",
        "error",
      )
      return
    }

    const workflow = await deps.assignWorkflowAlias(subcommand, alias)
    if (!workflow) {
      deps.flashFooter(`unknown workflow: ${subcommand}`, "error")
      return
    }
    deps.upsertWorkflowDefinition(workflow)
    deps.showWorkflowScreen()
    deps.flashFooter(`workflow ${workflow.id} aliased as ${workflow.alias}`, "info")
  }

  return {
    handleSessionCommand,
    handleProviderCommand,
    handleModelCommand,
    handleVariantCommand,
    handleViewCommand,
    handleCycleAgentFocus,
    handleAgentCommand,
    handleKernelCommand,
    handleMachineCommand,
    handleSliceCommand,
    handleRelayCommand,
    handleCloudCommand,
    handleConfigCommand,
    handleWorkspaceCommand,
    handleWorktreeCommand,
    handleWorkflowCommand,
    handleMcpCommand,
    handleSkillCommand,
  }
}
