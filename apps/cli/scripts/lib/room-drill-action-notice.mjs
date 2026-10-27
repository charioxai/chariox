import assert from "node:assert/strict"

export function assertRoomDrillCompletedActionNotice({
  action, actionId, kind, actorLabel = null, notices, baseline,
}) {
  assert.ok(typeof actionId === "string" && actionId.length > 0, "expected action identity is missing")
  assert.equal(action?.action_id, actionId, "notice action differs from the validated action")
  assert.ok(Number.isSafeInteger(action.sequence) && action.sequence > 0, "action sequence is invalid")
  assert.equal(action.kind, kind, "notice action kind differs")
  assert.equal(action.state, "completed", "notice action is not completed")
  assert.ok(Number.isSafeInteger(baseline) && baseline >= 0 && baseline <= notices.length,
    "notice baseline is invalid")
  const actor = actorLabel === null ? ".+" : escapeRegExp(actorLabel)
  const pattern = new RegExp(`^Room action #${action.sequence}: ${actor} · computer ${escapeRegExp(kind)} · desktop(?:, tab [^ ·]+)? · completed$`)
  assert.ok(notices.slice(baseline).some((notice) => pattern.test(notice)),
    "validated action completion notice is missing from the new notice window")
  return pattern
}

function escapeRegExp(value) {
  return String(value).replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}
