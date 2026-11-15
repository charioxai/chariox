import { getRelayStatus } from "./relay-api.js"
import { createSessionProcessController } from "./session-process-controller.js"
import { dialogPageSize, dialogTextWidth } from "./cli-dialog-panel-size.js"
import { createProjectEnvironmentController } from "./project-environment-controller.js"
import { randomUUID } from "node:crypto"
import { MouseButton, type Selection } from "@opentui/core"
import { createEffect, on, onCleanup } from "solid-js"

import { createCliDialogOverlayController } from "./cli-dialog-overlay-controller.js"
import {
  renderCliDialogOverlay,
} from "./cli-dialog-overlay.js"
import { createClipboardController } from "./clipboard-controller.js"
import { createHotkeyDebugReporter } from "./hotkey-debug-reporter.js"
import { createHotkeysToggleController } from "./hotkeys-toggle-controller.js"
import { createPromptSurfaceMouseController } from "./prompt-surface-mouse-controller.js"
import {
  createTerminalPairingLink,
  renderTerminalPairingQr,
} from "./relay-api.js"
import { createSessionBrowserController } from "./session-browser-controller.js"
import {
  clampSessionBrowserIndex,
} from "@chariox/kernel-client/session-browser-policy"
import { createSessionBrowserProjectionController } from "./session-browser-projection-controller.js"
import { waitingRoomProjectsForNavigation } from "./waiting-room-project-rows.js"
import { waitingRoomMachineOfferRows } from "./waiting-room-start-rows.js"
import { createWaitingRoomMachineOfferController } from "./waiting-room-machine-offer-controller.js"

type AnyFn = (...args: any[]) => any

export type CliOverlayInteractionCompositionDeps = {
  client: any
  /** The user's other kernels an Environment can import from. */
  environmentSourceKernels: () => Promise<readonly import("./project-environment-projection.js").EnvironmentSourceKernel[]>
  renderer: any
  dimensions: AnyFn
  appLogger: any
  formatError: AnyFn
  debugLogsEnabled: boolean
  sessionState: AnyFn
  isAttached: AnyFn
  loadSessions: () => Promise<void>
  availableSessions: AnyFn
  waitingRoomProjects: AnyFn
  sessionBrowserIndex: AnyFn
  setSessionBrowserIndex: AnyFn
  currentFocusedRenderable: AnyFn
  promptInputRefController: {
    current: AnyFn
    currentOrNull: AnyFn
    focus: () => void
  }
  describeRenderableDebug: AnyFn
  scheduleTimer: AnyFn
  hotkeysOpen: AnyFn
  subAgentsOpen: AnyFn
  closeSubAgents: AnyFn
  subAgentsRows: AnyFn
  setHotkeysOpen: AnyFn
  terminalPairingOpen: AnyFn
  setTerminalPairingOpen: AnyFn
  terminalPairingState: AnyFn
  setTerminalPairingState: AnyFn
  terminalPairingQrLines: AnyFn
  setTerminalPairingQrLines: AnyFn
  sessionBrowserOpen: AnyFn
  setSessionBrowserOpen: AnyFn
  managedMachineDialogOpen: AnyFn
  setManagedMachineDialogOpen: AnyFn
  waitingRoomState: AnyFn
  reconcileWaitingRoom: AnyFn
  waitingRoomRemoteState: AnyFn
  providerCatalogState: AnyFn
  themeRegistryState: AnyFn
  options: any
  flashFooter: AnyFn
  attachBinding: AnyFn
  applyWaitingRoomSessionLifecycleAction: AnyFn
}

