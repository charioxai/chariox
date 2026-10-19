// MP-08 / MP-09 / MP-10 / MP-11 A02: show only what the kernel's durable task
// ledger adds to the ordinary prompt status, a wait or a block. Working and
// finished tasks keep the ordinary badge, so plain turns look as they always did.
import type { AgentTaskExecution, AgentWake } from "./kernel-types-session.js"

export type AgentTaskStatus = {
  state: "waiting" | "blocked"
  label: string
  deadline_ms?: number
  tone: "idle" | "error"
}
export type AgentTaskStatusSession = {
  agent_tasks?: AgentTaskExecution[]
  agent_wakes?: AgentWake[]
}

const ARMED = new Set(["scheduled", "starting", "running", "cancelling"])
const OPEN_OBLIGATION = new Set(["open", "settling", "failed"])

function date(value: number | null | undefined): string | null {
  return typeof value === "number" && Number.isFinite(value) && value > 0 && value < 8.64e15
    ? new Date(value).toISOString() : null
}

export function agentTaskStatus(session: AgentTaskStatusSession | null | undefined, agentId: string): AgentTaskStatus | null {
  const unfinished = (session?.agent_tasks ?? []).filter((task) =>
    task.agent_id === agentId && (task.state === "waiting" || task.state === "blocked"))
  const task = unfinished.find((item) => item.state === "blocked") ?? unfinished[0]
  if (!task) return null
  const open = (session?.agent_tasks ?? [])
    .filter((item) => item.agent_id === agentId)
    .flatMap((item) => item.obligations)
    .filter((item) => OPEN_OBLIGATION.has(item.status)).length
  const suffix = open ? ` (${open} obligations)` : ""
  if (task.state === "blocked") return { state: "blocked", label: `BLOCKED: ${task.reason}${suffix}`, tone: "error" }
  const deadline = date(task.wait?.deadline_ms)
  if (!deadline || !task.wait) return { state: "blocked", label: `BLOCKED: Invalid wait deadline${suffix}`, tone: "error" }
  const wakes = (session?.agent_wakes ?? []).filter((wake) => wake.agent_id === agentId && ARMED.has(wake.state))
  const wake = wakes.filter((item) => date(item.next_due_ms)).sort((a, b) => a.next_due_ms! - b.next_due_ms!)[0] ?? wakes[0]
  const next = !wake ? "" : date(wake.next_due_ms)
    ? ` · NEXT WAKE ${wake.label} AT ${date(wake.next_due_ms)}` : ` · WATCHING ${wake.label}`
  return {
    state: "waiting",
    deadline_ms: task.wait.deadline_ms,
    label: `WAITING ON ${task.reason || "result"} UNTIL ${deadline}${next}${suffix}`,
    tone: "idle",
  }
}
