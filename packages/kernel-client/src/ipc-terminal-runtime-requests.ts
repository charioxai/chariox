import type { PromptAttachmentPart } from "./kernel-types.js"

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

export function resizeTerminalRequest(sessionId: string, cols: number, rows: number) {
  return {
    ResizeTerminal: {
      session_id: sessionId,
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

export function completePromptRequest(sessionId: string) {
  return {
    CompletePrompt: {
      session_id: sessionId,
    },
  }
}

export function cancelActivePromptRequest(sessionId: string, attachmentId: string) {
  return {
    CancelActivePrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
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

export function pollRuntimeNoticesRequest(sessionId: string, attachmentId: string) {
  return {
    PollRuntimeNotices: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
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
