// MP-08 / MP-09 / MP-10 / MP-11: short event commands for outside agents.
import assert from "node:assert/strict"
import test from "node:test"
import { parseArguments, exitCodeFor } from "./agent-command-table.js"

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

import { runEventsVerb } from "./events-command.js"
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
test("cursor expiry preserves oldest_cursor for resynchronization", async () => {
  await assert.rejects(runEventsVerb("list", new Map(), [], async () => ({ EventsCursorExpired: { oldest_cursor: 42 } }), () => {}), error => error instanceof CommandFailure && error.code === "cursor_expired" && error.details?.oldest_cursor === 42)
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

const SOURCES = [
  { kind: "workflow_exit", source_id: "wf_source_a", kernel_id: "k", session_id: "room", workflow_id: "w", exit_node_id: "exit-1", exit_ordinal: 0, name: "Review", display_name: "Review", available: true },
  { kind: "owner_action", source_id: "owner_action_b", kernel_id: "k", session_id: "room", workflow_id: "owner_action", exit_node_id: "blocked", exit_ordinal: 0, name: "Agent needs your action", display_name: "Agent needs your action", available: true },
]
const SUBSCRIPTION = { EventSubscription: { subscription: { subscription_id: "s", targets: [], sources: [], kernels: [], acknowledged_cursor: 0, ended: null } } }

test("sources lists what this kernel lets the grant follow", async () => {
  const sent: unknown[] = [], output: unknown[] = []
  const code = await runEventsVerb("sources", new Map(), [], async request => { sent.push(request); return { NotificationSources: { sources: SOURCES } } }, value => output.push(value))
  assert.equal(code, 0)
  assert.deepEqual(sent, [{ ListNotificationSources: {} }])
  assert.deepEqual(output, [{ sources: SOURCES }])
})
test("subscribe --source follows one source by id with no targets, and by display name after one listing", async () => {
  const sent: unknown[] = []
  const send = async (request: unknown) => { sent.push(request); return "ListNotificationSources" in (request as object) ? { NotificationSources: { sources: SOURCES } } : SUBSCRIPTION }
  await runEventsVerb("subscribe", new Map<string, string | true>([["--source", "wf_source_a"]]), [], send, () => {})
  await runEventsVerb("subscribe", new Map<string, string | true>([["--source", "agent needs your action"]]), [], send, () => {})
  assert.deepEqual(sent.filter(request => "SubscribeEvents" in (request as object)), [
    { SubscribeEvents: { targets: [], sources: ["wf_source_a"] } },
    { SubscribeEvents: { targets: [], sources: ["owner_action_b"] } },
  ])
})
test("subscribe --source also resolves the name that events sources prints, unless two sources share it", async () => {
  const run = { ...SOURCES[0], kind: "workflow_run", source_id: "run_finished_a", name: "flow", display_name: "flow run finished" }
  const sent: unknown[] = []
  const send = (sources: unknown[]) => async (request: unknown) => { sent.push(request); return "ListNotificationSources" in (request as object) ? { NotificationSources: { sources } } : SUBSCRIPTION }
  await runEventsVerb("subscribe", new Map<string, string | true>([["--source", "flow"]]), [], send([...SOURCES, run]), () => {})
  assert.deepEqual(sent.filter(request => "SubscribeEvents" in (request as object)), [{ SubscribeEvents: { targets: [], sources: ["run_finished_a"] } }])
  const twin = { ...run, source_id: "run_finished_b" }
  await assert.rejects(runEventsVerb("subscribe", new Map<string, string | true>([["--source", "flow"]]), [], send([...SOURCES, run, twin]), () => {}), /several sources are named "flow".*run_finished_a.*run_finished_b/)
})
test("a source the grant cannot follow is refused before anything is subscribed, and --source takes no other target flag", async () => {
  const sent: unknown[] = []
  const send = async (request: unknown) => { sent.push(request); return { NotificationSources: { sources: SOURCES } } }
  await assert.rejects(runEventsVerb("subscribe", new Map<string, string | true>([["--source", "nope"]]), [], send, () => {}), error => error instanceof CommandFailure && error.code === "events_refused" && /chariox events sources/.test(error.message))
  assert.equal(sent.some(request => "SubscribeEvents" in (request as object)), false)
  await assert.rejects(runEventsVerb("subscribe", new Map<string, string | true>([["--source", "wf_source_a"], ["--kernel", "b3"]]), [], async () => assert.fail("must not send"), () => {}), /no other target flag/)
})
test("unsubscribe --source ends one source; --source all ends the followed sources and keeps targets", async () => {
  const sent: unknown[] = []
  const followed = { EventSubscription: { subscription: { ...SUBSCRIPTION.EventSubscription.subscription, sources: [{ source_id: "wf_source_a", display_name: "Review" }, { source_id: "owner_action_b", display_name: "Agent needs your action" }] } } }
  const send = async (request: unknown) => {
    sent.push(request)
    const name = Object.keys(request as object)[0]
    return name === "ListNotificationSources" ? { NotificationSources: { sources: SOURCES } } : name === "GetEventSubscription" ? followed : SUBSCRIPTION
  }
  await runEventsVerb("unsubscribe", new Map<string, string | true>([["--source", "wf_source_a"]]), [], send, () => {})
  await runEventsVerb("unsubscribe", new Map<string, string | true>([["--source", "all"]]), [], send, () => {})
  assert.deepEqual(sent.filter(request => "UnsubscribeEvents" in (request as object)), [
    { UnsubscribeEvents: { sources: ["wf_source_a"] } },
    { UnsubscribeEvents: { sources: ["wf_source_a", "owner_action_b"] } },
  ])
})
test("unsubscribe --source takes the name events sources prints, though a follow row carries only the display name", async () => {
  const app = { ...SOURCES[0], kind: "app_event", source_id: "app_event_a", name: "pull_request_updated@owner/repo", display_name: "Pull request updated (owner/repo)" }
  const followed = { EventSubscription: { subscription: { ...SUBSCRIPTION.EventSubscription.subscription, sources: [{ source_id: "app_event_a", display_name: "Pull request updated (owner/repo)" }] } } }
  const sent: unknown[] = []
  const send = async (request: unknown) => {
    sent.push(request)
    const name = Object.keys(request as object)[0]
    return name === "ListNotificationSources" ? { NotificationSources: { sources: [app] } } : name === "GetEventSubscription" ? followed : SUBSCRIPTION
  }
  await runEventsVerb("unsubscribe", new Map<string, string | true>([["--source", "pull_request_updated@owner/repo"]]), [], send, () => {})
  assert.deepEqual(sent.filter(request => "UnsubscribeEvents" in (request as object)), [{ UnsubscribeEvents: { sources: ["app_event_a"] } }])
})
test("a notification and a gap come out of wait whole: source, kind, name, occurrence, deadline", async () => {
  const notification = { cursor: 4, type: "notification", kernel: "k", machine: "m", session_id: "room", summary: "Review finished", at_ms: 1, refs: {}, value: { source_id: "wf_source_a", source_kind: "workflow_exit", display_name: "Review", occurrence_id: "run-1", output: { message: "approved" }, deadline_ms: 604800001 } }
  const gap = { cursor: 5, type: "gap", kernel: "k", machine: "m", session_id: "room", summary: "1 unread notification expired", at_ms: 2, refs: {}, value: { kernel: "k", dropped: 1, from: 4, to: 4 } }
  const output: unknown[] = []
  const code = await runEventsVerb("wait", new Map(), [], async () => ({ EventsListed: { events: [notification, gap], next_cursor: 5, oldest_cursor: 4, timed_out: false } }), value => output.push(value))
  assert.equal(code, 0)
  assert.deepEqual(output, [{ events: [notification, gap], next_cursor: 5, oldest_cursor: 4, timed_out: false }])
})
test("events sources and --source parse as flags of their commands", () => {
  assert.deepEqual([...parseArguments("events sources", []).flags], [])
  assert.equal(parseArguments("events subscribe", ["--source", "wf_source_a"]).flags.get("--source"), "wf_source_a")
  assert.equal(parseArguments("events unsubscribe", ["--source", "all"]).flags.get("--source"), "all")
})

test("unsubscribe --source detaches a followed source the kernel no longer lists because it is disabled", async () => {
  const sent: unknown[] = []
  const disabled = { source_id: "wf_disabled", display_name: "Old review", kind: "workflow_exit", session_id: "room", subscription_id: "s", available: false }
  const followed = { EventSubscription: { subscription: { ...SUBSCRIPTION.EventSubscription.subscription, sources: [disabled] } } }
  const send = async (request: unknown) => {
    sent.push(request)
    const name = Object.keys(request as object)[0]
    return name === "ListNotificationSources" ? { NotificationSources: { sources: SOURCES } } : name === "GetEventSubscription" ? followed : SUBSCRIPTION
  }
  await runEventsVerb("unsubscribe", new Map<string, string | true>([["--source", "wf_disabled"]]), [], send, () => {})
  await runEventsVerb("unsubscribe", new Map<string, string | true>([["--source", "old review"]]), [], send, () => {})
  assert.deepEqual(sent.filter(request => "UnsubscribeEvents" in (request as object)), [{ UnsubscribeEvents: { sources: ["wf_disabled"] } }, { UnsubscribeEvents: { sources: ["wf_disabled"] } }])
  await assert.rejects(runEventsVerb("unsubscribe", new Map<string, string | true>([["--source", "never followed"]]), [], send, () => {}), error => error instanceof CommandFailure && error.code === "events_refused" && /chariox events subscription/.test(error.message))
  assert.equal(sent.filter(request => "UnsubscribeEvents" in (request as object)).length, 2)
})
