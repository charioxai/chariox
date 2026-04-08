import test from "node:test"
import assert from "node:assert/strict"

import {
  computeAnchoredScrollTop,
  computeCollapsedHistoryScrollTop,
  computePrependedHistoryScrollTop,
  clampScrollTop,
  findPrependedHistoryMergedHeadId,
  findTurnPromptScrollTarget,
} from "./history-viewport.js"

test("findPrependedHistoryMergedHeadId returns the current head id when prepended history stitches into it", () => {
  assert.equal(
    findPrependedHistoryMergedHeadId(
      [
        {
          id: 10,
          historyEntryIndex: 4,
          historyFragmentStart: 0,
          historyFragmentEnd: 120,
        },
      ],
      [
        {
          id: 20,
          historyEntryIndex: 4,
          historyFragmentStart: 120,
          historyFragmentEnd: 240,
        },
      ],
    ),
    20,
  )
})

test("findPrependedHistoryMergedHeadId ignores unrelated prepended history", () => {
  assert.equal(
    findPrependedHistoryMergedHeadId(
      [
        {
          id: 10,
          historyEntryIndex: 3,
          historyFragmentStart: 0,
          historyFragmentEnd: 120,
        },
      ],
      [
        {
          id: 20,
          historyEntryIndex: 4,
          historyFragmentStart: 0,
          historyFragmentEnd: 120,
        },
      ],
    ),
    null,
  )
})

test("computePrependedHistoryScrollTop preserves viewport after prepending older content", () => {
  assert.equal(computePrependedHistoryScrollTop(0, 120, 220, 80), 100)
  assert.equal(computePrependedHistoryScrollTop(24, 220, 300, 80), 104)
})

test("computePrependedHistoryScrollTop clamps to the new scroll range", () => {
  assert.equal(computePrependedHistoryScrollTop(0, 40, 70, 80), 0)
  assert.equal(computePrependedHistoryScrollTop(0, 60, 160, 80), 80)
})

test("computeAnchoredScrollTop preserves an anchor's viewport position", () => {
  assert.equal(computeAnchoredScrollTop(12, 50, 180, 80), 38)
})

test("computeAnchoredScrollTop clamps to the top when needed", () => {
  assert.equal(computeAnchoredScrollTop(30, 10, 180, 80), 0)
})

test("computeAnchoredScrollTop returns null without a usable anchor", () => {
  assert.equal(computeAnchoredScrollTop(null, 10, 180, 80), null)
  assert.equal(computeAnchoredScrollTop(10, null, 180, 80), null)
})

test("clampScrollTop keeps a preserved scroll position within the current viewport range", () => {
  assert.equal(clampScrollTop(120, 300, 80), 120)
  assert.equal(clampScrollTop(500, 300, 80), 220)
  assert.equal(clampScrollTop(-20, 300, 80), 0)
})

test("computeCollapsedHistoryScrollTop shifts upward by removed height", () => {
  assert.equal(computeCollapsedHistoryScrollTop(90, 320, 220, 80), 0)
  assert.equal(computeCollapsedHistoryScrollTop(140, 420, 320, 80), 40)
})

test("findTurnPromptScrollTarget navigates between prompt anchors", () => {
  const prompts = [0, 18, 47, 90]
  assert.equal(findTurnPromptScrollTarget(prompts, 0, "next"), 18)
  assert.equal(findTurnPromptScrollTarget(prompts, 25, "previous"), 0)
  assert.equal(findTurnPromptScrollTarget(prompts, 25, "next"), 47)
  assert.equal(findTurnPromptScrollTarget(prompts, 95, "next"), 90)
})
