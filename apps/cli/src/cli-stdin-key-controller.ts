import { isSelectionCopyKey } from "./selection-copy-key.js"
import { isApprovalShortcut } from "./approval-shortcuts.js"
import { shouldCycleFocusOnTabEvent } from "./hotkeys.js"
import type { ParsedShortcut } from "./keybind.js"

export type CliStdinKeyEvent = ParsedShortcut & {
  alt?: boolean
}

export type CliStdinInputEvent = { type: string; key?: CliStdinKeyEvent }

export type CliStdinParser = {
  push(data: Uint8Array): void
  drain(onEvent: (event: CliStdinInputEvent) => void): void
}

export type CliStdinKeyControllerDeps = {
  handleNativeSelectionKey?: (event: CliStdinKeyEvent) => boolean
  handleNativeSelectionPaste?: () => boolean
  createStdinParser?: (onTimeoutFlush: () => void) => CliStdinParser
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
  replayCopyKey?: (event: CliStdinKeyEvent) => void
  copyPromptSelection: () => boolean
  flushTextSelectionRebuild?: () => void
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
  handleEvent(event: CliStdinInputEvent): boolean
}

export function createCliStdinKeyController(
  deps: CliStdinKeyControllerDeps,
): CliStdinKeyController {
  // MP-08 / MP-10: production uses renderer-decoded events. The optional
  // byte source supports isolated parser regressions without a second live parser.
  const parser = deps.createStdinParser?.(() => { drain() })
  // MP-08 / MP-10: decoded renderer events own selection mutation. Deferred
  // replay only flushes rebuilds; it must preserve a newer mouse
  // selection created by later events in this same stdin chunk.
  const flushSelectionRebuild = () => {
    if (!deps.hasPromptSelection?.()) deps.flushTextSelectionRebuild?.()
  }
  const drain = () => {
    let handled = false
    parser?.drain((event) => { handled = handleEvent(event) || handled })
    return handled
  }
  const handleEvent = (event: CliStdinInputEvent): boolean => {
    if (event.type === "paste" && !deps.handleNativeSelectionPaste?.()) flushSelectionRebuild()
    return event.type === "key" && event.key ? handleKey(event.key) : false
  }
  const handleKey = (event: CliStdinKeyEvent): boolean => {
    if (deps.handleNativeSelectionKey?.(event)) return true
    deps.replayCopyKey?.(event)
    // MP-08 / MP-10: OpenTUI also emits terminal theme notifications as
    // empty-name keys. Replay native ownership above, then ignore non-keys.
    if (!event.name) return false
    // F6 has a distinct legacy sequence, unlike Ctrl+Shift+C in terminals
    // without extended keyboard support (where it is indistinguishable from Ctrl+C).
    const copyKey = isSelectionCopyKey(event)
    if (event.eventType !== "release" && !copyKey) flushSelectionRebuild()
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
    handleEvent,
    handleData(chunk) {
      parser?.push(typeof chunk === "string" ? Buffer.from(chunk) : chunk)
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
