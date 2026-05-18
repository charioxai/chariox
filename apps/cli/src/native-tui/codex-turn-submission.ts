import { setTimeout as sleep } from "node:timers/promises"

import type { RuntimeProviderRun } from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import { submitPromptRequest } from "../ipc-requests.js"
import { preparePromptAttachmentsForSubmit } from "../prompt-attachment-transfer.js"
import type { CodexJsonRpcMessage } from "./codex-json-rpc.js"
import { extractCodexAttachments, extractCodexPrompt } from "./codex-prompt.js"

export type CodexNativeBindingState = {
  promise: Promise<RuntimeProviderRun> | null
  run: RuntimeProviderRun | null
  structuredEndpoint?: string | null
}

export async function handleCodexNativeTurnStart(
  message: CodexJsonRpcMessage,
  options: {
    client: LocalIpcClient
    sessionId: string
    attachmentId: string
    agentId: string
    bindState: CodexNativeBindingState
    inlineLocalAttachments: boolean
    debug: (label: string, payload: unknown) => void
  },
  sendClient: (message: unknown) => void,
) {
  try {
    const bindPromise = await waitForNativeBinding(options.bindState)
    if (!bindPromise) {
      throw new Error("Codex thread is not bound to Arroba yet")
    }
    await bindPromise
    const prompt = extractCodexPrompt(message.params)
    const attachments = await preparePromptAttachmentsForSubmit(extractCodexAttachments(message.params), {
      inlineLocalFiles: options.inlineLocalAttachments,
    })
    await options.client.send<Record<string, unknown>>(
      submitPromptRequest(options.sessionId, options.attachmentId, options.agentId, prompt, attachments),
    )
    const turnId = `arroba-native-${Date.now()}`
    sendClient({
      id: message.id,
      result: {
        turn: {
          id: turnId,
          items: [],
          itemsView: "notLoaded",
          status: "inProgress",
          error: null,
          startedAt: null,
          completedAt: null,
          durationMs: null,
        },
      },
    })
    options.debug("native_prompt_submitted", { agentId: options.agentId, prompt, attachmentCount: attachments.length })
  } catch (error) {
    sendClient({
      id: message.id,
      error: {
        code: -32000,
        message: error instanceof Error ? error.message : String(error),
      },
    })
  }
}

async function waitForNativeBinding(bindState: {
  promise: Promise<RuntimeProviderRun> | null
}): Promise<RuntimeProviderRun | null> {
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    if (bindState.promise) return bindState.promise
    await sleep(100)
  }
  return bindState.promise
}
