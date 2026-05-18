import { PROMPT_KEYBINDINGS } from "./cli-runtime-tuning.js"
import type { PromptMetaRenderableRefKey } from "./prompt-meta-render-controller.js"
import type { PromptMetaRenderableRefs } from "./prompt-meta-renderer.js"
import { promptAttachmentTokenStyle } from "./prompt-attachment-tokens.js"
import { WorkspaceLayout, type WorkspaceLayoutProps } from "./workspace-layout.js"

type MutableRefStore = {
  assignLayoutBox: (value: any) => void
  assignRowBox: (index: number, value: any) => void
  assignBorderRow: (index: number, value: any) => void
  assignBottomBorderRow: (value: any) => void
  assignHorizontalSegment: (rowIndex: number, segmentIndex: number, value: any) => void
  assignBottomHorizontalSegment: (segmentIndex: number, value: any) => void
  assignJunctionText: (rowIndex: number, junctionIndex: number, value: any) => void
  assignBottomJunctionText: (junctionIndex: number, value: any) => void
  assignVerticalSegment: (rowIndex: number, segmentIndex: number, value: any) => void
  assignPrimaryPane: (value: any) => void
  assignPrimaryInteractionBox: (value: any) => void
  assignPrimaryFooterBox: (value: any) => void
  assignAuxiliaryPane: (index: number, value: any) => void
  assignAuxiliaryScrollbox: (index: number, value: any) => void
  assignAuxiliaryInteractionBox: (index: number, value: any) => void
  assignAuxiliaryFooterBox: (index: number, value: any) => void
}

type PromptInputController = {
  assignInput: (value: any) => void
  setSyntaxStyle: (style: unknown) => void
}

type PromptTextController = {
  snapshot: () => string
}

export type CliAppWorkspaceViewProps = {
  width: number
  height: number
  fatalError: boolean
  themeRevision: number
  responsePaneRows: WorkspaceLayoutProps["responsePaneRows"]
  promptPlaceholder: string
  promptInputMaxHeight: number
  promptAreaBackground: WorkspaceLayoutProps["promptAreaBackground"]
  retainPromptFocus: () => void
  handlePromptSelectionSurfaceMouseUp: WorkspaceLayoutProps["onResponseSurfaceMouseUp"]
  responsePaneRenderRefStore: MutableRefStore
  historyLoadingRenderController: {
    assignBox: (value: any) => void
  }
  transcriptScrollboxRefController: {
    assignScrollbox: (value: any) => void
  }
  commandCenterController: {
    assignBox: (value: any) => void
  }
  promptInputRefController: PromptInputController
  promptTextController: PromptTextController
  assignPromptMetaRef: (key: PromptMetaRenderableRefKey) =>
    (value: PromptMetaRenderableRefs[PromptMetaRenderableRefKey]) => void
  assignStatusIndicatorBox: (value: any) => void
  assignFooterSummaryBox: (value: any) => void
  assignDialogOverlayBox: (value: any) => void
  handlePromptKeyDown: WorkspaceLayoutProps["onPromptKeyDown"]
  handlePromptContentChange: () => void
  focusedAgentInteraction: () => unknown
  submitFocusedInteractionChoice: () => Promise<unknown>
  commandCenterOpen: () => boolean
  selectCommandCenterFromSubmit: () => boolean
  submitPrompt: () => Promise<unknown>
  logViewDebug: (message: string, fields?: Record<string, unknown>) => void
  applyResponseLayout: () => void
  renderHistoryLoadingIndicator: () => void
  rebuildTranscript: () => void
  ensureBackgroundPollersStarted: () => void
  renderAgentInteractions: () => void
  renderSplitPaneFooters: () => void
  renderCommandCenter: () => void
  syncPromptPlaceholder: () => void
  setPromptText: (text: string) => void
  syncPromptTextSnapshot: () => void
  refreshPromptAttachmentHighlights: () => void
  updateSessionChrome: () => void
  renderHotkeysOverlay: () => void
}

