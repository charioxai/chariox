import type { PromptAttachmentPart } from "./kernel-types.js"

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

export function cancelActivePromptRequest(sessionId: string, attachmentId: string, targetAgentId?: string | null) {
  return {
    CancelActivePrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
      ...(targetAgentId ? { target_agent_id: targetAgentId } : {}),
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

export function respondToInteractionRequest(
  sessionId: string,
  interactionId: string,
  choiceId: string,
  customReply?: string | null,
) {
  return {
    RespondToInteraction: {
      session_id: sessionId,
      interaction_id: interactionId,
      choice_id: choiceId,
      custom_reply: customReply ?? null,
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

export function requestNativeProviderInteractionRequest(
  sessionId: string,
  agentId: string,
  interactionId: string,
  title: string | null,
  message: string,
  timeoutSec = 300,
) {
  return {
    RequestNativeProviderInteraction: {
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
