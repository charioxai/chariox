import assert from "node:assert/strict"
import test from "node:test"

import { applyResponseLayoutRenderables, requestRenderableTreeRender, syncAuxiliaryPane } from "./response-layout-render.js"

type FakeRenderable = {
  id?: string | undefined
  renderCount: number
  rebuildCount: number
  children: FakeRenderable[]
  requestRender: () => void
  requestRebuild: () => void
  getChildren: () => FakeRenderable[]
  visible?: boolean
  flexDirection?: "row" | "column" | undefined
  gap?: number | undefined
  flexGrow?: number | undefined
  width?: number | "auto" | undefined
  flexBasis?: number | "auto" | undefined
  minHeight?: number | null | undefined
  minWidth?: number | null | undefined
  maxWidth?: number | null | undefined
  maxHeight?: number | null | undefined
  backgroundColor?: unknown
  borderColor?: unknown
  border?: false | string[] | undefined
  paddingLeft?: number | undefined
  paddingRight?: number | undefined
  paddingTop?: number | undefined
  paddingBottom?: number | undefined
}

function renderable(id?: string): FakeRenderable {
  return {
    renderCount: 0,
    rebuildCount: 0,
    ...(id ? { id } : {}),
    children: [] as FakeRenderable[],
    requestRender() {
      this.renderCount += 1
    },
    requestRebuild() {
      this.rebuildCount += 1
    },
    getChildren() {
      return this.children
    },
  }
}

test("requestRenderableTreeRender rebuilds a tree without repeating seen nodes", () => {
  const root = renderable("root")
  const child = renderable("child")
  root.children.push(child, child)

  requestRenderableTreeRender(root)

  assert.equal(root.renderCount, 1)
  assert.equal(root.rebuildCount, 1)
  assert.equal(child.renderCount, 1)
  assert.equal(child.rebuildCount, 1)
})

test("syncAuxiliaryPane clears stale panes and mounts the empty split placeholder", () => {
  const removed: string[] = []
  const destroyed: string[] = []
  const scrollbox = {
    children: [{
      id: "old",
      destroyRecursively() {
        destroyed.push("old")
      },
    }] as Array<{ id: string; destroyRecursively?: () => void }>,
    getChildren() {
      return this.children
    },
    remove(id: string) {
      removed.push(id)
      this.children = this.children.filter((child) => child.id !== id)
    },
    add(child: { id: string; destroyRecursively?: () => void }) {
      this.children.push(child)
    },
    requestRenderCalled: 0,
    requestRender() {
      this.requestRenderCalled += 1
    },
  }
  const cleared: string[] = []
  const unregistered: string[] = []
  let assigned: string | null = "agent-old"

  syncAuxiliaryPane({
    scrollbox,
    nextAgentId: null,
    currentAgentId: "agent-old",
    splitMode: true,
    clearAuxiliaryAgentPane: (agentId) => {
      cleared.push(agentId)
    },
    unregisterAgentScrollbox: (agentId) => {
      unregistered.push(agentId)
    },
    assignCurrentAgentId: (value) => {
      assigned = value
    },
    registerAgentScrollbox: () => {
      throw new Error("should not register")
    },
    rebuildAuxiliaryAgentPane: () => {
      throw new Error("should not rebuild")
    },
    buildEmptyTranscriptRenderable: () => ({ id: "empty" }),
  })

  assert.deepEqual(cleared, ["agent-old"])
  assert.deepEqual(unregistered, ["agent-old"])
  assert.equal(assigned, null)
  assert.deepEqual(removed, ["old"])
  assert.deepEqual(destroyed, ["old"])
  assert.deepEqual(scrollbox.children.map((child) => child.id), ["empty"])
})

