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
  formatAgentInspectSummary,
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
  formatAgentInspectSummary,
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
      const session = deps.sessionState()
      const providerRun = deps.providerRunState()
      const slices = deps.listSlices ? await deps.listSlices().catch(() => []) : []
      deps.appendNotice(formatAgentListSummary(session.agents, slices, {
        activeProviderRunId: session.active_provider_run_id,
        activeProviderRunAgentId: providerRun?.agent_instance_id ?? null,
      }, {
        homeKernelId: session.host_daemon_id ?? null,
        homeMachineId: session.host_machine_id ?? null,
        ownerUserId: session.owner_user_id ?? null,
        workspaceLiveSyncMode: session.workspace_live_sync_mode ?? null,
        workspaceLiveSyncWorktree: session.worktree_id ?? null,
      }))
      deps.flashFooter(
        session.agents.length === 0
          ? "no agents in session"
          : `showing ${session.agents.length} agent${session.agents.length === 1 ? "" : "s"}`,
        "info",
      )
      return
    }
    case "inspect":
    case "info":
    case "show": {
      const resolved = deps.resolveSessionAgent(args[1] ?? deps.sessionState().focused_agent_id)
      if (resolved.error || !resolved.agent) {
        deps.flashFooter(resolved.error ?? "usage: /agent inspect [agent-ref]", "error")
        return
      }
      const slices = deps.listSlices ? await deps.listSlices().catch(() => []) : []
      const session = deps.sessionState()
      const providerRun = deps.providerRunState()
      deps.appendNotice(formatAgentInspectSummary(resolved.agent, slices, {
        activeProviderRunId: session.active_provider_run_id,
        activeProviderRunAgentId: providerRun?.agent_instance_id ?? null,
      }, {
        homeKernelId: session.host_daemon_id ?? null,
        homeMachineId: session.host_machine_id ?? null,
        ownerUserId: session.owner_user_id ?? null,
        workspaceLiveSyncMode: session.workspace_live_sync_mode ?? null,
        workspaceLiveSyncWorktree: session.worktree_id ?? null,
      }))
      deps.flashFooter(`showing agent ${deps.formatAgentLabel(resolved.agent)}`, "info")
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
        "usage: /agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>|--kernel <kernel-ref>|--slice off|new:headless|new:headed|<slice-ref>] | /agent spawn <count> | delete [agent-name|agent-alias] | focus <agent-id> | alias [agent-ref] <alias|clear> | provider/model/variant [agent-ref] <value> | list | inspect [agent-ref] | cycle | mode [agent-ref] <build|plan|inherit> | permissions [agent-ref] <required|yolo|inherit> | substitute ...",
        "error",
      )
  }
}
