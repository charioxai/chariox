import type { TranscriptEntry } from "./cli-types.js"
import { buildPaneGridModel } from "./response-pane-grid.js"
import {
  applyResponsePaneGridLayout,
  type ResponsePaneGridLayoutTheme,
  type ResponsePaneLayoutBox,
  type ResponsePaneLayoutScrollbox,
  type ResponsePaneLayoutText,
} from "./response-pane-grid-layout.js"
import type { ResponsePaneSelection } from "@chariox/kernel-client/response-pane-selection"
import {
  syncAuxiliaryPane,
} from "./response-layout-render.js"

type AuxiliaryPaneChild = {
  id: string | number
  destroyRecursively?: () => void
}

type AuxiliaryPaneScrollbox<TChild = AuxiliaryPaneChild> = {
  backgroundColor?: unknown
  requestRender?: () => void
  getChildren: () => AuxiliaryPaneChild[]
  remove: (id: string) => unknown
  add: (child: TChild) => unknown
}

type SyncAuxiliaryPaneFn<TChild extends AuxiliaryPaneChild, TScrollbox extends AuxiliaryPaneScrollbox<TChild>> = (options: {
  scrollbox: TScrollbox | undefined
  nextAgentId: string | null
  currentAgentId: string | null
  splitMode: boolean
  clearAuxiliaryAgentPane: (agentId: string) => void
  unregisterAgentScrollbox: (agentId: string) => void
  assignCurrentAgentId: (value: string | null) => void
  registerAgentScrollbox: (agentId: string, scrollbox: TScrollbox) => void
  rebuildAuxiliaryAgentPane: (agentId: string) => void
  buildEmptyTranscriptRenderable: () => TChild
}) => void

export type ResponseLayoutRefs<TChild extends AuxiliaryPaneChild, TScrollbox extends AuxiliaryPaneScrollbox<TChild>> = {
  layoutBox: ResponsePaneLayoutBox | undefined
  primaryPane: ResponsePaneLayoutBox | undefined
  primaryInteractionBox: ResponsePaneLayoutBox | undefined
  primaryFooterBox: ResponsePaneLayoutBox | undefined
  primaryScrollbox: ResponsePaneLayoutScrollbox | undefined
  historyLoadingBox: ResponsePaneLayoutBox | undefined
  auxiliaryPanes: readonly (ResponsePaneLayoutBox | undefined)[]
  auxiliaryInteractionBoxes: readonly (ResponsePaneLayoutBox | undefined)[]
  auxiliaryFooterBoxes: readonly (ResponsePaneLayoutBox | undefined)[]
  auxiliaryScrollboxes: readonly (TScrollbox | undefined)[]
  rowBoxes: readonly (ResponsePaneLayoutBox | undefined)[]
  borderRows: readonly (ResponsePaneLayoutBox | undefined)[]
  horizontalSegments: readonly (readonly (ResponsePaneLayoutBox | undefined)[] | undefined)[]
  verticalSegments: readonly (readonly (ResponsePaneLayoutBox | undefined)[] | undefined)[]
  junctionTexts: readonly (readonly (ResponsePaneLayoutText | undefined)[] | undefined)[]
  bottomBorderRow: ResponsePaneLayoutBox | undefined
  bottomHorizontalSegments: readonly (ResponsePaneLayoutBox | undefined)[]
  bottomJunctionTexts: readonly (ResponsePaneLayoutText | undefined)[]
}

export type ResponseLayoutControllerDeps<
  TAgent extends { id: string },
  TChild extends AuxiliaryPaneChild,
  TScrollbox extends AuxiliaryPaneScrollbox<TChild>,
> = {
  getRefs: () => ResponseLayoutRefs<TChild, TScrollbox>
  getSplit: () => boolean
  getVisibleAgents: () => readonly TAgent[]
  getPaneRows: () => readonly (readonly number[])[]
  getFocusedAgentId: () => string | null
  getShowWorkflowScreen: () => boolean
  getMaxAgentsPerScreen: () => number
  getResponsePaneSelection: () => Pick<ResponsePaneSelection<TAgent>, "visibleTranscriptAgentId" | "screenIndex" | "screenCount">
  getTheme: () => ResponsePaneGridLayoutTheme
  emptyTextAttributes: unknown
  panelBackgroundForFocus: (focused: boolean) => unknown
  renderSplitPaneFooters: () => void
  renderAgentInteractions: () => void
  clearAuxiliaryAgentPane: (agentId: string) => void
  unregisterAgentScrollbox: (agentId: string) => void
  getCurrentAuxiliaryAgentId: (auxiliaryIndex: number) => string | null
  setCurrentAuxiliaryAgentId: (auxiliaryIndex: number, agentId: string | null) => void
  registerAgentScrollbox: (agentId: string, scrollbox: TScrollbox) => void
  rebuildAuxiliaryAgentPane: (agentId: string) => void
  buildEmptyTranscriptRenderable: () => TChild
  getMountedTranscriptAgentId: () => string | null
  getAgentPaneEntries: (agentId: string) => readonly TranscriptEntry[]
  replaceTranscriptEntries: (entries: TranscriptEntry[], agentId: string) => void
  scheduleResponsePaneRepaint: () => void
  logViewDebug: (phase: string, fields: Record<string, unknown>) => void
  applyPaneGridLayout?: typeof applyResponsePaneGridLayout
  syncAuxiliaryPane?: SyncAuxiliaryPaneFn<TChild, TScrollbox>
}

