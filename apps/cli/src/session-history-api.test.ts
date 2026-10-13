import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import { getSessionHistoryOutline, getSessionHistoryBlobContents } from "./session-history-api.js"

test("MP-08 / MP-10 initial cross-agent content uses one outline RPC", async () => {
  const calls: string[] = []
  const contents = ["a", "b", "c"].map((id) => ({ blob_id: `blob-${id}`, entries: [] }))
  const client = { send: async (request: Record<string, any>) => {
    calls.push(Object.keys(request)[0]!)
    assert.equal(request.GetSessionHistoryOutline.include_prefetched_content, true)
    return { SessionHistoryOutline: { agents: ["a", "b", "c"].map((agent_id, i) => ({ agent_id, turns: [], prefetched_blobs: [contents[i]] })) } }
  } } as unknown as LocalIpcClient
  const value = await getSessionHistoryOutline(client, "s", ["a", "b", "c"])
  assert.deepEqual(calls, ["GetSessionHistoryOutline"])
  assert.deepEqual(value.agents.map((a) => a.loadedBlobs), contents.map((c) => [c]))
})

// MP-08 / MP-10 (panes Phase-1 drill, PANE-01/PANE-03): the TUI loads the
// prefetched blobs with the outline in one batch per agent, so assistant rows
// read as text; an older kernel leaves them as click-to-load placeholders.
test("session history outline loads the prefetch batch with the outline", async () => {
  const requests: string[] = []
  const outline = {
    SessionHistoryOutline: {
      session_id: "session-1",
      agents: [{
        agent_id: "agent-1",
        next_cursor: null,
        turns: [{
          turn_id: "turn-1",
          prompt_id: "prompt-1",
          started_at_ms: 1,
          lifecycle: "completed",
          user_prompt: { entry_index: 1, fragment_start: 0, fragment_end: 2, total_chars: 2, entry: { kind: "user_prompt", text: "hi" } },
          entries: [],
          summary: null,
          blobs: [{ blob_id: "history:2:2", kind: "provider_output", title: "assistant output", summary: "", sequence_start: 2, sequence_end: 2, entry_count: 1, total_chars: 5, timestamp_ms: 2 }],
        }],
      }],
    },
  }
  const content = { blob_id: "history:2:2", entries: [{ entry_index: 2, fragment_start: 0, fragment_end: 5, total_chars: 5, entry: { kind: "provider_output", text: "hello" } }] }
  const client = (batch: "ok" | "unsupported") => ({
    send: async (request: Record<string, unknown>) => {
      const name = Object.keys(request)[0]!
      requests.push(name)
      if (name === "GetSessionHistoryOutline") return outline
      if (batch === "unsupported") throw new Error("unknown variant `GetSessionHistoryBlobContents`")
      return { SessionHistoryBlobContents: { blobs: [content] } }
    },
  }) as unknown as LocalIpcClient

  const loaded = await getSessionHistoryOutline(client("ok"), "session-1", ["agent-1"])
  assert.deepEqual(requests, ["GetSessionHistoryOutline", "GetSessionHistoryBlobContents"])
  assert.deepEqual(loaded.agents[0]?.loadedBlobs, [content])

  const fallback = await getSessionHistoryOutline(client("unsupported"), "session-1", ["agent-1"])
  assert.deepEqual(fallback.agents[0]?.loadedBlobs, [])
})

// MP-08 / MP-10, #987: explicit detail loading is bounded at 64 rows.
test("detail batches are bounded and preserve legacy sequential loading", async () => {
  const requests: Record<string, any>[] = []
  const ids = Array.from({ length: 130 }, (_, i) => "blob-" + i)
  const client = { send: async (request: Record<string, any>) => {
    requests.push(request)
    if (request.GetSessionHistoryBlobContents) return { SessionHistoryBlobContents: { blobs: request.GetSessionHistoryBlobContents.blob_ids.map((blob_id: string) => ({ blob_id, entries: [] })) } }
    return { SessionHistoryBlobContent: { blob_id: request.GetSessionHistoryBlobContent.blob_id, entries: [] } }
  } } as unknown as LocalIpcClient
  assert.equal((await getSessionHistoryBlobContents(client, "s", "a", ids, true)).length, 130)
  assert.deepEqual(requests.map(r => r.GetSessionHistoryBlobContents.blob_ids.length), [64, 64, 2])
  requests.length = 0
  const legacy = { send: async (request: Record<string, any>) => {
    requests.push(request)
    if (request.GetSessionHistoryBlobContents) throw new Error("unknown variant")
    return { SessionHistoryBlobContent: { blob_id: request.GetSessionHistoryBlobContent.blob_id, entries: [] } }
  } } as unknown as LocalIpcClient
  assert.equal((await getSessionHistoryBlobContents(legacy, "s", "a", ids.slice(0, 3), true)).length, 3)
  assert.equal(requests.length, 4)
})
