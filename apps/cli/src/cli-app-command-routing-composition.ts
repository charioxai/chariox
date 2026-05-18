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
  const commandCenterCommandExecutor = createCommandCenterCommandExecutor({
    onExit: () => deps.requestExit(),
    onWaiting: () => deps.requestWaitingRoom(),
    onStop: () => requestPromptStop(),
    handleAttachmentCommand: deps.handleAttachmentCommand,
    onSession: commandHandlers.handleSessionCommand,
    onProvider: commandHandlers.handleProviderCommand,
    onModel: commandHandlers.handleModelCommand,
    onVariant: commandHandlers.handleVariantCommand,
    onView: commandHandlers.handleViewCommand,
    onAgent: commandHandlers.handleAgentCommand,
    onKernel: commandHandlers.handleKernelCommand,
    onMachine: commandHandlers.handleMachineCommand,
    onSlice: commandHandlers.handleSliceCommand,
    onRelay: commandHandlers.handleRelayCommand,
    onCloud: commandHandlers.handleCloudCommand,
    onConfig: commandHandlers.handleConfigCommand,
    onWorkspace: commandHandlers.handleWorkspaceCommand,
    onWorktree: commandHandlers.handleWorktreeCommand,
    onWorkflow: commandHandlers.handleWorkflowCommand,
    onMcp: commandHandlers.handleMcpCommand,
    onSkill: commandHandlers.handleSkillCommand,
    flashFooter: deps.flashFooter,
    formatError: deps.formatError,
  })
  const inputRouting = createCliInputRoutingComposition({
    ...deps,
    ...commandHandlers,
  } as any)
  requestPromptStop = inputRouting.requestPromptStop

  return {
    ...commandHandlers,
    ...inputRouting,
    executeCommandCenterCommand: commandCenterCommandExecutor.execute,
  }
}
