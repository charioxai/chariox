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
const PAGE = 100
// An idle wait holds the inbox reader, so it lasts one short slice and another call's read or acknowledgement is never held for long.
const WAIT_SLICE_SECONDS = 5
// The first MCP version whose elicitation schema has titled single-select options (`oneOf` of `const`/`title`).
const TITLED_OPTIONS_SINCE = "2025-11-25"

type Json = Record<string, unknown>
type Interaction = Extract<OutsideEvent, { type: "interaction" }>

// Where an answer goes: the inbox is read and acknowledged on this computer's kernel, a decision is answered on the
// kernel that raised it.
export type KernelRoutes = {
  isLocal(kernel: string): Promise<boolean>
  sendTo(kernel: string, request: Json): Promise<unknown>
}

function pendingDecision(event: OutsideEvent): event is Interaction {
  return event.type === "interaction" && event.value.state === "pending"
}

// The same decision on the same kernel and session, whichever record (pending or resolved) names it.
function decisionKey(event: Interaction): string {
  return JSON.stringify([event.kernel, event.session_id, event.refs.interaction_id ?? ""])
}

// The records read since the last acknowledgement. The kernel acknowledges by cursor, so only the settled prefix may go:
// a decision nobody has answered, closed or seen expire keeps itself and everything after it in the inbox.
class Ledger {
  private entries: { cursor: number; key: string | null; settled: boolean }[] = []
  add(event: OutsideEvent): void {
    this.entries.push({ cursor: event.cursor, key: pendingDecision(event) ? decisionKey(event) : null, settled: !pendingDecision(event) })
  }
  settle(key: string): void { for (const entry of this.entries) if (entry.key === key) entry.settled = true }
  ackable(): number | null {
    let cursor: number | null = null
    for (const entry of this.entries) { if (!entry.settled) break; cursor = entry.cursor }
    return cursor
  }
  acknowledged(cursor: number): void { this.entries = this.entries.filter(entry => entry.cursor > cursor) }
}

// Labeled options as the negotiated MCP version defines them: 2025-06-18 `EnumSchema` is `enum` + `enumNames`,
// 2025-11-25 `TitledSingleSelectEnumSchema` is `oneOf` of `const`/`title`. ISO dates compare as strings.
function schema(question: EventQuestion, version: string | undefined): Json {
  const options = question.choices.map(choice => ({ id: choice.id, label: choice.label }))
  if (question.text_reply) options.push({ id: question.text_reply.id, label: question.text_reply.label })
  const choices = version !== undefined && version >= TITLED_OPTIONS_SINCE
    ? { oneOf: options.map(option => ({ const: option.id, title: option.label })) }
    : { enum: options.map(option => option.id), enumNames: options.map(option => option.label) }
  return {
    type: "object",
    properties: {
      choice: { type: "string", title: question.title ?? "Choice", ...choices },
      ...(question.text_reply ? { [REPLY]: { type: "string", title: question.text_reply.label, minLength: question.text_reply.min_length, ...(question.text_reply.max_length ? { maxLength: question.text_reply.max_length } : {}) } } : {}),
    },
    required: ["choice"],
  }
}

function text(value: unknown, isError = false): Json {
  return { content: [{ type: "text", text: JSON.stringify(value) }], ...(isError ? { isError } : {}) }
}

