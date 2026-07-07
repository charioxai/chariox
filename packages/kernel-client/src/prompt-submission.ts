import type { PromptAttachmentPart, PromptQueueItem, PromptSubmittedPayload, RuntimeSession } from "./kernel-types.js"
import type { TranscriptPromptMetadata } from "./transcript-entry-state.js"
import {
  promptOriginFromRecord,
} from "./prompt-origin.js"
import {
  sessionActivePromptIdForAgent,
  sessionPromptForAgent,
} from "./session-prompt-identity.js"
import {
  sessionHasTurnWork,
  sessionProjectedStreamingAgentId,
  sessionQueuedPromptCount,
} from "./session-prompt-work.js"

export type PromptSubmissionAttachmentInput = Pick<PromptAttachmentPart, "url" | "mime" | "filename">

type SparsePromptSubmissionPromptPayload = {
  readonly outcome?: Record<string, unknown> | null
  readonly session?: RuntimeSession | null
}

export type PromptSubmittedResponsePayload = {
  readonly outcome?: Record<string, unknown> | null
  readonly session?: RuntimeSession | null
  readonly agent_activity?: PromptSubmittedPayload["agent_activity"] | null
  readonly agent_activity_revision?: number | null
}

export type PromptSubmitPreparationDecision =
  | { readonly action: "clear_empty" }
  | { readonly action: "workspace_shell" }
  | { readonly action: "instructions_editor_open" }
  | {
    readonly action: "continue"
    readonly trimmedPrompt: string
    readonly allowSlashCommandSubmission: boolean
  }

export type DetachedPromptBootstrapResult = "unhandled" | "handled" | "bootstrapped"

export type DetachedPromptSubmitDecision =
  | { readonly action: "flash_start_or_join_session" }
  | { readonly action: "flash_attachments_require_session" }
  | { readonly action: "bootstrap" }
  | { readonly action: "keep_bootstrap_handled" }
  | { readonly action: "submit_bootstrapped_prompt" }
  | { readonly action: "flash_no_session_and_clear" }

export function formatPromptSubmissionBody(rawPrompt: string): string {
  return rawPrompt.trim() ? (rawPrompt.endsWith("\n") ? rawPrompt : `${rawPrompt}\n`) : ""
}

export function promptSubmitPreparationDecision(options: {
  readonly rawPrompt: string
  readonly pendingAttachmentCount: number
  readonly workflowScreenShowing: boolean
  readonly workspaceShellCommand: boolean
  readonly workflowNodeInstructionsEditorOpen: boolean
  readonly workflowCommandInput: boolean
}): PromptSubmitPreparationDecision {
  const trimmedPrompt = options.rawPrompt.trim()
  if (!trimmedPrompt && options.pendingAttachmentCount === 0) {
    return { action: "clear_empty" }
  }
  if (options.workflowScreenShowing && options.workspaceShellCommand) {
    return { action: "workspace_shell" }
  }
  if (options.workflowNodeInstructionsEditorOpen && !trimmedPrompt.startsWith("/")) {
    return { action: "instructions_editor_open" }
  }
  return {
    action: "continue",
    trimmedPrompt,
    allowSlashCommandSubmission: !options.workflowScreenShowing || options.workflowCommandInput,
  }
}

export function detachedPromptSubmitDecision(options: {
  readonly trimmedPrompt: string
  readonly pendingAttachmentCount: number
  readonly bootstrapResult?: DetachedPromptBootstrapResult | null
  readonly attachedAfterBootstrap?: boolean
}): DetachedPromptSubmitDecision {
  if (options.trimmedPrompt.startsWith("/")) {
    return { action: "flash_start_or_join_session" }
  }
  if (options.pendingAttachmentCount > 0) {
    return { action: "flash_attachments_require_session" }
  }
  if (!options.bootstrapResult) {
    return { action: "bootstrap" }
  }
  if (options.bootstrapResult === "handled") {
    return { action: "keep_bootstrap_handled" }
  }
  if (options.bootstrapResult === "bootstrapped" && options.attachedAfterBootstrap) {
    return { action: "submit_bootstrapped_prompt" }
  }
  return { action: "flash_no_session_and_clear" }
}

export function promptSubmissionAttachmentsToParts(
  attachments: readonly PromptSubmissionAttachmentInput[],
): PromptAttachmentPart[] {
  return attachments.map((file) => ({
    url: file.url,
    mime: file.mime,
    filename: file.filename,
  }))
}

export function resolvePromptSubmissionTargetAgentId(options: {
  readonly requestedTargetAgentId?: string | null
  readonly hasAgent: (agentId: string) => boolean
}): string | null {
  const requestedTargetAgentId = options.requestedTargetAgentId ?? null
  return requestedTargetAgentId && options.hasAgent(requestedTargetAgentId)
    ? requestedTargetAgentId
    : null
}

export function promptSubmittedPayloadFromResponse(
  response: Record<string, unknown>,
): PromptSubmittedResponsePayload | null {
  const payload = response.PromptSubmitted
  return payload && typeof payload === "object" && !Array.isArray(payload)
    ? payload as PromptSubmittedResponsePayload
    : null
}

