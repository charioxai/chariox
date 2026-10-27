import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeAttachment, StoredTransferArtifact } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  createPromptAttachmentIntakeController,
  type PromptAttachmentIntakeControllerDeps,
} from "./prompt-attachment-intake-controller.js"
import type { StoredPromptAttachment } from "./prompt-attachment-state.js"
import type { ParsedPromptAttachment } from "./prompt-attachments.js"

test("prompt attachment intake stores parsed files and appends pending attachments", async () => {
  const harness = createHarness()
  const controller = createPromptAttachmentIntakeController(harness.deps)

  await controller.attachFiles([parsedFile("/tmp/note.txt", "note.txt", "text")], 5)

  assert.deepEqual(harness.addedAttachments().map((entry) => ({
    at: entry.at,
    filename: entry.files[0]?.filename,
    url: entry.files[0]?.url,
  })), [{
    at: 5,
    filename: "stored-note.txt",
    url: "file:///tmp/stored-note.txt",
  }])
  assert.deepEqual(harness.footerMessages(), [{ message: "attached 1 file", tone: "info" }])
})

test("prompt attachment intake handles clear and empty attach commands", async () => {
  const harness = createHarness()
  const controller = createPromptAttachmentIntakeController(harness.deps)

  await controller.handleCommand("/attach")
  await controller.handleCommand("/attach clear")

  assert.equal(harness.clearCount(), 1)
  assert.deepEqual(harness.footerMessages(), [
    { message: "usage: /attach <path...> | /attach clear | /attach screenshot", tone: "error" },
    { message: "cleared prompt attachments", tone: "info" },
  ])
})

test("prompt attachment intake reports screenshot attachment requirements and failures", async () => {
  const harness = createHarness({ attachment: null })
  const controller = createPromptAttachmentIntakeController(harness.deps)

  await controller.capturePromptScreenshot()

  assert.deepEqual(harness.footerMessages(), [
    { message: "attach to a session before capturing screenshots", tone: "error" },
  ])
})

test("prompt attachment intake adds captured screenshots as image attachments", async () => {
  const harness = createHarness({
    captureScreenshot: async () => ({
      status: "Captured",
      artifact_path: "/tmp/capture.png",
      message: "ok",
    }),
    now: () => 42,
    promptInsertOffset: () => 7,
  })
  const controller = createPromptAttachmentIntakeController(harness.deps)

  await controller.handleCommand("/attach screenshot")

  assert.deepEqual(harness.addedAttachments().map((entry) => ({
    at: entry.at,
    file: entry.files[0],
  })), [{
    at: 7,
    file: {
      id: "screenshot-42",
      url: "file:///tmp/capture.png",
      mime: "image/png",
      filename: "capture.png",
      kind: "image",
    },
  }])
  assert.deepEqual(harness.footerMessages(), [{ message: "attached screenshot", tone: "info" }])
})

function createHarness(options: {
  attachment?: RuntimeAttachment | null
  captureScreenshot?: PromptAttachmentIntakeControllerDeps["captureScreenshot"]
  now?: () => number
  promptInsertOffset?: () => number
} = {}) {
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const addedAttachments: Array<{ files: StoredPromptAttachment[]; at: number }> = []
  let clearCount = 0
  const deps: PromptAttachmentIntakeControllerDeps = {
    client: {} as LocalIpcClient,
    cwd: () => "/tmp",
    sessionState: () => ({ id: "session-1" }),
    attachmentState: () => options.attachment === undefined ? { id: "attachment-1", session_id: "session-1" } : options.attachment,
    promptInsertOffset: options.promptInsertOffset ?? (() => 0),
    addPendingPromptAttachments: (files, at) => {
      addedAttachments.push({ files, at })
      return true
    },
    clearPendingPromptAttachments: () => {
      clearCount += 1
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    storeTransferredFile: async (_client, _sessionId, _attachmentId, _path, filename): Promise<StoredTransferArtifact> => ({
      artifact_id: `artifact-${filename}`,
      stored_path: `/tmp/stored-${filename}`,
      display_name: `stored-${filename}`,
    }),
    ...(options.captureScreenshot ? { captureScreenshot: options.captureScreenshot } : {}),
    ...(options.now ? { now: options.now } : {}),
  }
  return {
    deps,
    addedAttachments: () => addedAttachments,
    clearCount: () => clearCount,
    footerMessages: () => footerMessages,
  }
}

function parsedFile(filePath: string, filename: string, kind: ParsedPromptAttachment["kind"]): ParsedPromptAttachment {
  return {
    path: filePath,
    filename,
    mime: kind === "image" ? "image/png" : "text/plain",
    kind,
  }
}