export async function runEventsMcp(send: SendRequest, input: Readable, output: Writable, routes: KernelRoutes): Promise<number> {
  const write = (message: Json): void => { output.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n") }
  const waiting = new Map<string, (result: Json) => void>()
  let nextId = 1
  let elicitation = false
  let version: string | undefined
  const elicit = (params: Json) => new Promise<Json>(resolve => {
    const id = `elicit-${nextId++}`
    waiting.set(id, resolve)
    write({ id, method: "elicitation/create", params })
  })

  // The decisions this process has read and not yet handled; a restarted process reads them again from the inbox.
  const open = new Map<string, Interaction>()
  // Decisions a call is asking right now: overlapping calls skip them, and the call frees one when it ends, whatever the outcome.
  const inFlight = new Set<string>()
  const ledger = new Ledger()
  let ingested = 0
  let seen: number | null = null
  // The inbox is read once for every call, so the end of the subscription is state, not something one batch carried.
  let ended = false

  // The only way to the inbox: one request in flight, so replies cannot arrive out of order. Request bodies are built
  // inside the lock from the current cursor, never before waiting for it.
  let inboxQueue: Promise<unknown> = Promise.resolve()
  function exclusive<T>(work: () => Promise<T>): Promise<T> {
    const run = inboxQueue.then(work, work)
    inboxQueue = run.catch(() => undefined)
    return run
  }

  function acknowledge(): Promise<void> {
    return exclusive(async () => {
      const cursor = ledger.ackable()
      if (cursor === null) return
      await send({ AckEvents: { cursor } })
      ledger.acknowledged(cursor)
    })
  }

  // A kept record is only bookkeeping once the person was served: the next call acknowledges it again.
  async function settle(key: string): Promise<void> {
    ledger.settle(key)
    open.delete(key)
    await acknowledge().catch(() => undefined)
  }

  function ingest(events: readonly OutsideEvent[]): void {
    for (const event of events) {
      if (event.cursor <= ingested) continue
      ingested = event.cursor
      ledger.add(event)
      if (event.type === "subscription_ended") ended = true
      if (event.type !== "interaction") continue
      const key = decisionKey(event)
      if (pendingDecision(event)) open.set(key, event)
      else { ledger.settle(key); open.delete(key) }
    }
  }

  const expired = (decision: Interaction): boolean => (decision.value.question?.expires_at_ms ?? Infinity) <= Date.now()

  // The first decision still worth asking that no other call is asking, reserved for the caller. It stays in `open`
  // until a definitive outcome, so a failure before the person is served leaves it to the next call.
  async function claimFirstOpen(): Promise<Interaction | null> {
    for (const [key, decision] of open) {
      if (inFlight.has(key)) continue
      if (expired(decision)) { await settle(key); continue }
      inFlight.add(key)
      return decision
    }
    return null
  }

  // `seen` only moves forward. A cursor below the acknowledgement floor (another process acknowledged past it) is
  // recovered once by reading again from the floor; a second refusal fails loudly.
  function read(kind: "ListEvents" | "WaitEvents", timeoutMs?: number): Promise<EventsBatch> {
    return exclusive(async () => {
      for (let attempt = 0; ; attempt++) {
        const body = { ...(seen === null ? {} : { since: seen }), limit: PAGE, ...(timeoutMs === undefined ? {} : { timeout_ms: timeoutMs }) }
        const response = await send({ [kind]: body }) as { EventsListed?: EventsBatch; EventsCursorExpired?: { oldest_cursor: number } }
        if (response.EventsListed) {
          seen = Math.max(seen ?? 0, response.EventsListed.next_cursor)
          ingest(response.EventsListed.events)
          return response.EventsListed
        }
        if (!response.EventsCursorExpired || attempt > 0) throw new CommandFailure("cursor_expired", "event cursor expired; subscribe again")
        seen = Math.max(seen ?? 0, response.EventsCursorExpired.oldest_cursor - 1)
      }
    })
  }

  // Every record already in the inbox, page after page, so a decision answered elsewhere is closed before it is asked.
  async function drain(): Promise<void> {
    for (;;) if ((await read("ListEvents")).events.length < PAGE) return
  }

  async function nextChoice(seconds: number): Promise<Json> {
    const deadline = Date.now() + seconds * 1000
    while (Date.now() < deadline) {
      await drain()
      if (ended) return text({ ended: true })
      await acknowledge()
      const decision = await claimFirstOpen()
      if (decision) {
        try { return await present(decision) } finally { inFlight.delete(decisionKey(decision)) }
      }
      const wait = Math.max(1, Math.min(Math.ceil((deadline - Date.now()) / 1000), WAIT_SLICE_SECONDS))
      await read("WaitEvents", wait * 1000)
      if (ended) return text({ ended: true })
      await acknowledge()
    }
    return text({ timed_out: true })
  }

  // The record leaves the inbox only when the decision is answered, found closed, or served to the model as text.
  async function present(decision: Interaction): Promise<Json> {
    const key = decisionKey(decision)
    const id = decision.refs.interaction_id ?? ""
    const { question, pointer } = decision.value
    if (!question) { await settle(key); return text({ interaction_id: id, answerable: false, show_verbatim: pointer?.one_line ?? decision.summary }) }
    const local = await routes.isLocal(decision.kernel)
    if (!elicitation) { await settle(key); return text({ interaction_id: id, ...(local ? {} : { kernel: decision.kernel }), session_id: decision.session_id, question }) }
    const answer = await elicit({ message: question.message, requestedSchema: schema(question, version) })
    const content = (answer.content ?? {}) as Json
    // A dialog the person dismissed is not asked again by this process; the decision stays open in the inbox.
    if (answer.action !== "accept" || typeof content.choice !== "string") { open.delete(key); return text({ interaction_id: id, action: answer.action ?? "cancel" }) }
    const reply = question.text_reply?.id === content.choice && typeof content[REPLY] === "string" ? content[REPLY] : null
    const request = respondToInteractionRequest(decision.session_id, id, content.choice, reply)
    try {
      await (local ? send(request) : routes.sendTo(decision.kernel, request))
    } catch (error) {
      const code = error instanceof CommandFailure ? error.code : (error as { code?: string }).code
      if (code === "interaction_closed") await settle(key)
      return text({ interaction_id: id, action: "accept", choice_id: content.choice, answered: false, code: code ?? "request_failed" }, code !== "interaction_closed")
    }
    await settle(key)
    return text({ interaction_id: id, action: "accept", choice_id: content.choice, answered: true })
  }

  async function handle(message: Json): Promise<void> {
    const { id, method } = message as { id?: string | number; method?: string }
    if (id !== undefined && typeof id === "string" && waiting.has(id)) { waiting.get(id)!((message.result ?? message.error ?? {}) as Json); waiting.delete(id); return }
    if (method === "initialize") {
      const params = (message.params ?? {}) as { protocolVersion?: string; capabilities?: Json }
      elicitation = Boolean(params.capabilities?.elicitation)
      version = params.protocolVersion
      return write({ id, result: { protocolVersion: params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: "chariox-events", version: "1" } } })
    }
    if (method === "tools/list") {
      return write({ id, result: { tools: [{
        name: TOOL,
        description: "Wait for the next pending Chariox decision a person may answer and show it with this client's own chooser; sends the person's answer to the kernel. Acknowledges a decision only once it is answered or closed. Needs `chariox events subscribe` first.",
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
