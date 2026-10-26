import type { KernelSudoTurn } from "@chariox/kernel-client/kernel-types"

/** MP-08/MP-10/MP-11 A04: one line per live sudo window, from the kernel's
 * deadline (never a client countdown that survives reconnect). */
export function sudoWindowLine(window: KernelSudoTurn, now: number, agentLabel = window.agent_id): string {
  const expires = window.expires_at_ms ?? now
  const minutes = Math.max(0, Math.ceil((expires - now) / 60_000))
  const left = minutes >= 60 ? `${Math.floor(minutes / 60)}h ${minutes % 60}m` : `${minutes}m`
  const until = new Date(expires).toISOString().slice(11, 16)
  return `sudo · ${agentLabel} · ${left} left (until ${until} UTC)${window.warning_sent ? " · expiring soon" : ""} · ${window.entry_id}`
}
