import {
  readStringField,
} from "./kernel-response-parsing.js"

export const CHARIOX_PROMPT_ORIGIN = "chariox"
export const EXTERNAL_PROMPT_ORIGIN = "external"
/** Prompts Chariox shows as its own: typed by a person, or fired by a trigger or notification. */
const CHARIOX_PROMPT_ORIGINS = new Set([CHARIOX_PROMPT_ORIGIN, "schedule", "http", "notification", "app_event"])

export function terminalPromptOriginIsChariox(origin: string | null | undefined): boolean {
  const normalized = normalizeTerminalPromptOrigin(origin)
  return normalized !== null && CHARIOX_PROMPT_ORIGINS.has(normalized)
}
const WORKFLOW_PROMPT_SOURCE_ATTACHMENT_PREFIX = "workflow-run:"
const WORKFLOW_PROMPT_TEXT_MARKERS = [
  "<metaagent-event>",
  "<endpoint-prompt>",
  "<workflow-level-prompt>",
  "<node-level-prompt>",
  "<workflow-runtime-instructions>",
  "<system-node-level-prompt>",
  "<workflow-handoff-payloads>",
  "<outgoing-edge-contracts>",
  "<node-instruction-reference>",
  "<control-mailbox>",
  "You are an agent participating in a Chariox workflow turn.",
  "System node-level prompt:",
]

export function terminalPromptOriginFromRecord(
  record: TerminalPromptOriginRecord | null,
  fallback?: string | null,
): string | null {
  return normalizeTerminalPromptOrigin(readStringField(record, "prompt_origin"))
    ?? normalizeTerminalPromptOrigin(fallback)
}

export function terminalPromptOriginIsExternal(origin: string | null | undefined): boolean {
  return normalizeTerminalPromptOrigin(origin) === EXTERNAL_PROMPT_ORIGIN
}

export function normalizeTerminalPromptOrigin(origin: string | null | undefined): string | null {
  const normalized = origin?.trim().toLowerCase()
  return normalized ? normalized : null
}

export function terminalSourceAttachmentImpliesCharioxPromptOrigin(
  sourceAttachmentId: string | null | undefined,
): boolean {
  return sourceAttachmentId?.startsWith(WORKFLOW_PROMPT_SOURCE_ATTACHMENT_PREFIX) === true
}

export function terminalPromptTextImpliesCharioxPromptOrigin(
  text: string | null | undefined,
): boolean {
  return Boolean(text && WORKFLOW_PROMPT_TEXT_MARKERS.some((marker) => text.includes(marker)))
}

export function terminalPromptTextIsPrivateContinuation(
  text: string | null | undefined,
): boolean {
  const normalized = text?.trim()
  return normalized === "<metaagent-event/>"
    || Boolean(
      normalized
      && normalized.startsWith("<metaagent-event>")
      && normalized.endsWith("</metaagent-event>"),
    )
}

export type TerminalPromptOriginRecord = {
  readonly prompt_origin?: string | null | undefined
}
