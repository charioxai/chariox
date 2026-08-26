import type { BackendProviderId, ProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import type { ThemeRegistry } from "./theme-registry.js"
import {
  deriveWaitingRoomStateUpdate,
  type WaitingRoomStateUpdate,
} from "./waiting-room-controller.js"
import type {
  WaitingRoomRemoteState,
  WaitingRoomState,
} from "./waiting-room-types.js"

export type WaitingRoomProviderDefaults = {
  provider: BackendProviderId
  model: string
  effort: string
}

export type WaitingRoomReconcileControllerDeps = {
  getCurrentState: () => WaitingRoomState
  setWaitingRoomState: (state: WaitingRoomState) => void
  setProjectedWaitingRoomState?: (state: WaitingRoomState) => void
  getSessions: () => SessionListEntry[]
  getProviderCatalog: () => ProviderCatalog
  getRemoteState: () => WaitingRoomRemoteState
  getThemeRegistry: () => ThemeRegistry
  getCurrentProvider: () => BackendProviderId
  getCurrentModel: () => string
  setProviderDefaults: (defaults: WaitingRoomProviderDefaults) => void
  applyTheme: (themeId: string, registry: ThemeRegistry) => string
  resetTranscriptSyntax: () => void
  bumpThemeRevision: () => void
  saveUiThemePreference: (themeId: string) => void
  mergeUiThemePreference: (themeId: string) => void
  applyResponseLayout: () => void
  renderCommandCenter: () => void
  saveProviderPreferences: (
    provider: BackendProviderId,
    preferences: { model: string; effort: string },
  ) => void
  isAttached: () => boolean
  rebuildTranscript: () => void
  updateSessionChrome: () => void
  syncCommandCenter: () => void
  refreshProviderCatalogForSelection?: (state: WaitingRoomState) => void
  deriveStateUpdate?: typeof deriveWaitingRoomStateUpdate
}

export function createWaitingRoomReconcileController(
  deps: WaitingRoomReconcileControllerDeps,
) {
  const deriveStateUpdate = deps.deriveStateUpdate ?? deriveWaitingRoomStateUpdate

  const reconcileWith = (
    next: WaitingRoomState,
    setState: (state: WaitingRoomState) => void,
  ) => {
    const currentState = deps.getCurrentState()
    const update: WaitingRoomStateUpdate = deriveStateUpdate({
      currentState,
      nextState: next,
      sessions: deps.getSessions(),
      catalog: deps.getProviderCatalog(),
      remote: deps.getRemoteState(),
      themeRegistry: deps.getThemeRegistry(),
      currentProvider: deps.getCurrentProvider(),
      currentModel: deps.getCurrentModel(),
    })

    setState(update.normalizedState)
    if (
      currentState.providerId !== update.normalizedState.providerId
      || currentState.accountProfileId !== update.normalizedState.accountProfileId
      || currentState.selectedKernelRef !== update.normalizedState.selectedKernelRef
      || currentState.sliceSelectionId !== update.normalizedState.sliceSelectionId
    ) {
      deps.refreshProviderCatalogForSelection?.(update.normalizedState)
    }
    deps.setProviderDefaults({
      provider: update.nextProvider,
      model: update.nextModel,
      effort: update.nextEffort,
    })

    if (currentState.themeId !== update.normalizedState.themeId) {
      const nextThemeId = deps.applyTheme(update.normalizedState.themeId, deps.getThemeRegistry())
      deps.resetTranscriptSyntax()
      deps.bumpThemeRevision()
      deps.saveUiThemePreference(nextThemeId)
      deps.mergeUiThemePreference(nextThemeId)
      deps.applyResponseLayout()
      deps.renderCommandCenter()
    }

    if (update.shouldPersistProviderPreferences) {
      deps.saveProviderPreferences(update.nextProvider, {
        model: update.nextModel,
        effort: update.nextEffort,
      })
    }

    if (!deps.isAttached()) {
      deps.rebuildTranscript()
    }
    deps.updateSessionChrome()
    deps.syncCommandCenter()
    return update.normalizedState
  }

  return {
    reconcile: (next: WaitingRoomState) => reconcileWith(next, deps.setWaitingRoomState),
    reconcileProjection: (next: WaitingRoomState) => reconcileWith(
      next,
      deps.setProjectedWaitingRoomState ?? deps.setWaitingRoomState,
    ),
  }
}
