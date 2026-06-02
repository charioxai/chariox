import type {
  AgentInstance,
  ProviderAuthStatus,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import type { ArrobaPreferences } from "./preferences.js"
import {
  backendProviderLabel,
  isBackendProviderId,
  normalizeBackendProviderId,
  providerCatalogIsLocalFallback,
  selectConfiguredModel,
  selectConfiguredVariant,
  type BackendProviderId,
  type ProviderCatalog,
} from "./provider-catalog.js"
import { providerRunUsesNativeTui } from "./provider-api.js"
import type { ThemeRegistry } from "./theme-registry.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import {
  deriveWaitingRoomModelSelectionDecision,
  deriveWaitingRoomVariantSelectionDecision,
} from "./waiting-room-controller.js"
import type { SessionListEntry } from "./sessions.js"

type FooterTone = "info" | "error"

type CurrentProviderSelection = {
  provider: string
  model: string
  effort: string
}

type ProviderSelectionDefaults = {
  provider: BackendProviderId
  model: string
  effort: string
}

type ProviderSelectionControllerDeps = {
  currentProviderSelection: () => CurrentProviderSelection
  waitingRoomState: () => WaitingRoomState
  availableSessions: () => SessionListEntry[]
  providerCatalog: () => ProviderCatalog
  themeRegistry: () => ThemeRegistry
  preferences: () => ArrobaPreferences
  defaults: () => ProviderSelectionDefaults
  setDefaults: (selection: ProviderSelectionDefaults) => void
  reconcileWaitingRoom: (next: WaitingRoomState) => void
  isAttached: () => boolean
  focusedAgentId: () => string | null | undefined
  providerRunState: () => RuntimeProviderRun | null
  sessionState: () => RuntimeSession
  updateAgentProfile: (
    sessionId: string,
    agentId: string,
    profile: ProviderSelectionDefaults,
  ) => Promise<{ session: RuntimeSession; agent?: AgentInstance }>
  applySessionState: (session: RuntimeSession) => void
  clearProviderRunState: () => void
  getProviderAuthStatus: (provider: BackendProviderId) => Promise<ProviderAuthStatus>
  appendNotice: (text: string) => void
  flashFooter: (message: string, tone: FooterTone) => void
  warn?: (message: string, fields: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
}

export type ProviderSelectionController = {
  applyModelSelection(modelId: string): Promise<void>
  applyVariantSelection(variant: string): Promise<void>
  applyProviderSelection(providerId: string): Promise<void>
}

export function createProviderSelectionController(
  deps: ProviderSelectionControllerDeps,
): ProviderSelectionController {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  const updateFocusedAgentProfile = async (
    selection: ProviderSelectionDefaults,
    nativeTuiMessage?: string,
  ): Promise<boolean> => {
    const agentId = deps.focusedAgentId()
    if (!agentId) {
      deps.flashFooter("no focused agent to update", "error")
      return false
    }
    const activeRun = deps.providerRunState()
    if (nativeTuiMessage && activeRun?.agent_instance_id === agentId && providerRunUsesNativeTui(activeRun)) {
      deps.flashFooter(nativeTuiMessage, "error")
      return false
    }
    const payload = await deps.updateAgentProfile(deps.sessionState().id, agentId, selection)
    deps.applySessionState(payload.session)
    deps.clearProviderRunState()
    return true
  }

  return {
    async applyModelSelection(modelId) {
      const currentSelection = deps.currentProviderSelection()
      const catalog = deps.providerCatalog()
      const localFallback = providerCatalogIsLocalFallback(catalog)
      const decision = deriveWaitingRoomModelSelectionDecision({
        modelId,
        state: deps.waitingRoomState(),
        sessions: deps.availableSessions(),
        catalog,
        themeRegistry: deps.themeRegistry(),
        currentProvider: normalizeBackendProviderId(currentSelection.provider),
        configuredEffort: currentSelection.effort,
      })
      if (decision.kind === "error") {
        deps.flashFooter(decision.message, "error")
        return
      }
      deps.reconcileWaitingRoom(decision.nextState)
      if (!deps.isAttached()) {
        deps.flashFooter(selectionMessage(`selected model ${decision.selectedModelId}`, localFallback), "info")
        return
      }
      const updated = await updateFocusedAgentProfile(decision.launch, "model is controlled by the provider-native TUI for this agent")
      if (updated) {
        deps.flashFooter(selectionMessage(`model set to ${decision.selectedModelId}`, localFallback), "info")
      }
    },
    async applyVariantSelection(variant) {
      const currentSelection = deps.currentProviderSelection()
      const catalog = deps.providerCatalog()
      const localFallback = providerCatalogIsLocalFallback(catalog)
      const decision = deriveWaitingRoomVariantSelectionDecision({
        variant,
        currentModelId: currentSelection.model,
        currentProviderId: normalizeBackendProviderId(currentSelection.provider),
        state: deps.waitingRoomState(),
        sessions: deps.availableSessions(),
        catalog,
        themeRegistry: deps.themeRegistry(),
      })
      if (decision.kind === "error") {
        deps.flashFooter(decision.message, "error")
        return
      }
      deps.reconcileWaitingRoom(decision.nextState)
      if (!deps.isAttached()) {
        deps.flashFooter(selectionMessage(`selected variant ${decision.selectedVariant}`, localFallback), "info")
        return
      }
      const updated = await updateFocusedAgentProfile(decision.launch, "variant is controlled by the provider-native TUI for this agent")
      if (updated) {
        deps.flashFooter(selectionMessage(`variant set to ${decision.selectedVariant}`, localFallback), "info")
      }
    },
    async applyProviderSelection(providerId) {
      if (!isBackendProviderId(providerId)) {
        deps.flashFooter(`unknown provider: ${providerId}`, "error")
        return
      }
      const catalog = deps.providerCatalog()
      const localFallback = providerCatalogIsLocalFallback(catalog)
      const defaults = deps.defaults()
      const saved = deps.preferences().providers?.[providerId]
      const selected = selectConfiguredModel(
        catalog,
        saved?.model ?? defaults.model,
        providerId,
      )
      const nextDefaults = {
        provider: providerId,
        model: selected?.id ?? defaults.model,
        effort: saved?.effort ?? (selected ? selectConfiguredVariant(selected, defaults.effort) : defaults.effort),
      }

      deps.setDefaults(nextDefaults)
      deps.reconcileWaitingRoom({
        ...deps.waitingRoomState(),
        providerId,
        modelId: nextDefaults.model,
        effort: nextDefaults.effort,
      })

      if (deps.isAttached()) {
        try {
          const updated = await updateFocusedAgentProfile(nextDefaults)
          if (!updated) {
            return
          }
        } catch (error) {
          deps.warn?.("provider profile update failed", {
            provider: providerId,
            error: formatError(error),
          })
          deps.flashFooter(formatError(error), "error")
          return
        }
      }

      if (providerId === "codex") {
        try {
          const status = await deps.getProviderAuthStatus(providerId)
          if (status.auth_state !== "authenticated") {
            deps.appendNotice([
              "Codex is not logged in.",
              status.login_hint ?? "Run /provider login codex to authenticate.",
            ].join(" "))
          }
        } catch (error) {
          deps.warn?.("provider auth status lookup failed after selection", {
            provider: providerId,
            error: formatError(error),
          })
        }
      }
      deps.flashFooter(selectionMessage(`${backendProviderLabel(providerId)} selected`, localFallback), "info")
    },
  }
}

function selectionMessage(message: string, localFallback: boolean) {
  return localFallback ? `${message} (local provider list)` : message
}
