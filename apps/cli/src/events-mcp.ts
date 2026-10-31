// MP-08 / MP-09 / MP-10 / MP-11: an optional MCP tool that lets the harness draw a pending choice itself.
// `chariox_next_choice` waits for the next pending decision in the event inbox, asks the person through the
// client's own elicitation dialog and sends the answer; no model turn composes the question.
import { createInterface } from "node:readline"
import type { Readable, Writable } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"
import type { EventQuestion, EventsBatch, OutsideEvent } from "@chariox/kernel-client/kernel-types"
import { respondToInteractionRequest } from "./ipc-requests.js"

const TOOL = "chariox_next_choice"
const DEFAULT_WAIT_SECONDS = 540
const MAX_WAIT_SECONDS = 900
const REPLY = "reply"

type Json = Record<string, unknown>
type Interaction = Extract<OutsideEvent, { type: "interaction" }>

function pendingDecision(event: OutsideEvent): event is Interaction {
  return event.type === "interaction" && event.value.state === "pending"
}

function schema(question: EventQuestion): Json {
  const options = question.choices.map(choice => ({ const: choice.id, title: choice.label }))
  if (question.text_reply) options.push({ const: question.text_reply.id, title: question.text_reply.label })
  return {
    type: "object",
    properties: {
      choice: { type: "string", title: question.title ?? "Choice", oneOf: options },
      ...(question.text_reply ? { [REPLY]: { type: "string", title: question.text_reply.label, minLength: question.text_reply.min_length, ...(question.text_reply.max_length ? { maxLength: question.text_reply.max_length } : {}) } } : {}),
    },
    required: ["choice"],
  }
}

function text(value: unknown, isError = false): Json {
  return { content: [{ type: "text", text: JSON.stringify(value) }], ...(isError ? { isError } : {}) }
}

export async function runEventsMcp(send: SendRequest, input: Readable, output: Writable): Promise<number> {
  const write = (message: Json): void => { output.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n") }
  const waiting = new Map<string, (result: Json) => void>()
  let nextId = 1
  let elicitation = false
  const elicit = (params: Json) => new Promise<Json>(resolve => {
    const id = `elicit-${nextId++}`
    waiting.set(id, resolve)
    write({ id, method: "elicitation/create", params })
  })

  async function nextChoice(seconds: number): Promise<Json> {
    const deadline = Date.now() + seconds * 1000
    while (Date.now() < deadline) {
      const wait = Math.max(1, Math.min(Math.ceil((deadline - Date.now()) / 1000), 60))
      const response = await send({ WaitEvents: { limit: 100, timeout_ms: wait * 1000 } }) as { EventsListed?: EventsBatch; EventsCursorExpired?: unknown }
      if (!response.EventsListed) throw new CommandFailure("cursor_expired", "event cursor expired; subscribe again")
      const batch = response.EventsListed
      if (batch.events.some(event => event.type === "subscription_ended")) return text({ ended: true })
      const decision = batch.events.find(pendingDecision)
      if (!decision) { if (batch.events.length > 0) await send({ AckEvents: { cursor: batch.next_cursor } }); continue }
      await send({ AckEvents: { cursor: decision.cursor } })
      return await present(decision)
    }
    return text({ timed_out: true })
  }

  async function present(decision: Interaction): Promise<Json> {
    const id = decision.refs.interaction_id ?? ""
    const { question, pointer } = decision.value
    if (!question) return text({ interaction_id: id, answerable: false, show_verbatim: pointer?.one_line ?? decision.summary })
    if (!elicitation) return text({ interaction_id: id, session_id: decision.session_id, question })
    const answer = await elicit({ message: question.message, requestedSchema: schema(question) })
    const content = (answer.content ?? {}) as Json
    if (answer.action !== "accept" || typeof content.choice !== "string") return text({ interaction_id: id, action: answer.action ?? "cancel" })
    const reply = question.text_reply?.id === content.choice && typeof content[REPLY] === "string" ? content[REPLY] : null
    try {
      await send(respondToInteractionRequest(decision.session_id, id, content.choice, reply))
    } catch (error) {
      const code = error instanceof CommandFailure ? error.code : (error as { code?: string }).code
      return text({ interaction_id: id, action: "accept", choice_id: content.choice, answered: false, code: code ?? "request_failed" }, code !== "interaction_closed")
    }
    return text({ interaction_id: id, action: "accept", choice_id: content.choice, answered: true })
  }

  async function handle(message: Json): Promise<void> {
    const { id, method } = message as { id?: string | number; method?: string }
    if (id !== undefined && typeof id === "string" && waiting.has(id)) { waiting.get(id)!((message.result ?? message.error ?? {}) as Json); waiting.delete(id); return }
    if (method === "initialize") {
      const params = (message.params ?? {}) as { protocolVersion?: string; capabilities?: Json }
      elicitation = Boolean(params.capabilities?.elicitation)
      return write({ id, result: { protocolVersion: params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: "chariox-events", version: "1" } } })
    }
    if (method === "tools/list") {
      return write({ id, result: { tools: [{
        name: TOOL,
        description: "Wait for the next pending Chariox decision a person may answer and show it with this client's own chooser; sends the person's answer to the kernel. Also acknowledges the records before it. Needs `chariox events subscribe` first.",
        inputSchema: { type: "object", properties: { timeout_s: { type: "number", description: `Seconds to wait (default ${DEFAULT_WAIT_SECONDS}, at most ${MAX_WAIT_SECONDS})` } } },
      }] } })
    }
    if (method === "tools/call") {
      const params = (message.params ?? {}) as { name?: string; arguments?: { timeout_s?: number } }
      if (params.name !== TOOL) return write({ id, error: { code: -32602, message: `unknown tool ${params.name}` } })
      const seconds = Math.min(Math.max(Number(params.arguments?.timeout_s ?? DEFAULT_WAIT_SECONDS) || DEFAULT_WAIT_SECONDS, 1), MAX_WAIT_SECONDS)
      try {
        return write({ id, result: await nextChoice(seconds) })
      } catch (error) {
        return write({ id, result: text({ error: error instanceof CommandFailure ? error.code : "request_failed" }, true) })
      }
    }
    if (method === "ping") return write({ id, result: {} })
    if (id !== undefined) write({ id, error: { code: -32601, message: `unknown method ${method}` } })
  }

  await new Promise<void>(done => {
    createInterface({ input }).on("line", line => { try { void handle(JSON.parse(line) as Json) } catch { /* not a JSON-RPC line */ } }).on("close", done)
  })
  return 0
}
