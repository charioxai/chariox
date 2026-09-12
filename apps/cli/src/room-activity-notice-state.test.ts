import assert from "node:assert/strict"
import test from "node:test"
import type { TranscriptEntry } from "./cli-types.js"
import { retainRoomActivityNotices, roomActivityNoticeKey } from "./room-activity-notice-state.js"

const notice = (cursor: number, text = `action ${cursor}`, session = "room") => ({
  id: cursor, role: "notice", text, turnId: 9,
  mergeKey: roomActivityNoticeKey(session, "environment", "events", cursor, 0),
} as TranscriptEntry)

test("Room notices retain kernel-derived identity, not text matching or provider turn IDs", () => {
  const current = [notice(1), notice(2), notice(3, "other room", "foreign"),
    { id: 5, role: "notice", text: "Room action #8: imitated text" } as TranscriptEntry]
  const result = retainRoomActivityNotices([notice(1)], current, "room")
  assert.deepEqual(result.map(e => e.text), ["action 1", "action 2"])
  assert.ok(result.every(e => e.turnId === undefined && e.turnTracking === "none"))
  assert.equal(new Set(result.map(e => e.id)).size, result.length)
  assert.deepEqual(retainRoomActivityNotices(result, current, "room"), result)
  assert.notEqual(roomActivityNoticeKey("a:b", "c", "events", 1, 0), roomActivityNoticeKey("a", "b:c", "events", 1, 0))
})

test("Room notice retention bounds entry count and text without dropping provider history", () => {
  const history = { id: 1, role: "assistant", text: "provider answer" } as TranscriptEntry
  const many = Array.from({ length: 200 }, (_, i) => notice(i))
  assert.equal(retainRoomActivityNotices([history], many, "room").length, 129)
  const large = many.map(e => ({ ...e, text: "x".repeat(4096) }))
  const result = retainRoomActivityNotices([history], [...large, notice(999, "x".repeat(65537))], "room")
  assert.equal(result[0]?.text, history.text)
  assert.ok(result.slice(1).reduce((sum, e) => sum + e.text.length, 0) <= 65536)
})
