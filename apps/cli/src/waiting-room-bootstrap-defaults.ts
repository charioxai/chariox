import { LocalIpcError, type LocalIpcClient } from "./ipc.js"
import { normalizeBackendProviderId } from "./provider-catalog.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

export type WaitingRoomBootstrapDefaults = { provider?: string; model?: string; effort?: string }

export function createWaitingRoomBootstrapDefaultsController(deps: {
  state(): WaitingRoomState
  ownershipRevision(): number
  isAttached(): boolean
  apply(state: WaitingRoomState): void
}) {
  const revision = deps.ownershipRevision()
  return {
    apply(defaults: WaitingRoomBootstrapDefaults) {
      if (deps.isAttached() || deps.ownershipRevision() !== revision) return
      const state = deps.state()
      deps.apply({ ...state,
        ...(defaults.provider ? { providerId: normalizeBackendProviderId(defaults.provider) } : {}),
        ...(defaults.model ? { modelId: defaults.model } : {}),
        ...(defaults.effort ? { effort: defaults.effort } : {}),
      })
    },
  }
}


// Retry this read once after real inventory arrives; relay admission can fail
// while a slow kernel misses heartbeats, before the waiting room recovers.
export async function readWaitingRoomConfiguredDefaults(
  client: LocalIpcClient,
  read?: (client: LocalIpcClient) => Promise<WaitingRoomBootstrapDefaults>,
): Promise<WaitingRoomBootstrapDefaults> {
  if (!read) return {}
  let reported = false
  let report!: () => void
  const liveReport = new Promise<void>(resolve => { report = resolve })
  const dispose = client.onKernelEvent?.(event => {
    if (event.event === "waiting_room_rows_changed") { reported = true; report() }
  })
  let timeout: ReturnType<typeof setTimeout> | undefined
  try {
    try { return await read(client) }
    catch (error) {
      if (!dispose || !(error instanceof LocalIpcError) || !error.retryable
        || !["target_disconnected", "connection_closed", "request_timeout", "write_failed"].includes(error.code ?? "")) throw error
      if (!reported) {
        await Promise.race([liveReport, new Promise<never>((_, reject) => {
          timeout = setTimeout(() => reject(error), 120_000)
          timeout.unref?.()
        })])
      }
      return await read(client)
    }
  } finally {
    if (timeout) clearTimeout(timeout)
    dispose?.()
  }
}
