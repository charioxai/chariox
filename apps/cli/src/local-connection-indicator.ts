import { loadLocalKernelPresences, localKernelEndpoint, localKernelPresenceFreshnessMs, type LocalKernelPresence } from "./local-kernel-presence.js"

let cached: readonly LocalKernelPresence[] = []
let nextRefreshAtMs = 0

/** MP-08/MP-11: discovery gates display only; it never authorizes a carrier. */
export function localKernelConnectionLabel(
  client: { readonly socketPath: string; isRelayTransport(): boolean; isLocalDirectTransport?(): boolean },
  kernelId: string | null | undefined,
  presences?: readonly LocalKernelPresence[],
  nowMs = Date.now(),
): string | null {
  if (!kernelId || kernelId.startsWith("slice:")) return null
  if (!presences) {
    if (nowMs >= nextRefreshAtMs) {
      cached = loadLocalKernelPresences(undefined, nowMs)
      nextRefreshAtMs = nowMs + 1_000
    }
    presences = cached
  }
  const local = presences.find(p => p.kernelId === kernelId && Math.abs(nowMs - p.heartbeatAtMs) <= localKernelPresenceFreshnessMs)
  if (!local) return null
  if (client.isLocalDirectTransport?.()) return "Local connection"
  if (client.isRelayTransport()) return "Relay connection"
  if (client.socketPath !== localKernelEndpoint(local) && !client.socketPath.startsWith("ws+unix://")) return null
  return "Local connection"
}
