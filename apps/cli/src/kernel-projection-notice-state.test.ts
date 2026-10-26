import assert from "node:assert/strict"
import test from "node:test"
import type { TranscriptEntry } from "./cli-types.js"
import { retainKernelProjectionNotices, roomActivityNoticeKey } from "./kernel-projection-notice-state.js"

const notice = (cursor: number, text = `action ${cursor}`, session = "room") => ({
  id: cursor, role: "notice", text, turnId: 9,
  mergeKey: roomActivityNoticeKey(session, "environment", "events", cursor, 0),
} as TranscriptEntry)

test("Room notices retain kernel-derived identity, not text matching or provider turn IDs", () => {
  const current = [notice(1), notice(2), notice(3, "other room", "foreign"),
    { id: 5, role: "notice", text: "Room action #8: imitated text" } as TranscriptEntry]
  const result = retainKernelProjectionNotices([notice(1)], current, "room")
  assert.deepEqual(result.map(e => e.text), ["action 1", "action 2"])
  assert.ok(result.every(e => e.turnId === undefined && e.turnTracking === "none"))
  assert.equal(new Set(result.map(e => e.id)).size, result.length)
  assert.deepEqual(retainKernelProjectionNotices(result, current, "room"), result)
  assert.notEqual(roomActivityNoticeKey("a:b", "c", "events", 1, 0), roomActivityNoticeKey("a", "b:c", "events", 1, 0))
})

test("Room notice retention bounds entry count and text without dropping provider history", () => {
  const history = { id: 1, role: "assistant", text: "provider answer" } as TranscriptEntry
  const many = Array.from({ length: 200 }, (_, i) => notice(i))
  assert.equal(retainKernelProjectionNotices([history], many, "room").length, 129)
  const large = many.map(e => ({ ...e, text: "x".repeat(4096) }))
  const result = retainKernelProjectionNotices([history], [...large, notice(999, "x".repeat(65537))], "room")
  assert.equal(result[0]?.text, history.text)
  assert.ok(result.slice(1).reduce((sum, e) => sum + e.text.length, 0) <= 65536)
})

// MP-08/MP-10: retain only the current session's kernel-sourced inspection.
test("wake inspection retention is scoped and preserves provider attribution", () => {
  const reply = { id: 1, role: "assistant", text: "reminder", turnId: 4 } as TranscriptEntry
  const inspection = { id: 2, role: "notice", text: "timer fired", turnId: 4,
    mergeKey: "agent-wakes:room:123" } as TranscriptEntry
  const foreign = { ...inspection, mergeKey: "agent-wakes:other:123" }
  const unkeyed = { id: 3, role: "notice", text: "client status" } as TranscriptEntry
  const result = retainKernelProjectionNotices([reply], [inspection, foreign, unkeyed], "room")
  assert.deepEqual(result.map(entry => entry.text), ["reminder", "timer fired"])
  assert.equal(result[0]?.turnId, 4)
  assert.equal(result[1]?.turnId, undefined)
  assert.equal(result[1]?.turnTracking, "none")
  assert.deepEqual(retainKernelProjectionNotices(result, [inspection], "room"), result)
})
