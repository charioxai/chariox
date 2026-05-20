import type { FooterFlash } from "./footer-flash-controller.js"
import {
  executeSlashCommand,
  type ParsedSlashCommand,
} from "./commands.js"

type Command<K extends ParsedSlashCommand["kind"]> = Extract<ParsedSlashCommand, { kind: K }>
type CommandHandler<K extends ParsedSlashCommand["kind"]> = (command: Command<K>) => Promise<unknown> | unknown

type CommandCenterCommandExecutorDeps = {
  onExit: () => Promise<unknown> | unknown
  onWaiting: () => Promise<unknown> | unknown
  onStop: () => Promise<unknown> | unknown
  handleAttachmentCommand: (raw: string) => Promise<unknown> | unknown
  onSession: CommandHandler<"session">
  onProvider: CommandHandler<"provider">
  onModel: CommandHandler<"model">
  onVariant: CommandHandler<"variant">
  onView: CommandHandler<"view">
  onAgent: CommandHandler<"agent">
  onKernel: CommandHandler<"kernel">
  onMachine: CommandHandler<"machine">
  onSlice: CommandHandler<"slice">
  onRelay: CommandHandler<"relay">
  onCloud: CommandHandler<"cloud">
  onConfig: CommandHandler<"config">
  onWorkspace: CommandHandler<"workspace">
  onWorktree: CommandHandler<"worktree">
  onWorkflow: CommandHandler<"workflow">
  onMcp: CommandHandler<"mcp">
  onSkill: CommandHandler<"skill">
  onEnv: CommandHandler<"env">
  onScript: CommandHandler<"script">
  onCredential: CommandHandler<"credential">
  onConnector: CommandHandler<"connector">
  onExtension: CommandHandler<"extension">
  flashFooter: (message: string, tone: FooterFlash["tone"]) => void
  formatError: (error: unknown) => string
}

export function createCommandCenterCommandExecutor(
  deps: CommandCenterCommandExecutorDeps,
) {
  const contained = <K extends ParsedSlashCommand["kind"]>(handler: CommandHandler<K>): CommandHandler<K> =>
    async (command) => {
      try {
        await handler(command)
      } catch (error) {
        deps.flashFooter(deps.formatError(error), "error")
      }
    }

  const execute = async (value: string) => {
    await executeSlashCommand(value, {
      onExit: deps.onExit,
      onWaiting: deps.onWaiting,
      onStop: deps.onStop,
      onAttachment: (command) => deps.handleAttachmentCommand(command.raw),
      onSession: deps.onSession,
      onProvider: deps.onProvider,
      onModel: deps.onModel,
      onVariant: deps.onVariant,
      onView: deps.onView,
      onAgent: contained(deps.onAgent),
      onKernel: contained(deps.onKernel),
      onMachine: contained(deps.onMachine),
      onSlice: contained(deps.onSlice),
      onRelay: contained(deps.onRelay),
      onCloud: contained(deps.onCloud),
      onConfig: contained(deps.onConfig),
      onWorkspace: contained(deps.onWorkspace),
      onWorktree: contained(deps.onWorktree),
      onWorkflow: contained(deps.onWorkflow),
      onMcp: contained(deps.onMcp),
      onSkill: contained(deps.onSkill),
      onEnv: contained(deps.onEnv),
      onScript: contained(deps.onScript),
      onCredential: contained(deps.onCredential),
      onConnector: contained(deps.onConnector),
      onExtension: contained(deps.onExtension),
    })
  }

  return { execute }
}
