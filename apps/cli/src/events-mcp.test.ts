import assert from "node:assert/strict"
import test from "node:test"
import { PassThrough } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import { runEventsMcp } from "./events-mcp.js"

type Json = Record<string, any>

const question = {
  title: "Colour", message: "Which colour?",
  choices: [{ id: "red", label: "Red" }, { id: "blue", label: "Blue", style: "primary" }],
  text_reply: { id: "other", label: "Something else", min_length: 1, max_length: 20 },
}
const decision = (cursor: number, value: Json, at: { kernel?: string; id?: string } = {}) => ({ cursor, kernel: at.kernel ?? "k", machine: "m", session_id: "room", at_ms: 1, summary: "Colour", refs: { interaction_id: at.id ?? "int-1" }, type: "interaction", value })
const pending = (cursor: number, at: { kernel?: string; id?: string } = {}, extra: Json = {}) => decision(cursor, { state: "pending", question: { ...question, ...extra } }, at)
const resolved = (cursor: number, at: { kernel?: string; id?: string } = {}) => decision(cursor, { state: "resolved", resolution: { via: "terminal" } }, at)

// The kernel's durable inbox: a wait replays what is not acknowledged yet, an ack removes the records up to its cursor.
class Inbox {
  acknowledged = 0
  constructor(readonly events: Json[]) {}
  wait(since?: number): Json[] { return this.events.filter(event => event.cursor > (since ?? this.acknowledged)) }
}

// A stub harness: it talks JSON-RPC over the same pipes a real one uses, and answers elicitation as scripted.
// `onElicit` returning undefined leaves the dialog open (the person has not answered). "k" is this computer's kernel.
async function harness(events: Json[] | Inbox, elicitation: Json | null, onElicit: (params: Json) => Json | undefined, failAnswers: string[] = []) {
  const inbox = events instanceof Inbox ? events : new Inbox(events)
  const sent: Json[] = []
  const routed: { kernel: string; request: Json }[] = []
  const input = new PassThrough(), output = new PassThrough()
  const lines: Json[] = []
  const replies = new Map<number, (message: Json) => void>()
  output.on("data", chunk => {
    for (const line of String(chunk).split("\n").filter(Boolean)) {
      const message = JSON.parse(line) as Json
      if (message.method === "elicitation/create") {
        lines.push(message)
        const result = onElicit(message.params)
        if (result) input.write(JSON.stringify({ jsonrpc: "2.0", id: message.id, result }) + "\n")
        continue
      }
      replies.get(message.id)?.(message)
    }
  })
  const fail = (request: Json) => { const code = failAnswers.shift(); if (code) throw new CommandFailure(code, "answer failed"); return request }
  const send = async (request: Json) => {
    sent.push(request)
    if ("WaitEvents" in request) {
      const events = inbox.wait((request.WaitEvents as Json).since as number | undefined)
      if (events.length === 0) await new Promise(resolve => setTimeout(resolve, 20))
      return { EventsListed: { events, next_cursor: events.at(-1)?.cursor ?? (request.WaitEvents as Json).since ?? inbox.acknowledged, oldest_cursor: 1, timed_out: events.length === 0 } }
    }
    if ("AckEvents" in request) inbox.acknowledged = Math.max(inbox.acknowledged, (request.AckEvents as Json).cursor as number)
    if ("RespondToInteraction" in request) fail(request)
    return { ack: true }
  }
  const routes = {
    isLocal: async (kernel: string) => kernel === "k",
    sendTo: async (kernel: string, request: Json) => { routed.push({ kernel, request }); fail(request); return { ack: true } },
  }
  const server = runEventsMcp(send as never, input, output, routes)
  let id = 0
  const call = (method: string, params: Json) => new Promise<Json>(resolve => { const n = ++id; replies.set(n, resolve); input.write(JSON.stringify({ jsonrpc: "2.0", id: n, method, params }) + "\n") })
  await call("initialize", { protocolVersion: "2025-06-18", capabilities: elicitation ? { elicitation } : {} })
  return { call, sent, routed, lines, inbox, close: async () => { input.end(); await server } }
}
const next = (h: Awaited<ReturnType<typeof harness>>, timeout_s = 5) => h.call("tools/call", { name: "chariox_next_choice", arguments: { timeout_s } })
const told = (reply: Json) => JSON.parse(reply.result.content[0].text) as Json
const acks = (h: Awaited<ReturnType<typeof harness>>) => h.sent.filter(request => "AckEvents" in request).map(request => (request.AckEvents as Json).cursor)
const elicitedQuestions = (h: Awaited<ReturnType<typeof harness>>) => h.lines.map(line => line.params.message)

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

