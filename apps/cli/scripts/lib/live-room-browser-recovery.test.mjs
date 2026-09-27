import assert from "node:assert/strict"
import test from "node:test"
import { observeRoomStaleToolError } from "./live-room-browser-recovery.mjs"

const toolEntry = (overrides = {}) => ({ entry: { kind: "provider_tool", text: JSON.stringify({
  tool: "slice_browser_fill", status: "failed", input: { text: "STALE ATTEMPT MUST NOT LAND" },
  error: "environment_stale_element_reference", ...overrides,
}) } })

function boundary(turns, contents = {}, options = {}) {
  const sent = []
  const timeouts = []
  const input = {
    sessionId: "room", requests: {
      getSessionHistoryOutlineRequest: (...args) => ({ kind: "outline", args }),
      getSessionHistoryBlobContentRequest: (...args) => ({ kind: "blob", args }),
    },
    client: { send: async (request) => {
      sent.push(request)
      await options.onSend?.(request)
      if (request.kind === "outline") return { SessionHistoryOutline: { agents: [{ agent_id: "agent", turns }] } }
      return { SessionHistoryBlobContent: { entries: contents[request.args[2]] ?? [] } }
    } },
    withTimeout: async (promise, milliseconds) => { timeouts.push(milliseconds); return promise },
  }
  return { input, sent, timeouts, observe: () => observeRoomStaleToolError(input, "agent", new Set(["old"])) }
}

test("only current-turn provider tool failures prove stale rejection", async () => {
  const ignored = toolEntry()
  const fixture = boundary([
    { turn_id: "old", entries: [ignored] },
    { turn_id: "new", user_prompt: ignored, entries: [
      { entry: { ...ignored.entry, kind: "provider_output" } },
      { entry: { ...ignored.entry, kind: "user_prompt" } },
      toolEntry({ input: { text: "another fill" } }),
      toolEntry({ tool: "slice_browser_find" }),
    ] },
  ])
  assert.equal(await fixture.observe(), false)
  assert.equal(await boundary([{ turn_id: "new", entries: [toolEntry()] }]).observe(), true)
})

test("history inspection has one deadline, not a renewed timeout for each blob", async (t) => {
  t.mock.timers.enable({ apis: ["Date"], now: 0 })
  const fixture = boundary([{ turn_id: "new", blobs: [{ kind: "provider_tool", blob_id: "one", total_chars: 10 }] }], {}, {
    onSend: () => t.mock.timers.tick(4_000),
  })
  assert.equal(await fixture.observe(), false)
  assert.deepEqual(fixture.timeouts, [5_000, 1_000])
})

test("entry and actual character limits apply across inline and hydrated history", async () => {
  const blob = { kind: "provider_tool", blob_id: "one", total_chars: 1 }
  assert.equal(await boundary([{ turn_id: "new", entries: Array.from({ length: 256 }, () => toolEntry({ error: "other" })), blobs: [blob] }], { one: [toolEntry()] }).observe(), false)
  const large = toolEntry({ error: "other", output: "x".repeat(26_000) })
  assert.equal(await boundary([{ turn_id: "new", entries: [large, large, large, large, large, large], blobs: [blob] }], { one: [toolEntry()] }).observe(), false)
})

test("history transport failures cannot leak payload text into evidence", async () => {
  const fixture = boundary([], {}, { onSend: () => { throw new Error("SECRET transport payload") } })
  await assert.rejects(fixture.observe(), (error) => error.message === "stale tool history unavailable" && !JSON.stringify(error).includes("SECRET"))
})

test("blob hydration stays bounded and does not inspect advertised summaries", async () => {
  const blob = (id, total_chars = 100) => ({ kind: "provider_tool", blob_id: id, total_chars, summary: toolEntry().entry.text })
  const fixture = boundary([{ turn_id: "new", blobs: [
    blob("oversized", 32769), blob("negative", -1), blob("fraction", 1.5),
    blob("duplicate"), blob("duplicate"), ...Array.from({ length: 9 }, (_, index) => blob(`blob-${index}`)),
  ] }])
  assert.equal(await fixture.observe(), false)
  assert.equal(fixture.sent.filter((item) => item.kind === "blob").length, 8)
  assert.equal(fixture.sent.filter((item) => item.args[2] === "duplicate").length, 1)
  const large = boundary([{ turn_id: "new", blobs: Array.from({ length: 8 }, (_, index) => blob(`large-${index}`, 32768)) }])
  assert.equal(await large.observe(), false)
  assert.equal(large.sent.filter((item) => item.kind === "blob").length, 4)
})

test("hydrated Codex MCP failure returns only a boolean, never its payload", async () => {
  // Codex tool_update.rs renders MCP result as a JSON output string.
  const output = JSON.stringify({ isError: true, content: [{ type: "text", text: "environment_stale_element_reference: SECRET field detail" }] })
  const fixture = boundary([{ turn_id: "new", blobs: [{ kind: "provider_tool", blob_id: "result", total_chars: 1024 }] }], {
    result: [toolEntry({ status: "completed", error: undefined, output })],
  })
  assert.strictEqual(await fixture.observe(), true)
})

test("a successful tool output mentioning the stale code does not prove rejection", async () => {
  const entry = toolEntry({ status: "completed", error: undefined, output: JSON.stringify({ isError: false,
    content: [{ type: "text", text: "environment_stale_element_reference" }],
  }) })
  assert.equal(await boundary([{ turn_id: "new", entries: [entry] }]).observe(), false)
})

test("deadline expiration starts no additional history request", async (t) => {
  t.mock.timers.enable({ apis: ["Date"], now: 0 })
  const fixture = boundary([{ turn_id: "new", blobs: [{ kind: "provider_tool", blob_id: "one", total_chars: 10 }] }], {}, {
    onSend: () => t.mock.timers.tick(5_000),
  })
  await assert.rejects(fixture.observe(), /stale tool history deadline/)
  assert.equal(fixture.sent.length, 1)
})
