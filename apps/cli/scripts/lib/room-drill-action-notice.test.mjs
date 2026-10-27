import assert from "node:assert/strict"
import test from "node:test"
import { assertRoomDrillCompletedActionNotice } from "./room-drill-action-notice.mjs"

const action = { action_id: "action-current", sequence: 12, kind: "pointer_click", state: "completed" }
const notice = (sequence, actor = "Agent") => `Room action #${sequence}: ${actor} · computer pointer_click · desktop, tab tab-1 · completed`
const input = (notices, baseline = 0) => ({
  action, actionId: action.action_id, kind: action.kind, notices, baseline,
})

test("exact action completion survives later unrelated tab and action notices", () => {
  const pattern = assertRoomDrillCompletedActionNotice(input([
    notice(11), notice(12), "Room tab: cancellation — http://fixture/cancellation", notice(13),
  ], 1))
  assert.equal(pattern.test(notice(12)), true)
  assert.equal(pattern.test(notice(11)), false)
  assert.equal(pattern.test(notice(13)), false)
})

test("prior or missing completion cannot satisfy the new action window", () => {
  for (const [notices, baseline] of [
    [[notice(12), "Room tab: cancellation"], 1],
    [[notice(11)], 0],
    [[notice(11), "Room tab: cancellation"], 0],
  ]) {
    assert.throws(() => assertRoomDrillCompletedActionNotice(input(notices, baseline)))
  }
})

test("completion remains bound to validated action ID, sequence, kind and state", () => {
  for (const changed of [
    { action_id: "other-action" }, { sequence: 0 }, { sequence: NaN },
    { sequence: Number.MAX_SAFE_INTEGER + 1 }, { kind: "clipboard_write" }, { state: "running" },
  ]) {
    assert.throws(() => assertRoomDrillCompletedActionNotice({
      ...input([notice(12)]), action: { ...action, ...changed },
    }))
  }
})

test("initial human action preserves Local user attribution", () => {
  assertRoomDrillCompletedActionNotice({ ...input([notice(12, "Local user")]), actorLabel: "Local user" })
  assert.throws(() => assertRoomDrillCompletedActionNotice({
    ...input([notice(12, "Agent")]), actorLabel: "Local user",
  }))
})