test("a decision of another kernel is answered on that kernel and named in the plain text", async () => {
  const h = await harness([pending(4, { id: "int-local" }), pending(5, { kernel: "remote", id: "int-remote" })], { form: {} }, () => ({ action: "accept", content: { choice: "red" } }))
  assert.equal(told(await next(h)).interaction_id, "int-local")
  assert.equal(told(await next(h)).interaction_id, "int-remote")
  assert.deepEqual(h.routed, [{ kernel: "remote", request: { RespondToInteraction: { session_id: "room", interaction_id: "int-remote", choice_id: "red", custom_reply: null } } }])
  assert.deepEqual(h.sent.filter(request => "RespondToInteraction" in request).map(request => (request.RespondToInteraction as Json).interaction_id), ["int-local"])
  assert.deepEqual(acks(h), [4, 5])
  await h.close()
  const plain = await harness([pending(4, { id: "int-local" }), pending(5, { kernel: "remote", id: "int-remote" })], null, () => assert.fail("no dialog without elicitation"))
  const [local, remote] = [told(await next(plain)), told(await next(plain))]
  assert.equal("kernel" in local, false)
  assert.equal(remote.kernel, "remote")
  await plain.close()
})

test("a question the person has not answered stays in the inbox for the next process", async () => {
  const inbox = new Inbox([pending(4)])
  const first = await harness(inbox, { form: {} }, () => undefined)
  void next(first)
  await until(() => first.lines.length === 1)
  await first.close()
  assert.deepEqual(acks(first), [])
  const second = await harness(inbox, { form: {} }, () => ({ action: "accept", content: { choice: "red" } }))
  assert.deepEqual(told(await next(second)), { interaction_id: "int-1", action: "accept", choice_id: "red", answered: true })
  assert.equal(inbox.acknowledged, 4)
  await second.close()
})

test("a failed answer keeps the decision and the retry asks again; a closed one is acknowledged", async () => {
  const h = await harness([pending(4)], { form: {} }, () => ({ action: "accept", content: { choice: "red" } }), ["request_failed", "interaction_closed"])
  const failed = await next(h)
  assert.equal(failed.result.isError, true)
  assert.deepEqual(told(failed), { interaction_id: "int-1", action: "accept", choice_id: "red", answered: false, code: "request_failed" })
  assert.deepEqual(acks(h), [])
  const retried = await next(h)
  assert.deepEqual(told(retried), { interaction_id: "int-1", action: "accept", choice_id: "red", answered: false, code: "interaction_closed" })
  assert.equal(retried.result.isError, undefined)
  assert.deepEqual(acks(h), [4])
  assert.equal(elicitedQuestions(h).length, 2)
  await h.close()
})

test("a dismissed dialog is not asked again by the same process", async () => {
  const h = await harness([pending(4), pending(5, { id: "int-2" })], { form: {} }, () => ({ action: "decline" }))
  assert.equal(told(await next(h)).interaction_id, "int-1")
  assert.equal(told(await next(h)).interaction_id, "int-2")
  assert.deepEqual(told(await next(h, 1)), { timed_out: true })
  assert.deepEqual(acks(h), [])
  await h.close()
})

test("a batch with a pending record, its resolution, an expired question and a live one asks only the live one", async () => {
  const h = await harness([pending(4), pending(5, { id: "int-expired" }, { expires_at_ms: 1 }), pending(6, { id: "int-live" }, { message: "Still open?" }), resolved(7)], { form: {} }, () => ({ action: "accept", content: { choice: "red" } }))
  assert.deepEqual(told(await next(h)), { interaction_id: "int-live", action: "accept", choice_id: "red", answered: true })
  assert.deepEqual(elicitedQuestions(h), ["Still open?"])
  assert.deepEqual(h.sent.filter(request => "RespondToInteraction" in request).map(request => (request.RespondToInteraction as Json).interaction_id), ["int-live"])
  assert.equal(h.inbox.acknowledged, 7)
  await h.close()
})

async function until(done: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 200 && !done(); attempt++) await new Promise(resolve => setTimeout(resolve, 10))
  assert.ok(done(), "the condition did not become true")
}
