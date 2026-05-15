import {
  normalizeRuntimeSession,
  type CliOptions,
  type PromptAttachmentPart,
  type PromptSubmittedPayload,
  type RuntimeSession,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import type { ArrobaLogger } from "./logging.js"
import {
  cancelActivePromptRequest,
  respondToInteractionRequest,
  submitPromptRequest,
} from "./ipc-requests.js"
import { expectVariant, firstVariantName } from "./ipc-response.js"
import { launchProviderRun } from "./provider-api.js"
import { describeCliError } from "./runtime.js"
import { resizeSessionTerminal } from "./session-runtime-api.js"

export type PromptSubmissionResult = {
  payload: PromptSubmittedPayload
  targetAgentId: string | null
  outcomeName: string
}

export async function submitPromptWithRecovery(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  targetAgentId: string | null,
  prompt: string,
  attachments: PromptAttachmentPart[],
  options: CliOptions,
  logger?: ArrobaLogger | null,
): Promise<PromptSubmissionResult> {
  try {
    return await submitPrompt(client, sessionId, attachmentId, targetAgentId, prompt, attachments)
  } catch (error) {
    if (!isRecoverableProviderError(error)) {
      throw error
    }

    logger?.warn("prompt submission hit recoverable provider error", {
      error: describeCliError(error),
      session_id: sessionId,
    })
    await launchProviderRun(
      client,
      sessionId,
      options.provider ?? "opencode",
      options.accountProfile,
      options.model,
      options.effort,
      targetAgentId,
    )
    await resizeSessionTerminal(client, sessionId)
    logger?.info("relaunched provider after recoverable prompt failure", {
      session_id: sessionId,
    })
    return submitPrompt(client, sessionId, attachmentId, targetAgentId, prompt, attachments)
  }
}

function submittedPromptTargetAgentId(payload: PromptSubmittedPayload) {
  const outcome = payload.outcome as Record<string, unknown>
  const variant = Object.values(outcome)[0]
  if (!variant || typeof variant !== "object") {
    return null
  }
  const prompt = (variant as { prompt?: { target_agent_id?: unknown } }).prompt
  return typeof prompt?.target_agent_id === "string" ? prompt.target_agent_id : null
}

async function submitPrompt(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  targetAgentId: string | null,
  prompt: string,
  attachments: PromptAttachmentPart[],
): Promise<PromptSubmissionResult> {
  const response = await client.send<Record<string, unknown>>(
    submitPromptRequest(sessionId, attachmentId, targetAgentId, prompt, attachments),
  )
  const payload = expectVariant<PromptSubmittedPayload>(response, "PromptSubmitted")
  const normalizedPayload = {
    ...payload,
    session: normalizeRuntimeSession(payload.session),
  }
  return {
    payload: normalizedPayload,
    targetAgentId: submittedPromptTargetAgentId(normalizedPayload),
    outcomeName: firstVariantName(normalizedPayload.outcome),
  }
}

export async function cancelActivePrompt(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
): Promise<void> {
  await client.send<Record<string, unknown>>(cancelActivePromptRequest(sessionId, attachmentId))
}

export async function respondToInteraction(
  client: LocalIpcClient,
  sessionId: string,
  interactionId: string,
  choiceId: string,
  customReply: string | null,
): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(
    respondToInteractionRequest(sessionId, interactionId, choiceId, customReply),
  )
  const payload = expectVariant<{ session: RuntimeSession }>(response, "InteractionResponded")
  return normalizeRuntimeSession(payload.session)
}

function isRecoverableProviderError(error: unknown): boolean {
  const message = describeCliError(error)
  return message.includes("has no active provider run") || message.includes("cannot perform `submit prompt` while ended")
}
