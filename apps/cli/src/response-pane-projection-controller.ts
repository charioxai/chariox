import type { RuntimeSession } from "./cli-types.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import {
  responsePaneRowSlots,
  selectResponsePaneAgents,
} from "./response-panes.js"
import { resolveTranscriptSurfaceTone } from "./transcript-render-theme-policy.js"
import {
  resolveWorkspaceVisibleAgents,
  resolveWorkspaceVisibleTranscriptAgentId,
  type WorkspaceScreenMode,
} from "./workspace-screen.js"

export type ResponsePaneProjectionControllerDeps = {
  isAttached: () => boolean
  getSession: () => RuntimeSession
  getFocusedAgentId: () => string | null | undefined
  getWorkspaceScreenMode: () => WorkspaceScreenMode
  getResponseLayout: () => MultiAgentResponseLayout
  getMaxAgentsPerScreen: () => number
  workflowScreenActive: () => boolean
}

export function createResponsePaneProjectionController(
  deps: ResponsePaneProjectionControllerDeps,
) {
  const multiAgentMode = () => deps.isAttached() && deps.getSession().agents.length > 1
  const workflowScreenShowing = () => deps.isAttached() && deps.getWorkspaceScreenMode() === "workflow"
  const splitAgentResponseMode = () =>
    multiAgentMode() && deps.getResponseLayout() === "split"
  const responsePaneSelection = () => selectResponsePaneAgents(
    deps.getSession().agents,
    deps.getFocusedAgentId(),
    splitAgentResponseMode(),
    deps.getMaxAgentsPerScreen(),
  )
  const responsePrimaryAgent = () => deps.workflowScreenActive() ? null : responsePaneSelection().primary
  const responseVisibleAgents = () =>
    resolveWorkspaceVisibleAgents(deps.getWorkspaceScreenMode(), responsePaneSelection().visibleAgents)
  const visibleTranscriptAgentId = () => resolveWorkspaceVisibleTranscriptAgentId(
    deps.getWorkspaceScreenMode(),
    responsePaneSelection().visibleTranscriptAgentId,
  )

  return {
    multiAgentMode,
    workflowScreenShowing,
    splitAgentResponseMode,
    responsePaneSelection,
    responsePaneAgentSignature: () => deps.getSession().agents.map((agent) => agent.id).join(","),
    responsePrimaryAgent,
    responseVisibleAgents,
    visibleTranscriptAgentId,
    responsePaneRows: () => responsePaneRowSlots(deps.getMaxAgentsPerScreen()),
    primaryTranscriptSurfaceTone: () =>
      resolveTranscriptSurfaceTone(splitAgentResponseMode(), responsePrimaryAgent()?.id === deps.getFocusedAgentId()),
    auxiliaryTranscriptSurfaceTone: (agentId: string | null | undefined) =>
      resolveTranscriptSurfaceTone(splitAgentResponseMode(), Boolean(agentId) && agentId === deps.getFocusedAgentId()),
  }
}
