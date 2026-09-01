import type { CliDialogOverlayMode } from "./cli-dialog-overlay.js"

export type CliDialogOverlayOpenState = {
  managedMachineOpen?: boolean
  hotkeysOpen: boolean
  terminalPairingOpen: boolean
  sessionBrowserOpen: boolean
}

export function resolveCliDialogOverlayMode(state: CliDialogOverlayOpenState): CliDialogOverlayMode {
  if (state.managedMachineOpen) {
    return "managed-machine"
  }
  if (state.sessionBrowserOpen) {
    return "session-browser"
  }
  if (state.terminalPairingOpen) {
    return "terminal-pairing"
  }
  if (state.hotkeysOpen) {
    return "hotkeys"
  }
  return "closed"
}

export function cliDialogOverlayIsOpen(state: CliDialogOverlayOpenState): boolean {
  return resolveCliDialogOverlayMode(state) !== "closed"
}
