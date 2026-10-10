import assert from "node:assert/strict"
import test from "node:test"

import {
  createClipboardController,
  type ClipboardControllerDeps,
  type ClipboardControllerRenderer,
} from "./clipboard-controller.js"

test("clipboard controller copies prompt selections with feedback", async () => {
  const harness = createHarness({
    promptText: "hello selection",
    promptSelection: { start: 6, end: 15 },
  })
  const controller = createClipboardController(harness.deps)

  assert.equal(controller.copyPromptSelection(), true)
  await flushMicrotasks()

  assert.deepEqual(harness.copiedText(), ["selection"])
  assert.deepEqual(harness.footerMessages(), [{ message: "copied to local clipboard", tone: "info" }])
})

test("clipboard controller ignores empty prompt selections", () => {
  const harness = createHarness({
    promptText: "hello",
    promptSelection: { start: 2, end: 2 },
  })
  const controller = createClipboardController(harness.deps)

  assert.equal(controller.copyPromptSelection(), false)
  assert.deepEqual(harness.copiedText(), [])
})

test("clipboard controller retains terminal selection after copying", async () => {
  const harness = createHarness({ rendererSelection: "terminal text" })
  const controller = createClipboardController(harness.deps)

  controller.copySelection()
  await flushMicrotasks()

  assert.deepEqual(harness.copiedText(), ["terminal text"])
  assert.equal(harness.clearCount(), 0)
})

test("clipboard controller reports copy failures", async () => {
  const harness = createHarness({
    rendererSelection: "terminal text",
    copyText: async () => {
      throw new Error("copy failed")
    },
  })
  const controller = createClipboardController(harness.deps)

  controller.copySelection()
  await flushMicrotasks()

  assert.deepEqual(harness.footerMessages(), [{ message: "F7: mouse off; drag-select, Cmd-C; Esc/F7: mouse on", tone: "error" }])
  assert.deepEqual(harness.warnings(), [{ message: "selection copy failed", error: "copy failed" }])
})

// MP-08 / MP-10: a denied raw copy still consumes its snapshot before the
// next copy, and an empty snapshot cannot borrow a later selection.
test("MP-08 / MP-10 clipboard replays event-time snapshots after skipped and empty copies", async () => {
  const state = { promptText: "draft", promptSelection: { start: 4, end: 5 } as { start: number; end: number } | null }
  const harness = createHarness(state)
  const controller = createClipboardController(harness.deps)
  const key = { name: "f6", sequence: "\x1b[17~" }
  controller.captureCopyKey(key)
  state.promptSelection = null
  controller.captureCopyKey(key)
  state.promptSelection = { start: 0, end: 5 }
  controller.captureCopyKey(key)
  controller.replayCopyKey(key) // A dialog/interaction owns this first copy.
  controller.replayCopyKey(key)
  assert.equal(controller.copyCapturedSelection(), false)
  controller.replayCopyKey(key)
  assert.equal(controller.copyCapturedSelection(), true)
  await flushMicrotasks()
  assert.deepEqual(harness.copiedText(), ["draft"])
})

test("MP-08 / MP-10 clipboard discard removes pending copy input", () => {
  const state = { rendererSelection: "old selection" }
  const harness = createHarness(state)
  const controller = createClipboardController(harness.deps)
  const key = { name: "f6", sequence: "\x1b[17~" }
  controller.captureCopyKey(key)
  controller.replayCopyKey(key)
  controller.discardCopyInput()
  state.rendererSelection = "current selection"
  controller.replayCopyKey(key)
  assert.equal(controller.copyCapturedSelection(), true)
  assert.deepEqual(harness.copiedText(), ["current selection"])
})

// MP-08 / MP-10: decoded invalidation excludes non-input responses, releases
// and copy keys, and never clears the textarea's selection before its edit.
test("MP-08 / MP-10 decoded edits clear only retained transcript selection", () => {
  const state = { promptText: "draft", promptSelection: null as { start: number; end: number } | null }
  const harness = createHarness(state)
  const controller = createClipboardController(harness.deps)
  for (const key of [{ name: "" }, { name: "z", eventType: "release" }, { name: "f6" }]) controller.captureCopyKey(key)
  assert.equal(harness.clearCount(), 0)
  controller.captureCopyKey({ name: "z" })
  controller.capturePaste()
  assert.equal(harness.clearCount(), 2)
  state.promptSelection = { start: 0, end: 5 }
  controller.captureCopyKey({ name: "z" })
  controller.capturePaste()
  assert.equal(harness.clearCount(), 2)
})

function createHarness(options: {
  promptText?: string
  promptSelection?: { start: number; end: number } | null
  rendererSelection?: string | null
  copyText?: ClipboardControllerDeps["copyText"]
} = {}) {
  const copiedText: string[] = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const warnings: Array<{ message: string; error: string | undefined }> = []
  let clearCount = 0
  const renderer: ClipboardControllerRenderer = {
    copyToClipboardOSC52: () => true,
    getSelection: () => options.rendererSelection === undefined
      ? null
      : { getSelectedText: () => options.rendererSelection },
    clearSelection: () => {
      clearCount += 1
    },
  }
  const deps: ClipboardControllerDeps = {
    renderer,
    promptInput: () => options.promptText === undefined
      ? null
      : {
          plainText: options.promptText,
          getSelection: () => options.promptSelection,
        },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    logWarning: (message, fields) => {
      warnings.push({ message, error: fields?.error as string | undefined })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    copyText: options.copyText ?? (async (text) => {
      copiedText.push(text)
      return "copied"
    }),
  }
  return {
    deps,
    copiedText: () => copiedText,
    footerMessages: () => footerMessages,
    warnings: () => warnings,
    clearCount: () => clearCount,
  }
}

async function flushMicrotasks() {
  await Promise.resolve()
  await Promise.resolve()
}
