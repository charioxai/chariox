import { isApprovalShortcut } from "./approval-shortcuts.js"
import { shouldCycleFocusOnTabEvent } from "./hotkeys.js"
import type { ParsedShortcut } from "./keybind.js"

export type CliStdinKeyEvent = ParsedShortcut & {
  alt?: boolean
}

// OpenTUI's StdinParser: buffers sequences split across stdin chunks and
// flushes a lone ESC after its timeout.
export type CliStdinParser = {
  push(data: Uint8Array): void
  drain(onEvent: (event: { type: string; key?: CliStdinKeyEvent }) => void): void
}

export type CliStdinKeyControllerDeps = {
  handleNativeSelectionKey?: (event: CliStdinKeyEvent) => boolean
  handleNativeSelectionPaste?: () => boolean
  createStdinParser: (onTimeoutFlush: () => void) => CliStdinParser
  kernelApprovalOwnsInput?: () => boolean
  dialogOverlayOpen: () => boolean
  closeActiveDialogOverlay: () => void
  handleManagedMachineDialogKey?: (event: CliStdinKeyEvent) => boolean
  handleSessionBrowserKey: (event: CliStdinKeyEvent) => boolean
  requestExit: () => void
  focusedInteractionActive: () => boolean
  handleFocusedInteractionKey: (event: CliStdinKeyEvent) => boolean
  handleQueuedPromptKey: (event: CliStdinKeyEvent) => boolean
  promptFocused: () => boolean
  commandCenterOpen: () => boolean
  commandCenterQuery: () => string
  clearCommandCenter: () => void
  toggleWorkspaceScreen: () => void
  isAttached: () => boolean
  workflowScreenActive: () => boolean
  cycleWorkflowCanvasNode: () => void
  handleWorkflowDetailPaneKey: (event: CliStdinKeyEvent) => boolean
  cycleAgentFocus: () => void
  copyPromptSelection: () => boolean
  clearTextSelection?: () => void
  hasPromptSelection?: () => boolean
  hasActiveTurnWork: () => boolean
  requestPromptStop: () => void
  removePromptAttachmentsForEdit: (edit: "backspace" | "delete") => boolean
  currentPromptText: () => string
  pendingAttachmentCount: () => number
  removeLastPendingPromptAttachment: () => void
  handlePromptTurnNavigationKey: (event: CliStdinKeyEvent) => boolean
  handleWaitingRoomKey: (event: CliStdinKeyEvent) => boolean
}

export type CliStdinKeyController = {
  handleData(chunk: Buffer | string): boolean
}

