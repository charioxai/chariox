import type { NativeInteractionOrigin } from "./kernel-types-runtime.js"

import type { PromptAttachmentPart } from "./kernel-types.js"
import type { AgentTriggerBusyPolicy } from "./kernel-types-session.js"

export const DEPLOYMENT_CREDENTIAL_ENROLLMENT_SERVICE_SUBJECT_PREFIX = "deployment-credential-enrollment:"

export function deploymentCredentialEnrollmentServiceSubject(enrollmentId: string): string {
  return `${DEPLOYMENT_CREDENTIAL_ENROLLMENT_SERVICE_SUBJECT_PREFIX}${enrollmentId}`
}

export function readDirectoryTreeRequest(sessionId: string, attachmentId: string, treePath: string | null, maxDepth: number) {
  return {
    ReadDirectoryTree: {
      session_id: sessionId,
      attachment_id: attachmentId,
      path: treePath,
      max_depth: maxDepth,
    },
  }
}

export function captureScreenshotRequest(sessionId: string, attachmentId: string) {
  return {
    CaptureScreenshot: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function storeTransferredFileRequest(sessionId: string, attachmentId: string, sourcePath: string, displayName?: string) {
  return {
    StoreTransferredFile: {
      session_id: sessionId,
      attachment_id: attachmentId,
      source_path: sourcePath,
      display_name: displayName ?? null,
    },
  }
}

export function resizeTerminalRequest(
  sessionId: string,
  cols: number,
  rows: number,
  providerRunId?: string | null,
) {
  return {
    ResizeTerminal: {
      session_id: sessionId,
      provider_run_id: providerRunId ?? null,
      cols,
      rows,
    },
  }
}

export function sendTerminalInputRequest(
  sessionId: string,
  attachmentId: string,
  input: string | Uint8Array,
  providerRunId?: string | null,
) {
  const bytes = typeof input === "string" ? Buffer.from(input, "utf8") : Buffer.from(input)
  return {
    SendTerminalInput: {
      session_id: sessionId,
      attachment_id: attachmentId,
      provider_run_id: providerRunId ?? null,
      data_base64: bytes.toString("base64"),
    },
  }
}

export function pumpTerminalOutputRequest(sessionId: string, attachmentId: string) {
  return {
    PumpTerminalOutput: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function appendNativeProviderOutputRequest(
  sessionId: string,
  attachmentId: string,
  providerRunId: string,
  kind: "provider_output" | "provider_reasoning" | "provider_tool" | "provider_error" | "provider_status",
  text: string,
  mergeKey?: string | null,
) {
  return {
    AppendNativeProviderOutput: {
      session_id: sessionId,
      attachment_id: attachmentId,
      provider_run_id: providerRunId,
      kind,
      merge_key: mergeKey ?? null,
      text,
    },
  }
}

export type AppendNativeProviderOutputBatchItem = {
  providerRunId: string
  kind: "provider_output" | "provider_reasoning" | "provider_tool" | "provider_error" | "provider_status"
  text: string
  mergeKey?: string | null
}

export function appendNativeProviderOutputBatchRequest(
  sessionId: string,
  attachmentId: string,
  outputs: AppendNativeProviderOutputBatchItem[],
) {
  return {
    AppendNativeProviderOutputBatch: {
      session_id: sessionId,
      attachment_id: attachmentId,
      outputs: outputs.map((output) => ({
        provider_run_id: output.providerRunId,
        kind: output.kind,
        merge_key: output.mergeKey ?? null,
        text: output.text,
      })),
    },
  }
}

export function submitPromptRequest(
  sessionId: string,
  attachmentId: string,
  targetAgentId: string | null,
  prompt: string,
  attachments: PromptAttachmentPart[],
) {
  return {
    SubmitPrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
      target_agent_id: targetAgentId,
      prompt,
      attachments,
    },
  }
}

export const agentTriggersMinimumProtocolVersion = 900
export const agentTriggersCapability = "agent_triggers_v1"
export const triggerDeliveryMinimumProtocolVersion = 901
export const triggerDeliveryResendCapability = "agent_trigger_resend_v1"

export function createAgentScheduleTriggerRequest(options: {
  sessionId: string
  agentId: string
  repeat: "once" | "every"
  intervalSeconds: number
  busyPolicy: AgentTriggerBusyPolicy
  prompt?: string | null
}) {
  return {
    CreateAgentTrigger: {
      session_id: options.sessionId,
      agent_id: options.agentId,
      source: { kind: "schedule", interval_seconds: options.intervalSeconds, repeat: options.repeat },
      busy_policy: options.busyPolicy,
      ...(options.prompt?.trim() ? { prompt: options.prompt.trim() } : {}),
    },
  }
}

export function updateAgentTriggerRequest(
  sessionId: string,
  triggerId: string,
  changes: { busyPolicy?: AgentTriggerBusyPolicy; prompt?: string; enabled?: boolean },
) {
  return {
    UpdateAgentTrigger: {
      session_id: sessionId,
      trigger_id: triggerId,
      ...(changes.busyPolicy ? { busy_policy: changes.busyPolicy } : {}),
      ...(changes.prompt !== undefined ? { prompt: changes.prompt } : {}),
      ...(changes.enabled !== undefined ? { enabled: changes.enabled } : {}),
    },
  }
}

export function deleteAgentTriggerRequest(sessionId: string, triggerId: string) {
  return { DeleteAgentTrigger: { session_id: sessionId, trigger_id: triggerId } }
}

export function resendTriggerDeliveryRequest(sessionId: string, promptId: string) {
  return { ResendTriggerDelivery: { session_id: sessionId, prompt_id: promptId } }
}

export type SubmitPromptBatchItem = {
  sessionId?: string | null
  attachmentId?: string | null
  targetAgentId: string
  prompt: string
  attachments: PromptAttachmentPart[]
}

export function submitPromptsRequest(
  sessionId: string,
  attachmentId: string,
  prompts: SubmitPromptBatchItem[],
  maxConcurrency?: number | null,
) {
  validatePromptBatch(sessionId, prompts)
  return {
    SubmitPrompts: {
      session_id: sessionId,
      attachment_id: attachmentId,
      max_concurrency: maxConcurrency ?? null,
      prompts: prompts.map((prompt) => ({
        session_id: prompt.sessionId ?? null,
        attachment_id: prompt.attachmentId ?? null,
        target_agent_id: prompt.targetAgentId,
        prompt: prompt.prompt,
        attachments: prompt.attachments,
      })),
    },
  }
}

function validatePromptBatch(sessionId: string, prompts: SubmitPromptBatchItem[]): void {
  const seenTargets = new Set<string>()
  for (const prompt of prompts) {
    const targetKey = `${prompt.sessionId ?? sessionId}\0${prompt.targetAgentId}`
    if (seenTargets.has(targetKey)) {
      throw new Error("prompt batch contains duplicate target agents")
    }
    seenTargets.add(targetKey)
  }
}

export function completePromptRequest(sessionId: string) {
  return {
    CompletePrompt: {
      session_id: sessionId,
    },
  }
}

// MP-08 / MP-10 / MP-11: old kernels can ignore the optional prompt fence.
export const exactPromptCancellationMinimumProtocolVersion = 491
export const exactPromptCancellationCapability = "exact_prompt_cancellation_v499"

export function cancelActivePromptRequest(sessionId: string, attachmentId: string, targetAgentId?: string | null, expectedPromptId?: string | null) {
  return {
    CancelActivePrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
      ...(targetAgentId ? { target_agent_id: targetAgentId } : {}),
      ...(expectedPromptId !== undefined ? { expected_prompt_id: expectedPromptId } : {}),
    },
  }
}

export function steerQueuedPromptRequest(
  sessionId: string,
  attachmentId: string,
  targetAgentId: string,
  promptId: string,
) {
  return {
    SteerQueuedPrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
      target_agent_id: targetAgentId,
      prompt_id: promptId,
    },
  }
}

// MP-08 / MP-10: Admission targets one home turn and never queues or starts
// a replacement turn if it has already settled.
export const activePromptSteeringMinimumProtocolVersion = 491

export function steerActivePromptRequest(
  sessionId: string,
  attachmentId: string,
  targetAgentId: string,
  expectedActivePromptId: string,
  prompt: string,
  attachments: PromptAttachmentPart[],
) {
  return { SteerActivePrompt: {
    session_id: sessionId,
    attachment_id: attachmentId,
    target_agent_id: targetAgentId,
    expected_active_prompt_id: expectedActivePromptId,
    prompt,
    attachments,
  } }
}

export function cancelQueuedPromptRequest(
  sessionId: string,
  attachmentId: string,
  targetAgentId: string,
  promptId: string,
) {
  return {
    CancelQueuedPrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
      target_agent_id: targetAgentId,
      prompt_id: promptId,
    },
  }
}

