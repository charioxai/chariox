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

const SOURCES = [
  { kind: "workflow_exit", source_id: "wf_source_a", kernel_id: "k", session_id: "room", workflow_id: "w", exit_node_id: "exit-1", exit_ordinal: 0, name: "Review", display_name: "Review", available: true },
]
const notification = (cursor: number, occurrence = `run-${cursor}`) => ({ cursor, type: "notification", kernel: "k", machine: "m", session_id: "room", at_ms: 1, summary: "Review finished", refs: {}, value: { source_id: "wf_source_a", source_kind: "workflow_exit", display_name: "Review", occurrence_id: occurrence, output: { message: "approved" }, deadline_ms: 604800001 } })
const gap = (cursor: number) => ({ cursor, type: "gap", kernel: "k", machine: "m", session_id: "room", at_ms: 2, summary: "1 unread notification expired", refs: {}, value: { kernel: "k", dropped: 1, from: 4, to: 4 } })

// The kernel's durable inbox: a wait replays what is not acknowledged yet, an ack removes the records up to its cursor.
class Inbox {
  acknowledged = 0
  constructor(readonly events: Json[]) {}
  wait(since?: number, limit = Infinity): Json[] { return this.events.filter(event => event.cursor > (since ?? this.acknowledged)).slice(0, limit) }
}

// A stub harness: it talks JSON-RPC over the same pipes a real one uses, and answers elicitation as scripted.
// `onElicit` returning undefined leaves the dialog open (the person has not answered). "k" is this computer's kernel.
async function harness(events: Json[] | Inbox, elicitation: Json | null, onElicit: (params: Json) => Json | undefined, failAnswers: string[] = [], failRoutes = 0, version = "2025-06-18") {
  const inbox = events instanceof Inbox ? events : new Inbox(events)
  const sent: Json[] = []
  const routed: { kernel: string; request: Json }[] = []
  const input = new PassThrough(), output = new PassThrough()
  const lines: Json[] = []
  const replies = new Map<number, (message: Json) => void>()
  // Everything the server wrote that is neither a reply to a call nor an elicitation request (notifications, stray replies).
  const other: Json[] = []
  output.on("data", chunk => {
    for (const line of String(chunk).split("\n").filter(Boolean)) {
      const message = JSON.parse(line) as Json
      if (message.method === "elicitation/create") {
        lines.push(message)
        const result = onElicit(message.params)
        if (result) input.write(JSON.stringify({ jsonrpc: "2.0", id: message.id, ...("error" in result ? { error: result.error } : { result }) }) + "\n")
        continue
      }
      const reply = replies.get(message.id)
      if (reply) reply(message)
      else other.push(message)
    }
  })
  const fail = (request: Json) => { const code = failAnswers.shift(); if (code) throw new CommandFailure(code, "answer failed"); return request }
  const send = async (request: Json) => {
    sent.push(request)
    if ("WaitEvents" in request || "ListEvents" in request) {
      const query = (request.WaitEvents ?? request.ListEvents) as Json
      const events = inbox.wait(query.since as number | undefined, query.limit as number)
      if (events.length === 0 && "WaitEvents" in request) await new Promise(resolve => setTimeout(resolve, 20))
      return { EventsListed: { events, next_cursor: events.at(-1)?.cursor ?? query.since ?? inbox.acknowledged, oldest_cursor: 1, timed_out: events.length === 0 } }
    }
    if ("AckEvents" in request) inbox.acknowledged = Math.max(inbox.acknowledged, (request.AckEvents as Json).cursor as number)
    if ("RespondToInteraction" in request) fail(request)
    if ("ListNotificationSources" in request) return { NotificationSources: { sources: SOURCES } }
    if ("SubscribeEvents" in request || "UnsubscribeEvents" in request || "GetEventSubscription" in request) return { EventSubscription: { subscription: { subscription_id: "s", targets: [], sources: [], kernels: [], acknowledged_cursor: 0, ended: null } } }
    return { ack: true }
  }
  const routes = {
    isLocal: async (kernel: string) => { if (failRoutes-- > 0) throw new Error("relay status failed"); return kernel === "k" },
    sendTo: async (kernel: string, request: Json) => { routed.push({ kernel, request }); fail(request); return { ack: true } },
  }
  const server = runEventsMcp(send as never, input, output, routes)
  let id = 0
  const call = (method: string, params: Json) => new Promise<Json>(resolve => { const n = ++id; replies.set(n, resolve); input.write(JSON.stringify({ jsonrpc: "2.0", id: n, method, params }) + "\n") })
  const initialized = await call("initialize", { protocolVersion: version, capabilities: elicitation ? { elicitation } : {} })
  const respond = (dialog: Json, result: Json) => input.write(JSON.stringify({ jsonrpc: "2.0", id: dialog.id, result }) + "\n")
  const raw = (text: string) => input.write(text + "\n")
  return { call, respond, raw, other, initialized, sent, routed, lines, inbox, close: async () => { input.end(); await server } }
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
  assert.deepEqual(form.requestedSchema.properties.choice.enum, ["red", "blue", "other"])
  assert.deepEqual(form.requestedSchema.properties.choice.enumNames, ["Red", "Blue", "Something else"])
  const answers = h.sent.filter(request => "RespondToInteraction" in request)
  assert.deepEqual(answers, [{ RespondToInteraction: { session_id: "room", interaction_id: "int-1", choice_id: "blue", custom_reply: null } }])
  assert.deepEqual(h.sent.filter(request => "AckEvents" in request), [{ AckEvents: { cursor: 4 } }])
  await h.close()
})

