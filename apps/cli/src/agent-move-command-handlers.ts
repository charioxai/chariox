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
  void reportMove(deps, deps.sessionState().id, agent.id, label, target)
}

/** Reports a Move in the Room it was requested in, whichever Room is shown by the time it settles. */
async function reportMove(deps: AgentMoveCommandHandlerDeps, roomId: string, agentId: string, label: string, target: string | null): Promise<void> {
  const where = target === null ? "home" : `to ${target}`
  let moved: AgentInstance
  try {
    moved = await deps.moveAgent(agentId, target)
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
    return
  }
  if (moved.agent_move?.state !== "waiting_for_turn") {
    await showMoved(deps, roomId, label, where)
    return
  }
  deps.flashFooter(`moving ${label} ${where} after its current turn; /agent move cancel stops it`, "info")
  try {
    await watchMove(deps, roomId, agentId, label, target)
  } catch (error) {
    deps.flashFooter(`cannot follow the move of ${label} ${where}: ${deps.formatError(error)}; the kernel may still complete it, /agent move cancel stops it`, "error")
  }
}

async function showMoved(deps: AgentMoveCommandHandlerDeps, roomId: string, label: string, where: string): Promise<void> {
  deps.flashFooter(`moved ${label} ${where}`, "info")
  // A Room the user has left refreshes when it is selected again.
  if (!isShown(deps, roomId)) return
  try {
    const session = await deps.refreshSessionState(roomId)
    // The user may have switched Room during the read; applying would replace the shown Room with this one.
    if (!isShown(deps, roomId)) return
    deps.applySessionState(session)
    await deps.refreshAgentPanes(session)
    deps.rebuildTranscript()
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

function isShown(deps: AgentMoveCommandHandlerDeps, roomId: string): boolean {
  return deps.sessionState().id === roomId
}

/** Follows a Move the kernel holds until it moves the agent, fails or is cancelled. */
async function watchMove(deps: AgentMoveCommandHandlerDeps, roomId: string, agentId: string, label: string, target: string | null): Promise<void> {
  const where = target === null ? "home" : `to ${target}`
  for (;;) {
    await deps.pause(moveStatusPollMs)
    const session = await deps.refreshSessionState(roomId)
    const agent = session.agents?.find(candidate => candidate.id === agentId)
    if (!agent) throw new Error("the agent is no longer in its Room")
    const status = agent.agent_move
    if (status?.state === "failed") {
      deps.flashFooter(`move of ${label} ${where} failed: ${status.reason}; /agent move cancel dismisses it`, "error")
      return
    }
    if (!status) {
      // The kernel clears the status when the agent moved and when the move was cancelled.
      const away = Boolean(agent.remote_execution)
      if (away === (target !== null)) {
        await showMoved(deps, roomId, label, where)
      } else {
        deps.flashFooter(`move of ${label} ${where} cancelled`, "info")
      }
      return
    }
  }
}
