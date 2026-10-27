import assert from "node:assert/strict"
import test from "node:test"

import {
  createPromptAttachmentController,
  type PromptAttachmentControllerDeps,
  type PromptAttachmentInput,
} from "./prompt-attachment-controller.js"
import type { PendingPromptAttachment, StoredPromptAttachment } from "./prompt-attachment-state.js"

test("prompt attachment controller adds stored files and inserts tokens", () => {
  const harness = createHarness({ text: "run" })
  const controller = createPromptAttachmentController(harness.deps)

  assert.equal(controller.addStoredFiles([storedFile("file-1", "note.txt", "text")], 3), true)

  assert.equal(harness.text(), "run [file 1]")
  assert.equal(harness.cursorOffset(), 12)
  assert.deepEqual(harness.attachments().map((file) => file.token), ["[file 1]"])
  assert.equal(harness.renderCount(), 1)
  assert.equal(harness.highlightCount(), 1)
})

test("prompt attachment controller ignores duplicate stored files", () => {
  const harness = createHarness({
    attachments: [pendingFile("file-1", "note.txt", "text", "[file 1]")],
    text: "[file 1]",
  })
  const controller = createPromptAttachmentController(harness.deps)

  assert.equal(controller.addStoredFiles([storedFile("file-1", "note.txt", "text")], 0), false)

  assert.equal(harness.text(), "[file 1]")
  assert.equal(harness.attachments().length, 1)
  assert.equal(harness.renderCount(), 0)
})

test("prompt attachment controller removes attachment tokens with edit keys", () => {
  const harness = createHarness({
    attachments: [pendingFile("file-1", "note.txt", "text", "[file 1]")],
    text: "look [file 1]",
    cursorOffset: "look [file 1]".length,
  })
  const controller = createPromptAttachmentController(harness.deps)

  assert.equal(controller.removeForEdit("backspace"), true)

  assert.equal(harness.text(), "look ")
  assert.deepEqual(harness.attachments(), [])
  assert.equal(harness.cursorOffset(), 5)
  assert.equal(harness.renderCount(), 1)
})

test("prompt attachment controller syncs and clears pending attachments", () => {
  const harness = createHarness({
    attachments: [
      pendingFile("file-1", "note.txt", "text", "[file 1]"),
      pendingFile("file-2", "image.png", "image", "[image 1]", "image/png"),
    ],
  })
  const controller = createPromptAttachmentController(harness.deps)

  controller.syncFromText("[image 1]")
  assert.deepEqual(harness.attachments().map((file) => file.token), ["[image 1]"])
  assert.equal(harness.highlightCount(), 1)

  controller.clear()
  assert.deepEqual(harness.attachments(), [])
  assert.equal(harness.highlightCount(), 2)
  assert.equal(harness.renderCount(), 1)
})

function createHarness(options: {
  text?: string
  cursorOffset?: number
  attachments?: PendingPromptAttachment[]
} = {}) {
  let text = options.text ?? ""
  let attachments = options.attachments ?? []
  let cursorOffset = options.cursorOffset ?? 0
  let renderCount = 0
  let highlightCount = 0
  let chromeUpdateCount = 0
  const input: PromptAttachmentInput = {
    get plainText() {
      return text
    },
    get cursorOffset() {
      return cursorOffset
    },
    set cursorOffset(value: number) {
      cursorOffset = value
    },
    getSelection: () => null,
  }
  const deps: PromptAttachmentControllerDeps = {
    getPromptInput: () => input,
    getPromptText: () => text,
    setPromptText: (value) => {
      text = value
    },
    pendingAttachments: () => attachments,
    setPendingAttachments: (nextAttachments) => {
      attachments = nextAttachments
    },
    updatePendingAttachments: (updater) => {
      attachments = updater(attachments)
    },
    refreshHighlights: () => {
      highlightCount += 1
    },
    updateSessionChrome: () => {
      chromeUpdateCount += 1
    },
    requestRender: () => {
      renderCount += 1
    },
  }
  return {
    deps,
    text: () => text,
    attachments: () => attachments,
    cursorOffset: () => cursorOffset,
    renderCount: () => renderCount,
    highlightCount: () => highlightCount,
    chromeUpdateCount: () => chromeUpdateCount,
  }
}

function storedFile(
  id: string,
  filename: string,
  kind: StoredPromptAttachment["kind"],
  mime = "text/plain",
): StoredPromptAttachment {
  return {
    id,
    url: `file:///tmp/${filename}`,
    mime,
    filename,
    kind,
  }
}

function pendingFile(
  id: string,
  filename: string,
  kind: PendingPromptAttachment["kind"],
  token: string,
  mime = "text/plain",
): PendingPromptAttachment {
  return {
    ...storedFile(id, filename, kind, mime),
    token,
  }
}
