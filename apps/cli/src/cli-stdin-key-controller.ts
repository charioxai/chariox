import { shouldCycleFocusOnTabEvent } from "./hotkeys.js"
import type { ParsedShortcut } from "./keybind.js"

export type CliStdinKeyEvent = ParsedShortcut & {
  alt?: boolean
}

export type CliStdinKeypressParser = (
  chunk: Buffer | string,
  options: { useKittyKeyboard: boolean },
) => CliStdinKeyEvent | null

export type CliStdinKeyControllerDeps = {
  parseKeypress: CliStdinKeypressParser
  dialogOverlayOpen: () => boolean
  closeActiveDialogOverlay: () => void
  handleSessionBrowserKey: (event: CliStdinKeyEvent) => boolean
  requestExit: () => void
  handleFocusedInteractionKey: (event: CliStdinKeyEvent) => boolean
  promptFocused: () => boolean
  commandCenterOpen: () => boolean
  commandCenterQuery: () => string
  clearCommandCenter: () => void
  toggleWorkspaceScreen: () => void
  isAttached: () => boolean
  workflowScreenActive: () => boolean
  cycleWorkflowCanvasNode: () => void
  cycleAgentFocus: () => void
  copyPromptSelection: () => boolean
  activePrompt: () => unknown
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
  return {
    handleData(chunk) {
      const event = deps.parseKeypress(chunk, { useKittyKeyboard: true })
      if (!event) {
        return false
      }
      if (event.eventType !== "release" && deps.dialogOverlayOpen() && event.name === "escape") {
        deps.closeActiveDialogOverlay()
        return true
      }
      if (deps.handleSessionBrowserKey(event)) {
        return true
      }
      if (event.eventType !== "release" && event.ctrl && event.name === "e") {
        deps.requestExit()
        return true
      }
      if (deps.handleFocusedInteractionKey(event)) {
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
      if (event.eventType !== "release" && event.meta && event.name === "c" && deps.copyPromptSelection()) {
        return true
      }
      if (event.ctrl && event.name === "c") {
        if (deps.activePrompt()) {
          deps.requestPromptStop()
        } else {
          deps.requestExit()
        }
        return true
      }
      if (deps.dialogOverlayOpen()) {
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
    },
  }
}
