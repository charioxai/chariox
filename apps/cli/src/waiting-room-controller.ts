import {
  catalogModelOptions,
  selectConfiguredVariant,
  type BackendProviderId,
  type ProviderCatalog,
} from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import type { ThemeRegistry } from "./theme-registry.js"
import { waitingRoomChoice } from "./waiting-room-choice.js"
import { moveWaitingRoomFocus } from "./waiting-room-focus-targets.js"
import {
  waitingRoomRemoteKernelCanDelete,
  waitingRoomRemoteMachineCanDelete,
} from "./waiting-room-remote-rows.js"
import { waitingRoomLaunchPlacement } from "@arroba/kernel-client/waiting-room-runtime-placement"
import { waitingRoomAllSlices } from "./waiting-room-slice-rows.js"
import { waitingRoomSliceSelectionUnavailable, waitingRoomSlices } from "./waiting-room-slices.js"
import { normalizeWaitingRoomState } from "./waiting-room-state.js"
import { cycleWaitingRoomValue } from "./waiting-room-value-cycling.js"
import {
  type WaitingRoomRemoteState,
  type WaitingRoomState,
  type WaitingRoomTerminalType,
} from "./waiting-room-types.js"
import {
  clearStagedWaitingRoomWorktreeSelection,
  stageWaitingRoomWorktreeSelection,
} from "./waiting-room-worktrees.js"

