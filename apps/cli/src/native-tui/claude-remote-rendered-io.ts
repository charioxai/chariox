import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import type { LocalIpcClient } from "../ipc.js"
import {
  pumpTerminalOutputRequest,
  resizeTerminalRequest,
  sendTerminalInputRequest,
  submitPromptRequest,
} from "../ipc-requests.js"
import { preparePromptAttachmentsForSubmit } from "../prompt-attachment-transfer.js"
import {
  extractClaudeNativePromptAttachmentReferences,
  stripClaudeAttachmentMentions,
  uniqueClaudeAttachmentReferences,
} from "./claude-attachments.js"

export function forwardClaudeRemoteRenderedStdin(options: {
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  providerRunId: string
  worktree: string
  inlineLocalAttachments: boolean
  debug: (label: string, payload: unknown) => void
}): () => void {
  const wasRaw = Boolean(process.stdin.isTTY && process.stdin.isRaw)
  const composer: RemoteRenderedComposerState = { text: "", escapeState: "none", swallowNextLf: false }
  let pending = Promise.resolve()
  const onData = (chunk: Buffer) => {
    pending = pending
      .then(() => forwardRemoteRenderedInputChunk({ ...options, composer, chunk }))
      .catch(() => {})
  }
  if (process.stdin.isTTY) process.stdin.setRawMode?.(true)
  process.stdin.resume()
  process.stdin.on("data", onData)
  return () => {
    process.stdin.off("data", onData)
    if (process.stdin.isTTY) process.stdin.setRawMode?.(wasRaw)
  }
}

type RemoteRenderedComposerState = {
  text: string
  escapeState: "none" | "esc" | "csi"
  swallowNextLf: boolean
}

async function forwardRemoteRenderedInputChunk(options: {
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  providerRunId: string
  worktree: string
  inlineLocalAttachments: boolean
  debug: (label: string, payload: unknown) => void
  composer: RemoteRenderedComposerState
  chunk: Buffer
}) {
  const text = options.chunk.toString("utf8")
  for (const char of text) {
    if (options.composer.swallowNextLf) {
      options.composer.swallowNextLf = false
      if (char === "\n") continue
    }
    if (char === "\r" || char === "\n") {
      if (char === "\r") options.composer.swallowNextLf = true
      await submitOrForwardRemoteRenderedEnter(options)
      continue
    }
    if (char === "\u007f" || char === "\b") {
      options.composer.text = options.composer.text.slice(0, -1)
      await sendRemoteRenderedInput(options.client, options.sessionId, options.attachmentId, options.providerRunId, char)
      continue
    }
    if (char === "\u0015" || char === "\u0003") {
      options.composer.text = ""
      await sendRemoteRenderedInput(options.client, options.sessionId, options.attachmentId, options.providerRunId, char)
      continue
    }
    if (char === "\u001b") {
      options.composer.escapeState = "esc"
      await sendRemoteRenderedInput(options.client, options.sessionId, options.attachmentId, options.providerRunId, char)
      continue
    }
    if (options.composer.escapeState === "esc") {
      options.composer.escapeState = char === "[" ? "csi" : "none"
      await sendRemoteRenderedInput(options.client, options.sessionId, options.attachmentId, options.providerRunId, char)
      continue
    }
    if (options.composer.escapeState === "csi") {
      if (/[@-~]/.test(char)) options.composer.escapeState = "none"
      await sendRemoteRenderedInput(options.client, options.sessionId, options.attachmentId, options.providerRunId, char)
      continue
    }
    if (char >= " ") {
      options.composer.text += char
    }
    await sendRemoteRenderedInput(options.client, options.sessionId, options.attachmentId, options.providerRunId, char)
  }
}

async function submitOrForwardRemoteRenderedEnter(options: {
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  providerRunId: string
  worktree: string
  inlineLocalAttachments: boolean
  debug: (label: string, payload: unknown) => void
  composer: RemoteRenderedComposerState
}) {
  const prompt = options.composer.text.trim()
  options.composer.text = ""
  const references = extractClaudeNativePromptAttachmentReferences(prompt, options.worktree)
  if (references.length === 0) {
    await sendRemoteRenderedInput(options.client, options.sessionId, options.attachmentId, options.providerRunId, "\r")
    return
  }
  const attachments = await preparePromptAttachmentsForSubmit(
    uniqueClaudeAttachmentReferences(references).map((reference) => reference.attachment),
    { inlineLocalFiles: options.inlineLocalAttachments },
  )
  if (attachments.length === 0) {
    await sendRemoteRenderedInput(options.client, options.sessionId, options.attachmentId, options.providerRunId, "\r")
    return
  }
  await sendRemoteRenderedInput(options.client, options.sessionId, options.attachmentId, options.providerRunId, "\u0015")
  const visiblePrompt = stripClaudeAttachmentMentions(prompt, references)
  options.debug("remote_rendered_attachments_intercepted", {
    attachmentCount: attachments.length,
    mimeTypes: attachments.map((attachment) => attachment.mime),
    inlinedCount: attachments.filter((attachment) => attachment.contents_base64).length,
  })
  await options.client.send<Record<string, unknown>>(
    submitPromptRequest(
      options.sessionId,
      options.attachmentId,
      options.agentId,
      visiblePrompt || "Please use the attached file.",
      attachments,
    ),
  )
}

async function sendRemoteRenderedInput(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  providerRunId: string,
  data: Buffer | string,
) {
  await client.send<Record<string, unknown>>(
    sendTerminalInputRequest(sessionId, attachmentId, data, providerRunId),
  )
}

export function installClaudeRemoteRenderedResizeForwarder(client: LocalIpcClient, sessionId: string) {
  const sendResize = () => {
    const cols = process.stdout.columns
    const rows = process.stdout.rows
    if (!cols || !rows) return
    void client.send<Record<string, unknown>>(resizeTerminalRequest(sessionId, cols, rows)).catch(() => {})
  }
  process.stdout.on("resize", sendResize)
  sendResize()
}

export function startClaudeRemoteRenderedPumpLoop(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  providerRunId: string,
): { stop: () => void } {
  let stopped = false
  const loop = async () => {
    while (!stopped) {
      const response = await client
        .send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId))
        .catch(() => ({}))
      const records = "TerminalOutput" in response ? (response.TerminalOutput as { records?: unknown[] }).records : null
      writeClaudeRemoteRenderedTerminalRecords(records, providerRunId)
      await sleep(250)
    }
  }
  void loop()
  return {
    stop: () => {
      stopped = true
    },
  }
}

export function writeClaudeRemoteRenderedTerminalRecords(records: unknown, providerRunId: string) {
  if (!Array.isArray(records)) return
  for (const record of records) {
    if (!record || typeof record !== "object") continue
    const payload = record as { provider_run_id?: unknown; bytes?: unknown }
    if (payload.provider_run_id !== providerRunId) continue
    const bytes = Array.isArray(payload.bytes) ? Buffer.from(payload.bytes as number[]) : null
    if (bytes?.length) process.stdout.write(bytes)
  }
}
