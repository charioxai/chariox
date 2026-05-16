import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  McpImportOutcome,
  RuntimeAttachment,
  QueuedWorkflowLaunch,
  RuntimeProviderRun,
  RuntimeSession,
  SessionConfigState,
  SliceDisplayEndpoint,
  SliceRecord,
  SkillImportOutcome,
  WorkflowEdgeDefinition,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
  WorkflowRun,
  WorkflowWatchdogDefinition,
  WorkspaceLinkDefinition,
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
  parseAgentSpawnOptions,
  parsePlacementOptions,
  prepareLocalGitWorktree,
  resolveExistingLocalDirectory,
  resolveLocalPlacement,
  suggestNamedWorktreePath,
  worktreeAliasConfigPath,
  type LocalGitWorktreeOptions,
  type RemoteGitWorktreePlacement,
} from "./command-worktree-placement.js"
import { responsePaneBindingsMatch, selectResponsePaneAgents } from "./response-panes.js"
import type { SessionListEntry } from "./sessions.js"
import { readFile } from "node:fs/promises"
import { resolve as resolvePath } from "node:path"

const WORKFLOW_MAX_TURNS_CONFIG_KEY = "workflow.max_turns"
const WORKFLOW_LAUNCH_POLICY_CONFIG_KEY = "workflow.launch_policy"
const SESSION_AGENT_MODE_CONFIG_KEY = "agents.mode"
const SESSION_AGENT_PERMISSION_CONFIG_KEY = "agents.permissions"

type FooterTone = "info" | "error"

type CreateSessionResult = Pick<RuntimeSession, "id" | "alias">
type ResolveSessionResult = Pick<RuntimeSession, "id" | "alias">
type DeleteSessionResult = Pick<RuntimeSession, "id" | "alias">

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

type WorkspaceLinkPayload = {
  link: WorkspaceLinkDefinition
  session?: RuntimeSession
}

export { parseRequestedViewLayout } from "./selection-command-handlers.js"

type CommandActionDeps = ProviderCommandHandlerDeps & SelectionCommandHandlerDeps & ConfigCommandHandlerDeps & {
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
  createSession: (
    workspace: string,
    worktree: string,
    alias?: string,
    agentDefaults?: RuntimeSession["agent_defaults"],
  ) => Promise<CreateSessionResult>
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
  resolveSession: (reference: string, workspace: string) => Promise<ResolveSessionResult>
  listSessions: () => Promise<RuntimeSession[]>
  deleteSessionByRef: (reference: string, workspace: string) => Promise<DeleteSessionResult>
  deleteKernel?: () => Promise<{ kernelId: string; deletedSessions: RuntimeSession[] }>
  assignSessionAlias?: (sessionId: string, alias: string) => Promise<RuntimeSession>
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
  listRemoteMachines?: () => Promise<Array<{
    machine_id: string
    machine_alias?: string | null
    registry_alias?: string | null
    display_name?: string
    trust_status?: "approved" | "pending" | "forgotten"
    online?: boolean
    pending?: boolean
    kernel_count: number
    available_providers?: string[]
  }>>
  approveRemoteMachine?: (machineRef: string) => Promise<{
    machine_id: string
    display_name?: string
    trust_status?: "approved" | "pending" | "forgotten"
    online?: boolean
  }>
  forgetRemoteMachine?: (machineRef: string) => Promise<{
    machine_id: string
    display_name?: string
    trust_status?: "approved" | "pending" | "forgotten"
    online?: boolean
  }>
  renameRemoteMachine?: (machineRef: string, alias: string) => Promise<{
    machine_id: string
    display_name?: string
    trust_status?: "approved" | "pending" | "forgotten"
    online?: boolean
  }>
  listRemoteMachineKernels?: (machineRef: string) => Promise<Array<{
    kernel_id: string
    machine_id: string
    machine_alias?: string | null
    relay_alias?: string | null
    kernel_alias?: string | null
    available_providers?: string[]
    capabilities?: string[]
    accepting_remote_leases?: boolean
    leased_agent_count?: number
    local_session_count?: number
  }>>
  listSlices?: () => Promise<SliceRecord[]>
  createSlice?: (options: {
    name: string
    backend?: "local_docker" | "ssh_docker"
    os?: string
    workspaceMount?: string | null
    workerKernelRef?: string | null
    displayUrl?: string | null
  }) => Promise<SliceRecord>
  getSlice?: (sliceRef: string) => Promise<SliceRecord>
  startSlice?: (sliceRef: string) => Promise<SliceRecord>
  stopSlice?: (sliceRef: string) => Promise<SliceRecord>
  deleteSlice?: (sliceRef: string) => Promise<SliceRecord>
  importSliceProviderAuth?: (sliceRef: string, provider: string) => Promise<{ slice: SliceRecord; provider: string; status: string }>
  getSliceDisplayEndpoint?: (sliceRef: string) => Promise<SliceDisplayEndpoint>
  listMcpServers?: () => Promise<ArrobaMcpServerConfig[]>
  installMcpServer?: (config: ArrobaMcpServerConfig) => Promise<ArrobaMcpServerConfig>
  updateMcpServer?: (config: ArrobaMcpServerConfig) => Promise<ArrobaMcpServerConfig>
  uninstallMcpServer?: (name: string) => Promise<string>
  importMcpServers?: (provider: string, name?: string | null) => Promise<McpImportOutcome>
  getMcpServer?: (name: string) => Promise<ArrobaMcpServerConfig>
  grantAgentMcp?: (agentRef: string, name: string) => Promise<AgentInstance>
  revokeAgentMcp?: (agentRef: string, name: string) => Promise<AgentInstance>
  listSkills?: () => Promise<ArrobaSkillMetadata[]>
  installSkill?: (sourcePath: string) => Promise<ArrobaSkillMetadata>
  updateSkill?: (sourcePath: string) => Promise<ArrobaSkillMetadata>
  uninstallSkill?: (name: string) => Promise<ArrobaSkillMetadata>
  importSkills?: (provider: string, name?: string | null) => Promise<SkillImportOutcome>
  getSkill?: (name: string) => Promise<ArrobaSkillMetadata>
  grantAgentSkill?: (agentRef: string, name: string) => Promise<AgentInstance>
  revokeAgentSkill?: (agentRef: string, name: string) => Promise<AgentInstance>
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
  createWorkspaceLink?: (name: string) => Promise<WorkspaceLinkPayload>
  listWorkspaceLinks?: () => Promise<WorkspaceLinkDefinition[]>
  showWorkspaceLink?: (linkRef: string) => Promise<WorkspaceLinkDefinition>
  attachWorkspaceLink?: (linkRef: string, repoRoot?: string | null) => Promise<WorkspaceLinkPayload>
  detachWorkspaceLink?: (linkRef: string, repoRoot?: string | null) => Promise<WorkspaceLinkPayload & { detached: unknown[] }>
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
  formatSessionList: (sessions: SessionListEntry[], currentSessionId?: string) => string
}

export const parseMcpInstallConfig = (args: string[]): ArrobaMcpServerConfig | null => {
  const name = args[1]
  if (!name) return null
  let command: string | null = null
  let url: string | null = null
  const mcpArgs: string[] = []
  const envVars: string[] = []
  let bearerTokenEnvVar: string | null = null
  for (let index = 2; index < args.length; index += 1) {
    const arg = args[index]
    const next = args[index + 1]
    if (arg === "--command" && next) {
      command = next
      index += 1
    } else if (arg === "--arg" && next) {
      mcpArgs.push(next)
      index += 1
    } else if (arg === "--env" && next) {
      envVars.push(next)
      index += 1
    } else if (arg === "--url" && next) {
      url = next
      index += 1
    } else if (arg === "--bearer-token-env-var" && next) {
      bearerTokenEnvVar = next
      index += 1
    } else {
      return null
    }
  }
  if (command && !url) {
    return {
      name,
      transport: { type: "stdio", command, args: mcpArgs, env: {}, env_vars: envVars },
      enabled: true,
      required: false,
    }
  }
  if (url && !command) {
    return {
      name,
      transport: {
        type: "streamable_http",
        url,
        bearer_token_env_var: bearerTokenEnvVar,
        http_headers: {},
        env_http_headers: {},
      },
      enabled: true,
      required: false,
    }
  }
  return null
}

function parseExecutionMode(value: string | null | undefined): "build" | "plan" | null {
  if (!value) return null
  const normalized = value.trim().toLowerCase()
  return normalized === "build" || normalized === "plan" ? normalized : null
}

function parsePermissionLevel(value: string | null | undefined): "required" | "yolo" | null {
  if (!value) return null
  const normalized = value.trim().toLowerCase()
  return normalized === "required" || normalized === "yolo" ? normalized : null
}

