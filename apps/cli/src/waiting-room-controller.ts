import {
  catalogModelOptions,
  selectConfiguredVariant,
  type BackendProviderId,
  type ProviderCatalog,
} from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import type { ThemeRegistry } from "./theme-registry.js"
import {
  normalizeWaitingRoomState,
  waitingRoomChoice,
  type WaitingRoomState,
} from "./waiting-room.js"

export type WaitingRoomLaunchConfig = {
  provider: BackendProviderId
  model: string
  effort: string
}

export type WaitingRoomStateUpdate = {
  normalizedState: WaitingRoomState
  nextProvider: BackendProviderId
  nextModel: string
  nextEffort: string
  shouldPersistProviderPreferences: boolean
}

export type WaitingRoomActivationDecision =
  | { action: "create"; launch: WaitingRoomLaunchConfig }
  | { action: "join"; session: SessionListEntry; launch: WaitingRoomLaunchConfig }
  | { action: "error"; message: string }
  | { action: "none" }

export type WaitingRoomModelSelectionDecision =
  | {
      kind: "success"
      selectedModelId: string
      nextState: WaitingRoomState
      launch: WaitingRoomLaunchConfig
    }
  | { kind: "error"; message: string }

export type WaitingRoomVariantSelectionDecision =
  | {
      kind: "success"
      selectedVariant: string
      nextState: WaitingRoomState
      launch: WaitingRoomLaunchConfig
    }
  | { kind: "error"; message: string }

export function deriveWaitingRoomStateUpdate(options: {
  currentState: WaitingRoomState
  nextState: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
  themeRegistry?: ThemeRegistry
  currentProvider: BackendProviderId
  currentModel: string
}): WaitingRoomStateUpdate {
  const normalizedState = normalizeWaitingRoomState(
    options.nextState,
    options.sessions,
    options.catalog,
    options.themeRegistry,
  )
  const nextModel = normalizedState.modelId || options.currentModel

  return {
    normalizedState,
    nextProvider: normalizedState.providerId,
    nextModel,
    nextEffort: normalizedState.effort,
    shouldPersistProviderPreferences:
      normalizedState.modelId.length > 0
      && (
        options.currentState.providerId !== normalizedState.providerId
        || options.currentProvider !== normalizedState.providerId
        || options.currentState.modelId !== normalizedState.modelId
        || options.currentState.effort !== normalizedState.effort
      ),
  }
}

export function deriveWaitingRoomActivationDecision(options: {
  state: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
  currentProvider: BackendProviderId
  currentModel: string
}): WaitingRoomActivationDecision {
  const choice = waitingRoomChoice(options.state, options.sessions, options.catalog)
  const launch = {
    provider: choice.providerId ?? options.currentProvider,
    model: choice.model?.id ?? options.currentModel,
    effort: choice.effort,
  }

  if (options.state.focus !== "session") {
    return { action: "create", launch }
  }

  if (!choice.session) {
    return { action: "error", message: "no session available to join" }
  }

  return {
    action: "join",
    session: choice.session,
    launch,
  }
}

export function deriveWaitingRoomModelSelectionDecision(options: {
  modelId: string
  state: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
  themeRegistry?: ThemeRegistry
  currentProvider: BackendProviderId
  configuredEffort: string
}): WaitingRoomModelSelectionDecision {
  const selected = catalogModelOptions(options.catalog, options.currentProvider).find(
    (option) => option.id === options.modelId,
  )
  if (!selected) {
    return {
      kind: "error",
      message: `unknown model: ${options.modelId}`,
    }
  }

  const effort = selectConfiguredVariant(selected, options.configuredEffort)
  return {
    kind: "success",
    selectedModelId: selected.id,
    nextState: normalizeWaitingRoomState(
      {
        ...options.state,
        modelId: selected.id,
        effort,
      },
      options.sessions,
      options.catalog,
      options.themeRegistry,
    ),
    launch: {
      provider: options.currentProvider,
      model: selected.id,
      effort,
    },
  }
}

export function deriveWaitingRoomVariantSelectionDecision(options: {
  variant: string
  currentModelId: string
  currentProviderId: BackendProviderId
  state: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
  themeRegistry?: ThemeRegistry
}): WaitingRoomVariantSelectionDecision {
  const selected = catalogModelOptions(options.catalog, options.currentProviderId).find(
    (option) => option.id === options.currentModelId,
  )
  if (!selected || !selected.variants.includes(options.variant)) {
    return {
      kind: "error",
      message: `unknown variant: ${options.variant}`,
    }
  }

  return {
    kind: "success",
    selectedVariant: options.variant,
    nextState: normalizeWaitingRoomState(
      {
        ...options.state,
        modelId: selected.id,
        effort: options.variant,
      },
      options.sessions,
      options.catalog,
      options.themeRegistry,
    ),
    launch: {
      provider: options.currentProviderId,
      model: selected.id,
      effort: options.variant,
    },
  }
}