export function createCliOverlayInteractionComposition(deps: CliOverlayInteractionCompositionDeps) {
  const sessionBrowserProjectionController = createSessionBrowserProjectionController({
    isAttached: deps.isAttached,
    availableSessions: deps.availableSessions,
    selectedIndex: deps.sessionBrowserIndex,
    setSelectedIndex: deps.setSessionBrowserIndex,
    selectedProject: () => {
      const state = deps.waitingRoomState()
      return state.focus === "project-entry"
        ? waitingRoomProjectsForNavigation(deps.waitingRoomProjects())[state.projectIndex ?? 0] ?? null
        : null
    },
  })
  const hotkeySections = sessionBrowserProjectionController.hotkeySections
  const sessionBrowserSessions = sessionBrowserProjectionController.sessions
  const normalizeSessionBrowserIndex = sessionBrowserProjectionController.normalizeIndex

  const hotkeyDebugReporter = createHotkeyDebugReporter({
    debugLogsEnabled: deps.debugLogsEnabled,
    logDebug: (message, fields) => deps.appLogger?.debug(message, fields),
    flashFooter: deps.flashFooter,
  })
  const hotkeyDebug = hotkeyDebugReporter.report

  const processController = createSessionProcessController({
    processes: () => deps.isAttached() ? deps.sessionState().processes ?? [] : [],
    ownerLabel: row => row.owner.kind === "agent"
      ? deps.sessionState().agents?.find((agent: {id: string}) => agent.id === row.owner.id)?.alias ?? row.owner.id
      : row.owner.id,
    render: () => dialogOverlayController.render(), schedule: deps.scheduleTimer,
    pageSize: () => dialogPageSize(deps.dimensions().height),
  })
  const environmentController = createProjectEnvironmentController({
    client: () => deps.client.currentClient(),
    kernelId: async client => (await getRelayStatus(client as Parameters<typeof getRelayStatus>[0])).daemon_id,
    otherKernels: deps.environmentSourceKernels,
    pageSize: () => dialogPageSize(deps.dimensions().height),
    textWidth: () => dialogTextWidth(deps.dimensions().width),
    render: () => dialogOverlayController.render(),
    schedule: deps.scheduleTimer,
    newOperationId: randomUUID,
    now: Date.now,
  })
  // The dialog shows one kernel's Environment; switching kernels ends it.
  onCleanup(deps.client.onClientChanged(() => { if (environmentController.isOpen()) environmentController.close() }))
  const handleEnvironmentPaste = (event: { text: string; preventDefault(): void }) => {
    if (environmentController.handlePaste(event.text)) event.preventDefault()
  }
  deps.renderer.keyInput.on("paste", handleEnvironmentPaste)
  onCleanup(() => deps.renderer.keyInput.off("paste", handleEnvironmentPaste))
  const dialogOverlayController = createCliDialogOverlayController<any, any>({
    getOpenState: () => ({
      processesOpen: processController.isOpen(),
      environmentOpen: environmentController.isOpen(),
      hotkeysOpen: deps.hotkeysOpen(),
      subAgentsOpen: deps.subAgentsOpen(),
      terminalPairingOpen: deps.terminalPairingOpen(),
      sessionBrowserOpen: deps.sessionBrowserOpen(),
      managedMachineOpen: deps.managedMachineDialogOpen(),
    }),
    closeProcesses: processController.close,
    closeSubAgents: deps.closeSubAgents,
    closeEnvironment: environmentController.dismiss,
    getCurrentFocus: deps.currentFocusedRenderable,
    getPromptFocus: () => deps.promptInputRefController.current() as any,
    describeFocus: deps.describeRenderableDebug,
    scheduleFocusRestore: (callback) => {
      deps.scheduleTimer(callback, 1)
    },
    setHotkeysOpen: deps.setHotkeysOpen,
    setTerminalPairingOpen: deps.setTerminalPairingOpen,
    setSessionBrowserOpen: deps.setSessionBrowserOpen,
    setManagedMachineOpen: deps.setManagedMachineDialogOpen,
    onManagedMachineClosed: () => {
      deps.reconcileWaitingRoom({
        ...deps.waitingRoomState(),
        focus: "launch-machine",
      })
    },
    setTerminalPairing: deps.setTerminalPairingState,
    setTerminalPairingQrLines: deps.setTerminalPairingQrLines,
    loadSessions: deps.loadSessions,
    getSessionCount: () => sessionBrowserSessions().length,
    getWaitingRoomSessionIndex: () => deps.waitingRoomState().sessionIndex,
    setSessionBrowserIndex: deps.setSessionBrowserIndex,
    clampSessionBrowserIndex,
    renderOverlay: (mode, onDismiss, overlayBox) => {
      renderCliDialogOverlay({
        overlayBox,
        renderer: deps.renderer,
        dimensions: deps.dimensions(),
        mode,
        onDismiss,
        processConfirmation: processController.confirming(),
        processLines: processController.visibleLines(),
        environmentView: environmentController.view(),
        subAgentsRows: mode === "sub-agents" ? deps.subAgentsRows() : [],
        sessions: sessionBrowserSessions(),
        normalizeSessionBrowserIndex,
        sessionBrowserProject: sessionBrowserProjectionController.selectedProject(),
        terminalPairing: deps.terminalPairingState(),
        terminalPairingQrLines: deps.terminalPairingQrLines(),
        hotkeySections: hotkeySections(),
        managedMachineRows: mode === "managed-machine"
          ? waitingRoomMachineOfferRows(
              deps.waitingRoomState(),
              deps.waitingRoomRemoteState(),
            )
          : [],
      })
    },
    createTerminalPairingLink: () => createTerminalPairingLink(deps.client, "cli"),
    renderTerminalPairingQr,
    flashFooter: (message, tone) => deps.flashFooter(message, tone),
    debugHotkey: (message) => hotkeyDebug(message),
    logDebug: (message, fields) => deps.appLogger?.debug(message, fields),
    formatError: deps.formatError,
  })

  const renderHotkeysOverlay = dialogOverlayController.render
  const closeSessionBrowserDialog = dialogOverlayController.closeSessionBrowser

  createEffect(on(
    () => deps.sessionBrowserOpen() ? [sessionBrowserSessions(), deps.sessionBrowserIndex()] : null,
    state => { if (state) renderHotkeysOverlay() },
  ))

  const sessionBrowserController = createSessionBrowserController({
    isOpen: deps.sessionBrowserOpen,
    visibleSessions: sessionBrowserSessions,
    availableSessions: deps.availableSessions,
    selectedProject: sessionBrowserProjectionController.selectedProject,
    normalizeSelectedIndex: normalizeSessionBrowserIndex,
    setSelectedIndex: (updater) => deps.setSessionBrowserIndex((index: number) => updater(index)),
    waitingRoomState: deps.waitingRoomState,
    providerCatalog: deps.providerCatalogState,
    currentProvider: () => deps.options.provider ?? "opencode",
    currentModel: () => deps.options.model,
    closeDialog: closeSessionBrowserDialog,
    renderOverlay: renderHotkeysOverlay,
    flashFooter: deps.flashFooter,
    attachSession: (session, createNew, launch) => deps.attachBinding(session, createNew, launch),
    applyLifecycleAction: deps.applyWaitingRoomSessionLifecycleAction,
    formatError: deps.formatError,
  })

  const managedMachineDialogController = createWaitingRoomMachineOfferController({
    isOpen: deps.managedMachineDialogOpen,
    state: deps.waitingRoomState,
    remote: deps.waitingRoomRemoteState,
    openOverlay: dialogOverlayController.openManagedMachine,
    closeOverlay: dialogOverlayController.closeManagedMachine,
  })

  const hotkeysToggleController = createHotkeysToggleController({
    hotkeysOpen: deps.hotkeysOpen,
    toggleHotkeys: dialogOverlayController.toggleHotkeys,
    debugHotkey: hotkeyDebug,
    logDebug: (message, fields) => deps.appLogger?.debug(message, fields),
    currentFocus: deps.currentFocusedRenderable,
    describeFocus: deps.describeRenderableDebug,
    savedFocusDebug: () => dialogOverlayController.savedFocusDebug(),
  })

  const clipboardController = createClipboardController({
    renderer: deps.renderer,
    promptInput: deps.promptInputRefController.currentOrNull,
    flashFooter: deps.flashFooter,
    logWarning: (message, fields) => deps.appLogger?.warn(message, fields),
    formatError: deps.formatError,
  })

  const promptSurfaceMouseController = createPromptSurfaceMouseController({
    delayMs: 0,
    scheduleTimer: deps.scheduleTimer,
    isPrimaryButton: (event: { button: MouseButton; isDragging?: boolean }) => event.button === MouseButton.LEFT,
    copyText: clipboardController.copyTextWithFeedback,
    retainPromptFocus: () => deps.promptInputRefController.focus(),
  })

  const onSelection = (selection: Selection) => promptSurfaceMouseController.handleSelection(selection.getSelectedText())
  deps.renderer.on("selection", onSelection)
  onCleanup(() => deps.renderer.off("selection", onSelection))

  return {
    assignDialogOverlayBox: dialogOverlayController.assignOverlayBox,
    closeActiveDialogOverlay: dialogOverlayController.closeActive,
    closeHotkeys: dialogOverlayController.closeHotkeys,
    closeSessionBrowserDialog,
    closeTerminalPairingDialog: dialogOverlayController.closeTerminalPairing,
    captureCopyKey: clipboardController.captureCopyKey,
    capturePaste: clipboardController.capturePaste,
    replayCopyKey: clipboardController.replayCopyKey,
    discardCopyInput: clipboardController.discardCopyInput,
    copyPromptSelection: clipboardController.copyCapturedSelection,
    dialogOverlayOpen: dialogOverlayController.isOpen,
    handleHotkeysToggleShortcut: hotkeysToggleController.handle,
    handlePromptSelectionSurfaceMouseUp: promptSurfaceMouseController.handleMouseUp,
    handleSessionBrowserKey: sessionBrowserController.handleKey,
    handleManagedMachineDialogKey: managedMachineDialogController.handleKey,
    confirmProcessStop: processController.confirmAgentStop,
    openSessionProcesses: processController.open,
    handleSessionProcessesKey: processController.handleKey,
    openProjectEnvironment: environmentController.open,
    handleProjectEnvironmentKey: environmentController.handleKey,
    openHotkeys: dialogOverlayController.openHotkeys,
    openSessionBrowserDialog: dialogOverlayController.openSessionBrowser,
    openManagedMachineDialog: managedMachineDialogController.open,
    openTerminalPairingDialog: dialogOverlayController.openTerminalPairing,
    renderHotkeysOverlay,
  }
}
