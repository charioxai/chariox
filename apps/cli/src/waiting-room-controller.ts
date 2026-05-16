import {
  catalogModelOptions,
  selectConfiguredVariant,
  type BackendProviderId,
  type ProviderCatalog,
} from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import type { ThemeRegistry } from "./theme-registry.js"
import {
  cycleWaitingRoomValue,
  moveWaitingRoomFocus,
  normalizeWaitingRoomState,
  waitingRoomRemoteKernelCanDelete,
  waitingRoomRemoteMachineCanDelete,
  waitingRoomChoice,
  type WaitingRoomRemoteState,
  type WaitingRoomState,
} from "./waiting-room.js"
import {
  clearStagedWaitingRoomWorktreeSelection,
  stageWaitingRoomWorktreeSelection,
} from "./waiting-room-worktrees.js"

export type WaitingRoomLaunchConfig = {
  provider: BackendProviderId
  model: string
  effort: string
  sliceRef?: string | null
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

export type WaitingRoomSessionLifecycleAction = "archive" | "delete"

export type WaitingRoomKeyEvent = {
  name: string
  eventType?: string | undefined
  ctrl?: boolean | undefined
  meta?: boolean | undefined
  alt?: boolean | undefined
  super?: boolean | undefined
}

export type WaitingRoomArrowKey = "up" | "down" | "left" | "right"

export type WaitingRoomKeyNavigationDecision =
  | { action: "ignore" }
  | { action: "release"; key: WaitingRoomArrowKey; nextState: WaitingRoomState }
  | { action: "navigate"; key: WaitingRoomArrowKey; nextState: WaitingRoomState }

export type WaitingRoomSessionLifecycleDecision =
  | { action: WaitingRoomSessionLifecycleAction; session: SessionListEntry }
  | { action: "archive-all"; sessions: SessionListEntry[] }
  | { action: "error"; message: string }

export type WaitingRoomDeleteDecision =
  | { action: "delete-session"; session: SessionListEntry }
  | { action: "delete-all-sessions"; sessions: SessionListEntry[] }
  | { action: "delete-machine"; machineId: string; label: string }
  | { action: "delete-kernel"; kernelId: string; label: string }
  | { action: "error"; message: string }

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

export function deriveWaitingRoomKeyNavigationDecision(options: {
  event: WaitingRoomKeyEvent
  state: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
  remote?: WaitingRoomRemoteState
  themeRegistry?: ThemeRegistry
}): WaitingRoomKeyNavigationDecision {
  const key = waitingRoomArrowKeyForEvent(options.event)
  if (!key) {
    return { action: "ignore" }
  }

  const next = {
    ...options.state,
    keyState: {
      ...options.state.keyState,
      [key]: options.event.eventType !== "release",
    },
  }
  if (options.event.eventType === "release") {
    return { action: "release", key, nextState: next }
  }
  if (key === "up" || key === "down") {
    return {
      action: "navigate",
      key,
      nextState: moveWaitingRoomFocus(
        next,
        options.sessions,
        key === "up" ? -1 : 1,
        options.remote,
      ),
    }
  }
  return {
    action: "navigate",
    key,
    nextState: cycleWaitingRoomValue(
      next,
      options.sessions,
      options.catalog,
      key === "left" ? -1 : 1,
      options.themeRegistry,
      options.remote,
    ),
  }
}

export function waitingRoomSessionLifecycleActionForEvent(options: {
  event: WaitingRoomKeyEvent
  promptFocused: boolean
}): WaitingRoomSessionLifecycleAction | null {
  const event = options.event
  if (
    event.eventType === "release"
    || event.ctrl
    || event.meta
    || event.alt
    || event.super
    || options.promptFocused
  ) {
    return null
  }
  if (event.name === "a") {
    return "archive"
  }
  if (event.name === "d" || event.name === "delete") {
    return "delete"
  }
  return null
}

function waitingRoomArrowKeyForEvent(event: WaitingRoomKeyEvent): WaitingRoomArrowKey | null {
  return event.name === "up" || event.name === "down" || event.name === "left" || event.name === "right"
    ? event.name
    : null
}

export function deriveWaitingRoomStateUpdate(options: {
  currentState: WaitingRoomState
  nextState: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
  remote?: WaitingRoomRemoteState
  themeRegistry?: ThemeRegistry
  currentProvider: BackendProviderId
  currentModel: string
}): WaitingRoomStateUpdate {
  const normalizedState = normalizeWaitingRoomState(
    options.nextState,
    options.sessions,
    options.catalog,
    options.themeRegistry,
    options.remote,
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
  remote?: WaitingRoomRemoteState
}): WaitingRoomActivationDecision {
  const choice = waitingRoomChoice(options.state, options.sessions, options.catalog, options.remote)
  const launch = {
    provider: choice.providerId ?? options.currentProvider,
    model: choice.model?.id ?? options.currentModel,
    effort: choice.effort,
    ...(choice.sliceRef ? { sliceRef: choice.sliceRef } : {}),
  }

  if (options.state.focus === "join-sessions") {
    clearStagedWaitingRoomWorktreeSelection()
    return { action: "none" }
  }

  if (options.state.focus !== "session") {
    const worktreeSelection = stageWaitingRoomWorktreeSelection(options.state.worktreeSelectionId)
    if (!worktreeSelection.ok) {
      return { action: "error", message: worktreeSelection.message }
    }
    return { action: "create", launch }
  }

  clearStagedWaitingRoomWorktreeSelection()

  if (!choice.session) {
    return { action: "error", message: "no session available to join" }
  }

  return {
    action: "join",
    session: choice.session,
    launch,
  }
}

export function deriveWaitingRoomSessionLifecycleDecision(options: {
  action: WaitingRoomSessionLifecycleAction
  state: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
}): WaitingRoomSessionLifecycleDecision {
  const choice = waitingRoomChoice(options.state, options.sessions, options.catalog)
  if (options.state.focus === "join-sessions" && options.action === "archive") {
    const sessions = options.sessions.filter((session) => session.status !== "Ended")
    if (sessions.length === 0) {
      return {
        action: "error",
        message: "no sessions available to archive",
      }
    }
    return {
      action: "archive-all",
      sessions,
    }
  }
  if (options.state.focus !== "session" || !choice.session) {
    return {
      action: "error",
      message: `select a session to ${options.action}`,
    }
  }

  return {
    action: options.action,
    session: choice.session,
  }
}

export function deriveWaitingRoomDeleteDecision(options: {
  state: WaitingRoomState
  sessions: SessionListEntry[]
  catalog: ProviderCatalog
  remote?: WaitingRoomRemoteState
}): WaitingRoomDeleteDecision {
  const choice = waitingRoomChoice(options.state, options.sessions, options.catalog, options.remote)
  if (options.state.focus === "join-sessions") {
    const sessions = options.sessions.filter((session) => session.status !== "Ended")
    if (sessions.length === 0) {
      return {
        action: "error",
        message: "no sessions available to delete",
      }
    }
    return {
      action: "delete-all-sessions",
      sessions,
    }
  }

  if (options.state.focus === "session" && choice.session) {
    return {
      action: "delete-session",
      session: choice.session,
    }
  }

  if (options.state.focus === "machine" && choice.remoteMachine) {
    const label = choice.remoteMachine.display_name
      ?? choice.remoteMachine.registry_alias
      ?? choice.remoteMachine.machine_alias
      ?? choice.remoteMachine.machine_id
    if (!waitingRoomRemoteMachineCanDelete(choice.remoteMachine)) {
      return {
        action: "error",
        message: `machine ${label} is active`,
      }
    }
    return {
      action: "delete-machine",
      machineId: choice.remoteMachine.machine_id,
      label,
    }
  }

  if (options.state.focus === "remote-kernel" && choice.remoteKernel) {
    const label = choice.remoteKernel.relay_alias
      ?? choice.remoteKernel.kernel_alias
      ?? choice.remoteKernel.kernel_id
    if (!waitingRoomRemoteKernelCanDelete(choice.remoteKernel)) {
      return {
        action: "error",
        message: `kernel ${label} is active`,
      }
    }
    return {
      action: "delete-kernel",
      kernelId: choice.remoteKernel.kernel_id,
      label,
    }
  }

  return {
    action: "error",
    message: "select a session, inactive machine, or inactive kernel to delete",
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
