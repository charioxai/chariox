import assert from "node:assert/strict"
import test from "node:test"

import { createCliLoadingStateController } from "./cli-loading-state-controller.js"

test("cli loading state controller renders history loading changes", () => {
  const harness = createHarness()

  harness.controller.setHistoryLoadingState(true)
  harness.controller.setHistoryLoadingState(false)

  assert.deepEqual(harness.calls, [
    "setLoadingHistory:true",
    "setHistoryLoadingMessage:null",
    "renderHistoryLoadingIndicator",
    "setLoadingHistory:false",
    "setHistoryLoadingMessage:null",
    "renderHistoryLoadingIndicator",
  ])
})

test("cli loading state controller keeps history error visible after loading fails", () => {
  const harness = createHarness()

  harness.controller.setHistoryLoadingState(false, "Failed to load older history.")

  assert.deepEqual(harness.calls, [
    "setLoadingHistory:false",
    "setHistoryLoadingMessage:Failed to load older history.",
    "renderHistoryLoadingIndicator",
  ])
})

test("cli loading state controller ignores unchanged session hydration state", () => {
  const harness = createHarness({ sessionHydrating: true })

  assert.equal(harness.controller.setSessionHydratingState(true), false)

  assert.deepEqual(harness.calls, [])
})

test("cli loading state controller rebuilds empty attached transcript outside workflow screen", () => {
  const harness = createHarness({
    attached: true,
    visibleTranscriptEntryCount: 0,
    workflowScreenActive: false,
  })

  assert.equal(harness.controller.setSessionHydratingState(true), true)

  assert.equal(harness.sessionHydrating, true)
  assert.deepEqual(harness.calls, [
    "setSessionHydrating:true",
    "rebuildTranscript",
  ])
})

test("cli loading state controller requests transcript render for normal hydration changes", () => {
  const attachedWithEntries = createHarness({
    attached: true,
    visibleTranscriptEntryCount: 1,
  })
  assert.equal(attachedWithEntries.controller.setSessionHydratingState(true), true)
  assert.deepEqual(attachedWithEntries.calls, [
    "setSessionHydrating:true",
    "requestTranscriptRender",
  ])

  const workflowScreen = createHarness({
    attached: true,
    visibleTranscriptEntryCount: 0,
    workflowScreenActive: true,
  })
  assert.equal(workflowScreen.controller.setSessionHydratingState(true), true)
  assert.deepEqual(workflowScreen.calls, [
    "setSessionHydrating:true",
    "requestTranscriptRender",
  ])
})

function createHarness(options: {
  sessionHydrating?: boolean
  attached?: boolean
  visibleTranscriptEntryCount?: number
  workflowScreenActive?: boolean
} = {}) {
  const calls: string[] = []
  const harness = {
    calls,
    sessionHydrating: options.sessionHydrating ?? false,
    controller: null as ReturnType<typeof createCliLoadingStateController> | null,
  }
  harness.controller = createCliLoadingStateController({
    getSessionHydrating: () => harness.sessionHydrating,
    setSessionHydrating: (next) => {
      calls.push(`setSessionHydrating:${next}`)
      harness.sessionHydrating = next
    },
    setLoadingHistory: (next) => {
      calls.push(`setLoadingHistory:${next}`)
    },
    setHistoryLoadingMessage: (next) => {
      calls.push(`setHistoryLoadingMessage:${next}`)
    },
    renderHistoryLoadingIndicator: () => {
      calls.push("renderHistoryLoadingIndicator")
    },
    isAttached: () => options.attached ?? false,
    visibleTranscriptEntryCount: () => options.visibleTranscriptEntryCount ?? 0,
    workflowScreenActive: () => options.workflowScreenActive ?? false,
    rebuildTranscript: () => {
      calls.push("rebuildTranscript")
    },
    requestTranscriptRender: () => {
      calls.push("requestTranscriptRender")
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createCliLoadingStateController>
  }
}
