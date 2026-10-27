import {
  resolvePromptHistoryKeyNavigation,
  type PromptHistoryDirection,
} from "@chariox/kernel-client/prompt-history"

export type PromptKeyDownEvent = {
  name: string
  eventType?: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  shift?: boolean
  preventDefault?: () => void
  stopPropagation?: () => void
}

export type PromptKeyDownControllerDeps = {
  handleFocusedInteractionKey: (event: PromptKeyDownEvent) => boolean
  handleCommandCenterKey: (event: PromptKeyDownEvent) => boolean
  handleQueuedPromptKey: (event: PromptKeyDownEvent) => boolean
  isAttached: () => boolean
  promptFocused: () => boolean
  commandCenterOpen: () => boolean
  currentPromptText: () => string
  promptCursorOffset: () => number | undefined
  promptHistoryIndex: () => number | null
  promptHistoryDraft: () => string | null
  navigatePromptHistoryInput: (direction: PromptHistoryDirection) => boolean
  handleHotkeysToggleShortcut: (source: "textarea", event: PromptKeyDownEvent) => boolean
}

export type PromptKeyDownController = {
  handleKeyDown(event: PromptKeyDownEvent): boolean
}

export function createPromptKeyDownController(
  deps: PromptKeyDownControllerDeps,
): PromptKeyDownController {
  const handlePromptHistoryKey = (event: PromptKeyDownEvent) => {
    const direction = resolvePromptHistoryKeyNavigation({
      attached: deps.isAttached(),
      promptFocused: deps.promptFocused(),
      commandCenterOpen: deps.commandCenterOpen(),
      keyName: event.name,
      currentText: deps.currentPromptText(),
      cursorOffset: deps.promptCursorOffset(),
      eventType: event.eventType,
      ctrl: event.ctrl,
      meta: event.meta,
      alt: event.alt,
      shift: event.shift,
      navigationIndex: deps.promptHistoryIndex(),
      navigationDraft: deps.promptHistoryDraft(),
    })
    if (!direction) {
      return false
    }
    const handled = deps.navigatePromptHistoryInput(direction)
    if (handled) {
      event.preventDefault?.()
      event.stopPropagation?.()
    }
    return handled
  }

  return {
    handleKeyDown(event) {
      if (deps.handleFocusedInteractionKey(event)) {
        return true
      }
      if (deps.handleCommandCenterKey(event)) {
        return true
      }
      if (deps.handleQueuedPromptKey(event)) {
        return true
      }
      if (handlePromptHistoryKey(event)) {
        return true
      }
      return deps.handleHotkeysToggleShortcut("textarea", event)
    },
  }
}