export function createResponseLayoutController<
  TAgent extends { id: string },
  TChild extends AuxiliaryPaneChild,
  TScrollbox extends AuxiliaryPaneScrollbox<TChild>,
>(
  deps: ResponseLayoutControllerDeps<TAgent, TChild, TScrollbox>,
) {
  const applyPaneGridLayout = deps.applyPaneGridLayout ?? applyResponsePaneGridLayout
  const syncPane: SyncAuxiliaryPaneFn<TChild, TScrollbox> = deps.syncAuxiliaryPane ?? syncAuxiliaryPane

  const apply = () => {
    const refs = deps.getRefs()
    const split = deps.getSplit()
    const visibleAgents = deps.getVisibleAgents()
    const paneRows = deps.getPaneRows()
    const showWorkflowScreen = deps.getShowWorkflowScreen()
    const paneGrid = buildPaneGridModel({
      paneRows,
      visibleAgents,
      focusedAgentId: deps.getFocusedAgentId(),
      split,
      showWorkflowScreen,
    })

    const appliedPaneLayout = applyPaneGridLayout({
      layoutBox: refs.layoutBox,
      primaryPane: refs.primaryPane,
      primaryInteractionBox: refs.primaryInteractionBox,
      primaryFooterBox: refs.primaryFooterBox,
      primaryScrollbox: refs.primaryScrollbox,
      historyLoadingBox: refs.historyLoadingBox,
      auxiliaryPanes: refs.auxiliaryPanes,
      auxiliaryInteractionBoxes: refs.auxiliaryInteractionBoxes,
      auxiliaryFooterBoxes: refs.auxiliaryFooterBoxes,
      auxiliaryScrollboxes: refs.auxiliaryScrollboxes,
      rowBoxes: refs.rowBoxes,
      borderRows: refs.borderRows,
      horizontalSegments: refs.horizontalSegments,
      verticalSegments: refs.verticalSegments,
      junctionTexts: refs.junctionTexts,
      bottomBorderRow: refs.bottomBorderRow,
      bottomHorizontalSegments: refs.bottomHorizontalSegments,
      bottomJunctionTexts: refs.bottomJunctionTexts,
      paneRows,
      paneGrid,
      split,
      showWorkflowScreen,
      theme: deps.getTheme(),
      emptyTextAttributes: deps.emptyTextAttributes,
      panelBackgroundForFocus: deps.panelBackgroundForFocus,
      onMissingRefs: (details) => {
        deps.logViewDebug("apply response layout:missing refs", {
          has_layout_box: details.hasLayoutBox,
          has_primary_pane: details.hasPrimaryPane,
          auxiliary_pane_count: details.auxiliaryPaneCount,
        })
      },
    })
    if (!appliedPaneLayout) {
      return
    }

    deps.renderSplitPaneFooters()
    deps.renderAgentInteractions()
    for (let auxiliaryIndex = 0; auxiliaryIndex < deps.getMaxAgentsPerScreen() - 1; auxiliaryIndex += 1) {
      syncPane({
        scrollbox: refs.auxiliaryScrollboxes[auxiliaryIndex],
        nextAgentId: split ? (visibleAgents[auxiliaryIndex + 1]?.id ?? null) : null,
        currentAgentId: deps.getCurrentAuxiliaryAgentId(auxiliaryIndex),
        splitMode: split,
        clearAuxiliaryAgentPane: deps.clearAuxiliaryAgentPane,
        unregisterAgentScrollbox: deps.unregisterAgentScrollbox,
        assignCurrentAgentId: (value) => {
          deps.setCurrentAuxiliaryAgentId(auxiliaryIndex, value)
        },
        registerAgentScrollbox: deps.registerAgentScrollbox,
        rebuildAuxiliaryAgentPane: deps.rebuildAuxiliaryAgentPane,
        buildEmptyTranscriptRenderable: deps.buildEmptyTranscriptRenderable,
      })
    }

    const selection = deps.getResponsePaneSelection()
    const nextVisibleTranscriptAgentId = selection.visibleTranscriptAgentId
    if (
      nextVisibleTranscriptAgentId
      && nextVisibleTranscriptAgentId !== deps.getMountedTranscriptAgentId()
    ) {
      deps.replaceTranscriptEntries(
        deps.getAgentPaneEntries(nextVisibleTranscriptAgentId).map((entry) => ({ ...entry })),
        nextVisibleTranscriptAgentId,
      )
    }

    deps.scheduleResponsePaneRepaint()
    deps.logViewDebug("apply response layout", {
      split,
      visible_agent_ids: visibleAgents.map((agent) => agent.id),
      screen_index: selection.screenIndex,
      screen_count: selection.screenCount,
    })
  }

  return {
    apply,
  }
}
