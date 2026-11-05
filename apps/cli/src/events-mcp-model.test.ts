// MP-08 / MP-09 / MP-10 / MP-11: model-based test of `chariox_next_choice` under overlapping calls.
// A seeded generator drives 2 to 4 overlapping calls over 1 to 4 questions against an inbox that enforces the
// kernel's real acknowledgement floor (durable_state/agent_lifecycle/outside.rs `outside_list`: a cursor below the
// floor answers EventsCursorExpired), with random reply delays, dismissals, failed answers, answers given elsewhere
// and a second process acknowledging.
import assert from "node:assert/strict"
import test from "node:test"
import { PassThrough } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import { runEventsMcp } from "./events-mcp.js"

type Json = Record<string, any>

function random(seed: number): () => number {
  let state = seed >>> 0
  return () => {
    state = (state + 0x6d2b79f5) >>> 0
    let t = state
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

const record = (cursor: number, id: string, value: Json): Json => ({ cursor, kernel: "k", machine: "m", session_id: "room", at_ms: 1, summary: id, refs: { interaction_id: id }, type: "interaction", value })

// The kernel's durable inbox with its acknowledgement floor.
class FlooredInbox {
  events: Json[] = []
  ack = 0
  floor = 0
  latest = 0
  expiredReplies = 0
  lastSince = 0
  decreasedSince = false
  push(make: (cursor: number) => Json): void { this.events.push(make(++this.latest)) }
  list(since: number | undefined, limit: number): Json {
    const after = since ?? this.ack
    if (since !== undefined) { if (since < this.lastSince) this.decreasedSince = true; this.lastSince = Math.max(this.lastSince, since) }
    if (after < this.floor) { this.expiredReplies++; return { EventsCursorExpired: { oldest_cursor: this.floor + 1 } } }
    if (after > this.latest) throw new CommandFailure("refused", "cursor is ahead of the inbox")
    const events = this.events.filter(event => event.cursor > after).slice(0, limit)
    return { EventsListed: { events, next_cursor: events.at(-1)?.cursor ?? after, oldest_cursor: this.floor + 1, timed_out: false } }
  }
  acknowledge(cursor: number): void {
    if (cursor > this.latest) throw new CommandFailure("refused", "ack cursor is ahead of the inbox")
    this.ack = Math.max(this.ack, cursor)
    this.floor = Math.max(this.floor, this.ack)
    this.events = this.events.filter(event => event.cursor > this.ack)
  }
}

type Behaviour = { replyDelay(): number; dialogDelay(): number; dismiss(): boolean; failAnswer(): string | null }

// One MCP process: a stub harness over the same pipes a real one uses.
function process(inbox: FlooredInbox, behaviour: Behaviour, onDialog: (question: string, open: boolean) => void, onAnswer: (id: string, code: string | null) => string | null) {
  const input = new PassThrough(), output = new PassThrough()
  const replies = new Map<number, (message: Json) => void>()
  const results: Json[] = []
  output.on("data", chunk => {
    for (const line of String(chunk).split("\n").filter(Boolean)) {
      const message = JSON.parse(line) as Json
      if (message.method === "elicitation/create") {
        const question = message.params.message as string
        onDialog(question, true)
        void sleep(behaviour.dialogDelay()).then(() => {
          onDialog(question, false)
          input.write(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: behaviour.dismiss() ? { action: "decline" } : { action: "accept", content: { choice: "red" } } }) + "\n")
        })
        continue
      }
      replies.get(message.id)?.(message)
    }
  })
  // The kernel evaluates a request on arrival; its reply travels for a random time, so replies overtake each other.
  const send = async (request: Json): Promise<unknown> => {
    let response: unknown
    if ("ListEvents" in request) response = inbox.list(request.ListEvents.since, request.ListEvents.limit)
    else if ("WaitEvents" in request) {
      const query = request.WaitEvents as Json
      for (let waited = 0; ; waited += 5) {
        response = inbox.list(query.since, query.limit)
        const batch = (response as Json).EventsListed as Json | undefined
        if (!batch || batch.events.length > 0 || waited >= 25) break
        await sleep(5)
      }
    } else if ("AckEvents" in request) { inbox.acknowledge(request.AckEvents.cursor); response = { ack: true } }
    else if ("RespondToInteraction" in request) {
      const id = request.RespondToInteraction.interaction_id as string
      await sleep(behaviour.replyDelay())
      const code = onAnswer(id, behaviour.failAnswer())
      if (code) throw new CommandFailure(code, "answer failed")
      return { ack: true }
    } else response = { ack: true }
    await sleep(behaviour.replyDelay())
    return response
  }
  const routes = { isLocal: async () => true, sendTo: async () => ({ ack: true }) }
  const server = runEventsMcp(send as never, input, output, routes)
  let counter = 0
  const call = (method: string, params: Json) => new Promise<Json>(resolve => { const n = ++counter; replies.set(n, resolve); input.write(JSON.stringify({ jsonrpc: "2.0", id: n, method, params }) + "\n") })
  const next = async () => { const reply = await call("tools/call", { name: "chariox_next_choice", arguments: { timeout_s: 1 } }); const result = JSON.parse(reply.result.content[0].text) as Json; results.push(result); return result }
  return { call, next, results, close: async () => { input.end(); await server } }
}

