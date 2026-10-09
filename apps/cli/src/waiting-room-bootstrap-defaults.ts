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
