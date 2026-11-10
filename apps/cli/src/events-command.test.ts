// MP-08 / MP-09 / MP-10 / MP-11: short event commands for outside agents.
import assert from "node:assert/strict"
import test from "node:test"
import { parseArguments, exitCodeFor, guideText } from "./agent-command-table.js"

test("outside event commands parse session and single-agent targets", () => {
  const parsed = parseArguments("events subscribe", ["--kernel", "b3/lane", "--session", "room", "--agent", "agent"])
  assert.equal(parsed.flags.get("--kernel"), "b3/lane")
  assert.equal(parsed.flags.get("--session"), "room")
  assert.equal(parsed.flags.get("--agent"), "agent")
  assert.equal(parseArguments("events unsubscribe", ["--all"]).flags.has("--all"), true)
  assert.equal(parseArguments("events wait", ["--since", "12", "--timeout", "540"]).flags.get("--since"), "12")
  assert.deepEqual(parseArguments("events ack", ["12"]).positionals, ["12"])
})
test("outside event cursor expiry and ending have actionable exit codes", () => {
  assert.equal(exitCodeFor("cursor_expired"), 7)
  assert.equal(exitCodeFor("subscription_ended"), 3)
})

import { EVENT_VERBS, runEventsVerb } from "./events-command.js"
import { CommandFailure } from "./agent-command-table.js"

test("wait drains a batch, preserving the cursor and timeout outcome", async () => {
  const sent: unknown[] = [], output: unknown[] = []
  const code = await runEventsVerb("wait", new Map([["--since", "12"], ["--timeout", "540"]]), [], async request => {
    sent.push(request)
    return { EventsListed: { events: [], next_cursor: 12, oldest_cursor: 1, timed_out: true } }
  }, value => output.push(value))
  assert.equal(code, 5)
  assert.deepEqual(sent, [{ WaitEvents: { since: 12, timeout_ms: 540000, limit: 1000 } }])
  assert.deepEqual(output, [{ events: [], next_cursor: 12, oldest_cursor: 1, timed_out: true }])
})
test("wait --timeout 0 is the one-shot read and cursor expiry preserves oldest_cursor", async () => {
  const sent: unknown[] = []
  await assert.rejects(runEventsVerb("wait", new Map([["--timeout", "0"]]), [], async request => { sent.push(request); return { EventsCursorExpired: { oldest_cursor: 42 } } }, () => {}), error => error instanceof CommandFailure && error.code === "cursor_expired" && error.details?.oldest_cursor === 42)
  assert.deepEqual(sent, [{ WaitEvents: { limit: 1000, timeout_ms: 0 } }])
})
test("events list is gone: reading what is pending is wait --timeout 0", () => {
  assert.equal((EVENT_VERBS as readonly string[]).includes("list"), false)
  assert.throws(() => parseArguments("events list", []), /unknown command events list/)
  assert.match(guideText(), /events wait --timeout 0/)
  assert.doesNotMatch(guideText(), /events list/)
})
test("watch emits records once and carries next_cursor into the next wait", async () => {
  const sent: unknown[] = [], output: unknown[] = []
  const event = { cursor: 2, type: "agent", value: { run_state: "idle" } }
  const end = { cursor: 3, type: "subscription_ended", value: { reason: "revoked" } }
  const code = await runEventsVerb("watch", new Map(), [], async request => {
    sent.push(request)
    return { EventsListed: { events: [sent.length === 1 ? event : end], next_cursor: sent.length + 1, oldest_cursor: 1, timed_out: false } }
  }, value => output.push(value))
  assert.equal(code, 3)
  assert.deepEqual(output, [event, end])
  assert.deepEqual(sent, [{ WaitEvents: { timeout_ms: 540000, limit: 1000 } }, { WaitEvents: { since: 2, timeout_ms: 540000, limit: 1000 } }])
})
test("unsubscribe rejects mixed targets and --all before sending", async () => {
  await assert.rejects(runEventsVerb("unsubscribe", new Map<string,string|true>([["--all", true], ["--kernel", "b3"]]), [], async () => assert.fail("must not send"), () => {}), /targets or --all/)
})
