// Row 524 cells W1-W7: what a waiting agent waits on, in plain words and its age; the one wording web and CLI show.
import assert from "node:assert/strict"
import test from "node:test"
import type { AgentLiveness, AgentTreeNode, AgentTreeState } from "./kernel-types-runtime.js"
import { agentStatusLine, formatWaitAge, waitReasonText } from "./wait-reason.js"

const MIN = 60_000
const NOW = Date.parse("2026-11-24T05:00:00Z")
const ago = (ms: number) => NOW - ms
const events = (...list: Array<{ kind: "background_command" | "timer" | "source" | "card"; label: string; since: number; due?: number }>): AgentLiveness => ({
  waiting_on: { kind: list[0]!.kind, detail: list[0]!.label, since_ms: list[0]!.since, ...(list[0]!.due ? { due_at_ms: list[0]!.due } : {}) },
  waiting_events: list.map((e, i) => ({ id: `e${i}`, kind: e.kind, label: e.label, since_ms: e.since, ...(e.due ? { due_at_ms: e.due } : {}), task_id: "t", wait_started_at_ms: e.since })),
})
const text = (liveness: AgentLiveness) => waitReasonText(liveness, NOW, { timeZone: "UTC" })

test("W0 ages read in plain words", () => {
  assert.equal(formatWaitAge(30_000), "30 s")
  assert.equal(formatWaitAge(12 * MIN), "12 min")
  assert.equal(formatWaitAge(90 * MIN), "1 h 30 min")
  assert.equal(formatWaitAge(2 * 60 * MIN), "2 h")
  assert.equal(formatWaitAge(27 * 60 * MIN), "1 d 3 h")
  assert.equal(formatWaitAge(-5), "0 s")
})

test("W1 a command or job names the command and its age", () => {
  assert.equal(text(events({ kind: "background_command", label: "command cargo test -p chariox-kernel", since: ago(12 * MIN) })), "waiting: cargo test -p chariox-kernel (12 min)")
  const long = "command " + "x".repeat(80)
  assert.equal(text(events({ kind: "background_command", label: long, since: ago(MIN) })), `waiting: ${"x".repeat(47)}… (1 min)`)
})

test("W1 a command held or paused for room says which", () => {
  const held: AgentLiveness = { waiting_on: { kind: "command_held", detail: "cargo build", resource: "memory", since_ms: ago(3 * MIN) } }
  assert.equal(text(held), "waiting: cargo build held for memory (3 min)")
  const paused: AgentLiveness = { waiting_on: { kind: "command_paused", detail: "cargo build", resource: "disk", since_ms: ago(3 * MIN) } }
  assert.equal(text(paused), "waiting: cargo build paused for disk (3 min)")
})

test("W1 a held command keeps its reason and resource when its yielded event is listed too", () => {
  const held: AgentLiveness = {
    waiting_on: { kind: "command_held", detail: "cargo build", resource: "memory", since_ms: ago(3 * MIN) },
    waiting_events: [{ id: "w", kind: "background_command", label: "command cargo build", since_ms: ago(9 * MIN), task_id: "t", wait_started_at_ms: ago(9 * MIN) }],
  }
  assert.equal(text(held), "waiting: cargo build held for memory (3 min)")
  const paused: AgentLiveness = { ...held, waiting_on: { kind: "command_paused", detail: "cargo build", resource: "disk", since_ms: ago(2 * MIN) } }
  assert.equal(text(paused), "waiting: cargo build paused for disk (2 min)")
  const more: AgentLiveness = { ...held, waiting_events: [...held.waiting_events!, { id: "s", kind: "source", label: "sub-agent a", since_ms: ago(MIN), task_id: "t", wait_started_at_ms: ago(MIN) }] }
  assert.equal(text(more), "waiting: cargo build held for memory (3 min)")
  // No count is claimed beside a held command: no event can be tied to it (a labelled wake reads "command build").
  const foreground: AgentLiveness = {
    waiting_on: held.waiting_on!,
    waiting_events: [
      { id: "t1", kind: "timer", label: "timer check", since_ms: ago(20 * MIN), task_id: "t", wait_started_at_ms: ago(20 * MIN) },
      { id: "s1", kind: "source", label: "sub-agent a", since_ms: ago(MIN), task_id: "t", wait_started_at_ms: ago(MIN) },
    ],
  }
  assert.equal(text(foreground), "waiting: cargo build held for memory (3 min)")
})

