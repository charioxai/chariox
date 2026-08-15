import type {
  RuntimeAttachment,
  RuntimeProviderRun,
  RuntimeSession,
  SessionConfigState,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import {
  ROOT_WORKFLOW_SHORTCUTS,
  focusedSessionAgent,
  formatRootWorkflowRunSummary,
  type RootWorkflowShortcutDescriptor,
} from "./root-workflow-shortcuts.js"
import {
  handleCollabSlashCommand,
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
  handleModeSlashCommand,
  handlePermissionsSlashCommand,
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
  handleConnectorSlashCommand,
  handleCredentialSlashCommand,
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
  handleAgentForkCommand,
  handleAgentSlashCommand,
  handleCycleAgentFocus as cycleAgentFocusCommand,
  handleTurnUndoCommand,
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
import {
  handleNotificationSlashCommand,
  type NotificationCommandHandlerDeps,
} from "./notification-command-handler.js"
import { handlePromptSettingsSlashCommand } from "./prompt-settings-command-handler.js"
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
  formatHomeExtensionAuditEvents,
  parseMcpInstallConfig,
} from "./capability-command-handlers.js"

type CommandActionDeps =
  & ProviderCommandHandlerDeps
  & CloudCommandHandlerDeps
  & SelectionCommandHandlerDeps
  & ConfigCommandHandlerDeps
  & RemoteMachineCommandHandlerDeps
  & CapabilityCommandHandlerDeps
  & Omit<SliceCommandHandlerDeps, "currentWorkspaceTarget" | "currentWorktreeTarget">
  & Omit<WorkspaceCommandHandlerDeps, "currentWorkspaceTarget" | "currentWorktreeTarget" | "setWorkspaceTarget" | "setWorktreeTarget" | "baseWorktree" | "hasDynamicWorktreeTarget">
  & Omit<SessionCommandHandlerDeps, "currentWorkspaceTarget" | "currentWorktreeTarget">
  & Omit<AgentCommandHandlerDeps, "currentWorkspaceTarget" | "currentWorktreeTarget">
  & KernelCommandHandlerDeps
  & Omit<WorkflowCommandHandlerDeps, "currentWorkspaceTarget">
  & NotificationCommandHandlerDeps
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
  createAgentPromptSchedule?: (
    sessionId: string,
    agentId: string,
    kind: "once" | "recurring",
    intervalSeconds: number,
    prompt: string,
  ) => Promise<{ session: RuntimeSession }>
  runWorkflowRegistryEntry?: (
    name: string,
    endpointRef: string,
    prompt: string,
    queueRef?: string | null,
    options?: {
      agentRebindings?: Array<{ node: string; agent_ref: string }>
    },
  ) => Promise<{ entry: { name: string }; result: { apply?: { apply?: { workflow_id?: string; agent_ids?: Record<string, string> } }; invocation?: { kind?: string } }; session: RuntimeSession }>
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
  getDaemonHealth?: KernelCommandHandlerDeps["getDaemonHealth"]
  exportDebugBundle?: KernelCommandHandlerDeps["exportDebugBundle"]
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
    currentWorkspaceTarget,
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

  const handleModeCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "mode" }>,
  ): Promise<void> => {
    await handleModeSlashCommand(deps, command)
  }

  const handlePermissionsCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "permissions" }>,
  ): Promise<void> => {
    await handlePermissionsSlashCommand(deps, command)
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

  const handleUndoCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "undo" }>,
  ): Promise<void> => {
    await handleTurnUndoCommand(agentCommandDeps(), command.args)
  }

  const handleForkCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "fork" }>,
  ): Promise<void> => {
    await handleAgentForkCommand(agentCommandDeps(), command.args)
  }

  const handleRelayCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "relay" }>,
  ): Promise<void> => handleRelaySlashCommand(deps, command)

  const handleCloudCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "cloud" }>,
  ): Promise<void> => handleCloudSlashCommand(deps, command)

  const handleCollabCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "collab" }>,
  ): Promise<void> => handleCollabSlashCommand(deps, command)

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
    await handleSliceSlashCommand({ ...deps, currentWorkspaceTarget, currentWorktreeTarget }, command)
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

  const handleCredentialCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "credential" }>,
  ): Promise<void> => {
    await handleCredentialSlashCommand(deps, command)
  }

  const handleConnectorCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "connector" }>,
  ): Promise<void> => {
    await handleConnectorSlashCommand(deps, command)
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

  const handleNotificationsCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "notifications" }>,
  ): Promise<void> => {
    await handleNotificationSlashCommand(deps, command)
  }

  const handleSettingsCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "settings" }>,
  ): Promise<void> => {
    await handlePromptSettingsSlashCommand(deps, command)
  }

  const handleLoopCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "loop" }>,
  ): Promise<void> => {
    await handleRootWorkflowShortcut(deps, ROOT_WORKFLOW_SHORTCUTS.loop, command.prompt)
  }

  const handleGoalCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "goal" }>,
  ): Promise<void> => {
    await handleRootWorkflowShortcut(deps, ROOT_WORKFLOW_SHORTCUTS.goal, command.prompt)
  }

  const handleWaitCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "wait" }>,
  ): Promise<void> => {
    const commandName = command.scheduleKind === "once" ? "/wait-in" : "/wait-every"
    if (command.error || command.minutes === null || !command.prompt.trim()) {
      deps.flashFooter(command.error ?? `usage: ${commandName} <minutes> <prompt>`, "error")
      return
    }
    if (!deps.isAttached()) {
      deps.flashFooter("must be attached to a session to schedule agent prompts", "error")
      return
    }
    const agentId = deps.focusedAgentId()
    if (!agentId) {
      deps.flashFooter("no focused agent", "error")
      return
    }
    if (!deps.createAgentPromptSchedule) {
      deps.flashFooter(`${commandName} is unavailable in this daemon`, "error")
      return
    }
    const result = await deps.createAgentPromptSchedule(
      deps.sessionState().id,
      agentId,
      command.scheduleKind,
      Math.max(1, Math.round(command.minutes * 60)),
      command.prompt,
    )
    deps.applySessionState(result.session)
    await deps.refreshAgentPanes(result.session)
    deps.flashFooter(
      `${commandName} scheduled for ${command.minutes} ${command.minutes === 1 ? "minute" : "minutes"}`,
      "info",
    )
  }

  return {
    handleSessionCommand,
    handleProviderCommand,
    handleModelCommand,
    handleVariantCommand,
    handleModeCommand,
    handlePermissionsCommand,
    handleViewCommand,
    handleCycleAgentFocus,
    handleAgentCommand,
    handleUndoCommand,
    handleForkCommand,
    handleKernelCommand,
    handleMachineCommand,
    handleSliceCommand,
    handleRelayCommand,
    handleCloudCommand,
    handleCollabCommand,
    handleConfigCommand,
    handleWorkspaceCommand,
    handleWorktreeCommand,
    handleWorkflowCommand,
    handleNotificationsCommand,
    handleSettingsCommand,
    handleLoopCommand,
    handleGoalCommand,
    handleWaitCommand,
    handleMcpCommand,
    handleSkillCommand,
    handleEnvCommand,
    handleScriptCommand,
    handleCredentialCommand,
    handleConnectorCommand,
    handleExtensionCommand,
  }
}

