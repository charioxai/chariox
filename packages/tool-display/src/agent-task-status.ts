// MP-08 / MP-09 / MP-10 / MP-11: display the kernel's durable task state.
// Native provider inactivity never settles an unfinished task.
export type AgentTaskStatus = {
  state: "working" | "waiting" | "blocked" | "done" | "cancelled"
  label: string
  deadline_ms?: number
  tone: "working" | "idle" | "error"
}
export type AgentTaskStatusSession = { agent_tasks?: unknown; agent_wakes?: unknown }

function records(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.filter((item): item is Record<string, unknown> =>
    item !== null && typeof item === "object" && !Array.isArray(item)) : []
}
const STATES = new Set(["working", "waiting", "blocked", "done", "cancelled"])
const ARMED = new Set(["scheduled", "starting", "running", "cancelling"])
function date(value: unknown): string | null {
  return typeof value === "number" && Number.isFinite(value) && value > 0 && value < 8.64e15
    ? new Date(value).toISOString() : null
}
export function agentTaskStatus(session: AgentTaskStatusSession | null | undefined, agentId: string): AgentTaskStatus | null {
  const tasks = records(session?.agent_tasks).filter((task) => task.agent_id === agentId && STATES.has(String(task.state)))
  if (!tasks.length) return null
  const unfinished = tasks.filter((task) => task.state !== "done" && task.state !== "cancelled")
  const task = unfinished.find((task) => task.state === "blocked")
    ?? unfinished.find((task) => task.state === "working") ?? unfinished[0] ?? tasks[tasks.length - 1]!
  const open = tasks.flatMap((task) => records(task.obligations))
    .filter((item) => ["open", "settling", "failed"].includes(String(item.status))).length
  const suffix = open ? ` (${open} obligations)` : ""
  const state = task.state as AgentTaskStatus["state"]
  const reason = typeof task.reason === "string" ? task.reason : "result"
  if (state === "waiting") {
    const deadline = date(records([task.wait])[0]?.deadline_ms)
    if (!deadline) return { state: "blocked", label: `BLOCKED: Invalid wait deadline${suffix}`, tone: "error" }
    const wakes = records(session?.agent_wakes).filter((wake) => wake.agent_id === agentId && ARMED.has(String(wake.state)))
    const timed = wakes.filter((wake) => date(wake.next_due_ms)).sort((a, b) => Number(a.next_due_ms) - Number(b.next_due_ms))
    const wake = timed[0] ?? wakes[0]
    const next = !wake ? "" : date(wake.next_due_ms)
      ? ` · NEXT WAKE ${wake.label} AT ${date(wake.next_due_ms)}` : ` · WATCHING ${wake.label}`
    return { state, deadline_ms: Number(records([task.wait])[0]?.deadline_ms), label: `WAITING ON ${reason} UNTIL ${deadline}${next}${suffix}`, tone: "idle" }
  }
  if (state === "blocked") return { state, label: `BLOCKED: ${reason}${suffix}`, tone: "error" }
  if (state === "working" && waitsForOwner(task)) return { state, label: `WORKING: ${reason}${suffix}`, tone: "working" }
  return { state, label: `${state.toUpperCase()}${suffix}`, tone: state === "working" ? "working" : "idle" }
}

// The kernel pauses a completion check's bound while it waits on the owner (the Vault unlock).
function waitsForOwner(task: Record<string, unknown>): boolean {
  const evaluation = records([task.goal])[0]?.evaluation ?? records([task.completion_check])[0]?.evaluation
  return typeof records([evaluation])[0]?.paused_at_ms === "number"
}

// MP-08/MP-09/MP-10/MP-11 G13: bounded countdown for permanent client chrome.
export function agentWaitCountdown(deadlineMs: number, nowMs: number): string {
  if (!Number.isFinite(deadlineMs) || !Number.isFinite(nowMs)) return ""
  const seconds = Math.max(0, Math.ceil((deadlineMs - nowMs) / 1000))
  if (!seconds) return "due now"
  if (seconds >= 86400) return ">24h"
  const minutes = Math.floor(seconds / 60)
  return `${minutes}:${String(seconds % 60).padStart(2, "0")}`
}
