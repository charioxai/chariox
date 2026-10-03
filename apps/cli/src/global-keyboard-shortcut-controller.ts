export type GlobalKeyboardShortcutEvent = {
  name: string
  ctrl?: boolean
  preventDefault: () => void
  stopPropagation: () => void
}

export type GlobalKeyboardShortcutControllerDeps = {
  handleKernelApprovalKey?: (event: GlobalKeyboardShortcutEvent) => boolean
  handleHotkeysToggleShortcut: (source: "keyboard", event: GlobalKeyboardShortcutEvent) => boolean
  dialogOverlayOpen: () => boolean
  requestExit: () => void
  requestPromptStop: () => void
  hasActiveTurnWork: () => boolean
}

export type GlobalKeyboardShortcutController = {
  handleKey(event: GlobalKeyboardShortcutEvent): boolean
  handleSigint(): void
}

// Ctrl+C, Ctrl+E and Escape on a dialog overlay are owned by the stdin key
// controller, which sees the same terminal bytes; handling them here too
// would run each action twice.
export function createGlobalKeyboardShortcutController(
  deps: GlobalKeyboardShortcutControllerDeps,
): GlobalKeyboardShortcutController {
  return {
    handleSigint() {
      if (deps.hasActiveTurnWork()) {
        deps.requestPromptStop()
      } else {
        deps.requestExit()
      }
    },
    handleKey(event) {
      if (deps.handleKernelApprovalKey?.(event)) return true
      if (deps.handleHotkeysToggleShortcut("keyboard", event)) {
        return true
      }
      if (deps.dialogOverlayOpen()) {
        event.preventDefault()
        event.stopPropagation()
        return true
      }
      return false
    },
  }
}
