import { createCliCommandActionComposition } from "./cli-command-action-composition.js"
import { createCliInputRoutingComposition } from "./cli-input-routing-composition.js"
import { createCommandCenterCommandExecutor } from "./command-center-command-executor.js"

type AnyFn = (...args: any[]) => any

export type CliAppCommandRoutingCompositionDeps = Record<string, any> & {
  requestExit: AnyFn
  requestWaitingRoom: AnyFn
  handleAttachmentCommand: AnyFn
  flashFooter: AnyFn
  formatError: AnyFn
}

export function createCliAppCommandRoutingComposition(
  deps: CliAppCommandRoutingCompositionDeps,
) {
  const commandHandlers = createCliCommandActionComposition(deps as any)
  let requestPromptStop: AnyFn = () => undefined
  let routeSharedShellCommand: (rawCommand: string) => Promise<boolean> = async () => false
  const commandCenterCommandExecutor = createCommandCenterCommandExecutor({
    onExit: () => deps.requestExit(),
    onWaiting: () => deps.requestWaitingRoom(),
    onStop: () => requestPromptStop(),
    handleAttachmentCommand: deps.handleAttachmentCommand,
    onSession: commandHandlers.handleSessionCommand,
    onProvider: commandHandlers.handleProviderCommand,
    onModel: commandHandlers.handleModelCommand,
    onVariant: commandHandlers.handleVariantCommand,
    onMode: commandHandlers.handleModeCommand,
    onPermissions: commandHandlers.handlePermissionsCommand,
    onView: commandHandlers.handleViewCommand,
    onUndo: commandHandlers.handleUndoCommand,
    onFork: commandHandlers.handleForkCommand,
    onAgent: commandHandlers.handleAgentCommand,
    onKernel: commandHandlers.handleKernelCommand,
    onMachine: commandHandlers.handleMachineCommand,
    onSlice: commandHandlers.handleSliceCommand,
    onRelay: commandHandlers.handleRelayCommand,
    onCloud: commandHandlers.handleCloudCommand,
    onCollab: commandHandlers.handleCollabCommand,
    onConfig: commandHandlers.handleConfigCommand,
    onWorkspace: commandHandlers.handleWorkspaceCommand,
    onWorktree: commandHandlers.handleWorktreeCommand,
    onWorkflow: commandHandlers.handleWorkflowCommand,
    onLoop: commandHandlers.handleLoopCommand,
    onGoal: commandHandlers.handleGoalCommand,
    onWait: commandHandlers.handleWaitCommand,
    onMcp: commandHandlers.handleMcpCommand,
    onSkill: commandHandlers.handleSkillCommand,
    onEnv: commandHandlers.handleEnvCommand,
    onScript: commandHandlers.handleScriptCommand,
    onCredential: commandHandlers.handleCredentialCommand,
    onConnector: commandHandlers.handleConnectorCommand,
    onExtension: commandHandlers.handleExtensionCommand,
    flashFooter: deps.flashFooter,
    formatError: deps.formatError,
    handleSharedShellCommand: (rawCommand) => routeSharedShellCommand(rawCommand),
  })
  const inputRouting = createCliInputRoutingComposition({
    ...deps,
    ...commandHandlers,
    hasActiveTurnWork: deps.anyTurnWork,
  } as any)
  requestPromptStop = inputRouting.requestPromptStop
  routeSharedShellCommand = inputRouting.handleSharedShellCommand

  return {
    ...commandHandlers,
    ...inputRouting,
    executeCommandCenterCommand: commandCenterCommandExecutor.execute,
  }
}
