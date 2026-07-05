import type { WorkflowRun } from "./publication-types.js"

type WorkflowMessage = NonNullable<WorkflowRun["messages"]>[number]

export function visibleAssistantWorkflowMessage(message: WorkflowMessage): WorkflowMessage {
  const visible = { ...message }
  visible.handoff_payload = sanitizeAssistantHandoffPayload(message.handoff_payload)
  const record = visible as Record<string, unknown>
  if (record.parsed_handoff_payload !== undefined) {
    record.parsed_handoff_payload = sanitizeAssistantHandoffValue(record.parsed_handoff_payload)
  }
  return visible
}

export function sanitizeAssistantHandoffPayload(payload: string): string {
  try {
    return JSON.stringify(sanitizeAssistantHandoffValue(JSON.parse(payload)))
  } catch {
    return payload
  }
}

function sanitizeAssistantHandoffValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sanitizeAssistantHandoffValue)
  if (!value || typeof value !== "object") return value
  const record = value as Record<string, unknown>
  const sanitized = Object.fromEntries(
    Object.entries(record).map(([key, entry]) => [key, sanitizeAssistantHandoffValue(entry)]),
  ) as Record<string, unknown>
  if (sanitized.completion && typeof sanitized.completion === "object" && !Array.isArray(sanitized.completion)) {
    delete (sanitized.completion as Record<string, unknown>).summary
  }
  return sanitized
}
