import assert from "node:assert/strict"
import test from "node:test"

import {
  evaluateTranscriptScrollMonitor,
  nextWaitingRoomIntroStep,
  shouldLoadShortViewportHistory,
} from "./background-effects.js"

test("evaluateTranscriptScrollMonitor requests older history only when the user scrolls to the top", () => {
  assert.deepEqual(
    evaluateTranscriptScrollMonitor({
      hasScrollbox: true,
      pendingHistoryScrollRestore: 0,
      currentScrollTop: 0,
      lastTranscriptScrollTop: 12,
      hasMoreHistory: true,
      loadingHistory: false,
    }),
    {
      shouldLoadOlderHistory: true,
      nextLastScrollTop: 0,
    },
  )
})

test("evaluateTranscriptScrollMonitor stays idle while restoring scroll position", () => {
  assert.deepEqual(
    evaluateTranscriptScrollMonitor({
      hasScrollbox: true,
      pendingHistoryScrollRestore: 1,
      currentScrollTop: 0,
      lastTranscriptScrollTop: 12,
      hasMoreHistory: true,
      loadingHistory: false,
    }),
    {
      shouldLoadOlderHistory: false,
      nextLastScrollTop: 12,
    },
  )
})

test("shouldLoadShortViewportHistory only loads when the viewport is filled from the top", () => {
  assert.equal(
    shouldLoadShortViewportHistory({
      hasScrollbox: true,
      attached: true,
      loadingHistory: false,
      hasMoreHistory: true,
      scrollTop: 0,
      scrollHeight: 10,
      viewportHeight: 20,
    }),
    true,
  )
  assert.equal(
    shouldLoadShortViewportHistory({
      hasScrollbox: true,
      attached: true,
      loadingHistory: false,
      hasMoreHistory: true,
      scrollTop: 5,
      scrollHeight: 10,
      viewportHeight: 20,
    }),
    false,
  )
})

test("nextWaitingRoomIntroStep advances only while detached and below the cap", () => {
  assert.equal(nextWaitingRoomIntroStep(false, 3), 4)
  assert.equal(nextWaitingRoomIntroStep(false, 12), null)
  assert.equal(nextWaitingRoomIntroStep(true, 3), null)
})