async function handleRootWorkflowShortcut(
  deps: CommandActionDeps,
  descriptor: RootWorkflowShortcutDescriptor,
  prompt: string,
): Promise<void> {
  if (!prompt.trim()) {
    deps.flashFooter(`usage: ${descriptor.commandName} <prompt>`, "error")
    return
  }
  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a session to run workflow shortcuts", "error")
    return
  }
  if (!deps.runWorkflowRegistryEntry) {
    deps.flashFooter(`${descriptor.commandName} is unavailable in this daemon`, "error")
    return
  }
  const focusedAgentId = deps.focusedAgentId()
  const focusedAgent = focusedSessionAgent(deps.sessionState(), focusedAgentId)
  if (!focusedAgentId || !focusedAgent) {
    deps.flashFooter(`${descriptor.commandName} requires a focused agent in this session`, "error")
    return
  }
  const payload = await deps.runWorkflowRegistryEntry(descriptor.registryEntryName, descriptor.endpointRef, prompt.trim(), null, {
    agentRebindings: [{ node: descriptor.entryNode, agent_ref: focusedAgent.id }],
  })
  deps.applySessionState(payload.session)
  const workflowId = payload.result.apply?.apply?.workflow_id ?? null
  if (workflowId) {
    deps.selectWorkflowCanvas(workflowId)
    deps.showWorkflowScreen()
  }
  deps.flashFooter(
    formatRootWorkflowRunSummary({
      descriptor,
      focusedAgentId: focusedAgent.id,
      workflowId,
      invocationKind: payload.result.invocation?.kind ?? "invoked",
      ...(payload.result.apply?.apply?.agent_ids !== undefined
        ? { agentIdsByNode: payload.result.apply.apply.agent_ids }
        : {}),
    }),
    "info",
  )
}
