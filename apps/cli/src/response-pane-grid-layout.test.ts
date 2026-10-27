import assert from "node:assert/strict"
import test from "node:test"

import { buildPaneGridModel } from "./response-pane-grid.js"
import {
  applyResponsePaneGridLayout,
  type ResponsePaneLayoutBox,
  type ResponsePaneLayoutScrollbox,
  type ResponsePaneLayoutText,
} from "./response-pane-grid-layout.js"

test("response pane grid layout applies split pane visibility and focused colors", () => {
  const primaryPane = box()
  const auxiliaryPane = box()
  const primaryInteraction = box(["choice"])
  const auxiliaryInteraction = box()
  const primaryFooter = box()
  const auxiliaryFooter = box()
  const primaryScrollbox = scrollbox()
  const auxiliaryScrollbox = scrollbox()
  const historyLoadingBox = box()
  const theme = layoutTheme()
  const paneGrid = buildPaneGridModel({
    paneRows: [[0, 1]],
    visibleAgents: [{ id: "agent-1" }, { id: "agent-2" }],
    focusedAgentId: "agent-2",
    split: true,
    showWorkflowScreen: false,
  })

  const applied = applyResponsePaneGridLayout({
    layoutBox: box(),
    primaryPane,
    primaryInteractionBox: primaryInteraction,
    primaryFooterBox: primaryFooter,
    primaryScrollbox,
    historyLoadingBox,
    auxiliaryPanes: [auxiliaryPane],
    auxiliaryInteractionBoxes: [auxiliaryInteraction],
    auxiliaryFooterBoxes: [auxiliaryFooter],
    auxiliaryScrollboxes: [auxiliaryScrollbox],
    rowBoxes: [box()],
    borderRows: [box()],
    horizontalSegments: [[box(), box()]],
    verticalSegments: [[box(), box(), box()]],
    junctionTexts: [[text(), text(), text()]],
    bottomBorderRow: box(),
    bottomHorizontalSegments: [box(), box()],
    bottomJunctionTexts: [text(), text(), text()],
    paneRows: [[0, 1]],
    paneGrid,
    split: true,
    showWorkflowScreen: false,
    theme,
    emptyTextAttributes: "none",
    panelBackgroundForFocus: (focused) => focused ? "focused-panel" : "default-panel",
  })

  assert.equal(applied, true)
  assert.equal(primaryPane.visible, true)
  assert.equal(primaryPane.backgroundColor, "default-panel")
  assert.equal(primaryInteraction.visible, true)
  assert.equal(primaryFooter.visible, true)
  assert.equal(primaryScrollbox.backgroundColor, "default-panel")
  assert.equal(auxiliaryPane.visible, true)
  assert.equal(auxiliaryPane.backgroundColor, "focused-panel")
  assert.equal(auxiliaryInteraction.visible, false)
  assert.equal(auxiliaryFooter.visible, true)
  assert.equal(auxiliaryScrollbox.backgroundColor, "focused-panel")
  assert.equal(historyLoadingBox.backgroundColor, "default-panel")
})

test("response pane grid layout hides the primary footer for workflow screen", () => {
  const primaryFooter = box()
  const paneGrid = buildPaneGridModel({
    paneRows: [[0]],
    visibleAgents: [{ id: "agent-1" }],
    focusedAgentId: "agent-1",
    split: true,
    showWorkflowScreen: true,
  })

  applyResponsePaneGridLayout({
    layoutBox: box(),
    primaryPane: box(),
    primaryInteractionBox: box(),
    primaryFooterBox: primaryFooter,
    primaryScrollbox: scrollbox(),
    historyLoadingBox: undefined,
    auxiliaryPanes: [],
    auxiliaryInteractionBoxes: [],
    auxiliaryFooterBoxes: [],
    auxiliaryScrollboxes: [],
    rowBoxes: [box()],
    borderRows: [box()],
    horizontalSegments: [[box(), box()]],
    verticalSegments: [[box(), box(), box()]],
    junctionTexts: [[text(), text(), text()]],
    bottomBorderRow: box(),
    bottomHorizontalSegments: [box(), box()],
    bottomJunctionTexts: [text(), text(), text()],
    paneRows: [[0]],
    paneGrid,
    split: true,
    showWorkflowScreen: true,
    theme: layoutTheme(),
    emptyTextAttributes: "none",
    panelBackgroundForFocus: (focused) => focused ? "focused-panel" : "default-panel",
  })

  assert.equal(primaryFooter.visible, false)
})

test("response pane grid layout reports missing required pane refs", () => {
  const missing: unknown[] = []
  const applied = applyResponsePaneGridLayout({
    layoutBox: undefined,
    primaryPane: undefined,
    primaryInteractionBox: undefined,
    primaryFooterBox: undefined,
    primaryScrollbox: undefined,
    historyLoadingBox: undefined,
    auxiliaryPanes: [box(), undefined],
    auxiliaryInteractionBoxes: [],
    auxiliaryFooterBoxes: [],
    auxiliaryScrollboxes: [],
    rowBoxes: [],
    borderRows: [],
    horizontalSegments: [],
    verticalSegments: [],
    junctionTexts: [],
    bottomBorderRow: undefined,
    bottomHorizontalSegments: [],
    bottomJunctionTexts: [],
    paneRows: [],
    paneGrid: { split: false, rows: [], borderRows: [] },
    split: false,
    showWorkflowScreen: false,
    theme: layoutTheme(),
    emptyTextAttributes: "none",
    panelBackgroundForFocus: () => "panel",
    onMissingRefs: (details) => {
      missing.push(details)
    },
  })

  assert.equal(applied, false)
  assert.deepEqual(missing, [{
    hasLayoutBox: false,
    hasPrimaryPane: false,
    auxiliaryPaneCount: 1,
  }])
})

function layoutTheme() {
  return {
    primary: "primary",
    borderSubtle: "border",
    backgroundPanel: "panel",
    backgroundElement: "element",
  }
}

function box(children: readonly unknown[] = []): ResponsePaneLayoutBox & { renders: number } {
  return {
    renders: 0,
    getChildren: () => children,
    requestRender() {
      this.renders += 1
    },
  }
}

function text(): ResponsePaneLayoutText {
  return {}
}

function scrollbox(): ResponsePaneLayoutScrollbox & { renders: number } {
  return {
    renders: 0,
    requestRender() {
      this.renders += 1
    },
  }
}
