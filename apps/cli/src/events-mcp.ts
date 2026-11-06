// MP-08 / MP-09 / MP-10 / MP-11: an optional MCP tool that lets the harness draw a pending choice itself.
// `chariox_next_choice` waits for the next pending decision in the event inbox, asks the person through the
// client's own elicitation dialog and sends the answer; no model turn composes the question.
import { createInterface } from "node:readline"
import type { Readable, Writable } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"
import type { EventQuestion, EventsBatch, OutsideEvent } from "@chariox/kernel-client/kernel-types"
import { respondToInteractionRequest } from "./ipc-requests.js"
import { followSource, listSources, unfollowSource } from "./events-command.js"

const TOOL = "chariox_next_choice"
const NEXT_NOTIFICATION = "chariox_next_notification"
const LIST_SOURCES = "chariox_notification_sources"
const FOLLOW_SOURCE = "chariox_follow_notification_source"
const UNFOLLOW_SOURCE = "chariox_unfollow_notification_source"
const DEFAULT_WAIT_SECONDS = 540
const MAX_WAIT_SECONDS = 900
const REPLY = "reply"
const PAGE = 100
// An idle wait holds the inbox reader, so it lasts one short slice and another call's read or acknowledgement is never held for long.
const WAIT_SLICE_SECONDS = 5
// The MCP versions this server speaks; an initialize for any other is answered with the latest.
const VERSIONS = ["2025-06-18", "2025-11-25"]
const LATEST_VERSION = "2025-11-25"
// The first MCP version whose elicitation schema has titled single-select options (`oneOf` of `const`/`title`) and whose
// elicitation capability names its modes (`form`, `url`).
const TITLED_OPTIONS_SINCE = "2025-11-25"
// JSON-RPC 2.0 error codes.
const PARSE_ERROR = -32700
const INVALID_REQUEST = -32600
const METHOD_NOT_FOUND = -32601
const INVALID_PARAMS = -32602

type Json = Record<string, unknown>
type Interaction = Extract<OutsideEvent, { type: "interaction" }>
// What a followed source or the kernel's own drop of unread records leaves in the inbox for the model to read.
type Notice = Extract<OutsideEvent, { type: "notification" | "gap" }>
function notice(event: OutsideEvent): event is Notice { return event.type === "notification" || event.type === "gap" }

// Where an answer goes: the inbox is read and acknowledged on this computer's kernel, a decision is answered on the
// kernel that raised it.
export type KernelRoutes = {
  isLocal(kernel: string): Promise<boolean>
  sendTo(kernel: string, request: Json): Promise<unknown>
}

function pendingDecision(event: OutsideEvent): event is Interaction {
  return event.type === "interaction" && event.value.state === "pending"
}

// A notification or gap is read once by chariox_next_notification, so nothing past it leaves the inbox before then.
function noticeKey(event: Notice): string { return `notice:${event.cursor}` }

// The same decision on the same kernel and session, whichever record (pending or resolved) names it.
function decisionKey(event: Interaction): string {
  return JSON.stringify([event.kernel, event.session_id, event.refs.interaction_id ?? ""])
}

