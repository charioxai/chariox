import type { WorkflowRun } from "./publication-types.js"

type WorkflowMessage = NonNullable<WorkflowRun["messages"]>[number]
type WorkflowNodeRun = NonNullable<WorkflowRun["node_runs"]>[number]
type WorkflowRuntimeToolCall = NonNullable<NonNullable<WorkflowNodeRun["turn_envelope"]>["runtime_tool_calls"]>[number]

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

export function visibleRuntimeToolCall(toolCall: WorkflowRuntimeToolCall): WorkflowRuntimeToolCall {
  const visible = {
    tool_name: toolCall.tool_name,
    ok: toolCall.ok,
    timestamp_ms: toolCall.timestamp_ms,
  } as WorkflowRuntimeToolCall
  const record = toolCall as Record<string, unknown>
  const visibleRecord = visible as Record<string, unknown>
  if (typeof record.error === "string") visibleRecord.error = record.error
  return visible
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
