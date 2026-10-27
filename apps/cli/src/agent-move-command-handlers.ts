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
  moveAgent: (agentId: string, destination: string | null) => Promise<void>
  cancelAgentMove: (agentId: string) => Promise<void>
}

const usage = "usage: /agent move <kernel|home> [agent]  or  /agent move cancel [agent]"

/**
 * Moves an agent's execution to another kernel, home to return, or cancels a move that waits for the turn.
 * The kernel answers a move only when it is done, so the command returns at once and reports the outcome.
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
  deps.flashFooter(
    `moving ${label} ${target === null ? "home" : `to ${target}`} after its current turn; /agent move cancel stops it`,
    "info",
  )
  void deps.moveAgent(agent.id, target).then(
    async () => {
      const session = await deps.refreshSessionState(deps.sessionState().id)
      deps.applySessionState(session)
      await deps.refreshAgentPanes(session)
      deps.rebuildTranscript()
      deps.flashFooter(`moved ${label} ${target === null ? "home" : `to ${target}`}`, "info")
    },
    (error: unknown) => {
      // Cancelling answers the waiting request with an error; that is not a failure.
      const message = deps.formatError(error)
      deps.flashFooter(/cancelled/i.test(message) ? `move of ${label} cancelled` : message, /cancelled/i.test(message) ? "info" : "error")
    },
  )
}