export type WaitingRoomLaunchConfig = {
  provider: BackendProviderId
  model: string
  effort: string
  ownerMachineRef?: string | null
  ownerKernelRef?: string | null
  machineRef?: string | null
  kernelRef?: string | null
  workerKernelRef?: string | null
  workspaceLiveSyncMode?: "off" | "managed" | "tracked"
  sliceRef?: string | null
  sliceCreate?: { displayMode: "headless" | "headed" } | null
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
  | { action: "import-external-session"; externalSessionId: string }
  | { action: "error"; message: string }
  | { action: "none" }

export type WaitingRoomCreateSessionDecision =
  | { action: "create"; launch: WaitingRoomLaunchConfig }
  | { action: "error"; message: string }

export type WaitingRoomControlActivationDecision =
  | { action: "cloud" }
  | { action: "browse-kernel"; kernelId: string; machineId: string; label: string }
  | { action: "stage-command"; command: string; message: string }
  | { action: "info"; message: string }
  | { action: "error"; message: string }
  | { action: "open-terminal-pairing" }
  | { action: "open-session-browser" }
  | { action: "load-older-external-sessions" }
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
  | { action: "delete-slice"; sliceId: string; label: string }
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

function formatWaitingRoomTerminalTypeLabel(value: WaitingRoomTerminalType): string {
  switch (value) {
    case "web":
      return "Web terminal"
    case "ios":
      return "iOS terminal"
    case "android":
      return "Android terminal"
    case "cli":
    default:
      return "CLI"
  }
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
    ownerMachineRef: choice.machineRef,
    ownerKernelRef: choice.kernelRef,
    ...(choice.workerKernelRef ? { workerKernelRef: choice.workerKernelRef } : {}),
    workspaceLiveSyncMode: options.state.workspaceLiveSyncMode,
    ...(choice.sliceRef ? { sliceRef: choice.sliceRef } : {}),
    ...(choice.sliceCreate ? { sliceCreate: choice.sliceCreate } : {}),
  }

  if (options.state.focus === "join-sessions") {
    clearStagedWaitingRoomWorktreeSelection()
    return { action: "none" }
  }

  if (options.state.focus === "external-session") {
    clearStagedWaitingRoomWorktreeSelection()
    if (!choice.externalProviderSession) {
      return { action: "error", message: "no unattached agent available to open" }
    }
    return {
      action: "import-external-session",
      externalSessionId: choice.externalProviderSession.external_session_id,
    }
  }

  if (options.state.focus !== "session") {
    const worktreeSelection = stageWaitingRoomWorktreeSelection(options.state.worktreeSelectionId)
    if (!worktreeSelection.ok) {
      return { action: "error", message: worktreeSelection.message }
    }
    const staleSlice = waitingRoomUnavailableSliceMessage(options.state, options.remote)
    if (staleSlice) {
      return { action: "error", message: staleSlice }
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

export function deriveWaitingRoomCreateSessionDecision(options: {
  state: WaitingRoomState
  catalog: ProviderCatalog
  currentProvider: BackendProviderId
  currentModel: string
  remote?: WaitingRoomRemoteState
}): WaitingRoomCreateSessionDecision {
  const choice = waitingRoomChoice(options.state, [], options.catalog, options.remote)
  const worktreeSelection = stageWaitingRoomWorktreeSelection(options.state.worktreeSelectionId)
  if (!worktreeSelection.ok) {
    return { action: "error", message: worktreeSelection.message }
  }
  const staleSlice = waitingRoomUnavailableSliceMessage(options.state, options.remote)
  if (staleSlice) {
    return { action: "error", message: staleSlice }
  }
  const launch = {
    provider: choice.providerId ?? options.currentProvider,
    model: choice.model?.id ?? options.currentModel,
    effort: choice.effort,
    ownerMachineRef: choice.machineRef,
    ownerKernelRef: choice.kernelRef,
    ...(choice.workerKernelRef ? { workerKernelRef: choice.workerKernelRef } : {}),
    workspaceLiveSyncMode: options.state.workspaceLiveSyncMode,
    ...(choice.sliceRef ? { sliceRef: choice.sliceRef } : {}),
    ...(choice.sliceCreate ? { sliceCreate: choice.sliceCreate } : {}),
  }
  return {
    action: "create",
    launch,
  }
}

function waitingRoomUnavailableSliceMessage(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState | undefined,
): string | null {
  const placement = waitingRoomLaunchPlacement(state, remote)
  const slices = waitingRoomSlices(remote, {
    worktreeSelectionId: state.worktreeSelectionId,
    selectedMachineRef: placement.machineRef,
    selectedKernelRef: placement.kernelRef,
  })
  return waitingRoomSliceSelectionUnavailable(state.sliceSelectionId, slices)
    ? "Selected slice is unavailable for this worktree/kernel. Choose an available slice, new slice, or off."
    : null
}

export function deriveWaitingRoomControlActivationDecision(options: {
  state: WaitingRoomState
  workspacePath: string
  worktreePath: string
  remote?: WaitingRoomRemoteState
}): WaitingRoomControlActivationDecision {
  const remote = options.remote ?? {}
  switch (options.state.focus) {
    case "relay":
      return { action: "cloud" }
    case "workspace":
      return {
        action: "stage-command",
        command: `/workspace ${options.workspacePath}`,
        message: "edit the workspace path and press Enter",
      }
    case "worktree":
      return {
        action: "stage-command",
        command: `/worktree ${options.worktreePath}`,
        message: "edit the worktree path and press Enter",
      }
    case "live-sync":
      return {
        action: "info",
        message: "Use left/right to choose off, managed, or tracked before starting the session. Live sync applies only to the selected workspace/worktree; other repositories stay unrestricted.",
      }
    case "collaborators":
      return {
        action: "info",
        message: options.remote?.collaborationBackend === "cloud"
          ? "Open Arroba Cloud for saved collaborators before starting. After session start, use /cloud invite create."
          : "Create the session first, then use /relay invite create. Arroba Cloud adds saved collaborators and pre-session invites.",
      }
    case "machine": {
      const machine = remote.machines?.[options.state.machineIndex]
      if (!machine) {
        return { action: "error", message: "no machine selected" }
      }
      const label = machine.registry_alias ?? machine.machine_alias ?? machine.display_name ?? machine.machine_id
      if (machine.online === false || machine.pending || machine.kernel_count === 0) {
        return { action: "info", message: `press D twice to delete machine ${label}` }
      }
      const target = machine.registry_alias ?? machine.machine_alias ?? machine.machine_id
      return {
        action: "stage-command",
        command: `/machine kernels ${target}`,
        message: `press Enter to list kernels for ${label}`,
      }
    }
    case "remote-kernel": {
      const kernel = remote.kernels?.[options.state.remoteKernelIndex]
      if (!kernel) {
        return { action: "error", message: "no kernel selected" }
      }
      return {
        action: "browse-kernel",
        kernelId: kernel.kernel_id,
        machineId: kernel.machine_id,
        label: kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id,
      }
    }
    case "slice-entry": {
      const slice = waitingRoomAllSlices(remote)[options.state.sliceIndex ?? 0]
      if (!slice) {
        return { action: "error", message: "no slice selected" }
      }
      return {
        action: "stage-command",
        command: `/slice status ${slice.id}`,
        message: `press Enter to show slice ${slice.name || slice.id}`,
      }
    }
    case "terminal": {
      const terminal = remote.terminals?.[options.state.terminalIndex]
      if (!terminal) {
        return { action: "error", message: "no terminal selected" }
      }
      return {
        action: "info",
        message: `${terminal.terminal_id} is a ${formatWaitingRoomTerminalTypeLabel(terminal.terminal_type)}`,
      }
    }
    case "add-terminal":
      return { action: "open-terminal-pairing" }
    case "join-sessions":
      return { action: "open-session-browser" }
    case "external-session-more":
      return { action: "load-older-external-sessions" }
    default:
      return { action: "none" }
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
    const label = choice.remoteMachine.registry_alias
      ?? choice.remoteMachine.machine_alias
      ?? choice.remoteMachine.display_name
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

  if (options.state.focus === "slice-entry" && choice.sliceInventory) {
    const label = choice.sliceInventory.name || choice.sliceInventory.id
    const activeAgents = choice.sliceInventory.agent_ids?.length ?? 0
    if (activeAgents > 0) {
      return {
        action: "error",
        message: `slice ${label} has ${activeAgents} active agent${activeAgents === 1 ? "" : "s"}`,
      }
    }
    return {
      action: "delete-slice",
      sliceId: choice.sliceInventory.id,
      label,
    }
  }

  return {
    action: "error",
    message: "select a session, inactive machine, inactive kernel, or idle slice to delete",
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
