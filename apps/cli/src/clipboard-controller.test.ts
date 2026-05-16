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
  assert.deepEqual(harness.footerMessages(), [{ message: "selection copied to clipboard", tone: "info" }])
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

test("clipboard controller clears terminal selection after copying", async () => {
  const harness = createHarness({ rendererSelection: "terminal text" })
  const controller = createClipboardController(harness.deps)

  controller.copySelection()
  await flushMicrotasks()

  assert.deepEqual(harness.copiedText(), ["terminal text"])
  assert.equal(harness.clearCount(), 1)
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

  assert.deepEqual(harness.footerMessages(), [{ message: "failed to copy selection", tone: "error" }])
  assert.deepEqual(harness.warnings(), [{ message: "selection copy failed", error: "copy failed" }])
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
