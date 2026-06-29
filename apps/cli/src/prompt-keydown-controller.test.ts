import assert from "node:assert/strict"
import test from "node:test"

import {
  createPromptKeyDownController,
  type PromptKeyDownControllerDeps,
  type PromptKeyDownEvent,
} from "./prompt-keydown-controller.js"

test("prompt keydown controller delegates focused interactions before prompt handlers", () => {
  const harness = createHarness({ focusedHandled: true })
  const event = keyEvent("return")

  assert.equal(harness.controller.handleKeyDown(event), true)
  assert.deepEqual(harness.calls(), ["focused:return"])
})

test("prompt keydown controller delegates command center before prompt history", () => {
  const harness = createHarness({
    commandCenterHandled: true,
    attached: true,
    promptFocused: true,
  })
  const event = keyEvent("up")

  assert.equal(harness.controller.handleKeyDown(event), true)
  assert.deepEqual(harness.calls(), ["focused:up", "command-center:up"])
})

test("prompt keydown controller delegates queued prompt shortcuts before prompt history", () => {
  const harness = createHarness({
    queuedPromptHandled: true,
    attached: true,
    promptFocused: true,
    currentText: "first\nsecond",
    cursorOffset: 0,
    historyHandled: true,
  })
  const event = keyEvent("s", { alt: true })

  assert.equal(harness.controller.handleKeyDown(event), true)
  assert.deepEqual(harness.calls(), [
    "focused:s",
    "command-center:s",
    "queued-prompt:s",
  ])
})

test("prompt keydown controller navigates prompt history and consumes handled events", () => {
  const harness = createHarness({
    attached: true,
    promptFocused: true,
    currentText: "first\nsecond",
    cursorOffset: 0,
    historyHandled: true,
  })
  const event = keyEvent("up")

  assert.equal(harness.controller.handleKeyDown(event), true)
  assert.equal(event.prevented, true)
  assert.equal(event.stopped, true)
  assert.deepEqual(harness.calls(), [
    "focused:up",
    "command-center:up",
    "queued-prompt:up",
    "history:previous",
  ])
})

test("prompt keydown controller does not handle unowned prompt history directions", () => {
  const harness = createHarness({
    attached: true,
    promptFocused: true,
    historyHandled: true,
  })
  const event = keyEvent("down")

  assert.equal(harness.controller.handleKeyDown(event), false)
  assert.equal(event.prevented, false)
  assert.deepEqual(harness.calls(), [
    "focused:down",
    "command-center:down",
    "queued-prompt:down",
    "hotkeys:down",
  ])
})

test("prompt keydown controller delegates hotkey toggles when no prompt owner handles", () => {
  const harness = createHarness({ hotkeysHandled: true })
  const event = keyEvent("t", { ctrl: true })

  assert.equal(harness.controller.handleKeyDown(event), true)
  assert.deepEqual(harness.calls(), [
    "focused:t",
    "command-center:t",
    "queued-prompt:t",
    "hotkeys:t",
  ])
})

function createHarness(options: {
  focusedHandled?: boolean
  commandCenterHandled?: boolean
  attached?: boolean
  promptFocused?: boolean
  commandCenterOpen?: boolean
  currentText?: string
  cursorOffset?: number
  promptHistoryIndex?: number | null
  promptHistoryDraft?: string | null
  historyHandled?: boolean
  queuedPromptHandled?: boolean
  hotkeysHandled?: boolean
} = {}) {
  const calls: string[] = []
  const deps: PromptKeyDownControllerDeps = {
    handleFocusedInteractionKey: (event) => {
      calls.push(`focused:${event.name}`)
      return options.focusedHandled ?? false
    },
    handleCommandCenterKey: (event) => {
      calls.push(`command-center:${event.name}`)
      return options.commandCenterHandled ?? false
    },
    handleQueuedPromptKey: (event) => {
      calls.push(`queued-prompt:${event.name}`)
      return options.queuedPromptHandled ?? false
    },
    isAttached: () => options.attached ?? false,
    promptFocused: () => options.promptFocused ?? false,
    commandCenterOpen: () => options.commandCenterOpen ?? false,
    currentPromptText: () => options.currentText ?? "",
    promptCursorOffset: () => options.cursorOffset,
    promptHistoryIndex: () => options.promptHistoryIndex ?? null,
    promptHistoryDraft: () => options.promptHistoryDraft ?? null,
    navigatePromptHistoryInput: (direction) => {
      calls.push(`history:${direction}`)
      return options.historyHandled ?? false
    },
    handleHotkeysToggleShortcut: (_source, event) => {
      calls.push(`hotkeys:${event.name}`)
      return options.hotkeysHandled ?? false
    },
  }

  return {
    controller: createPromptKeyDownController(deps),
    calls: () => calls,
  }
}

function keyEvent(name: string, options: {
  eventType?: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  shift?: boolean
} = {}) {
  const event: PromptKeyDownEvent & {
    prevented: boolean
    stopped: boolean
  } = {
    name,
    prevented: false,
    stopped: false,
    preventDefault: () => {
      event.prevented = true
    },
    stopPropagation: () => {
      event.stopped = true
    },
  }
  if (options.eventType !== undefined) {
    event.eventType = options.eventType
  }
  if (options.ctrl !== undefined) {
    event.ctrl = options.ctrl
  }
  if (options.meta !== undefined) {
    event.meta = options.meta
  }
  if (options.alt !== undefined) {
    event.alt = options.alt
  }
  if (options.shift !== undefined) {
    event.shift = options.shift
  }
  return event
}
