import assert from "node:assert/strict"
import test from "node:test"

import { createRenderScheduler } from "./render-scheduler.js"

type FakeRenderable = {
  id: string
  renders: number
  rebuilds: number
  children: FakeRenderable[]
  requestRender: () => void
  requestRebuild: () => void
  getChildren: () => FakeRenderable[]
}

function renderable(id: string): FakeRenderable {
  return {
    id,
    renders: 0,
    rebuilds: 0,
    children: [],
    requestRender() {
      this.renders += 1
    },
    requestRebuild() {
      this.rebuilds += 1
    },
    getChildren() {
      return this.children
    },
  }
}

test("render scheduler coalesces multiple pane render requests", () => {
  const scheduled: Array<() => void> = []
  const pane = renderable("pane")
  let rootRenders = 0
  const scheduler = createRenderScheduler({
    schedule: (callback) => {
      scheduled.push(callback)
      return 1 as unknown as ReturnType<typeof setTimeout>
    },
    clearSchedule: () => {},
    requestRootRender: () => {
      rootRenders += 1
    },
  })

  scheduler.requestRenderable(pane)
  scheduler.requestRenderable(pane)

  assert.equal(scheduled.length, 1)
  scheduled[0]!()
  assert.equal(pane.renders, 1)
  assert.equal(rootRenders, 0)
})

test("render scheduler renders tree once and skips duplicate child render", () => {
  const scheduled: Array<() => void> = []
  const parent = renderable("parent")
  const child = renderable("child")
  parent.children.push(child)
  const scheduler = createRenderScheduler({
    schedule: (callback) => {
      scheduled.push(callback)
      return 1 as unknown as ReturnType<typeof setTimeout>
    },
    clearSchedule: () => {},
  })

  scheduler.requestTree(parent)
  scheduler.requestRenderable(child)
  assert.equal(scheduled.length, 1)
  scheduled[0]!()

  assert.equal(parent.renders, 1)
  assert.equal(parent.rebuilds, 1)
  assert.equal(child.renders, 1)
  assert.equal(child.rebuilds, 1)
})

test("render scheduler defers renderables past the per-flush budget", () => {
  const scheduled: Array<() => void> = []
  const panes = [renderable("pane-1"), renderable("pane-2"), renderable("pane-3")]
  const scheduler = createRenderScheduler({
    maxRenderablesPerFlush: 2,
    schedule: (callback) => {
      scheduled.push(callback)
      return scheduled.length as unknown as ReturnType<typeof setTimeout>
    },
    clearSchedule: () => {},
  })

  for (const pane of panes) {
    scheduler.requestRenderable(pane)
  }

  assert.equal(scheduled.length, 1)
  scheduled[0]!()

  assert.deepEqual(panes.map((pane) => pane.renders), [1, 1, 0])
  assert.equal(scheduled.length, 2)

  scheduled[1]!()
  assert.deepEqual(panes.map((pane) => pane.renders), [1, 1, 1])
})
