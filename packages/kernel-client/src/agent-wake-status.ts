// MP-08 / MP-09 / MP-10 / MP-11 A03: shared wake projection for TUI and web.
import type { AgentWake, RuntimeSession } from "./kernel-types-session.js"

const ARMED = new Set(["scheduled", "starting", "running", "cancelling"])

function clock(ms: number | null | undefined): string {
  return ms && Number.isFinite(ms) && ms < 8.64e15 ? new Date(ms).toISOString().slice(11, 19) + "Z" : "-"
}

export function sessionAgentWakes(session: RuntimeSession | null | undefined, agentId: string | null): AgentWake[] {
  return (session?.agent_wakes ?? []).filter((wake) => agentId === null || wake.agent_id === agentId)
}

export function sessionAgentNextWake(session: RuntimeSession | null | undefined, agentId: string): AgentWake | null {
  const armed = sessionAgentWakes(session, agentId).filter((wake) => ARMED.has(wake.state))
  return armed.filter((wake) => wake.next_due_ms !== null).sort((a, b) => a.next_due_ms! - b.next_due_ms!)[0]
    ?? armed[0] ?? null
}

export function formatAgentWakeLines(session: RuntimeSession | null | undefined, agentId: string | null): string[] {
  const wakes = sessionAgentWakes(session, agentId)
  if (!wakes.length) return ["no wakes"]
  return wakes.map((wake) => {
    const next = wake.kind === "timer"
      ? `next ${clock(wake.next_due_ms)}${wake.interval_ms ? ` every ${Math.round(wake.interval_ms / 1000)}s` : ""}`
      : `pid ${wake.pid ?? "-"}${wake.exit_code !== null ? ` exit ${wake.exit_code}` : ""}`
    const proof = wake.verified_at_ms ? "verified" : ARMED.has(wake.state) ? "UNVERIFIED" : "-"
    const receipt = wake.last_fired_at_ms
      ? `fired ${clock(wake.last_fired_at_ms)} delivered ${clock(wake.last_delivered_at_ms)} acked ${clock(wake.last_acknowledged_at_ms)} (${wake.last_delivery ?? "pending"})`
      : "not fired"
    const missed = wake.missed_fires ? ` missed ${wake.missed_fires}` : ""
    return `${wake.kind} '${wake.label}' ${wake.state} ${next} ${proof} | ${wake.fire_count}x, last ${receipt}${missed}`
  })
}