export function createCliStdinKeyController(
  deps: CliStdinKeyControllerDeps,
): CliStdinKeyController {
  // MP-08 / MP-10: decode complete events only. A mouse report split across
  // chunks (after ESC, inside the CSI parameters) must never act as a key.
  const parser = deps.createStdinParser(() => { drain() })
  // MP-08 / MP-10: the renderer already applied prompt editing/selection.
  // Clear retained transcript highlights only; clearing the textarea here
  // would erase the selection just created by Shift+Arrow/Home (or a binding).
  const clearRetainedSelection = () => {
    if (!deps.hasPromptSelection?.()) deps.clearTextSelection?.()
  }
  const drain = () => {
    let handled = false
    parser.drain((event) => {
      // Mouse reports and terminal responses keep the selection.
      if (event.type === "paste" && !deps.handleNativeSelectionPaste?.()) clearRetainedSelection()
      else if (event.type === "key" && event.key) handled = handleKey(event.key) || handled
    })
    return handled
  }
  const handleKey = (event: CliStdinKeyEvent): boolean => {
    if (deps.handleNativeSelectionKey?.(event)) return true
    // MP-08 / MP-10: OpenTUI also emits terminal theme notifications as
    // empty-name keys. Replay native ownership above, then ignore non-keys.
    if (!event.name) return false
    // F6 has a distinct legacy sequence, unlike Ctrl+Shift+C in terminals
    // without extended keyboard support (where it is indistinguishable from Ctrl+C).
    const copyKey = event.name === "f6" || (event.name === "c" && (event.meta || (event.ctrl && event.shift)))
    if (event.eventType !== "release" && !copyKey) clearRetainedSelection()
    // OpenTUI's global key handler owns this dialog. Do not also dispatch
    // its terminal bytes into focused-agent or workflow shortcuts.
    if (deps.kernelApprovalOwnsInput?.() || isApprovalShortcut(event)) return true
    if (event.eventType !== "release" && deps.dialogOverlayOpen() && event.name === "escape") {
      deps.closeActiveDialogOverlay()
      return true
    }
    if (deps.handleManagedMachineDialogKey?.(event)) {
      return true
    }
    if (deps.handleSessionBrowserKey(event)) {
      return true
    }
    if (event.eventType !== "release" && event.ctrl && event.name === "e") {
      deps.requestExit()
      return true
    }
    // The focused textarea receives the same terminal key through its
    // onKeyDown handler. Let it exclusively own an active interaction so a
    // printable key is not appended once here and once by the textarea.
    if (deps.promptFocused() && deps.focusedInteractionActive()) {
      return true
    }
    if (deps.handleFocusedInteractionKey(event)) {
      return true
    }
    // Meta+C shares Alt+C with queued-prompt cancel; a selection wins.
    if (event.eventType !== "release" && copyKey && deps.copyPromptSelection()) {
      return true
    }
    const queuedPromptKeyEvent = queuedPromptKeyEventFromStdin(event)
    if (queuedPromptKeyEvent && deps.handleQueuedPromptKey(queuedPromptKeyEvent)) {
      return true
    }
    if (deps.promptFocused() && deps.commandCenterOpen()) {
      if (event.eventType !== "release" && event.name === "escape") {
        deps.clearCommandCenter()
      }
      return true
    }
    if (event.eventType !== "release" && event.ctrl && event.name === "p") {
      if (deps.dialogOverlayOpen()) {
        return true
      }
      deps.toggleWorkspaceScreen()
      return true
    }
    if (shouldCycleFocusOnTabEvent(event, {
      attached: deps.isAttached(),
      hotkeysOpen: deps.dialogOverlayOpen(),
      promptFocused: deps.promptFocused(),
      commandCenterOpen: deps.commandCenterOpen(),
      commandCenterQuery: deps.commandCenterQuery(),
    })) {
      if (deps.workflowScreenActive()) {
        deps.cycleWorkflowCanvasNode()
      } else {
        deps.cycleAgentFocus()
      }
      return true
    }
    // Nothing selected: still never fall through to Ctrl+C stop/exit.
    if (event.eventType !== "release" && copyKey) {
      return true
    }
    if (event.ctrl && event.name === "c") {
      if (deps.hasActiveTurnWork()) {
        deps.requestPromptStop()
      } else {
        deps.requestExit()
      }
      return true
    }
    if (deps.dialogOverlayOpen()) {
      return true
    }
    if (deps.workflowScreenActive() && deps.handleWorkflowDetailPaneKey(event)) {
      return true
    }
    if (event.eventType !== "release" && deps.promptFocused()) {
      if (event.name === "backspace" && deps.removePromptAttachmentsForEdit("backspace")) {
        return true
      }
      if (event.name === "delete" && deps.removePromptAttachmentsForEdit("delete")) {
        return true
      }
    }
    if (
      event.eventType !== "release"
      && event.name === "backspace"
      && deps.isAttached()
      && !deps.currentPromptText()
      && deps.pendingAttachmentCount() > 0
    ) {
      deps.removeLastPendingPromptAttachment()
      return true
    }
    if (deps.handlePromptTurnNavigationKey(event)) {
      return true
    }
    if (deps.handleWaitingRoomKey(event)) {
      return true
    }
    return false
  }
  return {
    handleData(chunk) {
      parser.push(typeof chunk === "string" ? Buffer.from(chunk) : chunk)
      return drain()
    },
  }
}

function queuedPromptKeyEventFromStdin(event: CliStdinKeyEvent): CliStdinKeyEvent | null {
  if (
    event.ctrl
    || event.shift
  ) {
    return null
  }
  if (
    event.name !== "s"
    && event.name !== "c"
    && event.name !== "j"
    && event.name !== "k"
    && event.name !== "up"
    && event.name !== "down"
  ) {
    return null
  }
  if (event.alt) {
    return event
  }
  if (!event.meta) {
    return null
  }
  return {
    ...event,
    alt: true,
    meta: false,
  }
}