// The `elicitation/create` requestedSchema of each MCP version, as its schema.ts defines it
// (github.com/modelcontextprotocol/specification, schema/<version>/schema.ts, `ElicitRequest`):
// 2025-06-18 `EnumSchema` = { type: "string", title?, description?, enum: string[], enumNames?: string[] };
// 2025-11-25 `SingleSelectEnumSchema` = `UntitledSingleSelectEnumSchema` { type, title?, enum } | `TitledSingleSelectEnumSchema`
// { type, title?, oneOf: { const, title }[] }; `enumNames` is the deprecated `LegacyTitledEnumSchema`. Both versions allow a
// `StringSchema` { type: "string", title?, minLength?, maxLength? } beside it.
const KEYS = {
  "2025-06-18": { choice: ["type", "title", "enum", "enumNames"], valid: (choice: Json) => Array.isArray(choice.enum) && choice.enum.every((value: unknown) => typeof value === "string") && (choice.enumNames === undefined || (Array.isArray(choice.enumNames) && choice.enumNames.length === choice.enum.length && choice.enumNames.every((name: unknown) => typeof name === "string"))) },
  "2025-11-25": { choice: ["type", "title", "oneOf"], valid: (choice: Json) => Array.isArray(choice.oneOf) && choice.oneOf.every((option: Json) => typeof option.const === "string" && typeof option.title === "string" && Object.keys(option).length === 2) },
} as const

function conforms(version: keyof typeof KEYS, schema: Json): void {
  assert.equal(schema.type, "object")
  assert.deepEqual(schema.required, ["choice"])
  const choice = schema.properties.choice as Json
  assert.equal(choice.type, "string")
  assert.ok(Object.keys(choice).every(key => (KEYS[version].choice as readonly string[]).includes(key)), `${version}: unexpected keys ${Object.keys(choice)}`)
  assert.ok(KEYS[version].valid(choice), `${version}: the choice is not a ${version} single-select enum`)
  const reply = schema.properties.reply as Json
  assert.ok(reply.type === "string" && Object.keys(reply).every(key => ["type", "title", "minLength", "maxLength"].includes(key)))
}

