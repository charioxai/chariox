export type GlobalKeyboardShortcutEvent = {
  name: string
  ctrl?: boolean
  preventDefault: () => void
  stopPropagation: () => void
}

export type GlobalKeyboardShortcutControllerDeps = {
  handleHotkeysToggleShortcut: (source: "keyboard", event: GlobalKeyboardShortcutEvent) => boolean
  dialogOverlayOpen: () => boolean
  closeActiveDialogOverlay: () => void
  requestExit: () => void
  requestPromptStop: () => void
  activePrompt: () => unknown
}

export type GlobalKeyboardShortcutController = {
  handleKey(event: GlobalKeyboardShortcutEvent): boolean
  handleSigint(): void
}

export function createGlobalKeyboardShortcutController(
  deps: GlobalKeyboardShortcutControllerDeps,
): GlobalKeyboardShortcutController {
  const consume = (event: GlobalKeyboardShortcutEvent) => {
    event.preventDefault()
    event.stopPropagation()
  }
  const requestPromptStopOrExit = () => {
    if (deps.activePrompt()) {
      deps.requestPromptStop()
    } else {
      deps.requestExit()
    }
  }

  return {
    handleSigint() {
      requestPromptStopOrExit()
    },
    handleKey(event) {
      if (deps.handleHotkeysToggleShortcut("keyboard", event)) {
        return true
      }
      if (deps.dialogOverlayOpen() && event.name === "escape") {
        consume(event)
        deps.closeActiveDialogOverlay()
        return true
      }
      if (event.ctrl && event.name === "e") {
        consume(event)
        deps.requestExit()
        return true
      }
      if (event.ctrl && event.name === "c") {
        consume(event)
        requestPromptStopOrExit()
        return true
      }
      if (deps.dialogOverlayOpen()) {
        consume(event)
        return true
      }
      return false
    },
  }
}
