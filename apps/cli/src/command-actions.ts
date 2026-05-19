import type {
  RuntimeAttachment,
  RuntimeProviderRun,
  RuntimeSession,
  SessionConfigState,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import {
  handleCloudSlashCommand,
  handleRelaySlashCommand,
  type CloudCommandHandlerDeps,
} from "./cloud-command-handlers.js"
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
  handleEnvironmentSlashCommand,
  handleExtensionSlashCommand,
  handleScriptSlashCommand,
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
} from "./command-worktree-placement.js"

type FooterTone = "info" | "error"

export { parseRequestedViewLayout } from "./selection-command-handlers.js"
export {
  formatAgentListSummary,
} from "./agent-command-handlers.js"
export { formatAgentSubstituteSummary } from "./agent-substitute-command-handlers.js"
export {
  formatAgentCapabilityGrants,
  parseMcpInstallConfig,
} from "./capability-command-handlers.js"

type CommandActionDeps =
  & ProviderCommandHandlerDeps
  & CloudCommandHandlerDeps
  & SelectionCommandHandlerDeps
  & ConfigCommandHandlerDeps
  & RemoteMachineCommandHandlerDeps
  & CapabilityCommandHandlerDeps
  & Omit<SliceCommandHandlerDeps, "currentWorktreeTarget">
  & Omit<WorkspaceCommandHandlerDeps, "currentWorkspaceTarget" | "currentWorktreeTarget" | "setWorkspaceTarget" | "setWorktreeTarget" | "baseWorktree" | "hasDynamicWorktreeTarget">
  & Omit<SessionCommandHandlerDeps, "currentWorkspaceTarget" | "currentWorktreeTarget">
  & Omit<AgentCommandHandlerDeps, "currentWorktreeTarget">
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
  formatError: (error: unknown) => string
  prepareLocalGitWorktree?: (options: LocalGitWorktreeOptions) => Promise<string>
  attachBinding: (
    session: Pick<RuntimeSession, "id">,
    createdSession: boolean,
  ) => Promise<void>
  deleteKernel?: () => Promise<{ kernelId: string; deletedSessions: RuntimeSession[] }>
  transitionToNoSession: (message: string) => void
  updateSessionConfig: (
    sessionId: string,
    attachmentId: string,
    values: Record<string, string>,
    requiresIdle: boolean,
  ) => Promise<{ session: RuntimeSession; config: SessionConfigState }>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  rebuildTranscript: () => void
  requestRender: () => void
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

  const handleEnvCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "env" }>,
  ): Promise<void> => {
    await handleEnvironmentSlashCommand(deps, command)
  }

  const handleScriptCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "script" }>,
  ): Promise<void> => {
    await handleScriptSlashCommand(deps, command)
  }

  const handleExtensionCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "extension" }>,
  ): Promise<void> => {
    await handleExtensionSlashCommand(deps, command)
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
    handleEnvCommand,
    handleScriptCommand,
    handleExtensionCommand,
  }
}