// The records read since the last acknowledgement. The kernel acknowledges by cursor, so only the settled prefix may go:
// a decision nobody has answered, closed or seen expire keeps itself and everything after it in the inbox.
class Ledger {
  private entries: { cursor: number; key: string | null; settled: boolean }[] = []
  add(event: OutsideEvent): void {
    const key = pendingDecision(event) ? decisionKey(event) : notice(event) ? noticeKey(event) : null
    this.entries.push({ cursor: event.cursor, key, settled: key === null })
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

// Whether the client declared form mode, as the negotiated version defines it. 2025-06-18: the capability is an empty
// object and means forms. 2025-11-25: `{ form }` means forms, `{}` is the older spelling of `{ form: {} }`, and `{ url }`
// alone does not (a server MUST NOT send a mode the client did not declare).
function supportsForms(capability: unknown, version: string): boolean {
  if (typeof capability !== "object" || capability === null || Array.isArray(capability)) return false
  return version < TITLED_OPTIONS_SINCE || "form" in capability || !("url" in capability)
}

function text(value: unknown, isError = false): Json {
  return { content: [{ type: "text", text: JSON.stringify(value) }], ...(isError ? { isError } : {}) }
}

// One `tools/call` in progress; `notifications/cancelled` marks it and closes its open dialog.
type Call = { cancelled: boolean; dialog?: string | undefined }
// What a dialog came back with: the client's result, its JSON-RPC error, or our own cancellation of it.
type Elicited = { result: Json } | { error: { code?: number; message?: string } } | { cancelled: true }

export async function runEventsMcp(send: SendRequest, input: Readable, output: Writable, routes: KernelRoutes): Promise<number> {
  const write = (message: Json): void => { output.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n") }
  const waiting = new Map<string, (answer: Elicited) => void>()
  const calls = new Map<string | number, Call>()
  let nextId = 1
  let elicitation = false
  let version: string | undefined
  const elicit = (params: Json, call: Call) => new Promise<Elicited>(resolve => {
    const id = `elicit-${nextId++}`
    waiting.set(id, answer => { call.dialog = undefined; resolve(answer) })
    call.dialog = id
    try { write({ id, method: "elicitation/create", params }) } catch (error) {
      waiting.delete(id)
      call.dialog = undefined
      resolve({ error: { message: error instanceof Error ? error.message : "write failed" } })
    }
  })
  // The client is told to drop a dialog nobody waits for any more (2025-11-25 basic/utilities/cancellation).
  function cancel(call: Call): void {
    call.cancelled = true
    const dialog = call.dialog
    if (dialog === undefined) return
    const close = waiting.get(dialog)
    waiting.delete(dialog)
    try { write({ method: "notifications/cancelled", params: { requestId: dialog, reason: "the tool call was cancelled" } }) } catch { /* the client is gone */ }
    close?.({ cancelled: true })
  }

  // The decisions this process has read and not yet handled; a restarted process reads them again from the inbox.
  const open = new Map<string, Interaction>()
  const notices = new Map<string, Notice>()
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
      if (notice(event)) notices.set(noticeKey(event), event)
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

  async function nextChoice(seconds: number, call: Call): Promise<Json> {
    const deadline = Date.now() + seconds * 1000
    while (Date.now() < deadline && !call.cancelled) {
      await drain()
      if (ended) return text({ ended: true })
      await acknowledge()
      if (call.cancelled) break
      const decision = await claimFirstOpen()
      if (decision) {
        try { return await present(decision, call) } finally { inFlight.delete(decisionKey(decision)) }
      }
      const wait = Math.max(1, Math.min(Math.ceil((deadline - Date.now()) / 1000), WAIT_SLICE_SECONDS))
      await read("WaitEvents", wait * 1000)
      if (ended) return text({ ended: true })
      await acknowledge()
    }
    return text(call.cancelled ? { cancelled: true } : { timed_out: true })
  }

  // The oldest notification or gap not yet served. It leaves the inbox once it is the model's, and the result is
  // written without another await in between, so a cancelled call never takes one.
  async function nextNotification(seconds: number, call: Call): Promise<Json> {
    const deadline = Date.now() + seconds * 1000
    while (Date.now() < deadline && !call.cancelled) {
      await drain()
      const first = notices.entries().next().value
      if (first && !call.cancelled) {
        const [key, event] = first
        notices.delete(key)
        ledger.settle(key)
        void acknowledge().catch(() => undefined)
        return text(event.type === "gap"
          ? { type: "gap", kernel: event.kernel, session_id: event.session_id, dropped: event.value.dropped, from: event.value.from, to: event.value.to, summary: event.summary }
          : { type: "notification", kernel: event.kernel, session_id: event.session_id, source_id: event.value.source_id, source_kind: event.value.source_kind, display_name: event.value.display_name,
              occurrence_id: event.value.occurrence_id, message: event.value.output.message, artifacts: event.value.output.artifacts ?? [], deadline_ms: event.value.deadline_ms, refs: event.refs, summary: event.summary })
      }
      if (ended) return text({ ended: true })
      const wait = Math.max(1, Math.min(Math.ceil((deadline - Date.now()) / 1000), WAIT_SLICE_SECONDS))
      await read("WaitEvents", wait * 1000)
    }
    return text(call.cancelled ? { cancelled: true } : { timed_out: true })
  }

  // A question the person has not seen in a dialog goes to the model as text, with what it needs to ask and answer it.
  function asText(decision: Interaction, local: boolean, id: string): Json {
    return text({
      interaction_id: id, ...(local ? {} : { kernel: decision.kernel }), session_id: decision.session_id, question: decision.value.question,
      how_to_answer: "Ask the person with your own question tool (choices are question.choices, a free reply is question.text_reply), then send RespondToInteraction with the chosen id and interaction_id. Do not choose for the person.",
    })
  }

  // The answer must be one of the options offered, and a typed reply must fit the field.
  function validAnswer(question: EventQuestion, content: Json): boolean {
    const choice = content.choice
    if (question.choices.some(option => option.id === choice)) return true
    const reply = question.text_reply
    if (!reply || reply.id !== choice) return false
    const typed = content[REPLY]
    return typeof typed === "string" && typed.length >= (reply.min_length ?? 0) && (!reply.max_length || typed.length <= reply.max_length)
  }

  // The record leaves the inbox only when the decision is answered, found closed, or served to the model as text.
  async function present(decision: Interaction, call: Call): Promise<Json> {
    if (call.cancelled) return text({ cancelled: true })
    const key = decisionKey(decision)
    const id = decision.refs.interaction_id ?? ""
    const { question, pointer } = decision.value
    if (!question) { await settle(key); return text({ interaction_id: id, answerable: false, show_verbatim: pointer?.one_line ?? decision.summary }) }
    const local = await routes.isLocal(decision.kernel)
    if (!elicitation) { await settle(key); return asText(decision, local, id) }
    if (call.cancelled) return text({ cancelled: true })
    const elicited = await elicit({ message: question.message, requestedSchema: schema(question, version) }, call)
    if ("cancelled" in elicited) return text({ cancelled: true })
    // An error is the client refusing the request, not the person dismissing a dialog: the person has not seen the question.
    // It reaches the model as text and stays open for the next call. -32602 means the client does not take forms at all.
    if ("error" in elicited) {
      if (elicited.error.code === INVALID_PARAMS) elicitation = false
      return asText(decision, local, id)
    }
    const answer = elicited.result
    const content = (answer.content ?? {}) as Json
    // A dialog the person dismissed is not asked again by this process; the decision stays open in the inbox.
    if (answer.action !== "accept") { open.delete(key); return text({ interaction_id: id, action: answer.action ?? "cancel" }) }
    if (!validAnswer(question, content)) return text({ interaction_id: id, action: "accept", answered: false, code: "invalid_answer" }, true)
    const reply = question.text_reply?.id === content.choice ? content[REPLY] as string : null
    const request = respondToInteractionRequest(decision.session_id, id, content.choice as string, reply)
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

  const fail = (id: unknown, code: number, message: string): void => write({ id: id ?? null, error: { code, message } })
  const requestId = (id: unknown): id is string | number => typeof id === "string" || (typeof id === "number" && Number.isFinite(id))

  async function handle(message: unknown): Promise<void> {
    if (typeof message !== "object" || message === null || Array.isArray(message)) return fail(null, INVALID_REQUEST, "a JSON-RPC message is one object")
    const { id, method, params } = message as { id?: unknown; method?: unknown; params?: unknown }
    // A message without a method is a response: ours to read only if it answers a dialog, never answered itself.
    if (method === undefined) {
      if (typeof id === "string" && ("result" in message || "error" in message)) {
        const answer = "error" in message ? { error: (message.error ?? {}) as { code?: number; message?: string } } : { result: (message.result ?? {}) as Json }
        const done = waiting.get(id)
        waiting.delete(id)
        done?.(answer)
        return
      }
      return fail(requestId(id) ? id : null, INVALID_REQUEST, "a JSON-RPC message has a method or is a response")
    }
    if (typeof method !== "string" || (id !== undefined && !requestId(id))) return fail(requestId(id) ? id : null, INVALID_REQUEST, "invalid request: method is a string and id a string or number")
    if (id === undefined) {
      // A notification is never answered. Cancelling a call that is not running (or already answered) is ignored.
      if (method === "notifications/cancelled") {
        const target = (params as { requestId?: unknown } | undefined)?.requestId
        const call = typeof target === "string" || typeof target === "number" ? calls.get(target) : undefined
        if (call) cancel(call)
      }
      return
    }
    if (params !== undefined && (typeof params !== "object" || params === null || Array.isArray(params))) return fail(id, INVALID_PARAMS, "params is an object")
    const body = (params ?? {}) as Json
    if (method === "initialize") {
      const capabilities = body.capabilities
      const asked = typeof body.protocolVersion === "string" ? body.protocolVersion : undefined
      // The same version when this server speaks it, otherwise the latest it does speak.
      version = asked !== undefined && VERSIONS.includes(asked) ? asked : LATEST_VERSION
      elicitation = supportsForms((capabilities as Json | undefined)?.elicitation, version)
      return write({ id, result: { protocolVersion: version, capabilities: { tools: {} }, serverInfo: { name: "chariox-events", version: "1" } } })
    }
    if (method === "tools/list") {
      return write({ id, result: { tools: [{
        name: TOOL,
        description: "Wait for the next pending Chariox decision a person may answer and show it with this client's own chooser; sends the person's answer to the kernel. Acknowledges a decision only once it is answered or closed. Needs `chariox events subscribe` first.",
        inputSchema: { type: "object", properties: { timeout_s: { type: "number", description: `Seconds to wait (default ${DEFAULT_WAIT_SECONDS}, at most ${MAX_WAIT_SECONDS})` } } },
      }, {
        name: NEXT_NOTIFICATION,
        description: "Wait for the next notification of a source you follow (a workflow finished, or an agent needs the person's action) or a gap marker saying unread records were dropped. Returns it once and acknowledges it. Follow a source first with chariox_follow_notification_source.",
        inputSchema: { type: "object", properties: { timeout_s: { type: "number", description: `Seconds to wait (default ${DEFAULT_WAIT_SECONDS}, at most ${MAX_WAIT_SECONDS})` } } },
      }, {
        name: LIST_SOURCES,
        description: "List the notification sources this grant can follow on this kernel: workflow exits and \"Agent needs your action\".",
        inputSchema: { type: "object", properties: {} },
      }, {
        name: FOLLOW_SOURCE,
        description: "Follow one notification source by source_id or display name; each occurrence is then returned once by chariox_next_notification. Following does not poll.",
        inputSchema: { type: "object", properties: { source: { type: "string", description: "source_id or display name from chariox_notification_sources" } }, required: ["source"] },
      }, {
        name: UNFOLLOW_SOURCE,
        description: "Stop following one notification source by source_id or display name, or all of them with \"all\".",
        inputSchema: { type: "object", properties: { source: { type: "string", description: "source_id, display name, or all" } }, required: ["source"] },
      }] } })
    }
    if (method === "tools/call") {
      if (![TOOL, NEXT_NOTIFICATION, LIST_SOURCES, FOLLOW_SOURCE, UNFOLLOW_SOURCE].includes(body.name as string)) return fail(id, INVALID_PARAMS, `unknown tool ${String(body.name)}`)
      const args = body.arguments
      if (args !== undefined && (typeof args !== "object" || args === null || Array.isArray(args))) return fail(id, INVALID_PARAMS, "arguments is an object")
      if (body.name === LIST_SOURCES || body.name === FOLLOW_SOURCE || body.name === UNFOLLOW_SOURCE) {
        const source = (args as { source?: unknown } | undefined)?.source
        if (body.name !== LIST_SOURCES && (typeof source !== "string" || source === "")) return write({ id, result: text({ error: "invalid_source", message: "source is a source_id, a display name or all" }, true) })
        try {
          const result = body.name === LIST_SOURCES ? { sources: await exclusive(() => listSources(send)) }
            : await exclusive(() => (body.name === FOLLOW_SOURCE ? followSource : unfollowSource)(send, source as string))
          write({ id, result: text(result) })
        } catch (error) {
          write({ id, result: text({ error: error instanceof CommandFailure ? error.code : "request_failed", message: error instanceof CommandFailure ? error.message : undefined }, true) })
        }
        return
      }
      const wanted = (args as { timeout_s?: unknown } | undefined)?.timeout_s
      if (wanted !== undefined && (typeof wanted !== "number" || !Number.isFinite(wanted))) return write({ id, result: text({ error: "invalid_timeout_s", message: "timeout_s is a number of seconds" }, true) })
      const seconds = Math.min(Math.max(wanted ?? DEFAULT_WAIT_SECONDS, 1), MAX_WAIT_SECONDS)
      const call: Call = { cancelled: false }
      calls.set(id, call)
      try {
        const result = await (body.name === NEXT_NOTIFICATION ? nextNotification : nextChoice)(seconds, call)
        if (!call.cancelled) write({ id, result })
      } catch (error) {
        if (!call.cancelled) write({ id, result: text({ error: error instanceof CommandFailure ? error.code : "request_failed" }, true) })
      } finally {
        calls.delete(id)
      }
      return
    }
    if (method === "ping") return write({ id, result: {} })
    fail(id, METHOD_NOT_FOUND, `unknown method ${method}`)
  }

  await new Promise<void>(done => {
    createInterface({ input }).on("line", line => {
      if (line.trim() === "") return
      let message: unknown
      try { message = JSON.parse(line) } catch { return fail(null, PARSE_ERROR, "the line is not JSON") }
      void handle(message)
    }).on("close", done)
  })
  return 0
}
