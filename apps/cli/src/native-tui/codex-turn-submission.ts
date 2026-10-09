import { setTimeout as sleep } from "node:timers/promises"

import type { RuntimeProviderRun } from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import { cancelActivePromptRequest, submitPromptRequest } from "../ipc-requests.js"
import { preparePromptAttachmentsForSubmit } from "../prompt-attachment-transfer.js"
import type { CodexJsonRpcMessage } from "./codex-json-rpc.js"
import { extractCodexAttachments, extractCodexPrompt } from "./codex-prompt.js"
import { bindCodexDisplayTurn } from "./codex-conversation-controls.js"

export type CodexNativeBindingState = {
  promise: Promise<RuntimeProviderRun> | null
  run: RuntimeProviderRun | null
  structuredEndpoint?: string | null
  displayTurns?: Map<string, { threadId: string, promptId: string }>
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
    onPromptStarted?: (promptId: string) => void
    debug: (label: string, payload: unknown) => void
  },
  sendClient: (message: unknown) => void,
) {
  try {
    const bindPromise = await waitForNativeBinding(options.bindState)
    if (!bindPromise) {
      throw new Error("Codex thread is not bound to Chariox yet")
    }
    await bindPromise
    const prompt = extractCodexPrompt(message.params)
    const attachments = await preparePromptAttachmentsForSubmit(extractCodexAttachments(message.params), {
      inlineLocalFiles: options.inlineLocalAttachments,
    })
    const response = await options.client.send<{ PromptSubmitted: { outcome: {
      Started?: { prompt: { id: string } }, Queued?: { prompt: { id: string } },
    } } }>(
      submitPromptRequest(options.sessionId, options.attachmentId, options.agentId, prompt, attachments),
    )
    const outcome = response.PromptSubmitted?.outcome
    const promptId = (outcome?.Started ?? outcome?.Queued)?.prompt.id
    const threadId = message.params?.threadId
    if (!promptId || typeof threadId !== "string") throw new Error("kernel did not return a native prompt identity")
    const turnId = `chariox-native-${promptId}`
    bindCodexDisplayTurn(options.bindState, threadId, turnId, promptId)
    if (outcome?.Started) options.onPromptStarted?.(promptId)
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

// MP-08 / MP-10: Display thread/turn IDs are projections. Cancellation must
// resolve the active prompt on the kernel, which owns the real provider turn.
export async function handleCodexNativeTurnInterrupt(
  message: CodexJsonRpcMessage,
  options: { client: LocalIpcClient, sessionId: string, attachmentId: string, agentId: string },
  sendClient: (message: unknown) => void,
) {
  try {
    await options.client.send<Record<string, unknown>>(
      cancelActivePromptRequest(options.sessionId, options.attachmentId, options.agentId),
    )
    sendClient({ id: message.id, result: {} })
  } catch (error) {
    sendClient({ id: message.id, error: {
      code: -32000,
      message: error instanceof Error ? error.message : String(error),
    } })
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
