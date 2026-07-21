import process from "node:process"
import { StringDecoder } from "node:string_decoder"
import { setTimeout as sleep } from "node:timers/promises"

import type { LocalIpcClient } from "../ipc.js"
import {
  pumpTerminalOutputRequest,
  resizeTerminalRequest,
  sendTerminalInputRequest,
  submitPromptRequest,
} from "../ipc-requests.js"
import { preparePromptAttachmentsForSubmit } from "../prompt-attachment-transfer.js"
import { PROVIDER_TERMINAL_OUTPUT_KIND } from "@arroba/kernel-client/terminal-record-transcript"
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
  const composer = createClaudeRemoteRenderedComposerState()
  let pending = Promise.resolve()
  let inputGeneration = 0
  const onData = (chunk: Buffer) => {
    const generation = ++inputGeneration
    pending = pending
      .then(() => forwardRemoteRenderedInputChunk({ ...options, composer, chunk }))
      .then(async () => {
        if (composer.escapeState === "none") return
        await sleep(10)
        if (generation !== inputGeneration) return
        const data = takeClaudeRemoteRenderedPendingControl(composer)
        if (data) {
          await sendRemoteRenderedInput(
            options.client,
            options.sessionId,
            options.attachmentId,
            options.providerRunId,
            data,
          )
        }
      })
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

type RemoteRenderedEscapeState =
  | "none"
  | "esc"
  | "csi"
  | "ss3"
  | "osc"
  | "osc_esc"
  | "st"
  | "st_esc"

export type RemoteRenderedComposerState = {
  text: string
  escapeState: RemoteRenderedEscapeState
  escapeBuffer: string
  swallowNextLf: boolean
  decoder: StringDecoder
}

export type RemoteRenderedInputAction =
  | { type: "input"; data: string }
  | { type: "enter"; prompt: string }

export type ClaudeRemoteRenderedReadiness = {
  observe: (records: unknown, providerRunId: string, agentId: string) => void
  wait: (timeoutMs?: number) => Promise<void>
}

export function createClaudeRemoteRenderedReadiness(): ClaudeRemoteRenderedReadiness {
  let output = ""
  let ready = false
  const waiters = new Set<() => void>()
  return {
    observe: (records, providerRunId, agentId) => {
      if (ready) return
      for (const bytes of claudeRemoteRenderedTerminalBytes(records, providerRunId, agentId)) {
        output = `${output}${bytes.toString("utf8")}`.slice(-32_768)
      }
      if (!output.includes("Claude Code") || !output.includes("\u001b[?2004h")) return
      ready = true
      for (const resolve of waiters) resolve()
      waiters.clear()
    },
    wait: async (timeoutMs = 60_000) => {
      if (ready) return
      await new Promise<void>((resolve, reject) => {
        const onReady = () => {
          clearTimeout(timeout)
          resolve()
        }
        const timeout = setTimeout(() => {
          waiters.delete(onReady)
          reject(new Error(`timed out waiting ${timeoutMs}ms for the remote Claude TUI input surface`))
        }, timeoutMs)
        waiters.add(onReady)
        if (ready) {
          waiters.delete(onReady)
          onReady()
        }
      })
    },
  }
}

export async function submitClaudeRemoteRenderedInitialPrompt(options: {
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  providerRunId: string
  prompt: string
  readiness: ClaudeRemoteRenderedReadiness
}) {
  await options.readiness.wait()
  await sendRemoteRenderedInput(
    options.client,
    options.sessionId,
    options.attachmentId,
    options.providerRunId,
    options.prompt,
  )
  await sleep(250)
  await sendRemoteRenderedInput(
    options.client,
    options.sessionId,
    options.attachmentId,
    options.providerRunId,
    "\r",
  )
}

export function createClaudeRemoteRenderedComposerState(): RemoteRenderedComposerState {
  return {
    text: "",
    escapeState: "none",
    escapeBuffer: "",
    swallowNextLf: false,
    decoder: new StringDecoder("utf8"),
  }
}

export function planClaudeRemoteRenderedInput(
  composer: RemoteRenderedComposerState,
  chunk: Buffer,
): RemoteRenderedInputAction[] {
  const actions: RemoteRenderedInputAction[] = []
  let outbound = ""
  const flushOutbound = () => {
    if (!outbound) return
    actions.push({ type: "input", data: outbound })
    outbound = ""
  }
  const text = composer.decoder.write(chunk)
  for (const char of text) {
    if (composer.swallowNextLf) {
      composer.swallowNextLf = false
      if (char === "\n") continue
    }
    if (composer.escapeState !== "none") {
      const completed = appendClaudeRemoteRenderedControl(composer, char)
      if (completed) outbound += completed
      continue
    }
    if (char === "\u001b") {
      composer.escapeState = "esc"
      composer.escapeBuffer = char
      continue
    }
    if (char === "\r" || char === "\n") {
      if (char === "\r") composer.swallowNextLf = true
      flushOutbound()
      actions.push({ type: "enter", prompt: composer.text.trim() })
      composer.text = ""
      continue
    }
    if (char === "\u007f" || char === "\b") {
      composer.text = Array.from(composer.text).slice(0, -1).join("")
    } else if (char === "\u0015" || char === "\u0003") {
      composer.text = ""
    } else if (char >= " ") {
      composer.text += char
    }
    outbound += char
  }
  flushOutbound()
  return actions
}

function appendClaudeRemoteRenderedControl(
  composer: RemoteRenderedComposerState,
  char: string,
): string | null {
  composer.escapeBuffer += char
  switch (composer.escapeState) {
    case "esc":
      if (char === "[") composer.escapeState = "csi"
      else if (char === "O") composer.escapeState = "ss3"
      else if (char === "]") composer.escapeState = "osc"
      else if (["P", "X", "^", "_"].includes(char)) composer.escapeState = "st"
      else return takeClaudeRemoteRenderedPendingControl(composer)
      return null
    case "csi":
    case "ss3":
      return /[@-~]/.test(char) ? takeClaudeRemoteRenderedPendingControl(composer) : null
    case "osc":
      if (char === "\u0007") return takeClaudeRemoteRenderedPendingControl(composer)
      if (char === "\u001b") composer.escapeState = "osc_esc"
      return null
    case "osc_esc":
      if (char === "\\") return takeClaudeRemoteRenderedPendingControl(composer)
      composer.escapeState = char === "\u001b" ? "osc_esc" : "osc"
      return null
    case "st":
      if (char === "\u001b") composer.escapeState = "st_esc"
      return null
    case "st_esc":
      if (char === "\\") return takeClaudeRemoteRenderedPendingControl(composer)
      composer.escapeState = char === "\u001b" ? "st_esc" : "st"
      return null
    case "none":
      return null
  }
}

function takeClaudeRemoteRenderedPendingControl(composer: RemoteRenderedComposerState): string {
  const data = composer.escapeBuffer
  composer.escapeBuffer = ""
  composer.escapeState = "none"
  return data
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
  const actions = planClaudeRemoteRenderedInput(options.composer, options.chunk)
  for (const action of actions) {
    if (action.type === "enter") {
      await submitOrForwardRemoteRenderedEnter(options, action.prompt)
      continue
    }
    await sendRemoteRenderedInput(
      options.client,
      options.sessionId,
      options.attachmentId,
      options.providerRunId,
      action.data,
    )
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
}, prompt: string) {
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

export function installClaudeRemoteRenderedResizeForwarder(
  client: LocalIpcClient,
  sessionId: string,
  providerRunId: string,
): () => void {
  let disposed = false
  let generation = 0
  const sendResize = () => {
    const cols = process.stdout.columns
    const rows = process.stdout.rows
    if (!cols || !rows) return
    const resizeGeneration = ++generation
    void (async () => {
      let retryMs = 100
      while (!disposed && resizeGeneration === generation) {
        try {
          await client.send<Record<string, unknown>>(
            resizeTerminalRequest(sessionId, cols, rows, providerRunId),
          )
          return
        } catch {
          await sleep(retryMs)
          retryMs = Math.min(retryMs * 2, 2_000)
        }
      }
    })()
  }
  process.stdout.on("resize", sendResize)
  sendResize()
  return () => {
    disposed = true
    generation += 1
    process.stdout.off("resize", sendResize)
  }
}

export function startClaudeRemoteRenderedPumpLoop(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  providerRunId: string,
  agentId: string,
  onRecords?: (records: unknown) => void,
): { stop: () => void } {
  let stopped = false
  const loop = async () => {
    while (!stopped) {
      const response = await client
        .send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId))
        .catch(() => ({}))
      const records = "TerminalOutput" in response ? (response.TerminalOutput as { records?: unknown[] }).records : null
      onRecords?.(records)
      writeClaudeRemoteRenderedTerminalRecords(records, providerRunId, agentId)
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

export function writeClaudeRemoteRenderedTerminalRecords(
  records: unknown,
  providerRunId: string,
  agentId: string,
) {
  for (const bytes of claudeRemoteRenderedTerminalBytes(records, providerRunId, agentId)) {
    process.stdout.write(bytes)
  }
}

function claudeRemoteRenderedTerminalBytes(
  records: unknown,
  providerRunId: string,
  agentId: string,
): Buffer[] {
  const chunks: Buffer[] = []
  if (!Array.isArray(records)) return chunks
  for (const record of records) {
    if (!record || typeof record !== "object") continue
    const payload = record as { provider_run_id?: unknown; agent_id?: unknown; kind?: unknown; bytes?: unknown }
    if (payload.provider_run_id !== providerRunId && payload.agent_id !== agentId) continue
    if (payload.kind !== PROVIDER_TERMINAL_OUTPUT_KIND) continue
    const bytes = Array.isArray(payload.bytes) ? Buffer.from(payload.bytes as number[]) : null
    if (bytes?.length) chunks.push(bytes)
  }
  return chunks
}
