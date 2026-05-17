import type { AgentInstance } from "./cli-types.js"
import type {
  SplitPaneFooterRenderOptions,
  SplitPaneFooterRenderState,
} from "./split-pane-footer-renderer.js"

export type SplitPaneFooterRenderControllerDeps = {
  renderer: SplitPaneFooterRenderOptions["renderer"]
  state: SplitPaneFooterRenderState
  primaryBox: () => SplitPaneFooterRenderOptions["primaryBox"]
  auxiliaryBoxes: () => SplitPaneFooterRenderOptions["auxiliaryBoxes"]
  isAttached: () => boolean
  workflowScreenActive: () => boolean
  maxAgentsPerScreen: () => number
  visibleAgents: () => Array<AgentInstance | null | undefined>
  focusedAgentId: () => string | null
  providerRun: () => SplitPaneFooterRenderOptions["providerRun"]
  currentProviderSelection: () => SplitPaneFooterRenderOptions["currentProviderSelection"]
  agentActivityLabels: () => SplitPaneFooterRenderOptions["agentActivityLabels"]
  hasPromptWorkByAgent: () => SplitPaneFooterRenderOptions["hasPromptWorkByAgent"]
  streamingAgentId: () => string | null
  agentBusyLatch: SplitPaneFooterRenderOptions["agentBusyLatch"]
  sessionConfigValues: () => SplitPaneFooterRenderOptions["sessionConfigValues"]
  agentLocationLabel: SplitPaneFooterRenderOptions["agentLocationLabel"]
  badgeWidth: number
  animationFrame: () => number
  renderFooters: (options: SplitPaneFooterRenderOptions) => void
}

export function createSplitPaneFooterRenderController(
  deps: SplitPaneFooterRenderControllerDeps,
) {
  return {
    render() {
      const visibleAgents = deps.visibleAgents()
      deps.renderFooters({
        renderer: deps.renderer,
        state: deps.state,
        primaryBox: deps.primaryBox(),
        auxiliaryBoxes: deps.auxiliaryBoxes(),
        showAgentFooters: deps.isAttached() && !deps.workflowScreenActive() && visibleAgents.length > 0,
        maxAgentsPerScreen: deps.maxAgentsPerScreen(),
        visibleAgents,
        focusedAgentId: deps.focusedAgentId(),
        providerRun: deps.providerRun(),
        currentProviderSelection: deps.currentProviderSelection(),
        agentActivityLabels: deps.agentActivityLabels(),
        hasPromptWorkByAgent: deps.hasPromptWorkByAgent(),
        streamingAgentId: deps.streamingAgentId(),
        agentBusyLatch: deps.agentBusyLatch,
        sessionConfigValues: deps.sessionConfigValues(),
        agentLocationLabel: deps.agentLocationLabel,
        badgeWidth: deps.badgeWidth,
        animationFrame: deps.animationFrame(),
      })
    },
  }
}
