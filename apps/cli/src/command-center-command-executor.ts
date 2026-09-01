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
  onAccount?: CommandHandler<"account">
  onModel: CommandHandler<"model">
  onVariant: CommandHandler<"variant">
  onMode: CommandHandler<"mode">
  onPermissions: CommandHandler<"permissions">
  onView: CommandHandler<"view">
  onUndo: CommandHandler<"undo">
  onFork: CommandHandler<"fork">
  onAgent: CommandHandler<"agent">
  onKernel: CommandHandler<"kernel">
  onMachine: CommandHandler<"machine">
  onSlice: CommandHandler<"slice">
  onRelay: CommandHandler<"relay">
  onCloud: CommandHandler<"cloud">
  onCollab: CommandHandler<"collab">
  onConfig: CommandHandler<"config">
  onWorkspace: CommandHandler<"workspace">
  onWorktree: CommandHandler<"worktree">
  onWorkflow: CommandHandler<"workflow">
  onNotifications?: CommandHandler<"notifications">
  onSettings?: CommandHandler<"settings">
  onLoop: CommandHandler<"loop">
  onGoal: CommandHandler<"goal">
  onWait: CommandHandler<"wait">
  onMcp: CommandHandler<"mcp">
  onSkill: CommandHandler<"skill">
  onEnv: CommandHandler<"env">
  onScript: CommandHandler<"script">
  onCredential: CommandHandler<"credential">
  onConnector: CommandHandler<"connector">
  onExtension: CommandHandler<"extension">
  flashFooter: (message: string, tone: FooterFlash["tone"]) => void
  formatError: (error: unknown) => string
  handleSharedShellCommand?: (rawCommand: string) => Promise<boolean>
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
    if (deps.handleSharedShellCommand) {
      try {
        if (await deps.handleSharedShellCommand(value)) {
          return
        }
      } catch (error) {
        deps.flashFooter(deps.formatError(error), "error")
        return
      }
    }
    await executeSlashCommand(value, {
      onExit: deps.onExit,
      onWaiting: deps.onWaiting,
      onStop: deps.onStop,
      onAttachment: (command) => deps.handleAttachmentCommand(command.raw),
      onSession: deps.onSession,
      onProvider: deps.onProvider,
      ...(deps.onAccount ? { onAccount: deps.onAccount } : {}),
      onModel: deps.onModel,
      onVariant: deps.onVariant,
      onMode: deps.onMode,
      onPermissions: deps.onPermissions,
      onView: deps.onView,
      onUndo: contained(deps.onUndo),
      onFork: contained(deps.onFork),
      onAgent: contained(deps.onAgent),
      onKernel: contained(deps.onKernel),
      onMachine: contained(deps.onMachine),
      onSlice: contained(deps.onSlice),
      onRelay: contained(deps.onRelay),
      onCloud: contained(deps.onCloud),
      onCollab: contained(deps.onCollab),
      onConfig: contained(deps.onConfig),
      onWorkspace: contained(deps.onWorkspace),
      onWorktree: contained(deps.onWorktree),
      onWorkflow: contained(deps.onWorkflow),
      ...(deps.onNotifications ? { onNotifications: contained(deps.onNotifications) } : {}),
      ...(deps.onSettings ? { onSettings: contained(deps.onSettings) } : {}),
      onLoop: contained(deps.onLoop),
      onGoal: contained(deps.onGoal),
      onWait: contained(deps.onWait),
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
