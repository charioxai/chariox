import type { ParsedSlashCommand } from "./commands.js"
import {
  handleAgentAliasCommand,
  handleAgentModeCommand,
  handleAgentPermissionsCommand,
  handleAgentProfileCommand,
  type AgentConfigCommandHandlerDeps,
} from "./agent-config-command-handlers.js"
import {
  handleAgentSpawnCommand,
  type AgentSpawnCommandHandlerDeps,
} from "./agent-spawn-command-handlers.js"
import {
  handleAgentSubstituteCommand,
  type AgentSubstituteCommandHandlerDeps,
} from "./agent-substitute-command-handlers.js"
import {
  formatAgentListSummary,
  handleAgentDeleteCommand,
  handleAgentFocusCommand,
  handleCycleAgentFocus,
  type AgentLifecycleCommandHandlerDeps,
} from "./agent-lifecycle-command-handlers.js"

export type AgentCommandHandlerDeps =
  & AgentConfigCommandHandlerDeps
  & AgentLifecycleCommandHandlerDeps
  & AgentSpawnCommandHandlerDeps
  & AgentSubstituteCommandHandlerDeps

export {
  formatAgentListSummary,
  handleCycleAgentFocus,
} from "./agent-lifecycle-command-handlers.js"

export async function handleAgentSlashCommand(
  deps: AgentCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "agent" }>,
): Promise<void> {
  const args = command.args
  const subcommand = args[0]

  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a session to manage agents", "error")
    return
  }

  switch (subcommand) {
    case "spawn": {
      await handleAgentSpawnCommand(deps, args.slice(1))
      return
    }
    case "delete":
    case "destroy": {
      await handleAgentDeleteCommand(deps, args)
      return
    }
    case "focus": {
      await handleAgentFocusCommand(deps, args)
      return
    }
    case "alias":
    case "name": {
      await handleAgentAliasCommand(deps, args)
      return
    }
    case "list":
    case "ls": {
      deps.flashFooter(formatAgentListSummary(deps.sessionState().agents), "info")
      return
    }
    case "cycle": {
      await handleCycleAgentFocus(deps)
      return
    }
    case "mode": {
      await handleAgentModeCommand(deps, args)
      return
    }
    case "provider":
    case "model":
    case "variant": {
      await handleAgentProfileCommand(deps, args, subcommand)
      return
    }
    case "permissions": {
      await handleAgentPermissionsCommand(deps, args)
      return
    }
    case "substitute":
    case "subs": {
      await handleAgentSubstituteCommand(deps, args)
      return
    }
    default:
      deps.flashFooter(
        "usage: /agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>|--slice <slice-ref>] | /agent spawn <count> | delete [agent-name|agent-alias] | focus <agent-id> | alias [agent-ref] <alias|clear> | provider/model/variant [agent-ref] <value> | list | cycle | mode [agent-ref] <build|plan|inherit> | permissions [agent-ref] <required|yolo|inherit> | substitute ...",
        "error",
      )
  }
}
