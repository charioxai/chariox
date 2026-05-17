import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import { responsePaneBindingsMatch, selectResponsePaneAgents } from "./response-panes.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"

type FooterTone = "info" | "error"

type AgentCyclePayload = {
  agent: AgentInstance | null
  session: RuntimeSession
}

type AgentFocusPayload = {
  agent: AgentInstance
  session: RuntimeSession
}

export type AgentLifecycleCommandHandlerDeps = {
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  currentModelId: () => string
  currentVariantId: () => string
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
  deps: AgentLifecycleCommandHandlerDeps,
): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a session to cycle agents", "error")
    return
  }
  try {
    const previousSession = deps.sessionState()
    const payload = await deps.cycleAgentFocus()
    await applyFocusedAgentSession(deps, previousSession, payload.session, payload.agent)
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

export async function handleAgentDeleteCommand(
  deps: AgentLifecycleCommandHandlerDeps,
  args: string[],
): Promise<void> {
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
}

export async function handleAgentFocusCommand(
  deps: AgentLifecycleCommandHandlerDeps,
  args: string[],
): Promise<void> {
  const agentId = args[1]
  if (!agentId) {
    deps.flashFooter("usage: /agent focus <agent-id>", "error")
    return
  }
  try {
    const previousSession = deps.sessionState()
    const payload = await deps.focusAgent(agentId)
    await applyFocusedAgentSession(deps, previousSession, payload.session, payload.agent)
    deps.flashFooter(
      `focused on agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`,
      "info",
    )
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
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

async function applyFocusedAgentSession(
  deps: AgentLifecycleCommandHandlerDeps,
  previousSession: RuntimeSession,
  nextSession: RuntimeSession,
  agent: AgentInstance | null,
) {
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
  if (!nextSession.active_provider_run_id && agent) {
    const run = await deps.launchAgentProviderRun(
      agent.provider,
      agent.model ?? deps.currentModelId(),
      deps.currentVariantId(),
      agent.id,
    )
    deps.setProviderRunState(run)
    deps.applySessionState(await deps.refreshSessionState(nextSession.id))
  }
}