test("the chooser is encoded for the negotiated MCP version and conforms to that version's schema", async () => {
  for (const version of ["2025-06-18", "2025-11-25"] as const) {
    const h = await harness([decision(4, { state: "pending", question })], { form: {} }, () => ({ action: "accept", content: { choice: "blue" } }), [], 0, version)
    assert.equal(told(await next(h)).answered, true)
    const schema = h.lines[0]!.params.requestedSchema as Json
    conforms(version, schema)
    assert.equal(schema.properties.choice.oneOf === undefined, version === "2025-06-18")
    assert.equal(schema.properties.choice.enum === undefined, version === "2025-11-25")
    await h.close()
  }
  // A version this server does not speak is answered with the latest one it does, and that is the version it encodes for.
  const older = await harness([decision(4, { state: "pending", question })], { form: {} }, () => ({ action: "accept", content: { choice: "blue" } }), [], 0, "2025-03-26")
  await next(older)
  assert.equal(older.initialized.result.protocolVersion, "2025-11-25")
  conforms("2025-11-25", older.lines[0]!.params.requestedSchema as Json)
  await older.close()
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

test("a queued question that was answered elsewhere while another one was open is not asked", async () => {
  const inbox = new Inbox([pending(4), pending(5, { id: "int-2" }, { message: "Second?" })])
  const h = await harness(inbox, { form: {} }, params => {
    if (params.message === "Which colour?") inbox.events.push(resolved(6, { id: "int-2" }))
    return { action: "accept", content: { choice: "red" } }
  })
  assert.equal(told(await next(h)).interaction_id, "int-1")
  assert.deepEqual(told(await next(h, 1)), { timed_out: true })
  assert.deepEqual(elicitedQuestions(h), ["Which colour?"])
  assert.equal(inbox.acknowledged, 6)
  await h.close()
})

test("a pending record and its resolution on a later page of the inbox are reconciled before asking", async () => {
  const filler = Array.from({ length: 49 }, (_, index) => [pending(3 + index * 2, { id: `old-${index}` }), resolved(4 + index * 2, { id: `old-${index}` })]).flat()
  const inbox = new Inbox([pending(1), pending(2, { id: "int-2" }, { message: "Second?" }), ...filler, resolved(101, { id: "int-2" })])
  const h = await harness(inbox, { form: {} }, () => ({ action: "accept", content: { choice: "red" } }))
  assert.equal(told(await next(h)).interaction_id, "int-1")
  assert.deepEqual(told(await next(h, 1)), { timed_out: true })
  assert.deepEqual(elicitedQuestions(h), ["Which colour?"])
  assert.equal(inbox.acknowledged, 101)
  await h.close()
})

test("a failed routing lookup keeps the question for the retry in the same process", async () => {
  const h = await harness([pending(4)], { form: {} }, () => ({ action: "accept", content: { choice: "red" } }), [], 1)
  const failed = await next(h)
  assert.equal(failed.result.isError, true)
  assert.deepEqual(told(failed), { error: "request_failed" })
  assert.equal(h.lines.length, 0)
  assert.deepEqual(acks(h), [])
  assert.deepEqual(told(await next(h)), { interaction_id: "int-1", action: "accept", choice_id: "red", answered: true })
  assert.deepEqual(acks(h), [4])
  await h.close()
})

test("overlapping calls show different questions while the first dialog stays open", async () => {
  const h = await harness([pending(4), pending(5, { id: "int-2" }, { message: "Second?" })], { form: {} }, () => undefined)
  const first = next(h)
  await until(() => h.lines.length === 1)
  const second = next(h)
  await until(() => h.lines.length === 2)
  assert.deepEqual(elicitedQuestions(h), ["Which colour?", "Second?"])
  h.respond(h.lines[1]!, { action: "accept", content: { choice: "blue" } })
  assert.deepEqual(told(await second), { interaction_id: "int-2", action: "accept", choice_id: "blue", answered: true })
  h.respond(h.lines[0]!, { action: "accept", content: { choice: "red" } })
  assert.deepEqual(told(await first), { interaction_id: "int-1", action: "accept", choice_id: "red", answered: true })
  assert.equal(h.inbox.acknowledged, 5)
  await h.close()
})

test("a question in an open dialog is not shown again to an overlapping call, and a failed answer releases it", async () => {
  const h = await harness([pending(4)], { form: {} }, () => undefined, ["request_failed"])
  const first = next(h)
  await until(() => h.lines.length === 1)
  assert.deepEqual(told(await next(h, 1)), { timed_out: true })
  assert.equal(h.lines.length, 1)
  h.respond(h.lines[0]!, { action: "accept", content: { choice: "red" } })
  assert.equal(told(await first).answered, false)
  const retry = next(h)
  await until(() => h.lines.length === 2)
  h.respond(h.lines[1]!, { action: "accept", content: { choice: "red" } })
  assert.deepEqual(told(await retry), { interaction_id: "int-1", action: "accept", choice_id: "red", answered: true })
  assert.equal(h.inbox.acknowledged, 4)
  await h.close()
})

const asked = { name: "chariox_next_choice", arguments: { timeout_s: 5 } }

// MCP 2025-11-25 client/elicitation#capabilities: `{}` is `{ form: {} }`, `{ url: {} }` alone does not support forms and a
// server MUST NOT send the request; for 2025-06-18 the capability is the empty object and means forms.
test("a form chooser is sent only to a client whose negotiated elicitation capability supports forms", async () => {
  const cases: [string, Json | null, boolean][] = [
    ["2025-11-25", {}, true], ["2025-11-25", { form: {} }, true], ["2025-11-25", { form: {}, url: {} }, true],
    ["2025-11-25", { url: {} }, false], ["2025-11-25", null, false],
    ["2025-06-18", {}, true], ["2025-06-18", { form: {} }, true], ["2025-06-18", null, false],
  ]
  for (const [version, capability, forms] of cases) {
    const h = await harness([pending(4)], capability, () => ({ action: "accept", content: { choice: "red" } }), [], 0, version)
    const reply = told(await next(h))
    assert.equal(h.lines.length, forms ? 1 : 0, `${version} ${JSON.stringify(capability)}`)
    if (forms) assert.equal(reply.answered, true)
    else {
      assert.equal(reply.question.message, "Which colour?", `${version} ${JSON.stringify(capability)}: the question reaches the model as text`)
      assert.equal(reply.question.choices.length, 2)
      assert.equal(typeof reply.how_to_answer, "string")
    }
    await h.close()
  }
})

// An error reply to `elicitation/create` (the spec's -32602 for a mode the client did not declare, or any other) means the
// person was not asked: it is not a dismissal.
test("an error answer to the form request serves the question as text and keeps it open", async () => {
  const refused = await harness([pending(4)], { form: {} }, () => ({ error: { code: -32602, message: "elicitation mode not supported" } }))
  const shown = told(await next(refused))
  assert.equal(shown.question.message, "Which colour?")
  assert.equal(shown.interaction_id, "int-1")
  assert.equal(typeof shown.how_to_answer, "string")
  assert.deepEqual(acks(refused), [])
  // The client said forms are unsupported: the next call goes straight to text and now the person was served.
  const again = told(await next(refused))
  assert.equal(again.question.message, "Which colour?")
  assert.equal(refused.lines.length, 1)
  assert.deepEqual(acks(refused), [4])
  await refused.close()
  let failing = true
  const transient = await harness([pending(4)], { form: {} }, () => failing ? { error: { code: -32603, message: "internal error" } } : { action: "accept", content: { choice: "red" } })
  assert.equal(told(await next(transient)).question.message, "Which colour?")
  assert.deepEqual(acks(transient), [])
  failing = false
  assert.deepEqual(told(await next(transient)), { interaction_id: "int-1", action: "accept", choice_id: "red", answered: true })
  assert.deepEqual(acks(transient), [4])
  await transient.close()
})

test("a transport failure writing the form request is not a dismissal either", async () => {
  const input = new PassThrough(), output = new PassThrough()
  const write = output.write.bind(output)
  output.write = ((chunk: unknown, ...rest: unknown[]) => {
    if (String(chunk).includes("elicitation/create")) throw new Error("broken pipe")
    return (write as (...args: unknown[]) => boolean)(chunk, ...rest)
  }) as never
  const events = [pending(4)]
  const acked: number[] = []
  const send = async (request: Json) => {
    if ("WaitEvents" in request || "ListEvents" in request) return { EventsListed: { events: events.filter(event => event.cursor > (((request.WaitEvents ?? request.ListEvents) as Json).since ?? 0)), next_cursor: 4, oldest_cursor: 1, timed_out: false } }
    if ("AckEvents" in request) acked.push((request.AckEvents as Json).cursor as number)
    return { ack: true }
  }
  const server = runEventsMcp(send as never, input, output, { isLocal: async () => true, sendTo: async () => ({}) })
  const lines: Json[] = []
  output.on("data", chunk => { for (const line of String(chunk).split("\n").filter(Boolean)) lines.push(JSON.parse(line)) })
  input.write(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-11-25", capabilities: { elicitation: { form: {} } } } }) + "\n")
  input.write(JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/call", params: asked }) + "\n")
  await until(() => lines.some(line => line.id === 2))
  const shown = JSON.parse(lines.find(line => line.id === 2)!.result.content[0].text) as Json
  assert.equal(shown.question.message, "Which colour?")
  assert.deepEqual(acked, [])
  input.end()
  await server
})

// MCP 2025-11-25 basic/utilities/cancellation: the receiver SHOULD stop, free resources and not answer the cancelled request.
test("a cancelled tools/call releases its question, closes its dialog and sends no answer", async () => {
  const h = await harness([pending(4)], { form: {} }, () => undefined)
  h.raw(JSON.stringify({ jsonrpc: "2.0", id: 40, method: "tools/call", params: asked }))
  await until(() => h.lines.length === 1)
  h.raw(JSON.stringify({ jsonrpc: "2.0", method: "notifications/cancelled", params: { requestId: 40, reason: "user" } }))
  await until(() => h.other.some(message => message.method === "notifications/cancelled"))
  assert.equal(h.other.find(message => message.method === "notifications/cancelled")!.params.requestId, h.lines[0]!.id)
  // A late answer to the closed dialog changes nothing; the question is still open and the next call asks it again.
  h.respond(h.lines[0]!, { action: "accept", content: { choice: "red" } })
  const retry = next(h)
  await until(() => h.lines.length === 2)
  h.respond(h.lines[1]!, { action: "accept", content: { choice: "blue" } })
  assert.deepEqual(told(await retry), { interaction_id: "int-1", action: "accept", choice_id: "blue", answered: true })
  assert.equal(h.other.some(message => message.id === 40), false, "no response to the cancelled call")
  assert.deepEqual(h.sent.filter(request => "RespondToInteraction" in request).map(request => (request.RespondToInteraction as Json).choice_id), ["blue"])
  await h.close()
})

test("a call cancelled while it waits shows nothing when a question arrives later", async () => {
  const inbox = new Inbox([])
  const h = await harness(inbox, { form: {} }, () => ({ action: "accept", content: { choice: "red" } }))
  h.raw(JSON.stringify({ jsonrpc: "2.0", id: 41, method: "tools/call", params: asked }))
  await new Promise(resolve => setTimeout(resolve, 50))
  h.raw(JSON.stringify({ jsonrpc: "2.0", method: "notifications/cancelled", params: { requestId: 41 } }))
  await new Promise(resolve => setTimeout(resolve, 50))
  inbox.events.push(pending(4))
  await new Promise(resolve => setTimeout(resolve, 300))
  assert.equal(h.lines.length, 0)
  assert.equal(h.other.some(message => message.id === 41), false)
  await h.close()
})

// MCP 2025-06-18 and 2025-11-25 basic/lifecycle#version-negotiation, basic/index (JSON-RPC), basic/transports#stdio.
test("initialize answers a version it supports with the same one and another with the latest", async () => {
  for (const [offered, answered] of [["2025-06-18", "2025-06-18"], ["2025-11-25", "2025-11-25"], ["2024-11-05", "2025-11-25"], ["2099-01-01", "2025-11-25"]]) {
    const h = await harness([], null, () => undefined, [], 0, offered)
    assert.equal(h.initialized.result.protocolVersion, answered)
    assert.deepEqual(h.initialized.result.capabilities, { tools: {} })
    await h.close()
  }
  const h = await harness([], null, () => undefined)
  const missing = await h.call("initialize", { capabilities: {} })
  assert.equal(missing.result.protocolVersion, "2025-11-25")
  await h.close()
})

test("malformed JSON-RPC gets the JSON-RPC error and a reply is never answered", async () => {
  const h = await harness([], null, () => undefined)
  h.raw("{not json")
  h.raw(JSON.stringify([{ jsonrpc: "2.0", id: 1, method: "ping" }]))
  h.raw(JSON.stringify({ jsonrpc: "2.0", id: null, method: "ping" }))
  h.raw(JSON.stringify({ jsonrpc: "2.0", id: 7, method: "tools/call", params: "x" }))
  h.raw(JSON.stringify({ jsonrpc: "2.0", id: "elicit-9", result: {} }))
  h.raw(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }))
  h.raw(JSON.stringify({ jsonrpc: "2.0", method: "no/such" }))
  await until(() => h.other.length >= 4)
  await new Promise(resolve => setTimeout(resolve, 50))
  assert.deepEqual(h.other.map(message => [message.id ?? null, message.error?.code]), [[null, -32700], [null, -32600], [null, -32600], [7, -32602]])
  await h.close()
})

test("a request is answered even when its id looks like one of ours, an unknown method is -32601 and ping is empty", async () => {
  const h = await harness([pending(4)], { form: {} }, () => undefined)
  void next(h)
  await until(() => h.lines.length === 1)
  const ours = h.lines[0]!.id as string
  h.raw(JSON.stringify({ jsonrpc: "2.0", id: ours, method: "ping" }))
  await until(() => h.other.some(message => message.id === ours))
  assert.deepEqual(h.other.find(message => message.id === ours)!.result, {})
  assert.equal((await h.call("resources/list", {})).error.code, -32601)
  assert.deepEqual((await h.call("ping", {})).result, {})
  await h.close()
})

// MCP 2025-11-25 server/tools#error-handling: a bad argument is a tool execution error (isError), an unknown tool is -32602.
test("a bad timeout_s is a tool execution error and an unknown tool a protocol error", async () => {
  const h = await harness([], { form: {} }, () => undefined)
  // server/tools#listing-tools: name, description and an object inputSchema; no nextCursor, so the list is complete.
  const tool = (await h.call("tools/list", {})).result
  assert.deepEqual([Object.keys(tool).sort(), tool.tools.length, tool.tools[0].inputSchema.type, typeof tool.tools[0].description], [["tools"], 5, "object", "string"])
  const bad = await h.call("tools/call", { name: "chariox_next_choice", arguments: { timeout_s: "soon" } })
  assert.equal(bad.result.isError, true)
  assert.equal(h.sent.length, 0)
  assert.equal((await h.call("tools/call", { name: "nope", arguments: {} })).error.code, -32602)
  await h.close()
})

// MCP elicitation: "Servers SHOULD validate received data matches the requested schema".
test("an accepted answer that is not one of the offered options is rejected and the question stays open", async () => {
  const answers: Json[] = [{ choice: "purple" }, { choice: "other" }, { choice: "other", reply: "x".repeat(21) }, { choice: "red" }]
  const h = await harness([pending(4)], { form: {} }, () => ({ action: "accept", content: answers.shift() }))
  for (const code of ["invalid_answer", "invalid_answer", "invalid_answer"]) {
    const reply = await next(h)
    assert.equal(reply.result.isError, true)
    assert.equal(told(reply).code, code)
  }
  assert.equal(h.sent.some(request => "RespondToInteraction" in request), false)
  assert.deepEqual(told(await next(h)), { interaction_id: "int-1", action: "accept", choice_id: "red", answered: true })
  await h.close()
})

async function until(done: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 200 && !done(); attempt++) await new Promise(resolve => setTimeout(resolve, 10))
  assert.ok(done(), "the condition did not become true")
}

const nextNotice = (h: Awaited<ReturnType<typeof harness>>, timeout_s = 5) => h.call("tools/call", { name: "chariox_next_notification", arguments: { timeout_s } })
const body = (reply: Json): Json => JSON.parse(reply.result.content[0].text)

test("the source tools list, follow by name and unfollow, and are announced", async () => {
  const h = await harness([], null, () => undefined)
  const tools = (await h.call("tools/list", {})).result.tools.map((tool: Json) => tool.name)
  assert.deepEqual(tools, ["chariox_next_choice", "chariox_next_notification", "chariox_notification_sources", "chariox_follow_notification_source", "chariox_unfollow_notification_source"])
  assert.deepEqual(body(await h.call("tools/call", { name: "chariox_notification_sources", arguments: {} })), { sources: SOURCES })
  await h.call("tools/call", { name: "chariox_follow_notification_source", arguments: { source: "review" } })
  await h.call("tools/call", { name: "chariox_unfollow_notification_source", arguments: { source: "wf_source_a" } })
  assert.deepEqual(h.sent.filter(request => "SubscribeEvents" in request || "UnsubscribeEvents" in request), [
    { SubscribeEvents: { targets: [], sources: ["wf_source_a"] } },
    { UnsubscribeEvents: { sources: ["wf_source_a"] } },
  ])
  const refused = await h.call("tools/call", { name: "chariox_follow_notification_source", arguments: { source: "nope" } })
  assert.equal(refused.result.isError, true)
  assert.equal(body(refused).error, "events_refused")
  assert.equal((await h.call("tools/call", { name: "chariox_follow_notification_source", arguments: {} })).result.isError, true)
  await h.close()
})
test("a notification is served once and then acknowledged; a gap is served like it", async () => {
  const h = await harness([notification(3), gap(4)], null, () => undefined)
  const first = body(await nextNotice(h))
  assert.deepEqual(first, { type: "notification", kernel: "k", session_id: "room", source_id: "wf_source_a", source_kind: "workflow_exit", display_name: "Review", occurrence_id: "run-3", message: "approved", artifacts: [], deadline_ms: 604800001, refs: {}, summary: "Review finished" })
  const second = body(await nextNotice(h))
  assert.deepEqual([second.type, second.dropped, second.from, second.to], ["gap", 1, 4, 4])
  assert.deepEqual(body(await nextNotice(h, 1)), { timed_out: true })
  assert.equal(h.inbox.acknowledged, 4)
  await h.close()
})
test("waiting for a decision never acknowledges past a notification nobody has read", async () => {
  const h = await harness([notification(1), pending(2)], null, () => undefined)
  // Without a form dialog the question comes back as text; the notification stays unread before it.
  const choice = body(await next(h))
  assert.equal(choice.interaction_id, "int-1")
  assert.equal(h.inbox.acknowledged, 0)
  assert.equal(body(await nextNotice(h)).occurrence_id, "run-1")
  assert.equal(h.inbox.acknowledged, 2)
  await h.close()
})
