// MP-08 / MP-09 / MP-10 / MP-11 A03: shared wake projection for every client.
import type { AgentWake, RuntimeSession } from "./kernel-types-session.js"

const ARMED = new Set(["scheduled", "starting", "running", "cancelling"])

function clock(ms: number | null | undefined): string {
  return ms && Number.isFinite(ms) && ms < 8.64e15 ? new Date(ms).toISOString().slice(11, 19) + "Z" : "-"
}

export function sessionAgentWakes(session: RuntimeSession | null | undefined, agentId: string | null): AgentWake[] {
  return (session?.agent_wakes ?? []).filter((wake) => agentId === null || wake.agent_id === agentId)
}

// The kernel's driver renders the kind-specific part as `summary`.
export function formatAgentWakeLines(session: RuntimeSession | null | undefined, agentId: string | null): string[] {
  const wakes = sessionAgentWakes(session, agentId)
  if (!wakes.length) return ["no wakes"]
  return wakes.map((wake) => {
    const proof = wake.verified_at_ms ? "verified" : ARMED.has(wake.state) ? "UNVERIFIED" : "-"
    const receipt = wake.last_fired_at_ms
      ? `fired ${clock(wake.last_fired_at_ms)} delivered ${clock(wake.last_delivered_at_ms)} acked ${clock(wake.last_acknowledged_at_ms)} (${wake.last_delivery ?? "pending"})`
      : "not fired"
    const missed = wake.missed_fires ? ` missed ${wake.missed_fires}` : ""
    return `${wake.kind} '${wake.label}' ${wake.state} ${wake.summary} ${proof} | ${wake.fire_count}x, last ${receipt}${missed}`
  })
}
