// Row 524: what a waiting agent waits on, in plain words and its age. One wording for the agent tree node, the agent
// pane and the CLI listing, read from the kernel's `liveness`: nothing here guesses a reason the kernel did not give.
import type { AgentLiveness, AgentTreeNode, AgentWaitEvent, AgentWaitingOn } from "./kernel-types-runtime.js"

export interface WaitReasonOptions {
  /** The zone a timer's due time is shown in; the runtime's own by default. */
  readonly timeZone?: string
}

const COMMAND_WIDTH = 48

/** "30 s", "12 min", "1 h 30 min", "2 h", "1 d 3 h". */
export function formatWaitAge(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000))
  if (seconds < 60) return `${seconds} s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes} min`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return minutes % 60 === 0 ? `${hours} h` : `${hours} h ${minutes % 60} min`
  const days = Math.floor(hours / 24)
  return hours % 24 === 0 ? `${days} d` : `${days} d ${hours % 24} h`
}

const short = (command: string): string => {
  const one = command.replace(/\s+/g, " ").trim()
  return one.length > COMMAND_WIDTH ? `${one.slice(0, COMMAND_WIDTH - 1)}…` : one
}

const clock = (at: number, options: WaitReasonOptions): string =>
  new Intl.DateTimeFormat("en-GB", { hour: "2-digit", minute: "2-digit", hourCycle: "h23", ...(options.timeZone ? { timeZone: options.timeZone } : {}) }).format(new Date(at))

/** The first thing the agent waits on: the oldest listed event, else the oldest `waiting_on`. */
type Wait = Pick<AgentWaitEvent, "kind" | "since_ms"> & { readonly text: string; readonly due_at_ms?: number | undefined; readonly resource?: AgentWaitingOn["resource"] }

const withoutNoun = (label: string, nouns: readonly string[]): string => {
  for (const noun of nouns) if (label.startsWith(`${noun} `)) return label.slice(noun.length + 1)
  return label
}

function waitOf(liveness: AgentLiveness): { readonly first: Wait; readonly more: number } | undefined {
  const events = liveness.waiting_events ?? []
  const on = liveness.waiting_on
  // A command held or paused for room is the reason the person needs; an event that is the same command is not counted again, any other is.
  if (on && (on.kind === "command_held" || on.kind === "command_paused")) {
    const same = (event: AgentWaitEvent) => event.kind === "background_command" && withoutNoun(event.label, ["command", "terminal"]) === (on.detail ?? "")
    return { first: { kind: on.kind, since_ms: on.since_ms, text: on.detail ?? "", due_at_ms: on.due_at_ms, resource: on.resource }, more: events.filter(event => !same(event)).length }
  }
  if (events.length > 0) {
    const [first, ...rest] = events as [AgentWaitEvent, ...AgentWaitEvent[]]
    return { first: { kind: first.kind, since_ms: first.since_ms, text: first.label, due_at_ms: first.due_at_ms }, more: rest.length }
  }
  if (!on || on.kind === "provider") return undefined
  const noun = on.kind === "timer" ? "timer " : ""
  return { first: { kind: on.kind, since_ms: on.since_ms, text: `${noun}${on.detail ?? ""}`.trim(), due_at_ms: on.due_at_ms, resource: on.resource }, more: 0 }
}

function sentence(wait: Wait, now: number, options: WaitReasonOptions): string {
  const age = formatWaitAge(now - wait.since_ms)
  switch (wait.kind) {
    case "background_command": return `${short(withoutNoun(wait.text, ["command", "terminal"]))} (${age})`
    case "command_held": return `${short(wait.text)} held for ${wait.resource ?? "room"} (${age})`
    case "command_paused": return `${short(wait.text)} paused for ${wait.resource ?? "room"} (${age})`
    case "timer": {
      const label = withoutNoun(wait.text, ["timer"])
      const when = wait.due_at_ms === undefined ? age : `due ${clock(wait.due_at_ms, options)}`
      return label ? `timer '${label}' (${when})` : `timer (${when})`
    }
    case "card": return `${wait.text} (${age})`
    case "source": return wait.text.startsWith("agent ") ? `reply from ${wait.text.slice("agent ".length)} (${age})` : `${wait.text} (${age})`
    case "provider": return age
  }
}

/**
 * "waiting: <reason> (<age>)", or "stalled: no provider activity for <age>" when the provider turn is silent.
 * `undefined` when the liveness names nothing: an idle agent, or a provider turn that is only running.
 */
export function waitReasonText(liveness: AgentLiveness | undefined, now: number, options: WaitReasonOptions = {}): string | undefined {
  if (!liveness) return undefined
  if (liveness.stalled === true) {
    return liveness.last_activity_at_ms === undefined
      ? "stalled: no provider activity"
      : `stalled: no provider activity for ${formatWaitAge(now - liveness.last_activity_at_ms)}`
  }
  const wait = waitOf(liveness)
  if (!wait) return undefined
  return `waiting: ${sentence(wait.first, now, options)}${wait.more > 0 ? `, and ${wait.more} more` : ""}`
}

/**
 * One line for a tree node: its stall or wait reason, else its state in a word. A node that is idle, ended or in
 * error says so and never claims a wait.
 */
export function agentStatusLine(node: Pick<AgentTreeNode, "state" | "liveness">, now: number, options: WaitReasonOptions = {}): string {
  if (node.state === "idle" || node.state === "ended" || node.state === "error") return node.state
  return waitReasonText(node.liveness, now, options) ?? node.state
}