export function updateQueuedPromptRequest(
  sessionId: string,
  attachmentId: string,
  targetAgentId: string,
  promptId: string,
  prompt: string,
) {
  return {
    UpdateQueuedPrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
      target_agent_id: targetAgentId,
      prompt_id: promptId,
      prompt,
    },
  }
}

export function pollRuntimeNoticesRequest(sessionId: string, attachmentId: string) {
  return {
    PollRuntimeNotices: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function getTerminalCommandCatalogRequest() {
  return { GetTerminalCommandCatalog: null }
}

/** Protocol 392: the Chariox passkey for a `requires_passkey` choice, and an
 * optional window (1 to 15 minutes) in which the owner's critical approvals
 * need no passkey. */
export type InteractionPasskeyProof = {
  passkey: string
  rememberMinutes?: number | null
  /** Minutes, or `until_exit` for an access grant that ends with its agent. */
  accessLifetime?: number | "until_exit" | null
}

export function respondToInteractionRequest(
  sessionId: string,
  interactionId: string,
  choiceId: string,
  customReply?: string | null,
  proof?: InteractionPasskeyProof | null,
) {
  return {
    RespondToInteraction: {
      session_id: sessionId,
      interaction_id: interactionId,
      choice_id: choiceId,
      custom_reply: proof?.accessLifetime ? String(proof.accessLifetime) : customReply ?? null,
      ...(proof ? {
        passkey: proof.passkey,
        ...(proof.rememberMinutes ? { passkey_remember_minutes: proof.rememberMinutes } : {}),
      } : {}),
    },
  }
}

export function armDeploymentCredentialEnrollmentRequest(
  sessionId: string,
  attachmentId: string,
  agentId: string,
  enrollmentId: string,
  profileId: string,
  targetVersion: number,
) {
  return {
    ArmDeploymentCredentialEnrollment: {
      session_id: sessionId,
      attachment_id: attachmentId,
      agent_id: agentId,
      enrollment_id: enrollmentId,
      profile_id: profileId,
      target_version: targetVersion,
    },
  }
}

export function requestCredentialEnrollmentInteractionRequest(
  sessionId: string,
  agentId: string,
  enrollmentId: string,
  profileId: string,
  targetVersion: number,
  providerAuthorizationUrl: string,
  timeoutSec = 300,
) {
  return {
    RequestCredentialEnrollmentInteraction: {
      session_id: sessionId,
      agent_id: agentId,
      enrollment_id: enrollmentId,
      profile_id: profileId,
      target_version: targetVersion,
      provider_authorization_url: providerAuthorizationUrl,
      timeout_sec: timeoutSec,
    },
  }
}

export const nativeProviderInteractionMinimumProtocolVersion = 405

export function requestNativeProviderInteractionRequest(
  sessionId: string,
  agentId: string,
  interactionId: string,
  title: string | null,
  message: string,
  origin: NativeInteractionOrigin,
  timeoutSec = 300,
) {
  return {
    RequestNativeProviderTurnInteraction: {
      origin,
      session_id: sessionId,
      agent_id: agentId,
      interaction_id: interactionId,
      level: "warning",
      title,
      message,
      choices: [
        {
          id: "allow_once",
          label: "Allow once",
          reply: "allow",
          style: "primary",
        },
        {
          id: "deny",
          label: "Deny",
          reply: "deny",
          style: "danger",
        },
      ],
      custom_choice: null,
      timeout_sec: timeoutSec,
      default_on_timeout: "deny",
    },
  }
}

export function createAgentHttpTriggerRequest(options: { sessionId: string; agentId: string; busyPolicy: AgentTriggerBusyPolicy; prompt?: string | null }) {
  return { CreateAgentTrigger: { session_id: options.sessionId, agent_id: options.agentId, source: { kind: "http" }, busy_policy: options.busyPolicy,
    ...(options.prompt?.trim() ? { prompt: options.prompt.trim() } : {}) } }
}

export function invokeAgentTriggerRequest(sessionId: string, triggerId: string, payload: unknown) {
  return { InvokeAgentTrigger: { session_id: sessionId, trigger_id: triggerId, payload } }
}

export function readAgentTriggerTranscriptRequest(sessionId: string, triggerId: string, cursor: { before_sequence: number } | null = null) {
  return { ReadAgentTriggerTranscript: { session_id: sessionId, trigger_id: triggerId, cursor } }
}
