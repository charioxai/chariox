import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import { responsePaneBindingsMatch, selectResponsePaneAgents } from "./response-panes.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"
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

type FooterTone = "info" | "error"

type AgentCyclePayload = {
  agent: AgentInstance | null
  session: RuntimeSession
}

type AgentFocusPayload = {
  agent: AgentInstance
  session: RuntimeSession
}

export type AgentCommandHandlerDeps =
  & AgentConfigCommandHandlerDeps
  & AgentSpawnCommandHandlerDeps
  & AgentSubstituteCommandHandlerDeps
  & {
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  currentModelId: () => string
  currentVariantId: () => string
  focusedAgentId: () => string | null
  multiAgentResponseLayout: () => MultiAgentResponseLayout
  maxAgentsPerScreen: () => number
  flashFooter: (message: string, tone: FooterTone) => void
  formatError: (error: unknown) => string
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  rebuildTranscript: () => void
  cycleAgentFocus: () => Promise<AgentCyclePayload>
  launchAgentProviderRun: (
    provider: string,
    model: string,
    variant: string,
    agentId: string,
  ) => Promise<RuntimeProviderRun>
  setProviderRunState: (run: RuntimeProviderRun | null) => void
  refreshSessionState: (sessionId: string) => Promise<RuntimeSession>
  destroyAgent: (agentId: string) => Promise<RuntimeSession>
  focusAgent: (agentId: string) => Promise<AgentFocusPayload>
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  refreshSplitPaneFocusRepaint: () => void
}

export async function handleCycleAgentFocus(
  deps: AgentCommandHandlerDeps,
): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a session to cycle agents", "error")
    return
  }
  try {
    const previousSession = deps.sessionState()
    const previousSelection = selectResponsePaneAgents(
      previousSession.agents,
      previousSession.focused_agent_id,
      deps.multiAgentResponseLayout() === "split",
      deps.maxAgentsPerScreen(),
    )
    const payload = await deps.cycleAgentFocus()
    const nextSession = payload.session
    const nextSelection = selectResponsePaneAgents(
      nextSession.agents,
      nextSession.focused_agent_id,
      deps.multiAgentResponseLayout() === "split",
      deps.maxAgentsPerScreen(),
    )
    const shouldRefreshPaneContents = deps.multiAgentResponseLayout() !== "split"
      || !responsePaneBindingsMatch(previousSelection, nextSelection)
    deps.applySessionState(nextSession)
    if (shouldRefreshPaneContents) {
      await deps.refreshAgentPanes(nextSession)
    }
    if (!nextSession.active_provider_run_id && payload.agent) {
      const run = await deps.launchAgentProviderRun(
        payload.agent.provider,
        payload.agent.model ?? deps.currentModelId(),
        deps.currentVariantId(),
        payload.agent.id,
      )
      deps.setProviderRunState(run)
      deps.applySessionState(await deps.refreshSessionState(nextSession.id))
    }
    if (payload.agent) {
      deps.flashFooter(
        `cycled to agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`,
        "info",
      )
    } else {
      deps.flashFooter("no agents to cycle", "info")
    }
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

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
      const reference = args[1]
      const resolved = deps.resolveSessionAgent(reference)
      if (resolved.error || !resolved.agent) {
        deps.flashFooter(resolved.error ?? "usage: /agent delete <agent-name|agent-alias>", "error")
        return
      }
      try {
        const nextSession = await deps.destroyAgent(resolved.agent.id)
        deps.applySessionState(nextSession)
        await deps.refreshAgentPanes(nextSession)
        deps.rebuildTranscript()
        deps.refreshSplitPaneFocusRepaint()
        deps.flashFooter(`deleted agent ${deps.formatAgentLabel(resolved.agent)}`, "info")
      } catch (error) {
        deps.flashFooter(deps.formatError(error), "error")
      }
      return
    }
    case "focus": {
      const agentId = args[1]
      if (!agentId) {
        deps.flashFooter("usage: /agent focus <agent-id>", "error")
        return
      }
      try {
        const payload = await deps.focusAgent(agentId)
        const nextSession = payload.session
        const previousSession = deps.sessionState()
        const previousSelection = selectResponsePaneAgents(
          previousSession.agents,
          previousSession.focused_agent_id,
          deps.multiAgentResponseLayout() === "split",
          deps.maxAgentsPerScreen(),
        )
        const nextSelection = selectResponsePaneAgents(
          nextSession.agents,
          nextSession.focused_agent_id,
          deps.multiAgentResponseLayout() === "split",
          deps.maxAgentsPerScreen(),
        )
        const shouldRefreshPaneContents = deps.multiAgentResponseLayout() !== "split"
          || !responsePaneBindingsMatch(previousSelection, nextSelection)
        deps.applySessionState(nextSession)
        if (shouldRefreshPaneContents) {
          await deps.refreshAgentPanes(nextSession)
        }
        if (!nextSession.active_provider_run_id) {
          const run = await deps.launchAgentProviderRun(
            payload.agent.provider,
            payload.agent.model ?? deps.currentModelId(),
            deps.currentVariantId(),
            payload.agent.id,
          )
          deps.setProviderRunState(run)
          deps.applySessionState(await deps.refreshSessionState(nextSession.id))
        }
        deps.flashFooter(
          `focused on agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`,
          "info",
        )
      } catch (error) {
        deps.flashFooter(deps.formatError(error), "error")
      }
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

export function formatAgentListSummary(agents: AgentInstance[]): string {
  if (agents.length === 0) {
    return "no agents in session"
  }
  const agentList = agents
    .map((agent) => `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""} [${agent.state}]`)
    .join(", ")
  return `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`
}
