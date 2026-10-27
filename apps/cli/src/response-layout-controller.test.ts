import { strict as assert } from "node:assert"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  createResponseLayoutController,
  type ResponseLayoutControllerDeps,
  type ResponseLayoutRefs,
} from "./response-layout-controller.js"

test("response layout controller stops after missing required pane refs", () => {
  const calls: string[] = []
  const controller = createResponseLayoutController(createDeps({
    calls,
    refs: refs({ layoutBox: undefined }),
  }))

  controller.apply()

  assert.deepEqual(calls, [
    "log:apply response layout:missing refs",
  ])
})

test("response layout controller applies grid, syncs panes, swaps visible transcript, and schedules repaint", () => {
  const calls: string[] = []
  const agentPaneEntry = entry("pane-entry")
  let replacedEntry: TranscriptEntry | null = null
  const controller = createResponseLayoutController(createDeps({
    calls,
    visibleAgents: [{ id: "a" }, { id: "b" }, { id: "c" }],
    selection: {
      visibleTranscriptAgentId: "b",
      screenIndex: 0,
      screenCount: 1,
    },
    mountedTranscriptAgentId: "a",
    agentPaneEntries: {
      b: [agentPaneEntry],
    },
    replaceTranscriptEntries(entries, agentId) {
      calls.push(`replace:${agentId}:${entries.length}`)
      replacedEntry = entries[0] ?? null
    },
  }))

  controller.apply()

  assert.deepEqual(calls, [
    "grid:true:2",
    "footers",
    "interactions",
    "sync:old-b->b:true",
    "sync:none->c:true",
    "replace:b:1",
    "schedule",
    "log:apply response layout",
  ])
  assert.notEqual(replacedEntry, agentPaneEntry)
  assert.deepEqual(replacedEntry, agentPaneEntry)
})

type Agent = { id: string }
type Child = { id: string; destroyRecursively?: () => void }
type Scrollbox = {
  backgroundColor?: unknown
  requestRender?: () => void
  getChildren: () => Child[]
  remove: (id: string) => unknown
  add: (child: Child) => unknown
}

function createDeps(overrides: {
  calls?: string[]
  refs?: ResponseLayoutRefs<Child, Scrollbox>
  visibleAgents?: Agent[]
  selection?: { visibleTranscriptAgentId: string | null; screenIndex: number; screenCount: number }
  mountedTranscriptAgentId?: string | null
  agentPaneEntries?: Record<string, TranscriptEntry[]>
  replaceTranscriptEntries?: (entries: TranscriptEntry[], agentId: string) => void
} = {}): ResponseLayoutControllerDeps<Agent, Child, Scrollbox> {
  const calls = overrides.calls ?? []
  const currentAuxiliaryAgentIds = ["old-b", null]
  return {
    getRefs: () => overrides.refs ?? refs(),
    getSplit: () => true,
    getVisibleAgents: () => overrides.visibleAgents ?? [{ id: "a" }, { id: "b" }],
    getPaneRows: () => [[0, 1], [2]],
    getFocusedAgentId: () => "b",
    getShowWorkflowScreen: () => false,
    getMaxAgentsPerScreen: () => 3,
    getResponsePaneSelection: () => overrides.selection ?? {
      visibleTranscriptAgentId: "a",
      screenIndex: 0,
      screenCount: 1,
    },
    getTheme: () => ({
      primary: "primary",
      borderSubtle: "subtle",
      backgroundPanel: "panel",
      backgroundElement: "element",
    }),
    emptyTextAttributes: "none",
    panelBackgroundForFocus: (focused) => focused ? "focused-panel" : "panel",
    renderSplitPaneFooters: () => {
      calls.push("footers")
    },
    renderAgentInteractions: () => {
      calls.push("interactions")
    },
    clearAuxiliaryAgentPane: (agentId) => {
      calls.push(`clear:${agentId}`)
    },
    unregisterAgentScrollbox: (agentId) => {
      calls.push(`unregister:${agentId}`)
    },
    getCurrentAuxiliaryAgentId: (index) => currentAuxiliaryAgentIds[index] ?? null,
    setCurrentAuxiliaryAgentId: (index, agentId) => {
      currentAuxiliaryAgentIds[index] = agentId
    },
    registerAgentScrollbox: (agentId) => {
      calls.push(`register:${agentId}`)
    },
    rebuildAuxiliaryAgentPane: (agentId) => {
      calls.push(`rebuild:${agentId}`)
    },
    buildEmptyTranscriptRenderable: () => ({ id: "empty" }),
    getMountedTranscriptAgentId: () => overrides.mountedTranscriptAgentId ?? null,
    getAgentPaneEntries: (agentId) => overrides.agentPaneEntries?.[agentId] ?? [],
    replaceTranscriptEntries: overrides.replaceTranscriptEntries ?? ((entries, agentId) => {
      calls.push(`replace:${agentId}:${entries.length}`)
    }),
    scheduleResponsePaneRepaint: () => {
      calls.push("schedule")
    },
    logViewDebug: (phase) => {
      calls.push(`log:${phase}`)
    },
    applyPaneGridLayout: (options) => {
      if (!options.layoutBox || !options.primaryPane) {
        options.onMissingRefs?.({
          hasLayoutBox: Boolean(options.layoutBox),
          hasPrimaryPane: Boolean(options.primaryPane),
          auxiliaryPaneCount: options.auxiliaryPanes.filter(Boolean).length,
        })
        return false
      }
      calls.push(`grid:${options.split}:${options.paneGrid.rows.length}`)
      return true
    },
    syncAuxiliaryPane: (options) => {
      calls.push(`sync:${options.currentAgentId ?? "none"}->${options.nextAgentId ?? "none"}:${options.splitMode}`)
      options.assignCurrentAgentId(options.nextAgentId)
    },
  }
}

function refs(overrides: Partial<ResponseLayoutRefs<Child, Scrollbox>> = {}): ResponseLayoutRefs<Child, Scrollbox> {
  return {
    layoutBox: box(),
    primaryPane: box(),
    primaryInteractionBox: box(),
    primaryFooterBox: box(),
    primaryScrollbox: scrollbox(),
    historyLoadingBox: box(),
    auxiliaryPanes: [box(), box()],
    auxiliaryInteractionBoxes: [box(), box()],
    auxiliaryFooterBoxes: [box(), box()],
    auxiliaryScrollboxes: [scrollbox(), scrollbox()],
    rowBoxes: [box(), box()],
    borderRows: [box(), box(), box()],
    horizontalSegments: [[box(), box()], [box(), box()], [box(), box()]],
    verticalSegments: [[box(), box(), box()], [box(), box(), box()]],
    junctionTexts: [[text(), text(), text()], [text(), text(), text()], [text(), text(), text()]],
    bottomBorderRow: box(),
    bottomHorizontalSegments: [box(), box()],
    bottomJunctionTexts: [text(), text(), text()],
    ...overrides,
  }
}

function box() {
  return {
    getChildren: () => [],
  }
}

function scrollbox(): Scrollbox {
  return {
    getChildren: () => [],
    remove: () => undefined,
    add: () => undefined,
  }
}

function text() {
  return {}
}

function entry(id: string): TranscriptEntry {
  return {
    id,
    role: "assistant",
    text: "hello",
  } as unknown as TranscriptEntry
}