function parseSubstitutionTimeoutMs(value: string | null | undefined): number | undefined {
  if (!value || value === "inherit" || value === "default") return undefined
  const normalized = value.trim().toLowerCase()
  const match = normalized.match(/^(\d+)(ms|s|m)?$/)
  if (!match) return undefined
  const amount = Number.parseInt(match[1] ?? "", 10)
  const unit = match[2] ?? "ms"
  if (!Number.isFinite(amount)) return undefined
  if (unit === "m") return amount * 60_000
  if (unit === "s") return amount * 1_000
  return amount
}

function effectiveAgentExecutionMode(session: RuntimeSession, agent: AgentInstance | null | undefined): "build" | "plan" {
  return agent?.execution_mode_override
    ?? parseExecutionMode(session.config_state?.values?.[SESSION_AGENT_MODE_CONFIG_KEY])
    ?? parseExecutionMode(session.agent_defaults?.execution_mode)
    ?? "build"
}

function effectiveAgentPermissionLevel(session: RuntimeSession, agent: AgentInstance | null | undefined): "required" | "yolo" {
  return agent?.permission_level_override
    ?? parsePermissionLevel(session.config_state?.values?.[SESSION_AGENT_PERMISSION_CONFIG_KEY])
    ?? parsePermissionLevel(session.agent_defaults?.permission_level)
    ?? "yolo"
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
  const formatMcpSummary = (mcp: ArrobaMcpServerConfig): string => {
    const transportType = typeof mcp.transport?.type === "string" ? mcp.transport.type : "unknown"
    const status = mcp.enabled === false ? "disabled" : "enabled"
    return `${mcp.name} [${transportType}, ${status}]`
  }

  const formatSkillSummary = (skill: ArrobaSkillMetadata): string => {
    const summary = skill.short_description ?? skill.description
    return `${skill.name}: ${summary}`
  }
  const formatWorkspaceLinks = (links: WorkspaceLinkDefinition[]): string => {
    if (links.length === 0) {
      return "No workspace links in this session."
    }
    return links.map((link) => (
      `${link.name} (${link.link_id}) attachments=${link.attachments?.length ?? 0}`
    )).join("\n")
  }
  const formatWorkspaceLinkDetails = (link: WorkspaceLinkDefinition): string => {
    const lines = [
      `Workspace link ${link.name} (${link.link_id})`,
      `created_by=${link.created_by_user_id}`,
      `attachments=${link.attachments?.length ?? 0}`,
    ]
    for (const attachment of link.attachments ?? []) {
      const branch = attachment.branch ? ` branch=${attachment.branch}` : ""
      lines.push(`- ${attachment.user_id} ${attachment.repo_root}${branch}`)
    }
    return lines.join("\n")
  }
  const formatMcpImportOutcome = (outcome: McpImportOutcome): string => {
    const lines: string[] = []
    if (outcome.imported.length > 0) {
      lines.push(`Imported MCPs: ${outcome.imported.map((mcp) => mcp.name).join(", ")}`)
    }
    if (outcome.skipped.length > 0) {
      lines.push("Skipped MCPs:")
      for (const skip of outcome.skipped) {
        lines.push(`- ${skip.name}: ${skip.reason}`)
      }
    }
    return lines.length === 0 ? "No MCPs imported." : lines.join("\n")
  }
  const formatSkillImportOutcome = (outcome: SkillImportOutcome): string => {
    const lines: string[] = []
    if (outcome.imported.length > 0) {
      lines.push(`Imported skills: ${outcome.imported.map((skill) => skill.name).join(", ")}`)
    }
    if (outcome.skipped.length > 0) {
      lines.push("Skipped skills:")
      for (const skip of outcome.skipped) {
        const suffix = skip.path ? ` (${skip.path})` : ""
        lines.push(`- ${skip.name}${suffix}: ${skip.reason}`)
      }
    }
    return lines.length === 0 ? "No skills imported." : lines.join("\n")
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

  const parseSpawnCount = (value: string | undefined): number | null => {
    if (!value || !/^\d+$/.test(value)) {
      return null
    }
    const count = Number(value)
    return Number.isInteger(count) && count > 0 ? count : null
  }

  const currentWorkspaceTarget = () => deps.getWorkspaceTarget?.() ?? deps.workspace
  const currentWorktreeTarget = () => deps.getWorktreeTarget?.() ?? deps.worktree
  const setWorkspaceTarget = (workspace: string) => deps.setWorkspaceTarget?.(workspace)
  const setWorktreeTarget = (worktree: string) => deps.setWorktreeTarget?.(worktree)

  const spawnAndLaunchAgent = async (options: {
    provider?: string | null
    alias?: string | undefined
    model?: string | null
    effort?: string | null
    worktreeId?: string | undefined
    machineRef?: string | undefined
    worktreePlacement?: RemoteGitWorktreePlacement | undefined
    sliceRef?: string | undefined
  }): Promise<AgentSpawnPayload> => {
    const payload = await deps.spawnAgent(
      options.provider,
      options.alias,
      options.model,
      options.effort,
      options.worktreeId,
      options.machineRef,
      options.worktreePlacement,
      options.sliceRef,
    )
    deps.applySessionState(payload.session)
    await deps.refreshAgentPanes(payload.session)
    if (options.machineRef || options.sliceRef || payload.agent.remote_execution) {
      deps.setProviderRunState(null)
      const refreshedSession = await deps.refreshSessionState(payload.session.id)
      deps.applySessionState(refreshedSession)
      await deps.refreshAgentPanes(refreshedSession)
      deps.rebuildTranscript()
      deps.refreshSplitPaneFocusRepaint()
      return {
        agent: payload.agent,
        session: refreshedSession,
      }
    }
    const launchProvider = payload.agent.provider || options.provider || deps.currentProviderId()
    const launchModel = payload.agent.model || options.model || deps.currentModelId()
    const launchEffort = payload.agent.effort || options.effort || deps.currentVariantId()
    const run = await deps.launchAgentProviderRun(
      launchProvider,
      launchModel,
      launchEffort,
      payload.agent.id,
    )
    deps.setProviderRunState(run)
    const refreshedSession = await deps.refreshSessionState(payload.session.id)
    deps.applySessionState(refreshedSession)
    await deps.refreshAgentPanes(refreshedSession)
    deps.rebuildTranscript()
    deps.refreshSplitPaneFocusRepaint()
    return {
      agent: payload.agent,
      session: refreshedSession,
    }
  }

  const handleSessionCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "session" }>,
  ): Promise<boolean> => {
    const { action, value, args } = command

    switch (action) {
      case "create":
      case "new": {
        try {
          const parsed = parsePlacementOptions(args, "/session new", false)
          if (parsed.error) {
            deps.flashFooter(parsed.error, "error")
            return true
          }
          if (parsed.positional.length > 1) {
            deps.flashFooter("usage: /session new [directory] [--dir <directory>] [--worktree <directory> --branch <branch>]", "error")
            return true
          }
          let sessionWorktree = currentWorktreeTarget()
          const positionalDirectory = parsed.positional[0]
          if (positionalDirectory && !parsed.directory && !parsed.gitWorktree && !parsed.branch && !parsed.fromRef) {
            sessionWorktree = await resolveExistingLocalDirectory(positionalDirectory, currentWorktreeTarget(), "session working directory")
          } else {
            const resolvedPlacement = await resolveLocalPlacement({
              directory: parsed.directory,
              gitWorktree: parsed.gitWorktree,
              branch: parsed.branch,
              fromRef: parsed.fromRef,
              label: "session working directory",
            }, {
              baseDirectory: currentWorktreeTarget(),
              prepareLocalGitWorktree: deps.prepareLocalGitWorktree,
            })
            sessionWorktree = resolvedPlacement ?? currentWorktreeTarget()
          }
          const session = await deps.createSession(currentWorkspaceTarget(), sessionWorktree, undefined, {
            provider: deps.currentProviderId(),
            model: deps.currentModelId(),
            effort: deps.currentVariantId(),
            account_profile: deps.accountProfile ?? null,
            execution_mode: "build",
            permission_level: "yolo",
          })
          await deps.attachBinding(session, true)
          const placement = sessionWorktree !== currentWorktreeTarget() ? ` in ${sessionWorktree}` : ""
          deps.flashFooter(`attached to session ${session.alias ?? session.id}${placement}`, "info")
        } catch (error) {
          deps.flashFooter(deps.formatError(error), "error")
        }
        return true
      }
      case "attach": {
        if (!value) {
          deps.flashFooter("usage: /session attach <ref>", "error")
          return true
        }
        const session = await deps.resolveSession(value, currentWorkspaceTarget())
        await deps.attachBinding(session, false)
        deps.flashFooter(`attached to session ${session.alias ?? session.id}`, "info")
        return true
      }
      case "list":
      case "ls": {
        const sessions = await deps.listSessions()
        deps.appendNotice(deps.formatSessionList(sessions, deps.sessionState().id))
        deps.flashFooter(`listed ${sessions.length} session${sessions.length === 1 ? "" : "s"}`, "info")
        return true
      }
      case "mode": {
        if (!deps.attachmentState()) {
          deps.flashFooter("must be attached to change session mode", "error")
          return true
        }
        if (!value) {
          const current = parseExecutionMode(deps.sessionState().config_state?.values?.[SESSION_AGENT_MODE_CONFIG_KEY]) ?? "build"
          deps.flashFooter(`session mode: ${current}`, "info")
          return true
        }
        const nextMode = parseExecutionMode(value)
        if (!nextMode) {
          deps.flashFooter("usage: /session mode <build|plan>", "error")
          return true
        }
        const payload = await deps.updateSessionConfig(
          deps.sessionState().id,
          deps.attachmentState()!.id,
          { [SESSION_AGENT_MODE_CONFIG_KEY]: nextMode },
          false,
        )
        deps.applySessionState(payload.session)
        await deps.refreshAgentPanes(payload.session)
        deps.flashFooter(`session mode set to ${nextMode}`, "info")
        return true
      }
      case "permissions": {
        if (!deps.attachmentState()) {
          deps.flashFooter("must be attached to change session permissions", "error")
          return true
        }
        if (!value) {
          const current = parsePermissionLevel(deps.sessionState().config_state?.values?.[SESSION_AGENT_PERMISSION_CONFIG_KEY]) ?? "yolo"
          deps.flashFooter(`session permissions: ${current}`, "info")
          return true
        }
        const nextLevel = parsePermissionLevel(value)
        if (!nextLevel) {
          deps.flashFooter("usage: /session permissions <required|yolo>", "error")
          return true
        }
        const payload = await deps.updateSessionConfig(
          deps.sessionState().id,
          deps.attachmentState()!.id,
          { [SESSION_AGENT_PERMISSION_CONFIG_KEY]: nextLevel },
          false,
        )
        deps.applySessionState(payload.session)
        await deps.refreshAgentPanes(payload.session)
        deps.flashFooter(`session permissions set to ${nextLevel}`, "info")
        return true
      }
      case "delete": {
        const sessionRef = value || (deps.isAttached() ? deps.sessionState().id : "")
        if (!sessionRef) {
          deps.flashFooter("usage: /session delete <ref>", "error")
          return true
        }
        const deleted = await deps.deleteSessionByRef(sessionRef, currentWorkspaceTarget())
        if (deps.isAttached() && deleted.id === deps.sessionState().id) {
          deps.transitionToNoSession(`Session ${deleted.alias ?? deleted.id} was deleted.`)
        } else {
          deps.flashFooter(`deleted session ${deleted.alias ?? deleted.id}`, "info")
        }
        return true
      }
      default: {
        if (!action) {
          return false
        }
        if (args.length !== 0) {
          deps.flashFooter("usage: /session <alias>", "error")
          return true
        }
        const alias = action
        if (!deps.isAttached()) {
          deps.flashFooter("attach to a session before setting an alias", "error")
          return true
        }
        if (!deps.assignSessionAlias) {
          deps.flashFooter("session aliases are unavailable in this build", "error")
          return true
        }
        const session = await deps.assignSessionAlias(deps.sessionState().id, alias)
        deps.applySessionState(session)
        deps.flashFooter(`session ${session.id} aliased as ${session.alias}`, "info")
        return true
      }
    }
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
    if (!deps.isAttached()) {
      deps.flashFooter("must be attached to a session to cycle agents", "error")
      return
    }
    try {
      const previousSession = deps.sessionState()
      const previousSelection = selectResponsePaneAgents(
        previousSession.agents,
        previousSession.focused_agent_id,
        deps.multiAgentResponseLayout() === "split",
        deps.maxAgentsPerScreen(),
      )
      const payload = await deps.cycleAgentFocus()
      const nextSession = payload.session
      const nextSelection = selectResponsePaneAgents(
        nextSession.agents,
        nextSession.focused_agent_id,
        deps.multiAgentResponseLayout() === "split",
        deps.maxAgentsPerScreen(),
      )
      const shouldRefreshPaneContents = deps.multiAgentResponseLayout() !== "split"
        || !responsePaneBindingsMatch(previousSelection, nextSelection)
      deps.applySessionState(nextSession)
      if (shouldRefreshPaneContents) {
        await deps.refreshAgentPanes(nextSession)
      }
      if (!nextSession.active_provider_run_id && payload.agent) {
        const run = await deps.launchAgentProviderRun(
          payload.agent.provider,
          payload.agent.model ?? deps.currentModelId(),
          deps.currentVariantId(),
          payload.agent.id,
        )
        deps.setProviderRunState(run)
        deps.applySessionState(await deps.refreshSessionState(nextSession.id))
      }
      if (payload.agent) {
        deps.flashFooter(
          `cycled to agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`,
          "info",
        )
      } else {
        deps.flashFooter("no agents to cycle", "info")
      }
    } catch (error) {
      deps.flashFooter(deps.formatError(error), "error")
    }
  }

  const handleAgentCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "agent" }>,
  ): Promise<void> => {
    const args = command.args
    const subcommand = args[0]

    if (!deps.isAttached()) {
      deps.flashFooter("must be attached to a session to manage agents", "error")
      return
    }

    switch (subcommand) {
      case "spawn": {
        const spawnArgs = args.slice(1)
        try {
          const parsed = parseAgentSpawnOptions(spawnArgs)
          if (parsed.error) {
            deps.flashFooter(parsed.error, "error")
            return
          }
          const count = parseSpawnCount(parsed.positional[0])
          if (
            count !== null
            && (parsed.positional.length > 1 || parsed.directory || parsed.machineRef || parsed.sliceRef || parsed.gitWorktree || parsed.branch || parsed.fromRef)
          ) {
            deps.flashFooter("usage: /agent spawn <count>", "error")
            return
          }
          if (count !== null && parsed.positional.length === 1) {
            for (let index = 0; index < count; index += 1) {
              await spawnAndLaunchAgent({})
            }
            deps.flashFooter(
              `spawned ${count} agent${count === 1 ? "" : "s"} from session defaults`,
              "info",
            )
            return
          }

          const alias = parsed.positional[0]
          const model = parsed.positional[1]
          const provider = model ? deps.currentProviderId() : null
          const effort = model ? deps.currentVariantId() : null
          const remoteGitPlacement = parsed.machineRef && (parsed.gitWorktree || parsed.branch || parsed.fromRef)
            ? {
                target_directory: parsed.gitWorktree ?? null,
                branch: parsed.branch ?? null,
                from_ref: parsed.fromRef ?? null,
              }
            : undefined
          const worktreeId = await resolveLocalPlacement({
            directory: parsed.directory,
            gitWorktree: parsed.gitWorktree,
            branch: parsed.branch,
            fromRef: parsed.fromRef,
            machineRef: parsed.machineRef,
            label: "agent working directory",
          }, {
            baseDirectory: currentWorktreeTarget(),
            prepareLocalGitWorktree: deps.prepareLocalGitWorktree,
          })
          const payload = await spawnAndLaunchAgent({
            provider,
            alias,
            model: model ?? null,
            effort,
            worktreeId,
            machineRef: parsed.machineRef,
            worktreePlacement: remoteGitPlacement,
            sliceRef: parsed.sliceRef,
          })
          const placement = parsed.sliceRef
            ? ` in slice:${parsed.sliceRef}`
            : parsed.machineRef
            ? ` on ${parsed.machineRef}${worktreeId ? ` in ${worktreeId}` : ""}`
            : worktreeId
              ? ` in ${worktreeId}`
              : ""
          deps.flashFooter(`spawned agent ${payload.agent.agent_ref}${alias ? ` (${alias})` : ""}${placement}`, "info")
        } catch (error) {
          deps.flashFooter(deps.formatError(error), "error")
        }
        return
      }
      case "delete":
      case "destroy": {
        const reference = args[1]
        const resolved = deps.resolveSessionAgent(reference)
        if (resolved.error || !resolved.agent) {
          deps.flashFooter(resolved.error ?? "usage: /agent delete <agent-name|agent-alias>", "error")
          return
        }
        try {
          const nextSession = await deps.destroyAgent(resolved.agent.id)
          deps.applySessionState(nextSession)
          await deps.refreshAgentPanes(nextSession)
          deps.rebuildTranscript()
          deps.refreshSplitPaneFocusRepaint()
          deps.flashFooter(`deleted agent ${deps.formatAgentLabel(resolved.agent)}`, "info")
        } catch (error) {
          deps.flashFooter(deps.formatError(error), "error")
        }
        return
      }
      case "focus": {
        const agentId = args[1]
        if (!agentId) {
          deps.flashFooter("usage: /agent focus <agent-id>", "error")
          return
        }
        try {
          const payload = await deps.focusAgent(agentId)
          const nextSession = payload.session
          const previousSession = deps.sessionState()
          const previousSelection = selectResponsePaneAgents(
            previousSession.agents,
            previousSession.focused_agent_id,
            deps.multiAgentResponseLayout() === "split",
            deps.maxAgentsPerScreen(),
          )
          const nextSelection = selectResponsePaneAgents(
            nextSession.agents,
            nextSession.focused_agent_id,
            deps.multiAgentResponseLayout() === "split",
            deps.maxAgentsPerScreen(),
          )
          const shouldRefreshPaneContents = deps.multiAgentResponseLayout() !== "split"
            || !responsePaneBindingsMatch(previousSelection, nextSelection)
          deps.applySessionState(nextSession)
          if (shouldRefreshPaneContents) {
            await deps.refreshAgentPanes(nextSession)
          }
          if (!nextSession.active_provider_run_id) {
            const run = await deps.launchAgentProviderRun(
              payload.agent.provider,
              payload.agent.model ?? deps.currentModelId(),
              deps.currentVariantId(),
              payload.agent.id,
            )
            deps.setProviderRunState(run)
            deps.applySessionState(await deps.refreshSessionState(nextSession.id))
          }
          deps.flashFooter(
            `focused on agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`,
            "info",
          )
        } catch (error) {
          deps.flashFooter(deps.formatError(error), "error")
        }
        return
      }
      case "alias":
      case "name": {
        if (!deps.aliasAgent) {
          deps.flashFooter("agent aliases are unavailable in this build", "error")
          return
        }
        const reference = args[1]
        const explicitAliasArgs = args.length > 2 ? args.slice(2) : args.slice(1)
        const resolved = deps.resolveSessionAgent(args.length > 2 ? reference : deps.focusedAgentId() ?? undefined)
        if (!resolved.agent) {
          deps.flashFooter(resolved.error ?? "usage: /agent alias [agent-ref] <alias|clear>", "error")
          return
        }
        const rawAlias = explicitAliasArgs.join(" ").trim()
        if (!rawAlias) {
          deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} alias: ${resolved.agent.alias ?? "<none>"}`, "info")
          return
        }
        const shouldClearAgentAlias = rawAlias === "clear" || rawAlias === "none" || rawAlias === "-"
        try {
          const payload = await deps.aliasAgent(deps.sessionState().id, resolved.agent.id, shouldClearAgentAlias ? "" : rawAlias)
          deps.applySessionState(payload.session)
          await deps.refreshAgentPanes(payload.session)
          deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} alias: ${payload.agent.alias ?? "<none>"}`, "info")
        } catch (error) {
          deps.flashFooter(deps.formatError(error), "error")
        }
        return
      }
      case "list":
      case "ls": {
        deps.flashFooter(formatAgentListSummary(deps.sessionState().agents), "info")
        return
      }
      case "cycle": {
        await handleCycleAgentFocus()
        return
      }
      case "mode": {
        if (!deps.updateAgentConfig) {
          deps.flashFooter("agent config updates are unavailable in this build", "error")
          return
        }
        const reference = args[1]
        const rawValue = args[2] ?? (reference && (parseExecutionMode(reference) || reference === "inherit") ? reference : undefined)
        const resolved = deps.resolveSessionAgent(rawValue ? reference : deps.focusedAgentId() ?? undefined)
        if (!resolved.agent) {
          deps.flashFooter(resolved.error ?? "usage: /agent mode [agent-ref] <build|plan|inherit>", "error")
          return
        }
        if (!rawValue) {
          const effective = effectiveAgentExecutionMode(deps.sessionState(), resolved.agent)
          const source = resolved.agent.execution_mode_override ? "agent" : "session"
          deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} mode: ${effective} (${source})`, "info")
          return
        }
        if (rawValue !== "inherit" && !parseExecutionMode(rawValue)) {
          deps.flashFooter("usage: /agent mode [agent-ref] <build|plan|inherit>", "error")
          return
        }
        const payload = await deps.updateAgentConfig(deps.sessionState().id, resolved.agent.id, {
          executionMode: rawValue === "inherit" ? null : parseExecutionMode(rawValue),
          clearExecutionMode: rawValue === "inherit",
        })
        deps.applySessionState(payload.session)
        await deps.refreshAgentPanes(payload.session)
        const effective = effectiveAgentExecutionMode(payload.session, payload.agent)
        deps.flashFooter(
          `${deps.formatAgentLabel(payload.agent)} mode: ${effective}${rawValue === "inherit" ? " (session)" : " (agent)"}`,
          "info",
        )
        return
      }
      case "provider":
      case "model":
      case "variant": {
        if (!deps.updateAgentProfile) {
          deps.flashFooter("agent profile updates are unavailable in this build", "error")
          return
        }
        const reference = args[1]
        const rawValue = args.length > 2 ? args.slice(2).join(" ").trim() : args.slice(1).join(" ").trim()
        const resolved = deps.resolveSessionAgent(args.length > 2 ? reference : deps.focusedAgentId() ?? undefined)
        if (!resolved.agent) {
          deps.flashFooter(resolved.error ?? `usage: /agent ${subcommand} [agent-ref] <value>`, "error")
          return
        }
        if (!rawValue) {
          const value = subcommand === "provider"
            ? resolved.agent.provider
            : subcommand === "model"
              ? resolved.agent.model ?? "<none>"
              : resolved.agent.effort ?? "<none>"
          deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} ${subcommand}: ${value}`, "info")
          return
        }
        const shouldClearEffort = subcommand === "variant" && ["clear", "none", "-", "default"].includes(rawValue)
        const payload = await deps.updateAgentProfile(deps.sessionState().id, resolved.agent.id, {
          ...(subcommand === "provider" ? { provider: rawValue } : {}),
          ...(subcommand === "model" ? { model: rawValue } : {}),
          ...(subcommand === "variant" && !shouldClearEffort ? { effort: rawValue } : {}),
          ...(shouldClearEffort ? { clearEffort: true } : {}),
        })
        deps.applySessionState(payload.session)
        await deps.refreshAgentPanes(payload.session)
        const value = subcommand === "provider"
          ? payload.agent.provider
          : subcommand === "model"
            ? payload.agent.model ?? "<none>"
            : payload.agent.effort ?? "<none>"
        deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} ${subcommand}: ${value}`, "info")
        return
      }
      case "permissions": {
        if (!deps.updateAgentConfig) {
          deps.flashFooter("agent config updates are unavailable in this build", "error")
          return
        }
        const reference = args[1]
        const rawValue = args[2] ?? (reference && (parsePermissionLevel(reference) || reference === "inherit") ? reference : undefined)
        const resolved = deps.resolveSessionAgent(rawValue ? reference : deps.focusedAgentId() ?? undefined)
        if (!resolved.agent) {
          deps.flashFooter(resolved.error ?? "usage: /agent permissions [agent-ref] <required|yolo|inherit>", "error")
          return
        }
        if (!rawValue) {
          const effective = effectiveAgentPermissionLevel(deps.sessionState(), resolved.agent)
          const source = resolved.agent.permission_level_override ? "agent" : "session"
          deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} permissions: ${effective} (${source})`, "info")
          return
        }
        if (rawValue !== "inherit" && !parsePermissionLevel(rawValue)) {
          deps.flashFooter("usage: /agent permissions [agent-ref] <required|yolo|inherit>", "error")
          return
        }
        const payload = await deps.updateAgentConfig(deps.sessionState().id, resolved.agent.id, {
          permissionLevel: rawValue === "inherit" ? null : parsePermissionLevel(rawValue),
          clearPermissionLevel: rawValue === "inherit",
        })
        deps.applySessionState(payload.session)
        await deps.refreshAgentPanes(payload.session)
        const effective = effectiveAgentPermissionLevel(payload.session, payload.agent)
        deps.flashFooter(
          `${deps.formatAgentLabel(payload.agent)} permissions: ${effective}${rawValue === "inherit" ? " (session)" : " (agent)"}`,
          "info",
        )
        return
      }
      case "substitute":
      case "subs": {
        if (!deps.updateAgentSubstitutes) {
          deps.flashFooter("agent substitute updates are unavailable in this build", "error")
          return
        }
        const subcommand = args[1] ?? "list"
        const subArgs = args.slice(2)
        const agentFlagIndex = subArgs.indexOf("--agent")
        const agentRefFromFlag = agentFlagIndex >= 0 ? subArgs[agentFlagIndex + 1] : undefined
        const filteredArgs = agentFlagIndex >= 0
          ? subArgs.filter((_, index) => index !== agentFlagIndex && index !== agentFlagIndex + 1)
          : subArgs
        const resolved = deps.resolveSessionAgent(agentRefFromFlag ?? deps.focusedAgentId() ?? undefined)
        if (!resolved.agent) {
          deps.flashFooter(resolved.error ?? "no focused agent", "error")
          return
        }
        const agent = resolved.agent
        if (subcommand === "list" || subcommand === "ls") {
          deps.flashFooter(formatAgentSubstituteSummary(agent), "info")
          return
        }
        const applyUpdate = async (action: Record<string, unknown>) => {
          const payload = await deps.updateAgentSubstitutes!(
            deps.sessionState().id,
            agent.id,
            action,
          )
          deps.applySessionState(payload.session)
          await deps.refreshAgentPanes(payload.session)
          return payload
        }
        if (subcommand === "add") {
          const provider = filteredArgs[0]
          const model = filteredArgs[1]
          const variantIndex = filteredArgs.indexOf("--variant")
          const variant = variantIndex >= 0 ? filteredArgs[variantIndex + 1] : undefined
          const kernelIndex = filteredArgs.indexOf("--kernel")
          const kernelId = kernelIndex >= 0 ? filteredArgs[kernelIndex + 1] : undefined
          const worktreeIndex = filteredArgs.indexOf("--worktree")
          const worktreeId = worktreeIndex >= 0 ? filteredArgs[worktreeIndex + 1] : undefined
          if (!provider || !model) {
            deps.flashFooter("usage: /agent substitute add <provider> <model> [--variant v] [--kernel k] [--worktree dir] [--agent a]", "error")
            return
          }
          const payload = await applyUpdate({
            Add: {
              provider,
              model,
              variant: variant ?? null,
              kernel_id: kernelId ?? null,
              worktree_id: worktreeId ?? null,
            },
          })
          deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitute added: ${provider}/${model}${variant ? `/${variant}` : ""}`, "info")
          return
        }
        if (subcommand === "remove" || subcommand === "rm") {
          const index = Number.parseInt(filteredArgs[0] ?? "", 10)
          if (!Number.isFinite(index)) {
            deps.flashFooter("usage: /agent substitute remove <index> [--agent a]", "error")
            return
          }
          const payload = await applyUpdate({ Remove: { index } })
          deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitute ${index} removed`, "info")
          return
        }
        if (subcommand === "clear") {
          const payload = await applyUpdate({ Clear: {} })
          deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitutes cleared`, "info")
          return
        }
        if (subcommand === "timeout") {
          const timeoutMs = parseSubstitutionTimeoutMs(filteredArgs[0])
          if (timeoutMs === undefined && filteredArgs[0] !== "inherit" && filteredArgs[0] !== "default") {
            deps.flashFooter("usage: /agent substitute timeout <ms|Ns|inherit> [--agent a]", "error")
            return
          }
          const payload = await applyUpdate({ SetTimeout: { timeout_ms: timeoutMs ?? null } })
          deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitute timeout: ${timeoutMs == null ? "default" : `${timeoutMs}ms`}`, "info")
          return
        }
        if (subcommand === "activate") {
          const index = Number.parseInt(filteredArgs[0] ?? "", 10)
          if (!Number.isFinite(index)) {
            deps.flashFooter("usage: /agent substitute activate <index> [--agent a]", "error")
            return
          }
          const payload = await applyUpdate({ Activate: { index, reason: "manual" } })
          const profile = payload.agent.substitutes?.[index]
          if (!profile) {
            deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitute ${index} is not available`, "error")
            return
          }
          const run = await deps.launchAgentProviderRun(
            profile.provider,
            profile.model,
            profile.variant ?? "",
            payload.agent.id,
          )
          deps.setProviderRunState(run)
          const refreshedSession = await deps.refreshSessionState(payload.session.id)
          deps.applySessionState(refreshedSession)
          await deps.refreshAgentPanes(refreshedSession)
          deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} activated substitute ${index}: ${profile.provider}/${profile.model}`, "info")
          return
        }
        if (subcommand === "primary") {
          const payload = await applyUpdate({ Primary: {} })
          const run = await deps.launchAgentProviderRun(
            payload.agent.provider,
            payload.agent.model ?? deps.currentModelId(),
            payload.agent.effort ?? deps.currentVariantId(),
            payload.agent.id,
          )
          deps.setProviderRunState(run)
          const refreshedSession = await deps.refreshSessionState(payload.session.id)
          deps.applySessionState(refreshedSession)
          await deps.refreshAgentPanes(refreshedSession)
          deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} returned to primary profile`, "info")
          return
        }
        deps.flashFooter("usage: /agent substitute list|add|remove|clear|timeout|activate|primary", "error")
        return
      }
      default:
        deps.flashFooter(
          "usage: /agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>|--slice <slice-ref>] | /agent spawn <count> | delete [agent-name|agent-alias] | focus <agent-id> | alias [agent-ref] <alias|clear> | provider/model/variant [agent-ref] <value> | list | cycle | mode [agent-ref] <build|plan|inherit> | permissions [agent-ref] <required|yolo|inherit> | substitute ...",
          "error",
        )
    }
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
    const args = command.args
    const subcommand = args[0]
    if (subcommand === "list") {
      if (!deps.listRemoteMachines) {
        deps.flashFooter("remote machine discovery is unavailable in this build", "error")
        return
      }
      const machines = await deps.listRemoteMachines()
      if (machines.length === 0) {
        deps.flashFooter("no live remote machines available through relay", "info")
        return
      }
      deps.appendNotice(
        machines
          .map((machine) =>
            `${machine.display_name ?? machine.machine_alias ?? "-"} id=${machine.machine_id} status=${machine.trust_status ?? "pending"}${machine.online === false ? ",offline" : ""} kernels=${machine.kernel_count} providers=${(machine.available_providers ?? []).join(",") || "-"}`
          )
          .join("\n"),
      )
      deps.flashFooter(`listed ${machines.length} live remote machine(s)`, "info")
      return
    }
    if (subcommand === "kernels") {
      if (!deps.listRemoteMachineKernels) {
        deps.flashFooter("remote machine discovery is unavailable in this build", "error")
        return
      }
      const machineRef = args[1]
      if (!machineRef) {
        deps.flashFooter("usage: /machine kernels <machine-ref>", "error")
        return
      }
      const kernels = await deps.listRemoteMachineKernels(machineRef)
      if (kernels.length === 0) {
        deps.flashFooter(`no live kernels found for machine ${machineRef}`, "info")
        return
      }
      deps.appendNotice(
        kernels
          .map((kernel) => {
            const displayName = kernel.relay_alias ?? kernel.kernel_alias ?? "-"
            const kernelAlias =
              kernel.kernel_alias && kernel.kernel_alias !== displayName
                ? ` kernel_alias=${kernel.kernel_alias}`
                : ""
            return `${displayName} id=${kernel.kernel_id}${kernelAlias} machine=${kernel.machine_alias ?? kernel.machine_id} providers=${(kernel.available_providers ?? []).join(",") || "-"} accepting_remote_leases=${String(kernel.accepting_remote_leases ?? false)} leased_agents=${kernel.leased_agent_count ?? 0} local_sessions=${kernel.local_session_count ?? 0}`
          })
          .join("\n"),
      )
      deps.flashFooter(`listed ${kernels.length} live kernel(s) for ${machineRef}`, "info")
      return
    }
    if (subcommand === "approve") {
      if (!deps.approveRemoteMachine) {
        deps.flashFooter("remote machine registration is unavailable in this build", "error")
        return
      }
      const machineRef = args[1]
      if (!machineRef) {
        deps.flashFooter("usage: /machine approve <machine-ref>", "error")
        return
      }
      const machine = await deps.approveRemoteMachine(machineRef)
      await deps.refreshWaitingRoomData?.()
      deps.flashFooter(`approved remote machine ${machine.display_name ?? machine.machine_id}`, "info")
      return
    }
    if (subcommand === "forget") {
      if (!deps.forgetRemoteMachine) {
        deps.flashFooter("remote machine registration is unavailable in this build", "error")
        return
      }
      const machineRef = args[1]
      if (!machineRef) {
        deps.flashFooter("usage: /machine forget <machine-ref>", "error")
        return
      }
      const machine = await deps.forgetRemoteMachine(machineRef)
      await deps.refreshWaitingRoomData?.()
      deps.flashFooter(`forgot remote machine ${machine.display_name ?? machine.machine_id}`, "info")
      return
    }
    if (subcommand === "rename") {
      if (!deps.renameRemoteMachine) {
        deps.flashFooter("remote machine registration is unavailable in this build", "error")
        return
      }
      const machineRef = args[1]
      const alias = args.slice(2).join(" ").trim()
      if (!machineRef || !alias) {
        deps.flashFooter("usage: /machine rename <machine-ref> <alias>", "error")
        return
      }
      const machine = await deps.renameRemoteMachine(machineRef, alias)
      await deps.refreshWaitingRoomData?.()
      deps.flashFooter(`renamed remote machine ${machine.machine_id} to ${machine.display_name ?? alias}`, "info")
      return
    }
    deps.flashFooter("usage: /machine list | /machine kernels <machine-ref> | /machine approve <machine-ref> | /machine forget <machine-ref> | /machine rename <machine-ref> <alias>", "error")
  }

  const formatSliceLabel = (slice: SliceRecord): string => slice.name || slice.id

  const formatSlice = (slice: SliceRecord): string => {
    const display = slice.display_endpoint?.url ? ` screen=${slice.display_endpoint.url}` : ""
    const providers = (slice.providers ?? []).join(",") || "-"
    const worker = slice.worker_kernel_id ?? slice.worker_kernel_ref
    return `${formatSliceLabel(slice)} id=${slice.id} status=${slice.status} backend=${slice.backend} os=${slice.os} worker=${worker} providers=${providers}${slice.workspace_mount ? ` mount=${slice.workspace_mount}` : ""}${display}`
  }

  const resolveFocusedSliceRef = async (): Promise<string> => {
    const focusedId = deps.focusedAgentId()
    const resolved = deps.resolveSessionAgent(focusedId)
    const remote = resolved.agent?.remote_execution
    if (!remote) {
      throw new Error("no slice specified and focused agent is not running in a slice")
    }
    if (!deps.listSlices) {
      throw new Error("slice inventory is unavailable in this build")
    }
    const slices = await deps.listSlices()
    const match = slices.find((slice) =>
      slice.worker_kernel_id === remote.worker_kernel_id
      || slice.worker_kernel_ref === remote.worker_kernel_id
      || slice.worker_machine_id === remote.worker_machine_id
    )
    if (!match) {
      throw new Error("no slice specified and focused agent is not running in a slice")
    }
    return match.name || match.id
  }

  const explicitOrFocusedSliceRef = async (value: string | undefined): Promise<string> => value ?? resolveFocusedSliceRef()

  const parseSliceCreateOptions = (args: string[]): {
    name?: string
    backend?: "local_docker" | "ssh_docker"
    workerKernelRef?: string | null
    displayUrl?: string | null
    workspaceMount?: string | null
    error?: string
  } => {
    const name = args[0]
    let backend: "local_docker" | "ssh_docker" | undefined
    let workerKernelRef: string | null | undefined
    let displayUrl: string | null | undefined
    let workspaceMount: string | null | undefined = currentWorktreeTarget()
    let error: string | undefined
    for (let index = 1; index < args.length; index += 1) {
      const arg = args[index]
      const value = args[index + 1]
      if (arg === "--backend") {
        if (value !== "local_docker" && value !== "ssh_docker") {
          error = "usage: /slice create <name> [--backend local_docker|ssh_docker] [--kernel <worker-kernel-ref>] [--display-url <url>] [--mount <path|none>]"
          break
        }
        backend = value
        index += 1
        continue
      }
      if (arg === "--kernel") {
        if (!value || value.startsWith("--")) {
          error = "usage: /slice create <name> --kernel <worker-kernel-ref>"
          break
        }
        workerKernelRef = value
        index += 1
        continue
      }
      if (arg === "--display-url") {
        if (!value || value.startsWith("--")) {
          error = "usage: /slice create <name> --display-url <url>"
          break
        }
        displayUrl = value
        index += 1
        continue
      }
      if (arg === "--mount") {
        if (!value || value.startsWith("--")) {
          error = "usage: /slice create <name> --mount <path|none>"
          break
        }
        workspaceMount = value === "none" ? null : value
        index += 1
        continue
      }
      error = `unknown /slice create option ${arg}`
      break
    }
    return {
      ...(name !== undefined ? { name } : {}),
      ...(backend !== undefined ? { backend } : {}),
      ...(workerKernelRef !== undefined ? { workerKernelRef } : {}),
      ...(displayUrl !== undefined ? { displayUrl } : {}),
      ...(workspaceMount !== undefined ? { workspaceMount } : {}),
      ...(error !== undefined ? { error } : {}),
    }
  }

  const handleSliceCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "slice" }>,
  ): Promise<void> => {
    const [subcommand, ...args] = command.args
    if (!subcommand || subcommand === "list" || subcommand === "ls") {
      if (!deps.listSlices) {
        deps.flashFooter("slice inventory is unavailable in this build", "error")
        return
      }
      const slices = await deps.listSlices()
      deps.appendNotice(slices.length === 0 ? "No slices owned by this kernel." : slices.map(formatSlice).join("\n"))
      deps.flashFooter(`listed ${slices.length} slice${slices.length === 1 ? "" : "s"}`, "info")
      return
    }
    if (subcommand === "create") {
      if (!deps.createSlice) {
        deps.flashFooter("slice creation is unavailable in this build", "error")
        return
      }
      const parsed = parseSliceCreateOptions(args)
      if (!parsed.name || parsed.error) {
        deps.flashFooter(parsed.error ?? "usage: /slice create <name> [--kernel <worker-kernel-ref>] [--display-url <url>] [--mount <path|none>]", "error")
        return
      }
      const createOptions = {
        name: parsed.name,
        ...(parsed.backend !== undefined ? { backend: parsed.backend } : {}),
        ...(parsed.workspaceMount !== undefined ? { workspaceMount: parsed.workspaceMount } : {}),
        workerKernelRef: parsed.workerKernelRef ?? null,
        displayUrl: parsed.displayUrl ?? null,
      }
      const slice = await deps.createSlice(createOptions)
      deps.flashFooter(`created slice ${formatSliceLabel(slice)}`, "info")
      return
    }
    if (subcommand === "status" || subcommand === "show") {
      if (!deps.getSlice) {
        deps.flashFooter("slice inventory is unavailable in this build", "error")
        return
      }
      const slice = await deps.getSlice(await explicitOrFocusedSliceRef(args[0]))
      deps.appendNotice(formatSlice(slice))
      deps.flashFooter(`showing slice ${formatSliceLabel(slice)}`, "info")
      return
    }
    if (subcommand === "start" || subcommand === "stop") {
      const handler = subcommand === "start" ? deps.startSlice : deps.stopSlice
      if (!handler) {
        deps.flashFooter(`slice ${subcommand} is unavailable in this build`, "error")
        return
      }
      const slice = await handler(await explicitOrFocusedSliceRef(args[0]))
      deps.flashFooter(`${subcommand === "start" ? "started" : "stopped"} slice ${formatSliceLabel(slice)}`, "info")
      return
    }
    if (subcommand === "delete" || subcommand === "rm") {
      if (!deps.deleteSlice) {
        deps.flashFooter("slice delete is unavailable in this build", "error")
        return
      }
      const sliceRef = args[0]
      if (!sliceRef) {
        deps.flashFooter("usage: /slice delete <slice-ref>", "error")
        return
      }
      const slice = await deps.deleteSlice(sliceRef)
      deps.flashFooter(`deleted slice ${formatSliceLabel(slice)}`, "info")
      return
    }
    if (subcommand === "screen") {
      if (!deps.getSliceDisplayEndpoint) {
        deps.flashFooter("slice screen is unavailable in this build", "error")
        return
      }
      const endpoint = await deps.getSliceDisplayEndpoint(await explicitOrFocusedSliceRef(args[0]))
      deps.appendNotice(endpoint.url)
      const opened = await deps.openExternalUrl?.(endpoint.url)
      deps.flashFooter(`${opened ? "opened" : "screen"} ${endpoint.url}`, "info")
      return
    }
    if (subcommand === "auth" && args[0] === "import") {
      if (!deps.importSliceProviderAuth) {
        deps.flashFooter("slice auth import is unavailable in this build", "error")
        return
      }
      const provider = args.length >= 3 ? args[2] : args[1]
      const sliceRef = args.length >= 3 ? args[1]! : await resolveFocusedSliceRef()
      if (!provider) {
        deps.flashFooter("usage: /slice auth import [slice-ref] <provider>", "error")
        return
      }
      const result = await deps.importSliceProviderAuth(sliceRef, provider)
      deps.flashFooter(`slice auth import ${result.provider}: ${result.status}`, result.status === "imported" ? "info" : "error")
      return
    }
    deps.flashFooter("usage: /slice list | /slice create <name> | /slice status [slice-ref] | /slice start [slice-ref] | /slice stop [slice-ref] | /slice delete <slice-ref> | /slice screen [slice-ref] | /slice auth import [slice-ref] <provider>", "error")
  }

  const handleKernelCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "kernel" }>,
  ): Promise<void> => {
    const [subcommand, ...args] = command.args
    if (subcommand === "delete") {
      if (!deps.deleteKernel) {
        deps.flashFooter("kernel delete is unavailable in this build", "error")
        return
      }
      if (args.length > 0) {
        deps.flashFooter("usage: /kernel delete", "error")
        return
      }
      const deleted = await deps.deleteKernel()
      if (deps.isAttached() && deleted.deletedSessions.some((session) => session.id === deps.sessionState().id)) {
        deps.transitionToNoSession(`Kernel ${deleted.kernelId} was deleted.`)
        return
      }
      deps.flashFooter(`deleted kernel ${deleted.kernelId} (${deleted.deletedSessions.length} session${deleted.deletedSessions.length === 1 ? "" : "s"})`, "info")
      return
    }
    deps.flashFooter("usage: /kernel delete", "error")
  }

  const formatMcpDetails = (mcp: ArrobaMcpServerConfig): string => JSON.stringify(mcp, null, 2)
  const formatSkillDetails = (skill: ArrobaSkillMetadata): string => [
    `${skill.name}: ${skill.description}`,
    skill.short_description ? `short: ${skill.short_description}` : null,
    `path: ${skill.path}`,
  ].filter(Boolean).join("\n")
  const resolveGrantTarget = (agentRef: string | undefined, usage: string): AgentInstance | null => {
    if (!agentRef) {
      deps.flashFooter(usage, "error")
      return null
    }
    const resolved = deps.resolveSessionAgent(agentRef)
    if (!resolved.agent) {
      deps.flashFooter(resolved.error ?? `unknown agent ${agentRef}`, "error")
      return null
    }
    return resolved.agent
  }

  const handleMcpCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "mcp" }>,
  ): Promise<void> => {
    const [action] = command.args
    if (!action || action === "list" || action === "ls") {
      if (!deps.listMcpServers) {
        deps.flashFooter("MCP registry is not available in this daemon", "error")
        return
      }
      const mcps = await deps.listMcpServers()
      deps.appendNotice(mcps.length === 0 ? "No Arroba-managed MCPs installed." : mcps.map(formatMcpSummary).join("\n"))
      deps.flashFooter(`listed ${mcps.length} MCP${mcps.length === 1 ? "" : "s"}`, "info")
      return
    }
    if (action === "show") {
      const name = command.args[1]
      if (!name || !deps.getMcpServer) {
        deps.flashFooter("usage: /mcp show <name>", "error")
        return
      }
      const mcp = await deps.getMcpServer(name)
      deps.appendNotice(formatMcpDetails(mcp))
      deps.flashFooter(`showing MCP ${mcp.name}`, "info")
      return
    }
    if (action === "install") {
      if (!deps.installMcpServer) {
        deps.flashFooter("MCP install is not available in this daemon", "error")
        return
      }
      const config = parseMcpInstallConfig(command.args)
      if (!config) {
        deps.flashFooter("usage: /mcp install <name> --command <cmd> [--arg value] [--env VAR] | /mcp install <name> --url <url> [--bearer-token-env-var VAR]", "error")
        return
      }
      const mcp = await deps.installMcpServer(config)
      deps.flashFooter(`installed MCP ${mcp.name}`, "info")
      return
    }
    if (action === "update") {
      if (!deps.updateMcpServer) {
        deps.flashFooter("MCP update is not available in this daemon", "error")
        return
      }
      const config = parseMcpInstallConfig(["install", ...command.args.slice(1)])
      if (!config) {
        deps.flashFooter("usage: /mcp update <name> --command <cmd> [--arg value] [--env VAR] | /mcp update <name> --url <url> [--bearer-token-env-var VAR]", "error")
        return
      }
      const mcp = await deps.updateMcpServer(config)
      deps.flashFooter(`updated MCP ${mcp.name}`, "info")
      return
    }
    if (action === "uninstall" || action === "remove") {
      const name = command.args[1]
      if (!name || !deps.uninstallMcpServer) {
        deps.flashFooter(`usage: /mcp ${action} <name>`, "error")
        return
      }
      const removedName = await deps.uninstallMcpServer(name)
      deps.flashFooter(`uninstalled MCP ${removedName}`, "info")
      return
    }
    if (action === "import") {
      const provider = command.args[1]
      const name = command.args[2] ?? null
      if (!provider || !deps.importMcpServers) {
        deps.flashFooter("usage: /mcp import <codex|opencode> [name]", "error")
        return
      }
      const outcome = await deps.importMcpServers(provider, name)
      deps.appendNotice(formatMcpImportOutcome(outcome))
      deps.flashFooter(`imported ${outcome.imported.length} MCP${outcome.imported.length === 1 ? "" : "s"} from ${provider}`, "info")
      return
    }
    if (action === "grant" || action === "revoke") {
      const agentRef = command.args[1]
      const name = command.args[2]
      const handler = action === "grant" ? deps.grantAgentMcp : deps.revokeAgentMcp
      if (!agentRef || !name || !handler) {
        deps.flashFooter(`usage: /mcp ${action} <agent-ref> <name>`, "error")
        return
      }
      const agent = await handler(agentRef, name)
      deps.flashFooter(`${action === "grant" ? "granted" : "revoked"} MCP ${name} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, "info")
      return
    }
    if (action === "grants" || action === "agent") {
      const agent = resolveGrantTarget(command.args[1], `usage: /mcp ${action} <agent-ref>`)
      if (!agent) return
      deps.appendNotice(formatAgentCapabilityGrants(agent, "mcp"))
      deps.flashFooter(`showing MCP grants for ${agent.agent_ref}`, "info")
      return
    }
    deps.flashFooter("usage: /mcp list | /mcp show <name> | /mcp install ... | /mcp update ... | /mcp uninstall <name> | /mcp import <codex|opencode> [name] | /mcp grant <agent-ref> <name> | /mcp revoke <agent-ref> <name> | /mcp grants <agent-ref>", "error")
  }

  const handleSkillCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "skill" }>,
  ): Promise<void> => {
    const [action] = command.args
    if (!action || action === "list" || action === "ls") {
      if (!deps.listSkills) {
        deps.flashFooter("skill registry is not available in this daemon", "error")
        return
      }
      const skills = await deps.listSkills()
      deps.appendNotice(skills.length === 0 ? "No Arroba-managed skills installed." : skills.map(formatSkillSummary).join("\n"))
      deps.flashFooter(`listed ${skills.length} skill${skills.length === 1 ? "" : "s"}`, "info")
      return
    }
    if (action === "show") {
      const name = command.args[1]
      if (!name || !deps.getSkill) {
        deps.flashFooter("usage: /skill show <name>", "error")
        return
      }
      const skill = await deps.getSkill(name)
      deps.appendNotice(formatSkillDetails(skill))
      deps.flashFooter(`showing skill ${skill.name}`, "info")
      return
    }
    if (action === "install") {
      const sourcePath = command.args[1]
      if (!sourcePath || !deps.installSkill) {
        deps.flashFooter("usage: /skill install <path>", "error")
        return
      }
      const skill = await deps.installSkill(sourcePath)
      deps.flashFooter(`installed skill ${skill.name}`, "info")
      return
    }
    if (action === "update") {
      const sourcePath = command.args[1]
      if (!sourcePath || !deps.updateSkill) {
        deps.flashFooter("usage: /skill update <path>", "error")
        return
      }
      const skill = await deps.updateSkill(sourcePath)
      deps.flashFooter(`updated skill ${skill.name}`, "info")
      return
    }
    if (action === "uninstall" || action === "remove") {
      const name = command.args[1]
      if (!name || !deps.uninstallSkill) {
        deps.flashFooter(`usage: /skill ${action} <name>`, "error")
        return
      }
      const skill = await deps.uninstallSkill(name)
      deps.flashFooter(`uninstalled skill ${skill.name}`, "info")
      return
    }
    if (action === "import") {
      const provider = command.args[1]
      const name = command.args[2] ?? null
      if (!provider || !deps.importSkills) {
        deps.flashFooter("usage: /skill import <codex|opencode> [name]", "error")
        return
      }
      const outcome = await deps.importSkills(provider, name)
      deps.appendNotice(formatSkillImportOutcome(outcome))
      deps.flashFooter(`imported ${outcome.imported.length} skill${outcome.imported.length === 1 ? "" : "s"} from ${provider}`, "info")
      return
    }
    if (action === "grant" || action === "revoke") {
      const agentRef = command.args[1]
      const name = command.args[2]
      const handler = action === "grant" ? deps.grantAgentSkill : deps.revokeAgentSkill
      if (!agentRef || !name || !handler) {
        deps.flashFooter(`usage: /skill ${action} <agent-ref> <name>`, "error")
        return
      }
      const agent = await handler(agentRef, name)
      deps.flashFooter(`${action === "grant" ? "granted" : "revoked"} skill ${name} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, "info")
      return
    }
    if (action === "grants" || action === "agent") {
      const agent = resolveGrantTarget(command.args[1], `usage: /skill ${action} <agent-ref>`)
      if (!agent) return
      deps.appendNotice(formatAgentCapabilityGrants(agent, "skill"))
      deps.flashFooter(`showing skill grants for ${agent.agent_ref}`, "info")
      return
    }
    deps.flashFooter("usage: /skill list | /skill show <name> | /skill install <path> | /skill update <path> | /skill uninstall <name> | /skill import <codex|opencode> [name] | /skill grant <agent-ref> <name> | /skill revoke <agent-ref> <name> | /skill grants <agent-ref>", "error")
  }

  const handleWorkspaceCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "workspace" }>,
  ): Promise<void> => {
    const [resource, action, ...args] = command.args
    if (resource && resource !== "link") {
      const previousWorktreeTarget = currentWorktreeTarget()
      const previousWorkspaceTarget = currentWorkspaceTarget()
      const workspacePath = resolvePath(currentWorktreeTarget(), [resource, action, ...args].filter(Boolean).join(" "))
      setWorkspaceTarget(workspacePath)
      if (!deps.getWorktreeTarget || previousWorktreeTarget === deps.worktree || previousWorktreeTarget === previousWorkspaceTarget) {
        setWorktreeTarget(workspacePath)
      }
      deps.flashFooter(`next-session workspace set to ${workspacePath}`, "info")
      return
    }
    if (resource !== "link") {
      deps.flashFooter(`workspace target: ${currentWorkspaceTarget()}`, "info")
      return
    }
    if (!deps.isAttached()) {
      deps.flashFooter("attach to a session before managing workspace links", "error")
      return
    }
    if (action === "create" || action === "new") {
      const name = args[0]
      if (!name || !deps.createWorkspaceLink) {
        deps.flashFooter("usage: /workspace link create <name>", "error")
        return
      }
      const payload = await deps.createWorkspaceLink(name)
      if (payload.session) deps.applySessionState(payload.session)
      deps.flashFooter(`created workspace link ${payload.link.name}`, "info")
      return
    }
    if (!action || action === "list" || action === "ls") {
      if (!deps.listWorkspaceLinks) {
        deps.flashFooter("workspace links are not available", "error")
        return
      }
      const links = await deps.listWorkspaceLinks()
      deps.appendNotice(formatWorkspaceLinks(links))
      deps.flashFooter(`listed ${links.length} workspace link${links.length === 1 ? "" : "s"}`, "info")
      return
    }
    if (action === "show") {
      const linkRef = args[0]
      if (!linkRef || !deps.showWorkspaceLink) {
        deps.flashFooter("usage: /workspace link show <name-or-id>", "error")
        return
      }
      const link = await deps.showWorkspaceLink(linkRef)
      deps.appendNotice(formatWorkspaceLinkDetails(link))
      deps.flashFooter(`showing workspace link ${link.name}`, "info")
      return
    }
    if (action === "attach") {
      const linkRef = args[0]
      const repoRoot = args[1] ? resolvePath(currentWorktreeTarget(), args[1]) : currentWorktreeTarget()
      if (!linkRef || !deps.attachWorkspaceLink) {
        deps.flashFooter("usage: /workspace link attach <name-or-id> [repo-root]", "error")
        return
      }
      const payload = await deps.attachWorkspaceLink(linkRef, repoRoot)
      if (payload.session) deps.applySessionState(payload.session)
      deps.flashFooter(`attached ${repoRoot} to workspace link ${payload.link.name}`, "info")
      return
    }
    if (action === "detach") {
      const linkRef = args[0]
      const repoRoot = args[1] ? resolvePath(currentWorktreeTarget(), args[1]) : currentWorktreeTarget()
      if (!linkRef || !deps.detachWorkspaceLink) {
        deps.flashFooter("usage: /workspace link detach <name-or-id> [repo-root]", "error")
        return
      }
      const payload = await deps.detachWorkspaceLink(linkRef, repoRoot)
      if (payload.session) deps.applySessionState(payload.session)
      deps.flashFooter(`detached ${payload.detached.length} workspace link attachment${payload.detached.length === 1 ? "" : "s"} from ${payload.link.name}`, "info")
      return
    }
    deps.flashFooter("usage: /workspace link create|list|show|attach|detach", "error")
  }

  const handleWorktreeCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "worktree" }>,
  ): Promise<void> => {
    const [action, ...args] = command.args
    if (!action) {
      deps.flashFooter(`worktree target: ${currentWorktreeTarget()}`, "info")
      return
    }
    if (action === "name") {
      const alias = args.join(" ").trim()
      if (!deps.setUserConfigValue || !deps.unsetUserConfigValue) {
        deps.flashFooter("worktree naming is unavailable in this build", "error")
        return
      }
      const configPath = worktreeAliasConfigPath(currentWorktreeTarget())
      if (!alias) {
        await deps.unsetUserConfigValue(configPath)
        deps.flashFooter(`cleared worktree name for ${currentWorktreeTarget()}`, "info")
        return
      }
      await deps.setUserConfigValue(configPath, alias)
      deps.flashFooter(`named ${currentWorktreeTarget()} as ${alias}`, "info")
      return
    }
    if (action === "create" || action === "new") {
      const [branch, explicitPath, ...rest] = args
      if (!branch) {
        deps.flashFooter("usage: /worktree create <branch> [directory] [--from <ref>]", "error")
        return
      }
      let fromRef: string | undefined
      for (let index = 0; index < rest.length; index += 1) {
        if (rest[index] === "--from") {
          fromRef = rest[index + 1]
        }
      }
      const targetDirectory = suggestNamedWorktreePath(currentWorkspaceTarget(), branch, explicitPath)
      const createdPath = await prepareLocalGitWorktree({
        baseDirectory: currentWorkspaceTarget(),
        targetDirectory,
        branch,
        fromRef,
      }, deps.prepareLocalGitWorktree)
      setWorktreeTarget(createdPath)
      deps.flashFooter(`next-session worktree set to ${createdPath}`, "info")
      return
    }
    const worktreePath = resolvePath(currentWorkspaceTarget(), [action, ...args].join(" "))
    setWorktreeTarget(worktreePath)
    deps.flashFooter(`next-session worktree set to ${worktreePath}`, "info")
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

export function formatAgentListSummary(agents: AgentInstance[]): string {
  if (agents.length === 0) {
    return "no agents in session"
  }
  const agentList = agents
    .map((agent) => `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""} [${agent.state}]`)
    .join(", ")
  return `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`
}

export function formatAgentSubstituteSummary(agent: AgentInstance): string {
  const substitutes = agent.substitutes ?? []
  const label = `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
  if (substitutes.length === 0) {
    return `${label} has no substitutes`
  }
  const active = agent.active_substitute_index
  const lines = substitutes.map((substitute, index) => {
    const marker = active === index ? "*" : "-"
    const variant = substitute.variant ? `/${substitute.variant}` : ""
    return `${marker} ${index}: ${substitute.provider}/${substitute.model}${variant}`
  })
  const timeout = agent.substitution_timeout_ms == null
    ? "default"
    : `${agent.substitution_timeout_ms}ms`
  return `${label} substitutes (${substitutes.length}, timeout ${timeout}):\n${lines.join("\n")}`
}

export function formatAgentCapabilityGrants(agent: AgentInstance, kind: "mcp" | "skill"): string {
  const grants = kind === "mcp" ? (agent.mcp_grants ?? []) : (agent.skill_grants ?? [])
  const label = kind === "mcp" ? "MCP" : "skill"
  const agentLabel = `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
  if (grants.length === 0) {
    return `${agentLabel} has no ${label} grants.`
  }
  return `${agentLabel} ${label} grants:\n${grants.map((grant) => `- ${grant}`).join("\n")}`
}
