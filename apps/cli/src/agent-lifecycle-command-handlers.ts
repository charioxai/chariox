import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
} from "./cli-types.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import {
  responsePaneBindingsMatch,
  selectResponsePaneAgents,
} from "@chariox/kernel-client/response-pane-selection"
import type { ResolvedAgentReference } from "@chariox/kernel-client/session-agent-resolver"
import { resolveAttachTimeProviderLaunch } from "@chariox/kernel-client/session-lifecycle-state"
import {
  formatAgentInspectSummary as formatSharedAgentInspectSummary,
  formatAgentListSummary as formatSharedAgentListSummary,
  type AgentInstance as SharedAgentInstance,
  type ShellAgentProviderRunContext as AgentProviderRunContext,
  type ShellAgentSessionContext as AgentSessionContext,
  type SliceRecord as SharedSliceRecord,
} from "@chariox/kernel-client"

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
  providerRunState: () => RuntimeProviderRun | null
  multiAgentResponseLayout: () => MultiAgentResponseLayout
  maxAgentsPerScreen: () => number
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  listSlices?: () => Promise<SliceRecord[]>
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
    accountProfile?: string,
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

export function formatAgentListSummary(
  agents: AgentInstance[],
  slices: readonly SliceRecord[] = [],
  providerRunContext: AgentProviderRunContext = {},
  sessionContext: AgentSessionContext = {},
): string {
  return formatSharedAgentListSummary(
    agents as SharedAgentInstance[],
    slices as readonly SharedSliceRecord[],
    providerRunContext,
    sessionContext,
  )
}

export function formatAgentInspectSummary(
  agent: AgentInstance,
  slices: readonly SliceRecord[] = [],
  providerRunContext: AgentProviderRunContext = {},
  sessionContext: AgentSessionContext = {},
  sliceLookupError?: string | null,
): string {
  return formatSharedAgentInspectSummary(
    agent as SharedAgentInstance,
    slices as readonly SharedSliceRecord[],
    sliceLookupError ?? null,
    providerRunContext,
    sessionContext,
  )
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
  const launchDecision = resolveAttachTimeProviderLaunch(nextSession, {
    provider: agent?.provider ?? nextSession.agent_defaults?.provider ?? "opencode",
    model: agent?.model ?? deps.currentModelId(),
    effort: agent?.effort ?? deps.currentVariantId(),
  }, false)
  if (launchDecision.action === "skip_launch") {
    deps.setProviderRunState(null)
  }
  if (launchDecision.action === "launch_provider_run" && launchDecision.targetAgentId) {
    const run = await deps.launchAgentProviderRun(
      launchDecision.launch.provider,
      launchDecision.launch.model,
      launchDecision.launch.effort,
      launchDecision.targetAgentId,
      agent?.account_profile ?? undefined,
    )
    deps.setProviderRunState(run)
    deps.applySessionState(await deps.refreshSessionState(nextSession.id))
  }
}
