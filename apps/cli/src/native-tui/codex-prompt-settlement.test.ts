import assert from "node:assert/strict"
import test from "node:test"
import type { LocalIpcClient } from "../ipc.js"
import { loadCodexPromptSettlements } from "./codex-prompt-settlement.js"

test("MP-08/MP-11 settlement recovery pages past newer turns and accepts only matching settled prompts", async () => {
  const cursors: unknown[] = []
  const client = { send: async (request: { GetSessionHistoryOutline: { cursor: unknown } }) => {
    const cursor = request.GetSessionHistoryOutline.cursor
    cursors.push(cursor)
    return { SessionHistoryOutline: { agents: [{ agent_id: "agent", turns: cursor
      ? [{ prompt_id: "A", lifecycle: "cancelled", completed_at_ms: 2000 }, { prompt_id: "failed", lifecycle: "failed", completed_at_ms: 3000 }]
      : [{ prompt_id: "new", lifecycle: "open", completed_at_ms: null }, { prompt_id: "unrelated", lifecycle: "completed", completed_at_ms: 4000 }],
      next_cursor: cursor ? null : { before_sequence: 50 } }] } }
  } } as unknown as LocalIpcClient
  const settled = await loadCodexPromptSettlements(client, "session", "agent", ["A", "new", "failed"])
  assert.deepEqual(cursors, [null, { before_sequence: 50 }])
  assert.deepEqual([...settled], [["A", { completedAtMs: 2000, status: "cancelled" }], ["failed", { completedAtMs: 3000, status: "failed" }]])
})

test("MP-08/MP-11 settlement recovery rejects a nonadvancing kernel history cursor", async () => {
  const client = { send: async () => ({ SessionHistoryOutline: { agents: [{ agent_id: "agent", turns: [], next_cursor: { before_sequence: 50 } }] } }) } as unknown as LocalIpcClient
  await assert.rejects(loadCodexPromptSettlements(client, "session", "agent", ["A"]), /cursor did not advance/)
})