async function scenario(seed: number, secondProcess: boolean): Promise<void> {
  const rand = random(seed)
  const between = (low: number, high: number) => low + Math.floor(rand() * (high - low + 1))
  const inbox = new FlooredInbox()
  const names = Array.from({ length: between(1, 4) }, (_, index) => `q${index + 1}`)
  const expired = new Set(names.filter(() => rand() < 0.15))
  const settled = new Set(expired) // answered, closed or expired
  const cursors = new Map<string, number>()
  const live = new Set<string>() // dialogs open right now, by question
  let activity = Date.now()
  let timers = 0
  const touch = () => { activity = Date.now() }
  const later = (ms: number, work: () => void) => { timers++; void sleep(ms).then(() => { timers--; work(); touch() }) }

  const pushQuestion = (name: string) => inbox.push(cursor => { cursors.set(name, cursor); return record(cursor, name, { state: "pending", question: { title: "T", message: name, choices: [{ id: "red", label: "Red" }], expires_at_ms: expired.has(name) ? 1 : undefined } }) })
  const pushResolved = (name: string) => inbox.push(cursor => record(cursor, name, { state: "resolved", resolution: { via: "terminal" } }))
  names.forEach((name, index) => { if (index === 0 || rand() < 0.5) pushQuestion(name); else later(between(1, 20), () => pushQuestion(name)) })
  // An answer given somewhere else.
  for (const name of names) if (rand() < 0.2) later(between(2, 30), () => { if (!settled.has(name) && cursors.has(name)) { settled.add(name); pushResolved(name) } })
  // A second process acknowledging everything settled up to the first record that is not.
  if (rand() < 0.4 && secondProcess) later(between(2, 30), () => {
    let cursor = 0
    for (const event of [...inbox.events].sort((a, b) => a.cursor - b.cursor)) { if (event.value.state === "pending" && !settled.has(event.refs.interaction_id)) break; cursor = event.cursor }
    if (cursor > inbox.ack) inbox.acknowledge(cursor)
  })

  const behaviour: Behaviour = {
    replyDelay: () => between(0, 4),
    dialogDelay: () => between(0, 12),
    dismiss: () => rand() < 0.25,
    failAnswer: () => { const roll = rand(); return roll < 0.2 ? "request_failed" : roll < 0.3 ? "interaction_closed" : null },
  }
  const process1 = process(inbox, behaviour, (question, open) => {
    if (open) {
      assert.ok(!live.has(question), `seed ${seed}: ${question} is shown twice at once`)
      live.add(question)
    } else live.delete(question)
    touch()
  }, (id, code) => {
    // The kernel refuses a second answer to a decision that is already over.
    const result = settled.has(id) ? "interaction_closed" : code
    if (result === null || result === "interaction_closed") { settled.add(id); if (!inbox.events.some(event => event.value.state === "resolved" && event.refs.interaction_id === id)) pushResolved(id) }
    touch()
    return result
  })

  await process1.call("initialize", { protocolVersion: "2025-06-18", capabilities: { elicitation: { form: {} } } })
  const calls = Array.from({ length: between(2, 4) }, () => between(0, 15))
  let started = 0
  const running = calls.map(async delay => {
    await sleep(delay)
    started++
    touch()
    return process1.next()
  })
  void (async () => {
    while (started < calls.length || timers > 0 || live.size > 0 || Date.now() - activity < 40) await sleep(5)
    inbox.push(cursor => ({ cursor, kernel: "k", machine: "m", session_id: "", at_ms: 1, summary: "ended", refs: {}, type: "subscription_ended", value: { reason: "done" } }))
  })()
  const outcomes = await Promise.all(running)
  await process1.close()

  assert.equal(inbox.decreasedSince, false, `seed ${seed}: the read cursor moved backward`)
  for (const outcome of outcomes) assert.ok(outcome.error === undefined, `seed ${seed}: a call failed with ${outcome.error}`)
  for (const outcome of outcomes) assert.ok(outcome.timed_out === undefined, `seed ${seed}: a call timed out with the inbox quiet`)

  // Every question nobody answered, closed or saw expire is still in the inbox and a new process asks it once.
  const unsettled = names.filter(name => !settled.has(name) && cursors.has(name))
  for (const name of unsettled) assert.ok(cursors.get(name)! > inbox.floor, `seed ${seed}: ${name} was acknowledged away unanswered`)
  const second = new FlooredInbox()
  second.latest = inbox.latest
  second.ack = second.floor = inbox.floor
  second.events = inbox.events.filter(event => event.type !== "subscription_ended")
  const asked: string[] = []
  const calm: Behaviour = { replyDelay: () => 0, dialogDelay: () => 0, dismiss: () => false, failAnswer: () => null }
  const fresh = process(second, calm, (question, open) => { if (open) asked.push(question) }, () => null)
  await fresh.call("initialize", { protocolVersion: "2025-06-18", capabilities: { elicitation: { form: {} } } })
  for (let index = 0; index < unsettled.length; index++) await fresh.next()
  await fresh.close()
  assert.deepEqual([...asked].sort(), [...unsettled].sort(), `seed ${seed}: a new process must ask exactly the unsettled questions`)
}

// Without the second process the only way a call can see an expired cursor is its own reads overtaking each other.
for (const secondProcess of [false, true]) {
  test(`model: overlapping calls over a floored inbox with delayed, out-of-order replies (200 seeds${secondProcess ? ", a second process acknowledging" : ""})`, async () => {
    const failures: string[] = []
    for (let seed = 1; seed <= 200; seed++) await scenario(seed, secondProcess).catch((error: Error) => { failures.push(error.message) })
    assert.ok(failures.length === 0, `${failures.length} of 200 seeds failed, first: ${failures[0]}`)
  })
}
