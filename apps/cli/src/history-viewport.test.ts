import test from "node:test"
import assert from "node:assert/strict"

import { computePrependedHistoryScrollTop, findPrependedHistoryMergedHeadId } from "./history-viewport.js"

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