export function CliAppWorkspaceView(props: CliAppWorkspaceViewProps) {
  const assignResponseRef = (assign: () => void) => {
    assign()
    props.applyResponseLayout()
  }

  return (
    <WorkspaceLayout
      width={props.width}
      height={props.height}
      fatalError={props.fatalError}
      themeRevision={props.themeRevision}
      responsePaneRows={props.responsePaneRows}
      promptPlaceholder={props.promptPlaceholder}
      promptInputMaxHeight={props.promptInputMaxHeight}
      promptAreaBackground={props.promptAreaBackground}
      promptKeyBindings={PROMPT_KEYBINDINGS}
      onRootMouseUp={props.retainPromptFocus}
      onResponseSurfaceMouseUp={props.handlePromptSelectionSurfaceMouseUp}
      onFooterMouseUp={props.handlePromptSelectionSurfaceMouseUp}
      onResponseLayoutBoxRef={(value) => {
        props.responsePaneRenderRefStore.assignLayoutBox(value)
        props.logViewDebug("mounted response layout box")
        props.applyResponseLayout()
      }}
      onResponseRowBoxRef={(index, value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignRowBox(index, value))
      }}
      onPaneGridBorderRowRef={(index, value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignBorderRow(index, value))
      }}
      onPaneGridBottomBorderRowRef={(value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignBottomBorderRow(value))
      }}
      onPaneGridHorizontalSegmentRef={(rowIndex, segmentIndex, value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignHorizontalSegment(rowIndex, segmentIndex, value))
      }}
      onPaneGridBottomHorizontalSegmentRef={(segmentIndex, value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignBottomHorizontalSegment(segmentIndex, value))
      }}
      onPaneGridJunctionTextRef={(rowIndex, junctionIndex, value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignJunctionText(rowIndex, junctionIndex, value))
      }}
      onPaneGridBottomJunctionTextRef={(junctionIndex, value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignBottomJunctionText(junctionIndex, value))
      }}
      onPaneGridVerticalSegmentRef={(rowIndex, segmentIndex, value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignVerticalSegment(rowIndex, segmentIndex, value))
      }}
      onResponsePrimaryPaneRef={(value) => {
        props.responsePaneRenderRefStore.assignPrimaryPane(value)
        props.logViewDebug("mounted response primary pane")
        props.applyResponseLayout()
      }}
      onHistoryLoadingBoxRef={(value) => {
        props.historyLoadingRenderController.assignBox(value)
        props.logViewDebug("mounted history loading box")
        props.renderHistoryLoadingIndicator()
      }}
      onTranscriptScrollboxRef={(value) => {
        props.transcriptScrollboxRefController.assignScrollbox(value)
        props.logViewDebug("mounted primary transcript scrollbox")
        props.rebuildTranscript()
        props.ensureBackgroundPollersStarted()
      }}
      onResponsePrimaryInteractionBoxRef={(value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignPrimaryInteractionBox(value))
        props.renderAgentInteractions()
      }}
      onResponsePrimaryFooterBoxRef={(value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignPrimaryFooterBox(value))
        props.renderSplitPaneFooters()
      }}
      onResponseAuxiliaryPaneRef={(index, value) => {
        props.responsePaneRenderRefStore.assignAuxiliaryPane(index, value)
        props.logViewDebug("mounted response auxiliary pane", {
          pane_index: index + 1,
        })
        props.applyResponseLayout()
      }}
      onResponseAuxiliaryScrollboxRef={(index, value) => {
        props.responsePaneRenderRefStore.assignAuxiliaryScrollbox(index, value)
        props.logViewDebug("mounted response auxiliary scrollbox", {
          pane_index: index + 1,
        })
        props.applyResponseLayout()
      }}
      onResponseAuxiliaryInteractionBoxRef={(index, value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignAuxiliaryInteractionBox(index, value))
        props.renderAgentInteractions()
      }}
      onResponseAuxiliaryFooterBoxRef={(index, value) => {
        assignResponseRef(() => props.responsePaneRenderRefStore.assignAuxiliaryFooterBox(index, value))
        props.renderSplitPaneFooters()
      }}
      onCommandCenterBoxRef={(value) => {
        props.commandCenterController.assignBox(value)
        props.renderCommandCenter()
      }}
      onPromptInputRef={(value) => {
        props.promptInputRefController.assignInput(value)
        props.promptInputRefController.setSyntaxStyle(promptAttachmentTokenStyle)
        props.syncPromptPlaceholder()
        if (props.promptTextController.snapshot()) {
          props.setPromptText(props.promptTextController.snapshot())
        }
        props.syncPromptTextSnapshot()
        props.refreshPromptAttachmentHighlights()
        props.ensureBackgroundPollersStarted()
      }}
      onPromptKeyDown={props.handlePromptKeyDown}
      onPromptContentChange={props.handlePromptContentChange}
      onPromptSubmit={() => {
        if (props.focusedAgentInteraction()) {
          void props.submitFocusedInteractionChoice()
          return
        }
        if (props.commandCenterOpen()) {
          if (props.selectCommandCenterFromSubmit()) {
            return
          }
        }
        void props.submitPrompt()
      }}
      onPromptMetaProviderTextRef={props.assignPromptMetaRef("providerText")}
      onPromptMetaProviderDividerTextRef={props.assignPromptMetaRef("providerDividerText")}
      onPromptMetaModelTextRef={props.assignPromptMetaRef("modelText")}
      onPromptMetaModelDividerTextRef={props.assignPromptMetaRef("modelDividerText")}
      onPromptMetaVariantTextRef={props.assignPromptMetaRef("variantText")}
      onPromptMetaUsageDividerTextRef={props.assignPromptMetaRef("usageDividerText")}
      onPromptMetaUsageTokensTextRef={props.assignPromptMetaRef("usageTokensText")}
      onPromptMetaUsageBarOpenTextRef={props.assignPromptMetaRef("usageBarOpenText")}
      onPromptMetaUsageBarFilledTextRef={props.assignPromptMetaRef("usageBarFilledText")}
      onPromptMetaUsageBarEmptyTextRef={props.assignPromptMetaRef("usageBarEmptyText")}
      onPromptMetaUsageBarCloseTextRef={props.assignPromptMetaRef("usageBarCloseText")}
      onPromptMetaUsagePercentTextRef={props.assignPromptMetaRef("usagePercentText")}
      onStatusIndicatorBoxRef={(value) => {
        props.assignStatusIndicatorBox(value)
        props.updateSessionChrome()
      }}
      onFooterSummaryBoxRef={(value) => {
        props.assignFooterSummaryBox(value)
        props.updateSessionChrome()
      }}
      onHotkeysOverlayBoxRef={(value) => {
        props.assignDialogOverlayBox(value)
        props.renderHotkeysOverlay()
      }}
    />
  )
}
