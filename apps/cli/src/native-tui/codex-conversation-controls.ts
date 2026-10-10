import { traceCodexLifecycle } from "./codex-lifecycle-trace.js"
import { setTimeout as sleep } from "node:timers/promises"

import type { LocalIpcClient } from "../ipc.js"
import type { CodexJsonRpcMessage } from "./codex-json-rpc.js"
import type { CodexNativeBindingState } from "./codex-turn-submission.js"
import { getNativeProviderRun } from "./provider-run-control.js"
import { steerActivePromptRequest } from "../ipc-requests.js"
import { preparePromptAttachmentsForSubmit } from "../prompt-attachment-transfer.js"
import { extractCodexAttachments, extractCodexSteeringPrompt } from "./codex-prompt.js"

// MP-08 / MP-10: Both the turn/start reply and later output projections
// identify the home prompt. Keep bounded display aliases, never provider IDs.
export function bindCodexDisplayTurn(state: CodexNativeBindingState, threadId: string, turnId: string, promptId: string) {
  const turns = state.displayTurns ??= new Map()
  turns.set(turnId, { threadId, promptId })
  if (turns.size > 64) turns.delete(turns.keys().next().value!)
}

export async function handleCodexNativeTurnSteer(
  message: CodexJsonRpcMessage,
  options: {
    client: LocalIpcClient, sessionId: string, attachmentId: string, agentId: string,
    bindState: CodexNativeBindingState, inlineLocalAttachments: boolean,
  },
  sendClient: (message: unknown) => void,
) {
  try {
    traceCodexLifecycle("steer_received", { method: message.method,
      turnId: typeof message.params?.expectedTurnId === "string" ? message.params.expectedTurnId : null,
      clientId: typeof message.params?.clientUserMessageId === "string" ? message.params.clientUserMessageId : null,
      hasClientUserMessageId: typeof message.params?.clientUserMessageId === "string",
      hasClientId: typeof message.params?.clientId === "string" })
    const turnId = message.params?.expectedTurnId
    const bound = typeof turnId === "string" ? options.bindState.displayTurns?.get(turnId) : undefined
    if (!bound || bound.threadId !== message.params?.threadId) {
      const reason = typeof turnId !== "string" ? "expectedTurnId missing" : !bound ? (turnId.startsWith("chariox-projected-") ? "unbound anonymous projection" : turnId.startsWith("chariox-native-") ? "unbound home prompt projection" : "unbound provider turn") : "thread mismatch"
      throw new Error(`Codex display turn is not bound to a Chariox prompt (${reason})`)
    }
    const attachments = await preparePromptAttachmentsForSubmit(extractCodexAttachments(message.params), {
      inlineLocalFiles: options.inlineLocalAttachments,
    })
    const response = await options.client.send<Record<string, unknown>>(steerActivePromptRequest(
      options.sessionId, options.attachmentId, options.agentId, bound.promptId,
      extractCodexSteeringPrompt(message.params), attachments,
    ))
    if (!("ActivePromptSteered" in response)) throw new Error("kernel did not admit active prompt steering")
    // MP-08 / MP-10 / MP-11: The kernel echoes steering to other attachments.
    // Codex also needs a committed receipt on the submitting display to clear
    // its pending input. Send it only to this caller, after exact-turn admission.
    const promptId = (response.ActivePromptSteered as { prompt?: { id?: unknown } })?.prompt?.id
    if (typeof promptId !== "string") throw new Error("kernel did not return a steering prompt identity")
    const item = {
      type: "userMessage", id: `chariox-steering-${promptId}`,
      clientId: typeof message.params?.clientUserMessageId === "string" ? message.params.clientUserMessageId : null,
      content: Array.isArray(message.params?.input) ? message.params.input : [],
    }
    traceCodexLifecycle("steer_receipt", { promptId, turnId: typeof turnId === "string" ? turnId : null, threadId: bound.threadId, clientId: typeof item.clientId === "string" ? item.clientId : null })
    const receiptAtMs = Date.now()
    for (const method of ["item/started", "item/completed"]) {
      // The official typed app-server client requires each lifecycle timestamp.
      // Missing fields discard the receipt and leave already-admitted input pending.
      const timestamp = method === "item/started"
        ? { startedAtMs: receiptAtMs } : { completedAtMs: receiptAtMs }
      sendClient({ jsonrpc: "2.0", method, params: { threadId: bound.threadId, turnId, item, ...timestamp } })
    }
    sendClient({ id: message.id, result: { turnId } })
  } catch (error) {
    sendClient({ id: message.id, error: { code: -32000, message: error instanceof Error ? error.message : String(error) } })
  }
}

// MP-08 / MP-10: A display thread is not the managed conversation. Resolve
// the kernel-owned run on each control so a stale launch snapshot cannot
// compact an empty display thread or a previous execution conversation.
export async function mapCodexNativeCompaction(
  message: CodexJsonRpcMessage,
  options: {
    client: LocalIpcClient
    sessionId: string
    agentId: string
    bindState: CodexNativeBindingState
  },
  timeoutMs = 15_000,
): Promise<CodexJsonRpcMessage> {
  const boundRun = options.bindState.run
  if (!boundRun) throw new Error("Codex conversation is not bound to Chariox yet")
  const deadline = Date.now() + timeoutMs
  const timeoutError = () => new Error("timed out waiting for the managed Codex conversation")
  while (Date.now() < deadline) {
    let timer: ReturnType<typeof setTimeout> | undefined
    const run = await Promise.race([
      getNativeProviderRun(options.client, boundRun.id),
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(() => reject(timeoutError()), Math.max(0, deadline - Date.now()))
      }),
    ]).finally(() => clearTimeout(timer))
    if (run.id !== boundRun.id || run.session_id !== options.sessionId
      || run.agent_instance_id !== options.agentId || run.provider !== "codex"
      || run.state !== "Running" || run.structured_endpoint !== boundRun.structured_endpoint) {
      throw new Error("managed Codex conversation changed or ended before compaction")
    }
    if (run.provider_session_id) {
      return { ...message, params: { ...message.params, threadId: run.provider_session_id } }
    }
    await sleep(Math.min(100, Math.max(0, deadline - Date.now())))
  }
  throw timeoutError()
}
