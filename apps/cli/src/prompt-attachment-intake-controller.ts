import path from "node:path"
import { pathToFileURL } from "node:url"

import type { RuntimeAttachment, RuntimeSession } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  captureScreenshot as captureScreenshotApi,
  storeTransferredFile as storeTransferredFileApi,
} from "./prompt-attachment-api.js"
import {
  parsePromptAttachmentCommand,
  type ParsedPromptAttachment,
} from "./prompt-attachments.js"
import type { StoredPromptAttachment } from "./prompt-attachment-state.js"

type FooterTone = "info" | "error"

export type PromptAttachmentIntakeControllerDeps = {
  client: LocalIpcClient
  cwd: () => string
  sessionState: () => Pick<RuntimeSession, "id">
  attachmentState: () => RuntimeAttachment | null
  promptInsertOffset: () => number
  addPendingPromptAttachments: (files: StoredPromptAttachment[], at: number) => boolean
  clearPendingPromptAttachments: () => void
  flashFooter: (message: string, tone: FooterTone) => void
  storeTransferredFile?: typeof storeTransferredFileApi
  captureScreenshot?: typeof captureScreenshotApi
  now?: () => number
}

export function createPromptAttachmentIntakeController(deps: PromptAttachmentIntakeControllerDeps) {
  const storeTransferredFile = deps.storeTransferredFile ?? storeTransferredFileApi
  const captureScreenshot = deps.captureScreenshot ?? captureScreenshotApi
  const now = deps.now ?? Date.now

  const storePromptAttachment = async (file: ParsedPromptAttachment): Promise<StoredPromptAttachment> => {
    const attachment = deps.attachmentState()
    if (!attachment) {
      throw new Error("no active attachment available for storing prompt attachments")
    }
    const artifact = await storeTransferredFile(
      deps.client,
      deps.sessionState().id,
      attachment.id,
      file.path,
      file.filename,
    )
    return {
      id: artifact.artifact_id,
      url: pathToFileURL(artifact.stored_path).href,
      mime: file.mime,
      filename: artifact.display_name,
      kind: file.kind,
    }
  }

  const attachFiles = async (
    files: ParsedPromptAttachment[],
    at = deps.promptInsertOffset(),
  ) => {
    const stored: StoredPromptAttachment[] = []
    for (const file of files) {
      stored.push(await storePromptAttachment(file))
    }
    deps.addPendingPromptAttachments(stored, at)
    if (files.length > 0) {
      deps.flashFooter(`attached ${files.length} file${files.length === 1 ? "" : "s"}`, "info")
    }
  }

  const capturePromptScreenshot = async () => {
    const attachment = deps.attachmentState()
    if (!attachment) {
      deps.flashFooter("attach to a session before capturing screenshots", "error")
      return
    }
    const result = await captureScreenshot(deps.client, deps.sessionState().id, attachment.id)
    if (result.status !== "Captured" || !result.artifact_path) {
      deps.flashFooter(result.message, "error")
      return
    }
    deps.addPendingPromptAttachments([{
      id: `screenshot-${now()}`,
      url: pathToFileURL(result.artifact_path).href,
      mime: "image/png",
      filename: path.basename(result.artifact_path),
      kind: "image",
    }], deps.promptInsertOffset())
    deps.flashFooter("attached screenshot", "info")
  }

  const handleCommand = async (commandLine: string) => {
    const value = commandLine.replace(/^\/attach\s*/, "").trim()
    if (!value) {
      deps.flashFooter("usage: /attach <path...> | /attach clear | /attach screenshot", "error")
      return
    }
    if (value === "clear") {
      deps.clearPendingPromptAttachments()
      deps.flashFooter("cleared prompt attachments", "info")
      return
    }
    if (value === "screenshot") {
      await capturePromptScreenshot()
      return
    }
    const files = parsePromptAttachmentCommand(value, deps.cwd())
    if (!files || files.length === 0) {
      deps.flashFooter("drop or specify images, PDFs, or text files", "error")
      return
    }
    await attachFiles(files)
  }

  return {
    attachFiles,
    capturePromptScreenshot,
    handleCommand,
    storePromptAttachment,
  }
}
