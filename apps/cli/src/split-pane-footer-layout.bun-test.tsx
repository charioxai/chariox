import assert from "node:assert/strict"
import test from "node:test"
import { testRender, useRenderer } from "@opentui/solid"
import { WorkspaceLayout, type WorkspaceLayoutProps } from "./workspace-layout.js"
import { createSplitPaneFooterRenderState, renderSplitPaneFooters, type SplitPaneFooterRenderOptions } from "./split-pane-footer-renderer.js"
import { theme } from "./theme.js"

// MP-08 / MP-09 / MP-10 / MP-11: mount the actual workspace, including Solid's footer properties.
test("EA2-80 workspace keeps Continue below a bounded provider diagnostic", async () => {
  const agent = { id: "agent-1", state: "Idle", provider: "codex" }
  const options = {
    state: createSplitPaneFooterRenderState(), auxiliaryBoxes: [], showAgentFooters: true, maxAgentsPerScreen: 1,
    session: { id: "room", agents: [agent], agent_activity: { "agent-1": {
      busy: false, prompt_status: "none", last_completed_turn: { turn_id: "turn-1",
        outcome: { kind: "other", reason: "Provider diagnostic\n".repeat(200), retryable: true } },
    } } }, visibleAgents: [agent], metaagentTasks: [], focusedAgentId: "agent-1", providerRun: null,
    currentProviderSelection: { provider: "codex", model: "gpt-6-luna", effort: "low" },
    agentActivityLabels: {}, streamingAgentId: null, agentBusyLatch: () => false,
    sessionConfigValues: {}, agentLocationLabel: () => null, providerAccountLabel: () => "",
    badgeWidth: 15, animationFrame: 0,
  } as unknown as SplitPaneFooterRenderOptions
  const noop = () => {}
  const props: WorkspaceLayoutProps = {
    width: 40, height: 16, fatalError: false, themeRevision: 0, responsePaneRows: () => [[0]],
    promptPlaceholder: "Agent prompt", promptInputMaxHeight: 1, promptKeyBindings: [], promptAreaBackground: theme.background,
    onPaneGridBottomBorderRowRef: noop, onPaneGridBottomHorizontalSegmentRef: noop, onPaneGridBottomJunctionTextRef: noop,
    onRootMouseUp: noop, onResponseSurfaceMouseUp: noop, onFooterMouseUp: noop, onSudoBandRef: noop,
    onResponseLayoutBoxRef: noop, onResponseRowBoxRef: noop, onPaneGridBorderRowRef: noop,
    onPaneGridHorizontalSegmentRef: noop, onPaneGridJunctionTextRef: noop, onPaneGridVerticalSegmentRef: noop,
    onResponsePrimaryPaneRef: noop, onHistoryLoadingBoxRef: noop, onTranscriptScrollboxRef: noop,
    onResponsePrimaryInteractionBoxRef: noop, onResponsePrimaryFooterBoxRef: (box) => { options.primaryBox = box; renderSplitPaneFooters(options) },
    onResponseAuxiliaryPaneRef: noop, onResponseAuxiliaryScrollboxRef: noop,
    onResponseAuxiliaryInteractionBoxRef: noop, onResponseAuxiliaryFooterBoxRef: noop,
    onCommandCenterBoxRef: noop, onPromptInputRef: noop, onPromptKeyDown: noop, onPromptContentChange: noop, onPromptSubmit: noop,
    onPromptMetaProviderTextRef: noop, onPromptMetaProviderDividerTextRef: noop, onPromptMetaModelTextRef: noop,
    onPromptMetaModelDividerTextRef: noop, onPromptMetaVariantTextRef: noop, onPromptMetaUsageDividerTextRef: noop,
    onPromptMetaUsageTokensTextRef: noop, onPromptMetaUsageBarOpenTextRef: noop, onPromptMetaUsageBarFilledTextRef: noop,
    onPromptMetaUsageBarEmptyTextRef: noop, onPromptMetaUsageBarCloseTextRef: noop, onPromptMetaUsagePercentTextRef: noop,
    onStatusIndicatorBoxRef: noop, onFooterSummaryBoxRef: noop, onHotkeysOverlayBoxRef: noop,
    onKernelApprovalBannerRef: noop, onKernelApprovalBoxRef: noop, onPasskeyPopupBoxRef: noop,
  }
  const ui = await testRender(() => {
    options.renderer = useRenderer()
    return <WorkspaceLayout {...props} />
  }, { width: 40, height: 16, useThread: false, exitOnCtrlC: false })
  try {
    renderSplitPaneFooters(options)
    await ui.renderOnce()
    const frame = ui.captureCharFrame()
    assert.ok(frame.includes("Continue: /continue"), frame)
    assert.equal(options.state.primary.outcomeText!.height, 1)
  } finally { ui.renderer.destroy() }
})
