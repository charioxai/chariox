import {
  executeSlashCommand,
  parseSlashCommand,
  shouldClearCommandCenterForSlashCommand,
  type ParsedSlashCommand,
} from "./commands.js"
import type { FooterFlash } from "./footer-flash-controller.js"

type SlashCommand<K extends ParsedSlashCommand["kind"]> = Extract<ParsedSlashCommand, { kind: K }>

export type SlashCommandSubmitControllerDeps = {
  isAttached: () => boolean
  getSessionId: () => string
  recordPromptAreaHistoryEntry: (sessionId: string, rawPrompt: string) => void
  clearPromptText: () => void
  setPromptHistoryIndex: (index: number | null) => void
  setPromptHistoryDraft: (draft: string | null) => void
  clearCommandCenter: () => void
  flashFooter: (message: string, tone: FooterFlash["tone"]) => void
  logError?: (message: string, fields: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
  onExit: () => Promise<unknown> | unknown
  onWaiting: () => Promise<unknown> | unknown
  onStop: () => Promise<unknown> | unknown
  handleAttachmentCommand: (raw: string) => Promise<unknown> | unknown
  handleSessionCommand: (command: SlashCommand<"session">) => Promise<boolean> | boolean
  handleProviderCommand: (command: SlashCommand<"provider">) => Promise<unknown> | unknown
  handleModelCommand: (command: SlashCommand<"model">) => Promise<unknown> | unknown
  handleVariantCommand: (command: SlashCommand<"variant">) => Promise<unknown> | unknown
  handleViewCommand: (command: SlashCommand<"view">) => Promise<unknown> | unknown
  handleUndoCommand: (command: SlashCommand<"undo">) => Promise<unknown> | unknown
  handleForkCommand: (command: SlashCommand<"fork">) => Promise<unknown> | unknown
  handleAgentCommand: (command: SlashCommand<"agent">) => Promise<unknown> | unknown
  handleKernelCommand: (command: SlashCommand<"kernel">) => Promise<unknown> | unknown
  handleMachineCommand: (command: SlashCommand<"machine">) => Promise<unknown> | unknown
  handleSliceCommand: (command: SlashCommand<"slice">) => Promise<unknown> | unknown
  handleRelayCommand: (command: SlashCommand<"relay">) => Promise<unknown> | unknown
  handleCloudCommand: (command: SlashCommand<"cloud">) => Promise<unknown> | unknown
  handleCollabCommand: (command: SlashCommand<"collab">) => Promise<unknown> | unknown
  handleConfigCommand: (command: SlashCommand<"config">) => Promise<unknown> | unknown
  handleWorkspaceCommand: (command: SlashCommand<"workspace">) => Promise<unknown> | unknown
  handleWorktreeCommand: (command: SlashCommand<"worktree">) => Promise<unknown> | unknown
  handleWorkflowCommand: (command: SlashCommand<"workflow">) => Promise<unknown> | unknown
  handleMcpCommand: (command: SlashCommand<"mcp">) => Promise<unknown> | unknown
  handleSkillCommand: (command: SlashCommand<"skill">) => Promise<unknown> | unknown
  handleEnvCommand: (command: SlashCommand<"env">) => Promise<unknown> | unknown
  handleScriptCommand: (command: SlashCommand<"script">) => Promise<unknown> | unknown
  handleCredentialCommand: (command: SlashCommand<"credential">) => Promise<unknown> | unknown
  handleConnectorCommand: (command: SlashCommand<"connector">) => Promise<unknown> | unknown
  handleExtensionCommand: (command: SlashCommand<"extension">) => Promise<unknown> | unknown
}

export type SlashCommandSubmitController = {
  submit(rawPrompt: string, options: { allowSlashCommandSubmission: boolean; trimmedPrompt?: string }): Promise<ParsedSlashCommand | null>
}

export function createSlashCommandSubmitController(
  deps: SlashCommandSubmitControllerDeps,
): SlashCommandSubmitController {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  const reportCommandError = (message: string, command: string, error: unknown) => {
    deps.logError?.(message, {
      command,
      error: formatError(error),
    })
    deps.flashFooter(formatError(error), "error")
  }

  const runWithFooterError = async <TCommand extends ParsedSlashCommand>(
    handler: (command: TCommand) => Promise<unknown> | unknown,
    command: TCommand,
  ) => {
    try {
      await handler(command)
    } catch (error) {
      deps.flashFooter(formatError(error), "error")
    }
  }

  const clearHandledCommandUi = (command: ParsedSlashCommand) => {
    deps.clearPromptText()
    deps.setPromptHistoryIndex(null)
    deps.setPromptHistoryDraft(null)
    if (shouldClearCommandCenterForSlashCommand(command)) {
      deps.clearCommandCenter()
    }
  }

  return {
    async submit(rawPrompt, options) {
      if (!options.allowSlashCommandSubmission) {
        return null
      }
      const slashCommand = parseSlashCommand(rawPrompt)
      if (slashCommand && deps.isAttached()) {
        deps.recordPromptAreaHistoryEntry(deps.getSessionId(), rawPrompt)
      }

      const trimmed = options.trimmedPrompt ?? rawPrompt.trim()
      const handledCommand = await executeSlashCommand(rawPrompt, {
        onExit: deps.onExit,
        onWaiting: deps.onWaiting,
        onStop: deps.onStop,
        onAttachment: async (command) => {
          try {
            await deps.handleAttachmentCommand(command.raw)
          } catch (error) {
            reportCommandError("attachment command failed", trimmed, error)
          }
        },
        onSession: async (command) => {
          try {
            const handled = await deps.handleSessionCommand(command)
            if (!handled) {
              deps.flashFooter("unknown /session command", "error")
            }
          } catch (error) {
            reportCommandError("session command failed", trimmed, error)
          }
        },
        onProvider: (command) => runWithFooterError(deps.handleProviderCommand, command),
        onModel: (command) => runWithFooterError(deps.handleModelCommand, command),
        onVariant: (command) => runWithFooterError(deps.handleVariantCommand, command),
        onView: (command) => runWithFooterError(deps.handleViewCommand, command),
        onUndo: (command) => runWithFooterError(deps.handleUndoCommand, command),
        onFork: (command) => runWithFooterError(deps.handleForkCommand, command),
        onAgent: (command) => runWithFooterError(deps.handleAgentCommand, command),
        onKernel: (command) => runWithFooterError(deps.handleKernelCommand, command),
        onMachine: (command) => runWithFooterError(deps.handleMachineCommand, command),
        onSlice: (command) => runWithFooterError(deps.handleSliceCommand, command),
        onRelay: (command) => runWithFooterError(deps.handleRelayCommand, command),
        onCloud: (command) => runWithFooterError(deps.handleCloudCommand, command),
        onCollab: (command) => runWithFooterError(deps.handleCollabCommand, command),
        onConfig: (command) => runWithFooterError(deps.handleConfigCommand, command),
        onWorkspace: (command) => runWithFooterError(deps.handleWorkspaceCommand, command),
        onWorktree: (command) => runWithFooterError(deps.handleWorktreeCommand, command),
        onWorkflow: (command) => runWithFooterError(deps.handleWorkflowCommand, command),
        onMcp: (command) => runWithFooterError(deps.handleMcpCommand, command),
        onSkill: (command) => runWithFooterError(deps.handleSkillCommand, command),
        onEnv: (command) => runWithFooterError(deps.handleEnvCommand, command),
        onScript: (command) => runWithFooterError(deps.handleScriptCommand, command),
        onCredential: (command) => runWithFooterError(deps.handleCredentialCommand, command),
        onConnector: (command) => runWithFooterError(deps.handleConnectorCommand, command),
        onExtension: (command) => runWithFooterError(deps.handleExtensionCommand, command),
      })
      if (!handledCommand) {
        return null
      }
      clearHandledCommandUi(handledCommand)
      return handledCommand
    },
  }
}