test("applyResponseLayoutRenderables mutates pane geometry and visibility", () => {
  const renderables = {
    responseLayoutBox: renderable("layout"),
    responseTopRowBox: renderable("top"),
    responsePrimaryPane: renderable("primary"),
    responseSecondaryPane: renderable("secondary"),
    responseTertiaryPane: renderable("tertiary"),
    historyLoadingBox: renderable("history"),
    transcriptScrollbox: renderable("transcript"),
    responseSecondaryScrollbox: {
      ...renderable("secondary-scroll"),
      getChildren() {
        return []
      },
      remove() {},
      add() {},
    },
    responseTertiaryScrollbox: {
      ...renderable("tertiary-scroll"),
      getChildren() {
        return []
      },
      remove() {},
      add() {},
    },
    responsePrimaryFooterBox: renderable("primary-footer"),
    responseSecondaryFooterBox: renderable("secondary-footer"),
    responseTertiaryFooterBox: renderable("tertiary-footer"),
  }

  const summary = applyResponseLayoutRenderables({
    renderables,
    geometry: {
      showSecondaryPane: true,
      showTertiaryPane: false,
      splitPaneWidth: 56,
      layoutDirection: "row",
      layoutGap: 1,
      topRowVisible: true,
      topRowGap: 1,
      topRowFlexBasis: "auto",
      topRowMinHeight: null,
      primaryFlexGrow: 0,
      primaryWidth: 56,
      primaryFlexBasis: 56,
      primaryMinWidth: 56,
      primaryMaxWidth: 56,
      secondaryWidth: 56,
      secondaryFlexBasis: 56,
      secondaryMinWidth: 56,
      secondaryMaxWidth: 56,
      tertiaryWidth: 0,
      tertiaryFlexGrow: 0,
      tertiaryFlexBasis: 0,
      tertiaryMinHeight: 0,
    },
    split: true,
    primaryFocused: true,
    secondaryFocused: false,
    tertiaryFocused: false,
    primaryBackground: "primary-bg",
    secondaryBackground: "secondary-bg",
    tertiaryBackground: "tertiary-bg",
    primaryBorderColor: "focus",
    secondaryBorderColor: "focus",
    tertiaryBorderColor: "focus",
    subtleBorderColor: "subtle",
  })

  assert.equal(renderables.responseLayoutBox.flexDirection, "row")
  assert.equal(renderables.responsePrimaryPane.width, 56)
  assert.equal(renderables.responsePrimaryPane.borderColor, "focus")
  assert.equal(renderables.responseSecondaryPane.visible, true)
  assert.equal(renderables.responseSecondaryPane.backgroundColor, "secondary-bg")
  assert.equal(renderables.responseTertiaryPane.visible, false)
  assert.equal(renderables.responsePrimaryFooterBox.visible, true)
  assert.equal(summary.splitPaneWidth, 56)
  assert.equal(summary.secondaryVisible, true)
  assert.equal(summary.tertiaryVisible, false)
})

test("applyResponseLayoutRenderables clears auto-sized pane widths", () => {
  const renderables = {
    responseLayoutBox: renderable("layout"),
    responseTopRowBox: renderable("top"),
    responsePrimaryPane: { ...renderable("primary"), width: 0, flexBasis: 0 },
    responseSecondaryPane: renderable("secondary"),
    responseTertiaryPane: { ...renderable("tertiary"), width: 0 },
    historyLoadingBox: renderable("history"),
    transcriptScrollbox: renderable("transcript"),
    responseSecondaryScrollbox: {
      ...renderable("secondary-scroll"),
      getChildren() {
        return []
      },
      remove() {},
      add() {},
    },
    responseTertiaryScrollbox: {
      ...renderable("tertiary-scroll"),
      getChildren() {
        return []
      },
      remove() {},
      add() {},
    },
    responsePrimaryFooterBox: renderable("primary-footer"),
    responseSecondaryFooterBox: renderable("secondary-footer"),
    responseTertiaryFooterBox: renderable("tertiary-footer"),
  }

  applyResponseLayoutRenderables({
    renderables,
    geometry: {
      showSecondaryPane: false,
      showTertiaryPane: false,
      splitPaneWidth: 56,
      layoutDirection: "row",
      layoutGap: 0,
      topRowVisible: true,
      topRowGap: 0,
      topRowFlexBasis: "auto",
      topRowMinHeight: null,
      primaryFlexGrow: 1,
      primaryWidth: "auto",
      primaryFlexBasis: "auto",
      primaryMinWidth: null,
      primaryMaxWidth: null,
      secondaryWidth: 0,
      secondaryFlexBasis: 0,
      secondaryMinWidth: 0,
      secondaryMaxWidth: 0,
      tertiaryWidth: "auto",
      tertiaryFlexGrow: 0,
      tertiaryFlexBasis: 0,
      tertiaryMinHeight: 0,
    },
    split: false,
    primaryFocused: false,
    secondaryFocused: false,
    tertiaryFocused: false,
    primaryBackground: "primary-bg",
    secondaryBackground: "secondary-bg",
    tertiaryBackground: "tertiary-bg",
    primaryBorderColor: "focus",
    secondaryBorderColor: "focus",
    tertiaryBorderColor: "focus",
    subtleBorderColor: "subtle",
  })

  assert.equal(renderables.responseTopRowBox.width, undefined)
  assert.equal(renderables.responseTopRowBox.flexBasis, undefined)
  assert.equal(renderables.responsePrimaryPane.width, undefined)
  assert.equal(renderables.responsePrimaryPane.flexBasis, undefined)
  assert.equal(renderables.responseTertiaryPane.width, undefined)
})
