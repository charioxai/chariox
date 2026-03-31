import {
  catalogModelOptions,
  selectConfiguredVariant,
  type ProviderCatalog,
} from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import {
  normalizeWaitingRoomState,
  waitingRoomChoice,
  type WaitingRoomState,
} from "./waiting-room.js"

export type WaitingRoomLaunchConfig = {
  model: string
  effort: string
}

export type WaitingRoomStateUpdate = {
  normalizedState: WaitingRoomState
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
  currentModel: string
}): WaitingRoomStateUpdate {
  const normalizedState = normalizeWaitingRoomState(
    options.nextState,
    options.sessions,
    options.catalog,
  )
  const nextModel = normalizedState.modelId || options.currentModel

  return {
    normalizedState,
    nextModel,
    nextEffort: normalizedState.effort,
    shouldPersistProviderPreferences:
      normalizedState.modelId.length > 0
      && (
        options.currentState.modelId !== normalizedState.modelId
        || options.currentState.effort !== normalizedState.effort
      ),
  }
}

export function deriveWaitingRoomActivationDecision(options: {
  state: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
  currentModel: string
}): WaitingRoomActivationDecision {
  const choice = waitingRoomChoice(options.state, options.sessions, options.catalog)
  const launch = {
    model: choice.model?.id ?? options.currentModel,
    effort: choice.effort,
  }

  if (options.state.focus === "new") {
    return { action: "create", launch }
  }

  if (options.state.focus === "join") {
    if (!choice.session) {
      return { action: "error", message: "no session available to join" }
    }

    return {
      action: "join",
      session: choice.session,
      launch,
    }
  }

  return { action: "none" }
}

export function deriveWaitingRoomModelSelectionDecision(options: {
  modelId: string
  state: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
  configuredEffort: string
}): WaitingRoomModelSelectionDecision {
  const selected = catalogModelOptions(options.catalog).find(
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
    ),
    launch: {
      model: selected.id,
      effort,
    },
  }
}

export function deriveWaitingRoomVariantSelectionDecision(options: {
  variant: string
  currentModelId: string
  state: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
}): WaitingRoomVariantSelectionDecision {
  const selected = catalogModelOptions(options.catalog).find(
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
    ),
    launch: {
      model: selected.id,
      effort: options.variant,
    },
  }
}
