import assert from "node:assert/strict"
import test from "node:test"

import {
  sessionHistoryFragmentsAreAdjacent,
  transcriptHistoryFragmentsAreAdjacent,
  transcriptHistoryFragmentShouldDefer,
} from "./session-history-fragments.js"

test("session history fragments are adjacent only for same entry and touching ranges", () => {
  assert.equal(sessionHistoryFragmentsAreAdjacent({
    entry_index: 7,
    fragment_start: 0,
    fragment_end: 12,
  }, {
    entry_index: 7,
    fragment_start: 12,
    fragment_end: 24,
  }), true)
  assert.equal(sessionHistoryFragmentsAreAdjacent({
    entry_index: 7,
    fragment_end: 12,
  }, {
    entry_index: 8,
    fragment_start: 12,
  }), false)
  assert.equal(sessionHistoryFragmentsAreAdjacent({
    entry_index: 7,
    fragment_end: 12,
  }, {
    entry_index: 7,
    fragment_start: 13,
  }), false)
  assert.equal(sessionHistoryFragmentsAreAdjacent(null, {
    entry_index: 7,
    fragment_start: 12,
  }), false)
})

test("transcript history fragments are adjacent only for same entry and touching ranges", () => {
  assert.equal(transcriptHistoryFragmentsAreAdjacent({
    historyEntryIndex: 7,
    historyFragmentStart: 0,
    historyFragmentEnd: 12,
  }, {
    historyEntryIndex: 7,
    historyFragmentStart: 12,
    historyFragmentEnd: 24,
  }), true)
  assert.equal(transcriptHistoryFragmentsAreAdjacent({
    historyEntryIndex: 7,
    historyFragmentEnd: 12,
  }, {
    historyEntryIndex: 8,
    historyFragmentStart: 12,
  }), false)
  assert.equal(transcriptHistoryFragmentsAreAdjacent({
    historyEntryIndex: 7,
    historyFragmentEnd: 12,
  }, {
    historyEntryIndex: 7,
    historyFragmentStart: 13,
  }), false)
})

test("transcript history fragment deferral starts after the first fragment", () => {
  assert.equal(transcriptHistoryFragmentShouldDefer({
    historyEntryIndex: 7,
    historyFragmentStart: 0,
  }), false)
  assert.equal(transcriptHistoryFragmentShouldDefer({
    historyEntryIndex: 7,
    historyFragmentStart: 12,
  }), true)
  assert.equal(transcriptHistoryFragmentShouldDefer({}), false)
})
