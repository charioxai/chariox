import { MouseButton } from "@opentui/core"

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
import { waitingRoomManagedMachineDialogRows } from "./waiting-room-start-rows.js"
import { createWaitingRoomManagedMachineDialogController } from "./waiting-room-managed-machine-dialog-controller.js"

type AnyFn = (...args: any[]) => any

export type CliOverlayInteractionCompositionDeps = {
  client: any
  renderer: any
  dimensions: AnyFn
  appLogger: any
  formatError: AnyFn
  debugLogsEnabled: boolean
  isAttached: AnyFn
  availableSessions: AnyFn
  waitingRoomProjects: AnyFn
  sessionBrowserIndex: AnyFn
  setSessionBrowserIndex: AnyFn
  currentFocusedRenderable: AnyFn
  promptInputRefController: {
    current: AnyFn
    currentOrNull: AnyFn
  }
  describeRenderableDebug: AnyFn
  scheduleTimer: AnyFn
  hotkeysOpen: AnyFn
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
  retainPromptFocus: AnyFn
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

  const dialogOverlayController = createCliDialogOverlayController<any, any>({
    getOpenState: () => ({
      hotkeysOpen: deps.hotkeysOpen(),
      terminalPairingOpen: deps.terminalPairingOpen(),
      sessionBrowserOpen: deps.sessionBrowserOpen(),
      managedMachineOpen: deps.managedMachineDialogOpen(),
    }),
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
        sessions: sessionBrowserSessions(),
        normalizeSessionBrowserIndex,
        sessionBrowserProject: sessionBrowserProjectionController.selectedProject(),
        terminalPairing: deps.terminalPairingState(),
        terminalPairingQrLines: deps.terminalPairingQrLines(),
        hotkeySections: hotkeySections(),
        managedMachineRows: mode === "managed-machine"
          ? waitingRoomManagedMachineDialogRows(
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

  const managedMachineDialogController = createWaitingRoomManagedMachineDialogController({
    isOpen: deps.managedMachineDialogOpen,
    state: deps.waitingRoomState,
    sessions: deps.availableSessions,
    catalog: deps.providerCatalogState,
    remote: deps.waitingRoomRemoteState,
    themeRegistry: deps.themeRegistryState,
    setState: deps.reconcileWaitingRoom,
    openOverlay: dialogOverlayController.openManagedMachine,
    closeOverlay: dialogOverlayController.closeManagedMachine,
    renderOverlay: dialogOverlayController.render,
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
  const copySelection = clipboardController.copySelection

  const promptSurfaceMouseController = createPromptSurfaceMouseController({
    delayMs: 0,
    scheduleTimer: deps.scheduleTimer,
    isPrimaryButton: (event: { button: MouseButton }) => event.button === MouseButton.LEFT,
    copySelection,
    retainPromptFocus: deps.retainPromptFocus,
  })

  return {
    assignDialogOverlayBox: dialogOverlayController.assignOverlayBox,
    closeActiveDialogOverlay: dialogOverlayController.closeActive,
    closeHotkeys: dialogOverlayController.closeHotkeys,
    closeSessionBrowserDialog,
    closeTerminalPairingDialog: dialogOverlayController.closeTerminalPairing,
    copyPromptSelection: clipboardController.copyPromptSelection,
    dialogOverlayOpen: dialogOverlayController.isOpen,
    handleHotkeysToggleShortcut: hotkeysToggleController.handle,
    handlePromptSelectionSurfaceMouseUp: promptSurfaceMouseController.handleMouseUp,
    handleSessionBrowserKey: sessionBrowserController.handleKey,
    handleManagedMachineDialogKey: managedMachineDialogController.handleKey,
    openHotkeys: dialogOverlayController.openHotkeys,
    openSessionBrowserDialog: dialogOverlayController.openSessionBrowser,
    openManagedMachineDialog: managedMachineDialogController.open,
    openTerminalPairingDialog: dialogOverlayController.openTerminalPairing,
    renderHotkeysOverlay,
  }
}
