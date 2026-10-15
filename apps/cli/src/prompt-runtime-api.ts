import { hasSkillReferences, skillsRuntimeAvailable } from "@chariox/kernel-client/skills-client"
import type { SkillSelection, SkillsV2Response } from "@chariox/kernel-client/kernel-types-skills-v2"
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
import type { LocalIpcClient } from "./ipc.js"
import type { CharioxLogger } from "./logging.js"
import {
  cancelActivePromptRequest,
  cancelQueuedPromptRequest,
  respondToInteractionRequest,
  steerQueuedPromptRequest,
  submitPromptRequest,
  type InteractionPasskeyProof,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import { launchProviderRun } from "./provider-api.js"
import { describeCliError } from "./runtime.js"
import { resizeSessionTerminal } from "./session-runtime-api.js"
import { resolvePromptRecoveryProviderLaunch } from "@chariox/kernel-client/session-lifecycle-state"
import {
  expectPromptSubmittedPayload,
  promptSubmissionOutcomeName,
  promptSubmissionTargetAgentId,
  promptSubmissionTranscriptMetadata,
} from "@chariox/kernel-client/prompt-submission"

export {
  promptSubmissionTranscriptMetadata,
} from "@chariox/kernel-client/prompt-submission"

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
  getSession: () => RuntimeSession,
  options: CliOptions,
  logger?: CharioxLogger | null,
  selections?: SkillSelection[] | null,
): Promise<PromptSubmissionResult> {
  try {
    return await submitPrompt(client, sessionId, attachmentId, targetAgentId, prompt, attachments,selections)
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
    return submitPrompt(client, sessionId, attachmentId, targetAgentId, prompt, attachments,selections)
  }
}

async function submitPrompt(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  targetAgentId: string | null,
  prompt: string,
  attachments: PromptAttachmentPart[],
  selections?: SkillSelection[] | null,
): Promise<PromptSubmissionResult> {
  let response:Record<string,unknown>
  const needsSkills = selections !== null && selections !== undefined || hasSkillReferences(prompt)
  const available = needsSkills && await skillsRuntimeAvailable(request => client.send(request))
  if (needsSkills && !available && selections?.length) throw new Error("Skills v2 runtime is unavailable. Upgrade the kernel and reselect Skills chips.")
  if(available) {
    if(!targetAgentId)throw new Error("Focus an agent to submit Skills selections")
    const result=await client.send<{SkillsV2?:{result?:SkillsV2Response}}>({SkillsV2:{action:"submit",session_id:sessionId,attachment_id:attachmentId,target_agent_id:targetAgentId,prompt,attachments,selections:selections??[]}})
    if(result.SkillsV2?.result?.kind!=="submitted")throw new Error("Kernel did not acknowledge the Skills prompt")
    const state=await client.send<{SessionState:{session:RuntimeSession}}>({GetSessionState:{session_id:sessionId}})
    response={PromptSubmitted:{...state.SessionState,outcome:result.SkillsV2.result.outcome,session:normalizeRuntimeSessionWithAgentActivity(state.SessionState)}}
  } else {
    response=await client.send<Record<string,unknown>>(submitPromptRequest(sessionId,attachmentId,targetAgentId,prompt,attachments))
  }
  const payload = expectPromptSubmittedPayload(response) as PromptSubmittedPayload
  const normalizedPayload = {
    ...payload,
    session: normalizeRuntimeSessionWithAgentActivity(payload),
  }
  return {
    payload: normalizedPayload,
    targetAgentId: promptSubmissionTargetAgentId(normalizedPayload),
    outcomeName: promptSubmissionOutcomeName(normalizedPayload),
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
  proof?: InteractionPasskeyProof | null,
): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(
    respondToInteractionRequest(sessionId, interactionId, choiceId, customReply, proof),
  )
  const payload = expectVariant<{ interaction_id: string; session: RuntimeSession }>(response, "InteractionResponded")
  if (payload.interaction_id !== interactionId || payload.session?.id !== sessionId) {
    throw new Error("interaction response identity mismatch")
  }
  return normalizeRuntimeSession(payload.session)
}

function isRecoverableProviderError(error: unknown): boolean {
  const message = describeCliError(error)
  return message.includes("has no active provider run") || message.includes("cannot perform `submit prompt` while ended")
}

/** MP-08 / MP-10 / MP-11: kernel-wide access popup; no session projection. */
export async function respondToKernelAccessDecision(
  client: LocalIpcClient, interactionId: string, choiceId: string,
  proof?: InteractionPasskeyProof | null,
): Promise<void> {
  const response = await client.send<Record<string, unknown>>(
    respondToInteractionRequest("kernel-access", interactionId, choiceId, null, proof),
  )
  const payload = expectVariant<{ interaction_id: string }>(response, "KernelAccessDecisionResponded")
  if (payload.interaction_id !== interactionId) throw new Error("access response identity mismatch")
}
