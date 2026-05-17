import {
  captureCliDialogFocus,
  restoreCliDialogFocus,
  type CliDialogFocusSnapshot,
  type CliDialogFocusTarget,
} from "./cli-dialog-focus-controller.js"
import type { CliDialogOverlayMode } from "./cli-dialog-overlay.js"
import {
  cliDialogOverlayIsOpen,
  resolveCliDialogOverlayMode,
  type CliDialogOverlayOpenState,
} from "./cli-dialog-overlay-state.js"
import type { FooterFlash } from "./footer-flash-controller.js"
import type { TerminalPairingLinkView } from "./relay-api.js"

type CliDialogOverlayControllerDeps<TFocus extends CliDialogFocusTarget> = {
  getOpenState: () => CliDialogOverlayOpenState
  getCurrentFocus: () => TFocus | null
  getPromptFocus: () => TFocus | null | undefined
  describeFocus?: (target: TFocus | null | undefined) => CliDialogFocusSnapshot | null
  scheduleFocusRestore: (callback: () => void) => void
  setHotkeysOpen: (open: boolean) => void
  setTerminalPairingOpen: (open: boolean) => void
  setSessionBrowserOpen: (open: boolean) => void
  setTerminalPairing: (pairing: TerminalPairingLinkView | null) => void
  setTerminalPairingQrLines: (lines: string[]) => void
  getSessionCount: () => number
  getWaitingRoomSessionIndex: () => number
  setSessionBrowserIndex: (index: number) => void
  clampSessionBrowserIndex: (index: number, sessionCount: number) => number
  renderOverlay: (mode: CliDialogOverlayMode, onDismiss: () => void) => void
  createTerminalPairingLink: () => Promise<TerminalPairingLinkView>
  renderTerminalPairingQr: (pairingLink: string) => Promise<string[]>
  flashFooter: (message: string, tone: FooterFlash["tone"]) => void
  debugHotkey?: (message: string) => void
  logDebug?: (message: string, fields: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
}

export type CliDialogOverlayController = {
  isOpen(): boolean
  savedFocusDebug(): CliDialogFocusSnapshot | null
  closeActive(): void
  render(): void
  closeHotkeys(): void
  closeTerminalPairing(): void
  closeSessionBrowser(): void
  openHotkeys(): void
  openTerminalPairing(): Promise<void>
  openSessionBrowser(): void
  toggleHotkeys(): void
}

export function createCliDialogOverlayController<TFocus extends CliDialogFocusTarget>(
  deps: CliDialogOverlayControllerDeps<TFocus>,
): CliDialogOverlayController {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))
  let savedFocus: TFocus | null = null

  const focusSnapshot = (target: TFocus | null | undefined) => deps.describeFocus?.(target) ?? null
  const focusType = (target: TFocus | null | undefined) => focusSnapshot(target)?.type ?? "none"
  const mode = () => resolveCliDialogOverlayMode(deps.getOpenState())

  const controller: CliDialogOverlayController = {
    isOpen() {
      return cliDialogOverlayIsOpen(deps.getOpenState())
    },
    savedFocusDebug() {
      return focusSnapshot(savedFocus)
    },
    closeActive() {
      const activeMode = mode()
      if (activeMode === "session-browser") {
        controller.closeSessionBrowser()
      } else if (activeMode === "terminal-pairing") {
        controller.closeTerminalPairing()
      } else if (activeMode === "hotkeys") {
        controller.closeHotkeys()
      }
    },
    render() {
      deps.renderOverlay(mode(), controller.closeActive)
    },
    closeHotkeys() {
      if (!deps.getOpenState().hotkeysOpen) {
        return
      }
      const restoreTarget = savedFocus
      deps.debugHotkey?.(`close start open=true saved=${focusType(restoreTarget)} current=${focusType(deps.getCurrentFocus())}`)
      deps.logDebug?.("closing hotkeys overlay", {
        hotkeys_open: true,
        restore_focus: focusSnapshot(restoreTarget),
        current_focus: focusSnapshot(deps.getCurrentFocus()),
      })
      deps.setHotkeysOpen(false)
      controller.render()
      restoreFocusLater(
        restoreTarget,
        () => {
          deps.debugHotkey?.(`close restored saved=${focusType(restoreTarget)} current=${focusType(deps.getCurrentFocus())}`)
          deps.logDebug?.("hotkeys overlay restored focus", {
            restore_focus: focusSnapshot(restoreTarget),
            current_focus: focusSnapshot(deps.getCurrentFocus()),
          })
        },
        () => {
          deps.debugHotkey?.(`close skip-restore saved=${focusType(restoreTarget)}`)
          deps.logDebug?.("hotkeys overlay skipped focus restore", {
            restore_focus: focusSnapshot(restoreTarget),
            current_focus: focusSnapshot(deps.getCurrentFocus()),
          })
        },
      )
    },
    closeTerminalPairing() {
      if (!deps.getOpenState().terminalPairingOpen) {
        return
      }
      const restoreTarget = savedFocus
      deps.setTerminalPairingOpen(false)
      controller.render()
      restoreFocusLater(restoreTarget)
    },
    closeSessionBrowser() {
      if (!deps.getOpenState().sessionBrowserOpen) {
        return
      }
      const restoreTarget = savedFocus
      deps.setSessionBrowserOpen(false)
      controller.render()
      restoreFocusLater(restoreTarget)
    },
    openHotkeys() {
      if (deps.getOpenState().hotkeysOpen) {
        return
      }
      const focused = captureFocus()
      deps.debugHotkey?.(`open start current=${focusType(focused)} saved=${focusType(savedFocus)}`)
      deps.logDebug?.("opening hotkeys overlay", {
        hotkeys_open: false,
        current_focus: focusSnapshot(focused),
        saved_focus: focusSnapshot(savedFocus),
      })
      deps.debugHotkey?.(`open blurred saved=${focusType(savedFocus)} current=${focusType(deps.getCurrentFocus())}`)
      deps.logDebug?.("hotkeys overlay blurred saved focus", {
        saved_focus: focusSnapshot(savedFocus),
        current_focus: focusSnapshot(deps.getCurrentFocus()),
      })
      deps.setHotkeysOpen(true)
      controller.render()
      deps.debugHotkey?.(`open done open=true saved=${focusType(savedFocus)}`)
      deps.logDebug?.("hotkeys overlay opened", {
        hotkeys_open: true,
        saved_focus: focusSnapshot(savedFocus),
        current_focus: focusSnapshot(deps.getCurrentFocus()),
      })
    },
    async openTerminalPairing() {
      if (deps.getOpenState().terminalPairingOpen) {
        return
      }
      captureFocus()
      deps.setTerminalPairing(null)
      deps.setTerminalPairingQrLines([])
      deps.setHotkeysOpen(false)
      deps.setTerminalPairingOpen(true)
      controller.render()
      try {
        const pairing = await deps.createTerminalPairingLink()
        const qrLines = await deps.renderTerminalPairingQr(pairing.pairing_link)
        deps.setTerminalPairing(pairing)
        deps.setTerminalPairingQrLines(qrLines)
        controller.render()
        deps.flashFooter("terminal pairing link created", "info")
      } catch (error) {
        controller.closeTerminalPairing()
        deps.flashFooter(formatError(error), "error")
      }
    },
    openSessionBrowser() {
      if (deps.getOpenState().sessionBrowserOpen) {
        return
      }
      const sessionCount = deps.getSessionCount()
      if (sessionCount === 0) {
        deps.flashFooter("no sessions available to join", "error")
        return
      }
      captureFocus()
      deps.setHotkeysOpen(false)
      deps.setTerminalPairingOpen(false)
      deps.setSessionBrowserIndex(deps.clampSessionBrowserIndex(deps.getWaitingRoomSessionIndex(), sessionCount))
      deps.setSessionBrowserOpen(true)
      controller.render()
      deps.flashFooter("select a session to open, archive, or delete", "info")
    },
    toggleHotkeys() {
      deps.debugHotkey?.(`toggle open=${deps.getOpenState().hotkeysOpen} current=${focusType(deps.getCurrentFocus())}`)
      deps.logDebug?.("toggleHotkeys invoked", {
        hotkeys_open: deps.getOpenState().hotkeysOpen,
        saved_focus: focusSnapshot(savedFocus),
        current_focus: focusSnapshot(deps.getCurrentFocus()),
      })
      if (deps.getOpenState().hotkeysOpen) {
        controller.closeHotkeys()
        return
      }
      if (deps.getOpenState().terminalPairingOpen) {
        controller.closeTerminalPairing()
      }
      if (deps.getOpenState().sessionBrowserOpen) {
        controller.closeSessionBrowser()
      }
      controller.openHotkeys()
    },
  }

  const captureFocus = () => {
    const focused = deps.getCurrentFocus()
    savedFocus = captureCliDialogFocus(focused, deps.getPromptFocus())
    return focused
  }

  const restoreFocusLater = (
    restoreTarget: TFocus | null,
    onRestored?: () => void,
    onSkipped?: () => void,
  ) => {
    deps.scheduleFocusRestore(() => {
      const restored = restoreCliDialogFocus(restoreTarget)
      if (restored) {
        onRestored?.()
      } else {
        onSkipped?.()
      }
      savedFocus = null
    })
  }

  return controller
}
