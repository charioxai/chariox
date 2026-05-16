import type {
  AgentInstance,
  RuntimeAttachment,
  RuntimeProviderRun,
  RuntimeSession,
  SessionConfigState,
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
  handleWorkflowSlashCommand,
  type WorkflowCommandHandlerDeps,
} from "./workflow-command-handlers.js"
import type {
  LocalGitWorktreeOptions,
  RemoteGitWorktreePlacement,
} from "./command-worktree-placement.js"

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
  & Omit<WorkflowCommandHandlerDeps, "currentWorkspaceTarget">
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
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  refreshSplitPaneFocusRepaint: () => void
}

export function createCommandActionHandlers(deps: CommandActionDeps) {
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
  const workflowCommandDeps = (): WorkflowCommandHandlerDeps => ({
    ...deps,
    currentWorkspaceTarget,
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
    await handleWorkflowSlashCommand(workflowCommandDeps(), command)
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
