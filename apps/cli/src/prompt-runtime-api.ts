import {
  normalizeRuntimeSession,
  normalizeRuntimeSessionWithAgentActivity,
  type CliOptions,
  type PromptAttachmentPart,
  type PromptSubmittedPayload,
  type QueuedPromptCancelledPayload,
  type QueuedPromptSteeredPayload,
  type RuntimeSession,
} from "./cli-types.js"
import type { TranscriptPromptMetadata } from "@arroba/kernel-client/transcript-entry-state"
import type { LocalIpcClient } from "./ipc.js"
import type { ArrobaLogger } from "./logging.js"
import {
  cancelActivePromptRequest,
  cancelQueuedPromptRequest,
  respondToInteractionRequest,
  steerQueuedPromptRequest,
  submitPromptRequest,
} from "./ipc-requests.js"
import { expectVariant, firstVariantName } from "./ipc-response.js"
import { launchProviderRun } from "./provider-api.js"
import { describeCliError } from "./runtime.js"
import { resizeSessionTerminal } from "./session-runtime-api.js"
import { resolvePromptRecoveryProviderLaunch } from "@arroba/kernel-client/session-lifecycle-state"
import { sessionPromptForAgent } from "@arroba/kernel-client/session-prompt-identity"
import { promptOriginFromRecord } from "@arroba/kernel-client/prompt-origin"

export type PromptSubmissionResult = {
  payload: PromptSubmittedPayload
  targetAgentId: string | null
  outcomeName: string
}

export function promptSubmissionTranscriptMetadata(
  payload: PromptSubmittedPayload,
  targetAgentId: string | null,
): TranscriptPromptMetadata {
  const prompt = submittedPrompt(payload, targetAgentId)
  if (!prompt) {
    return {}
  }
  const promptOrigin = promptOriginFromRecord(prompt)
  return {
    promptId: prompt.id,
    sourceAttachmentId: prompt.source_attachment_id,
    ...(promptOrigin !== null || Object.prototype.hasOwnProperty.call(prompt, "prompt_origin")
      ? { promptOrigin }
      : {}),
  }
}

export async function submitPromptWithRecovery(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  targetAgentId: string | null,
  prompt: string,
  attachments: PromptAttachmentPart[],
  getSession: () => RuntimeSession,
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
    const session = getSession()
    const recoveryLaunch = resolvePromptRecoveryProviderLaunch(session, {
      provider: options.provider ?? "opencode",
      model: options.model,
      effort: options.effort,
    }, targetAgentId)
    if (recoveryLaunch.action === "skip_launch") {
      logger?.warn("skipping provider recovery launch", {
        session_id: sessionId,
        reason: recoveryLaunch.reason,
        target_agent_id: targetAgentId,
      })
      throw error
    }
    await launchProviderRun(
      client,
      sessionId,
      recoveryLaunch.launch.provider,
      options.accountProfile,
      recoveryLaunch.launch.model,
      recoveryLaunch.launch.effort,
      recoveryLaunch.targetAgentId,
    )
    await resizeSessionTerminal(client, sessionId)
    logger?.info("relaunched provider after recoverable prompt failure", {
      session_id: sessionId,
    })
    return submitPrompt(client, sessionId, attachmentId, targetAgentId, prompt, attachments)
  }
}

function submittedPromptTargetAgentId(payload: PromptSubmittedPayload) {
  const prompt = submittedPromptFromOutcome(payload)
  return typeof prompt?.target_agent_id === "string" ? prompt.target_agent_id : null
}

function submittedPrompt(
  payload: PromptSubmittedPayload,
  targetAgentId: string | null,
) {
  return submittedPromptFromOutcome(payload)
    ?? (targetAgentId ? sessionPromptForAgent(payload.session, targetAgentId) : null)
}

function submittedPromptFromOutcome(payload: PromptSubmittedPayload) {
  const outcome = payload.outcome as Record<string, unknown>
  for (const variant of Object.values(outcome)) {
    if (!variant || typeof variant !== "object") {
      continue
    }
    const prompt = (variant as { prompt?: unknown }).prompt
    if (prompt && typeof prompt === "object" && !Array.isArray(prompt)) {
      return prompt as {
        id: string
        source_attachment_id: string
        target_agent_id?: string | null
        prompt_origin?: string | null
      }
    }
  }
  return null
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
    session: normalizeRuntimeSessionWithAgentActivity(payload),
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

export async function steerQueuedPrompt(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  targetAgentId: string,
  promptId: string,
): Promise<QueuedPromptSteeredPayload> {
  const response = await client.send<Record<string, unknown>>(
    steerQueuedPromptRequest(sessionId, attachmentId, targetAgentId, promptId),
  )
  const payload = expectVariant<QueuedPromptSteeredPayload>(response, "QueuedPromptSteered")
  return {
    ...payload,
    session: normalizeRuntimeSessionWithAgentActivity(payload),
  }
}

export async function cancelQueuedPrompt(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  targetAgentId: string,
  promptId: string,
): Promise<QueuedPromptCancelledPayload> {
  const response = await client.send<Record<string, unknown>>(
    cancelQueuedPromptRequest(sessionId, attachmentId, targetAgentId, promptId),
  )
  const payload = expectVariant<QueuedPromptCancelledPayload>(response, "QueuedPromptCancelled")
  return {
    ...payload,
    session: normalizeRuntimeSessionWithAgentActivity(payload),
  }
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