export function expectPromptSubmittedPayload(response: Record<string, unknown>): PromptSubmittedPayload {
  const payload = promptSubmittedPayloadFromResponse(response)
  if (!payload) {
    throw new Error("unexpected response variant: expected PromptSubmitted")
  }
  return payload as PromptSubmittedPayload
}

export function promptSubmissionOutcomeName(payload: { readonly outcome?: Record<string, unknown> | null }): string {
  return Object.keys(payload.outcome ?? {})[0] ?? "unknown"
}

export function promptSubmissionTargetAgentId(payload: PromptSubmittedPayload): string | null {
  const prompt = promptSubmissionPromptFromOutcome(payload)
  return typeof prompt?.target_agent_id === "string" ? prompt.target_agent_id : null
}

export function promptSubmissionTranscriptMetadata(
  payload: PromptSubmittedPayload,
  targetAgentId: string | null,
): TranscriptPromptMetadata {
  const prompt = promptSubmissionPrompt(payload, targetAgentId)
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

export function promptSubmissionPrompt(
  payload: PromptSubmittedPayload | SparsePromptSubmissionPromptPayload,
  targetAgentId: string | null,
): PromptQueueItem | null {
  return promptSubmissionPromptFromOutcome(payload)
    ?? (payload.session && targetAgentId ? sessionPromptForAgent(payload.session, targetAgentId) : null)
}

function promptSubmissionPromptFromOutcome(payload: PromptSubmittedPayload | SparsePromptSubmissionPromptPayload): PromptQueueItem | null {
  const outcome = payload.outcome && typeof payload.outcome === "object"
    ? payload.outcome
    : {}
  for (const variant of Object.values(outcome)) {
    if (!variant || typeof variant !== "object") {
      continue
    }
    const prompt = (variant as { prompt?: unknown }).prompt
    if (prompt && typeof prompt === "object" && !Array.isArray(prompt)) {
      return prompt as PromptQueueItem
    }
  }
  return null
}

export function formatPromptSubmissionStatusLine(options: {
  readonly outcomeName: string
  readonly activePromptId?: string | null
}): string {
  return options.outcomeName === "Queued"
    ? `Prompt queued behind ${options.activePromptId ?? "the active turn"}.`
    : "Prompt submitted."
}

export function promptSubmissionRuntimeState(options: {
  readonly session: RuntimeSession
  readonly outcomeName: string
  readonly submittedTargetAgentId?: string | null
}): {
  readonly streamingAgentId: string | null
  readonly working: boolean
} {
  const projectedStreamingAgentId = sessionProjectedStreamingAgentId(options.session)
  if (options.outcomeName === "Queued") {
    return {
      streamingAgentId: projectedStreamingAgentId,
      working: sessionHasTurnWork(options.session),
    }
  }
  return {
    streamingAgentId: projectedStreamingAgentId ?? options.submittedTargetAgentId ?? null,
    working: sessionHasTurnWork(options.session) || options.submittedTargetAgentId != null,
  }
}

export function promptSubmissionSuccessTransition(options: {
  readonly session: RuntimeSession
  readonly outcomeName: string
  readonly submittedTargetAgentId?: string | null
}): {
  readonly shouldAppendUserPrompt: boolean
  readonly activePromptId: string | null
  readonly queuedPromptCount: number
  readonly statusLine: string
  readonly streamingAgentId: string | null
  readonly working: boolean
} {
  const activePromptId = sessionActivePromptIdForAgent(options.session, options.submittedTargetAgentId)
  const runtimeState = promptSubmissionRuntimeState(options)
  return {
    shouldAppendUserPrompt: options.outcomeName !== "Queued",
    activePromptId,
    queuedPromptCount: sessionQueuedPromptCount(options.session, options.submittedTargetAgentId),
    statusLine: formatPromptSubmissionStatusLine({
      outcomeName: options.outcomeName,
      activePromptId,
    }),
    streamingAgentId: runtimeState.streamingAgentId,
    working: runtimeState.working,
  }
}

export function promptSubmissionFailureRuntimeState(session: RuntimeSession): {
  readonly streamingAgentId: string | null
  readonly working: boolean
} {
  return {
    streamingAgentId: sessionProjectedStreamingAgentId(session),
    working: sessionHasTurnWork(session),
  }
}

export function promptSubmissionFailureTransition(options: {
  readonly session: RuntimeSession
  readonly submittingAgentId?: string | null
}): {
  readonly clearBusyAgentId: string | null
  readonly submittingAgentId: null
  readonly submitting: false
  readonly streamingAgentId: string | null
  readonly working: boolean
} {
  const runtimeState = promptSubmissionFailureRuntimeState(options.session)
  return {
    clearBusyAgentId: options.submittingAgentId ?? null,
    submittingAgentId: null,
    submitting: false,
    streamingAgentId: runtimeState.streamingAgentId,
    working: runtimeState.working,
  }
}
