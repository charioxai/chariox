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
  handleAgentTaskCommand,
  type AgentTaskCommandHandlerDeps,
} from "./agent-task-command-handlers.js"
import {
  formatAgentInspectSummary,
  formatAgentListSummary,
  handleAgentDeleteCommand,
  handleAgentFocusCommand,
  handleCycleAgentFocus,
  type AgentLifecycleCommandHandlerDeps,
} from "./agent-lifecycle-command-handlers.js"
import type { AgentForkPayload, RuntimeSession, TurnUndoResult } from "./cli-types.js"

export type AgentCommandHandlerDeps =
  & AgentConfigCommandHandlerDeps
  & AgentLifecycleCommandHandlerDeps
  & AgentSpawnCommandHandlerDeps
  & AgentSubstituteCommandHandlerDeps
  & AgentTaskCommandHandlerDeps
  & {
    undoTurn?: (agentRef?: string | null, turnRef?: string | null) => Promise<TurnUndoResult>
    forkAgent?: (sourceAgentRef?: string | null, alias?: string | null) => Promise<AgentForkPayload>
    refreshSessionState: (sessionId: string) => Promise<RuntimeSession>
    applySessionState: (session: RuntimeSession) => void
    refreshAgentPanes: (session: RuntimeSession) => Promise<void>
    rebuildTranscript: () => void
  }

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
    case "fork": {
      await handleAgentForkCommand(deps, args.slice(1))
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
        session,
        homeKernelId: session.host_daemon_id ?? null,
        homeMachineId: session.host_machine_id ?? null,
        ownerUserId: session.owner_user_id ?? null,
        workspaceLiveSyncMode: session.workspace_live_sync_mode ?? null,
        workspaceLiveSyncWorktree: session.worktree_id ?? null,
        agentActivity: session.agent_activity ?? null,
        promptStates: session.prompt_states ?? null,
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
      let sliceLookupError: string | null = null
      const slices = deps.listSlices
        ? await deps.listSlices().catch((error) => {
            sliceLookupError = deps.formatError(error)
            return []
          })
        : []
      const session = deps.sessionState()
      const providerRun = deps.providerRunState()
      deps.appendNotice(formatAgentInspectSummary(resolved.agent, slices, {
        activeProviderRunId: session.active_provider_run_id,
        activeProviderRunAgentId: providerRun?.agent_instance_id ?? null,
      }, {
        session,
        homeKernelId: session.host_daemon_id ?? null,
        homeMachineId: session.host_machine_id ?? null,
        ownerUserId: session.owner_user_id ?? null,
        workspaceLiveSyncMode: session.workspace_live_sync_mode ?? null,
        workspaceLiveSyncWorktree: session.worktree_id ?? null,
        agentActivity: session.agent_activity ?? null,
        promptStates: session.prompt_states ?? null,
      }, sliceLookupError))
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
    case "task": {
      await handleAgentTaskCommand(deps, args)
      return
    }
    default:
      deps.flashFooter(
        "usage: /agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>|--kernel <kernel-ref>|--slice off|new:headless|new:headed|<slice-ref>] | /agent spawn <count> | fork [agent-ref] | delete [agent-name|agent-alias] | focus <agent-id> | alias [agent-ref] <alias|clear> | provider/model/variant [agent-ref] <value> | list | inspect [agent-ref] | cycle | mode [agent-ref] <build|plan|inherit> | permissions [agent-ref] <required|yolo|inherit> | task [show|edit|plan|pause|resume|abort] | substitute ...",
        "error",
      )
  }
}

export async function handleTurnUndoCommand(
  deps: AgentCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a session to undo a turn", "error")
    return
  }
  if (args.length > 1) {
    deps.flashFooter("usage: /undo [agent-ref]", "error")
    return
  }
  if (!deps.undoTurn) {
    deps.flashFooter("turn undo is unavailable in this client", "error")
    return
  }

  const sessionId = deps.sessionState().id
  const result = await deps.undoTurn(args[0] ?? null, null)
  const session = await deps.refreshSessionState(sessionId)
  deps.applySessionState(session)
  await deps.refreshAgentPanes(session)
  deps.rebuildTranscript()
  deps.appendNotice(`undid turn ${result.turn_id} for ${result.agent_id}; reverted ${result.reverted_paths.length} path${result.reverted_paths.length === 1 ? "" : "s"}`)
  deps.flashFooter(`undid turn for ${result.agent_id}`, "info")
}

export async function handleAgentForkCommand(
  deps: AgentCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a session to fork an agent", "error")
    return
  }
  if (args.length > 1) {
    deps.flashFooter("usage: /fork [agent-ref]", "error")
    return
  }
  if (!deps.forkAgent) {
    deps.flashFooter("agent fork is unavailable in this client", "error")
    return
  }

  const payload = await deps.forkAgent(args[0] ?? null, null)
  deps.applySessionState(payload.session)
  await deps.refreshAgentPanes(payload.session)
  deps.setProviderRunState(payload.provider_run)
  deps.rebuildTranscript()
  deps.appendNotice(`forked ${payload.source_agent_id} as ${deps.formatAgentLabel(payload.agent)}`)
  deps.flashFooter(`forked agent ${deps.formatAgentLabel(payload.agent)}`, "info")
}
