import assert from "node:assert/strict"
import test from "node:test"
import { PassThrough } from "node:stream"
import { runEventsMcp } from "./events-mcp.js"

type Json = Record<string, any>

const question = {
  title: "Colour", message: "Which colour?",
  choices: [{ id: "red", label: "Red" }, { id: "blue", label: "Blue", style: "primary" }],
  text_reply: { id: "other", label: "Something else", min_length: 1, max_length: 20 },
}
const decision = (cursor: number, value: Json) => ({ cursor, kernel: "k", machine: "m", session_id: "room", at_ms: 1, summary: "Colour", refs: { interaction_id: "int-1" }, type: "interaction", value })

// A stub harness: it talks JSON-RPC over the same pipes a real one uses, and answers elicitation as scripted.
async function harness(events: Json[], elicitation: Json | null, onElicit: (params: Json) => Json) {
  const sent: Json[] = []
  const input = new PassThrough(), output = new PassThrough()
  const lines: Json[] = []
  const replies = new Map<number, (message: Json) => void>()
  output.on("data", chunk => {
    for (const line of String(chunk).split("\n").filter(Boolean)) {
      const message = JSON.parse(line) as Json
      if (message.method === "elicitation/create") { input.write(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: onElicit(message.params) }) + "\n"); lines.push(message); continue }
      replies.get(message.id)?.(message)
    }
  })
  const send = async (request: Json) => {
    sent.push(request)
    if ("WaitEvents" in request) return { EventsListed: { events, next_cursor: 9, oldest_cursor: 1, timed_out: events.length === 0 } }
    return { ack: true }
  }
  const server = runEventsMcp(send as never, input, output)
  let id = 0
  const call = (method: string, params: Json) => new Promise<Json>(resolve => { const n = ++id; replies.set(n, resolve); input.write(JSON.stringify({ jsonrpc: "2.0", id: n, method, params }) + "\n") })
  await call("initialize", { protocolVersion: "2025-06-18", capabilities: elicitation ? { elicitation } : {} })
  return { call, sent, lines, close: async () => { input.end(); await server } }
}

test("the tool shows the pending question in the client's own dialog and sends the answer once", async () => {
  const h = await harness([decision(4, { state: "pending", question })], { form: {} }, params => ({ action: "accept", content: { choice: "blue" } }))
  const listed = await h.call("tools/list", {})
  assert.equal(listed.result.tools[0].name, "chariox_next_choice")
  const reply = await h.call("tools/call", { name: "chariox_next_choice", arguments: { timeout_s: 5 } })
  assert.deepEqual(JSON.parse(reply.result.content[0].text), { interaction_id: "int-1", action: "accept", choice_id: "blue", answered: true })
  const form = h.lines[0]!.params
  assert.equal(form.message, "Which colour?")
  assert.deepEqual(form.requestedSchema.properties.choice.oneOf, [{ const: "red", title: "Red" }, { const: "blue", title: "Blue" }, { const: "other", title: "Something else" }])
  const answers = h.sent.filter(request => "RespondToInteraction" in request)
  assert.deepEqual(answers, [{ RespondToInteraction: { session_id: "room", interaction_id: "int-1", choice_id: "blue", custom_reply: null } }])
  assert.deepEqual(h.sent.filter(request => "AckEvents" in request), [{ AckEvents: { cursor: 4 } }])
  await h.close()
})

test("a typed reply travels as custom_reply and a declined dialog answers nothing", async () => {
  const typed = await harness([decision(4, { state: "pending", question })], { form: {} }, () => ({ action: "accept", content: { choice: "other", reply: "teal" } }))
  await typed.call("tools/call", { name: "chariox_next_choice", arguments: { timeout_s: 5 } })
  assert.deepEqual(typed.sent.find(request => "RespondToInteraction" in request), { RespondToInteraction: { session_id: "room", interaction_id: "int-1", choice_id: "other", custom_reply: "teal" } })
  await typed.close()
  const declined = await harness([decision(4, { state: "pending", question })], { form: {} }, () => ({ action: "decline" }))
  const reply = await declined.call("tools/call", { name: "chariox_next_choice", arguments: { timeout_s: 5 } })
  assert.deepEqual(JSON.parse(reply.result.content[0].text), { interaction_id: "int-1", action: "decline" })
  assert.equal(declined.sent.some(request => "RespondToInteraction" in request), false)
  await declined.close()
})

test("a client without elicitation gets the question text and an owner pointer is passed on verbatim", async () => {
  const plain = await harness([decision(4, { state: "pending", question })], null, () => assert.fail("no dialog without elicitation"))
  const reply = await plain.call("tools/call", { name: "chariox_next_choice", arguments: { timeout_s: 5 } })
  assert.equal(JSON.parse(reply.result.content[0].text).question.message, "Which colour?")
  assert.equal(plain.sent.some(request => "RespondToInteraction" in request), false)
  await plain.close()
  const pointer = await harness([decision(5, { state: "pending", pointer: { approval_id: "a", kind: "critical_approval", kernel: "k", one_line: "Approval pending on kernel k. Open Chariox there and answer its approval popup.", expires_at: 1 } })], { form: {} }, () => assert.fail("an owner-only decision is not asked"))
  const shown = await pointer.call("tools/call", { name: "chariox_next_choice", arguments: { timeout_s: 5 } })
  assert.deepEqual(JSON.parse(shown.result.content[0].text), { interaction_id: "int-1", answerable: false, show_verbatim: "Approval pending on kernel k. Open Chariox there and answer its approval popup." })
  await pointer.close()
})