test("W2 a timer names its label and when it is due", () => {
  const due = Date.parse("2026-11-24T05:30:00Z")
  assert.equal(text(events({ kind: "timer", label: "timer check review", since: ago(MIN), due })), "waiting: timer 'check review' (due 05:30)")
  assert.equal(text(events({ kind: "timer", label: "timer check review", since: ago(4 * MIN) })), "waiting: timer 'check review' (4 min)")
})

test("W3 a sub-agent and W5 a reply from an agent", () => {
  assert.equal(text(events({ kind: "source", label: "sub-agent reviewer", since: ago(7 * MIN) })), "waiting: sub-agent reviewer (7 min)")
  assert.equal(text(events({ kind: "source", label: "agent orch15", since: ago(2 * MIN) })), "waiting: reply from orch15 (2 min)")
  assert.equal(text(events({ kind: "source", label: "workflow nightly", since: ago(2 * MIN) })), "waiting: workflow nightly (2 min)")
})

test("W4 a card names its title and the person's answer", () => {
  assert.equal(text(events({ kind: "card", label: "your answer on 'Value for the Vault'", since: ago(5 * MIN) })), "waiting: your answer on 'Value for the Vault' (5 min)")
})

test("several events name the oldest and count the rest", () => {
  const liveness = events(
    { kind: "source", label: "sub-agent a", since: ago(9 * MIN) },
    { kind: "source", label: "sub-agent b", since: ago(8 * MIN) },
    { kind: "timer", label: "timer t", since: ago(7 * MIN) },
  )
  assert.equal(text(liveness), "waiting: sub-agent a (9 min), and 2 more")
})

test("W6 a stalled turn says so with the time since its last output", () => {
  assert.equal(text({ stalled: true, last_activity_at_ms: ago(90 * MIN) }), "stalled: no provider activity for 1 h 30 min")
  assert.equal(text({ stalled: true }), "stalled: no provider activity")
  // a stalled agent that also waits on a command reads as stalled: the provider is what is silent
  assert.equal(text({ ...events({ kind: "background_command", label: "command sleep 9", since: ago(MIN) }), stalled: true, last_activity_at_ms: ago(20 * MIN) }), "stalled: no provider activity for 20 min")
})

test("W7 nothing armed reads idle, never waiting", () => {
  assert.equal(text({}), undefined)
  assert.equal(text({ waiting_on: { kind: "provider", since_ms: ago(MIN) } }), undefined)
  const node = (state: AgentTreeState, extra: Partial<AgentTreeNode> = {}): AgentTreeNode => ({
    agent_id: "a", parent_agent_id: "r", kernel_id: null, alias: "a", provider: "p", model: null, depth: 1, state, outcome: null,
    needs_user: false, deadline_ms: null, created_at_ms: 1, ...extra,
  })
  assert.equal(agentStatusLine(node("idle"), NOW, { timeZone: "UTC" }), "idle")
  assert.equal(agentStatusLine(node("running"), NOW, { timeZone: "UTC" }), "running")
  assert.equal(agentStatusLine(node("ended"), NOW, { timeZone: "UTC" }), "ended")
  assert.equal(agentStatusLine(node("waiting"), NOW, { timeZone: "UTC" }), "waiting")
  assert.equal(agentStatusLine(node("waiting", { liveness: events({ kind: "source", label: "sub-agent x", since: ago(MIN) }) }), NOW, { timeZone: "UTC" }), "waiting: sub-agent x (1 min)")
  assert.equal(agentStatusLine(node("running", { liveness: { stalled: true, last_activity_at_ms: ago(15 * MIN) } }), NOW, { timeZone: "UTC" }), "stalled: no provider activity for 15 min")
  // an idle node never claims a wait it does not have
  assert.equal(agentStatusLine(node("idle", { liveness: { waiting_on: { kind: "provider", since_ms: 1 } } }), NOW, { timeZone: "UTC" }), "idle")
})

test("row 36 (g): a long tool call reads running <tool> since its start, never waiting or stalled", () => {
  const liveness: AgentLiveness = { waiting_on: { kind: "tool", detail: "Bash", since_ms: Date.parse("2026-11-24T04:40:00Z") } }
  assert.equal(text(liveness), "running Bash since 04:40 (20 min)")
  assert.equal(agentStatusLine({ state: "working", liveness }, NOW, { timeZone: "UTC" }), "running Bash since 04:40 (20 min)")
})
