import assert from "node:assert/strict"
import test from "node:test"

import { requestRenderableTreeRender, syncAuxiliaryPane } from "./response-layout-render.js"

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

test("syncAuxiliaryPane keeps an unchanged agent pane mounted without rebuilding it", () => {
  const scrollbox = {
    children: [] as Array<{ id: string; destroyRecursively?: () => void }>,
    getChildren() {
      return this.children
    },
    remove() {
      throw new Error("should not remove")
    },
    add() {
      throw new Error("should not add")
    },
    requestRender() {},
  }
  const registered: string[] = []
  let rebuildCount = 0
  let assigned: string | null = null

  syncAuxiliaryPane({
    scrollbox,
    nextAgentId: "agent-a",
    currentAgentId: "agent-a",
    splitMode: true,
    clearAuxiliaryAgentPane: () => {
      throw new Error("should not clear")
    },
    unregisterAgentScrollbox: () => {
      throw new Error("should not unregister")
    },
    assignCurrentAgentId: (value) => {
      assigned = value
    },
    registerAgentScrollbox: (agentId) => {
      registered.push(agentId)
    },
    rebuildAuxiliaryAgentPane: () => {
      rebuildCount += 1
    },
    buildEmptyTranscriptRenderable: () => ({ id: "empty" }),
  })

  assert.equal(assigned, "agent-a")
  assert.deepEqual(registered, ["agent-a"])
  assert.equal(rebuildCount, 0)
})
