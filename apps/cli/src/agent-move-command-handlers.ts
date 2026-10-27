import type { AgentInstance } from "@chariox/kernel-client"

import type { AgentLifecycleCommandHandlerDeps } from "./agent-lifecycle-command-handlers.js"

export type AgentMoveCommandHandlerDeps = Pick<
  AgentLifecycleCommandHandlerDeps,
  | "flashFooter"
  | "formatError"
  | "resolveSessionAgent"
  | "formatAgentLabel"
  | "refreshSessionState"
  | "applySessionState"
  | "refreshAgentPanes"
  | "rebuildTranscript"
  | "sessionState"
> & {
  /** Resolves with the agent as the kernel answered: moved, or holding a Move that waits for its turn. */
  moveAgent: (agentId: string, destination: string | null) => Promise<AgentInstance>
  cancelAgentMove: (agentId: string) => Promise<void>
  pause: (ms: number) => Promise<void>
}

const moveStatusPollMs = 2000

const usage = "usage: /agent move <kernel|home> [agent]  or  /agent move cancel [agent]"

/**
 * Moves an agent's execution to another kernel, home to return, or cancels a move that waits for the turn.
 * The command returns at once. The kernel owns a move that waits for the turn and publishes it on the agent, so the outcome is read from there.
 */
export async function handleAgentMoveCommand(
  deps: AgentMoveCommandHandlerDeps,
  args: string[],
): Promise<void> {
  const destination = args[1]
  if (!destination || args.length > 3) {
    deps.flashFooter(usage, "error")
    return
  }
  const resolved = deps.resolveSessionAgent(args[2])
  if (resolved.error || !resolved.agent) {
    deps.flashFooter(resolved.error ?? usage, "error")
    return
  }
  const agent = resolved.agent
  const label = deps.formatAgentLabel(agent)
  if (destination === "cancel") {
    try {
      await deps.cancelAgentMove(agent.id)
      deps.flashFooter(`cancelled the move of ${label}`, "info")
    } catch (error) {
      deps.flashFooter(deps.formatError(error), "error")
    }
    return
  }
  const target = destination === "home" ? null : destination
  const where = target === null ? "home" : `to ${target}`
  deps.flashFooter(`moving ${label} ${where}`, "info")
  void deps.moveAgent(agent.id, target).then(
    async moved => {
      if (moved.agent_move?.state === "waiting_for_turn") {
        deps.flashFooter(`moving ${label} ${where} after its current turn; /agent move cancel stops it`, "info")
        await watchMove(deps, agent.id, label, target)
        return
      }
      await showSettledMove(deps)
      deps.flashFooter(`moved ${label} ${where}`, "info")
    },
    (error: unknown) => { deps.flashFooter(deps.formatError(error), "error") },
  )
}

async function showSettledMove(deps: AgentMoveCommandHandlerDeps): Promise<void> {
  const session = await deps.refreshSessionState(deps.sessionState().id)
  deps.applySessionState(session)
  await deps.refreshAgentPanes(session)
  deps.rebuildTranscript()
}

/** Follows a Move the kernel holds until it moves the agent, fails or is cancelled. */
async function watchMove(deps: AgentMoveCommandHandlerDeps, agentId: string, label: string, target: string | null): Promise<void> {
  const where = target === null ? "home" : `to ${target}`
  for (;;) {
    await deps.pause(moveStatusPollMs)
    const session = await deps.refreshSessionState(deps.sessionState().id)
    const agent = session.agents?.find(candidate => candidate.id === agentId)
    const status = agent?.agent_move
    if (status?.state === "failed") {
      deps.flashFooter(`move of ${label} ${where} failed: ${status.reason}; /agent move cancel dismisses it`, "error")
      return
    }
    if (!status) {
      // The kernel clears the status when the agent moved and when the move was cancelled.
      const away = Boolean(agent?.remote_execution)
      if (away === (target !== null)) {
        await showSettledMove(deps)
        deps.flashFooter(`moved ${label} ${where}`, "info")
      } else {
        deps.flashFooter(`move of ${label} ${where} cancelled`, "info")
      }
      return
    }
  }
}
