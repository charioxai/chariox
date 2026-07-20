import type {
  PublicationTraceEvent,
  PublicationTraceLevel,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"
import {
  visibleAssistantWorkflowMessage,
  visibleRuntimeToolCall,
} from "./publication-workflow-message-visibility.js"

export type PublicationTraceStreamState = {
  traceKeys: Set<string>
  nextSequence: number
}

export function createPublicationTraceStreamState(): PublicationTraceStreamState {
  return { traceKeys: new Set(), nextSequence: 1 }
}

export function collectPublicationTraceEvents(
  publication: WorkflowPublicationConfig,
  workflowRun: WorkflowRun,
  state: PublicationTraceStreamState,
): PublicationTraceEvent[] {
  const policy = publication.trace_exposure?.nodes ?? {}
  const events: PublicationTraceEvent[] = []
  const prompt = publicationPrompt(workflowRun)
  for (const nodeRun of workflowRun.node_runs ?? []) {
    const levels = new Set(policy[nodeRun.node_id] ?? [])
    if (levels.size === 0) continue
    if (prompt) {
      pushTraceEvent(events, state, publication, workflowRun, nodeRun, "user_prompt", {
        key: `prompt:${nodeRun.id}:${prompt}`,
        timestampMs: workflowRun.created_at_ms ?? 0,
        message: prompt,
        data: { source: "publication_input" },
      })
    }
    if (levels.has("output_summary")) {
      const summary = nodeRun.completion?.summary?.trim() || nodeRun.summary?.trim()
      if (summary) {
        pushTraceEvent(events, state, publication, workflowRun, nodeRun, "output_summary", {
          key: `summary:${nodeRun.id}:${nodeRun.summary ?? ""}:${nodeRun.completion?.summary ?? ""}`,
          timestampMs: completedTimestamp(nodeRun, workflowRun),
          message: summary,
          data: {
            summary: nodeRun.summary ?? null,
            completion_summary: nodeRun.completion?.summary ?? null,
          },
        })
      }
    }
    if (levels.has("assistant_messages")) {
      for (const message of workflowRun.messages ?? []) {
        if (message.source_node_run_id !== nodeRun.id) continue
        const visibleMessage = visibleAssistantWorkflowMessage(message)
        pushTraceEvent(events, state, publication, workflowRun, nodeRun, "assistant_messages", {
          key: `message:${visibleMessage.id}`,
          timestampMs: visibleMessage.created_at_ms,
          message: visibleMessage.summary || visibleMessage.handoff_payload,
          data: {
            message_id: visibleMessage.id,
            message_type: visibleMessage.message_type,
            summary: visibleMessage.summary,
            handoff_payload: visibleMessage.handoff_payload,
          },
        })
      }
      const outputMessage = completionOutputMessage(nodeRun.completion?.output)
        ?? workflowFinalOutputMessage(workflowRun, nodeRun)
      if (outputMessage) {
        pushTraceEvent(events, state, publication, workflowRun, nodeRun, "assistant_messages", {
          key: `completion-output:${nodeRun.id}:${outputMessage}`,
          timestampMs: completedTimestamp(nodeRun, workflowRun),
          message: outputMessage,
          data: { source: "completion_output" },
        })
      }
    }
    if (levels.has("thinking")) {
      for (const trace of nodeRun.thinking_traces ?? []) {
        pushTraceEvent(events, state, publication, workflowRun, nodeRun, "thinking", {
          key: `thinking:${trace.id}`,
          timestampMs: trace.timestamp_ms,
          message: trace.message,
          data: trace,
        })
      }
    }
    if (levels.has("tool_use")) {
      for (const [index, toolCall] of (nodeRun.turn_envelope?.runtime_tool_calls ?? []).entries()) {
        const visibleToolCall = visibleRuntimeToolCall(toolCall)
        pushTraceEvent(events, state, publication, workflowRun, nodeRun, "tool_use", {
          key: `tool:${nodeRun.id}:${index}:${visibleToolCall.timestamp_ms}`,
          timestampMs: visibleToolCall.timestamp_ms,
          message: `${visibleToolCall.tool_name} ${visibleToolCall.ok ? "ok" : "failed"}`,
          data: visibleToolCall,
        })
      }
    }
  }
  events.sort((left, right) => {
    const levelOrder = traceLevelOrder(left.level) - traceLevelOrder(right.level)
    return levelOrder || left.timestamp_ms - right.timestamp_ms
  })
  for (const event of events) event.sequence = state.nextSequence++
  return events
}

function traceLevelOrder(level: PublicationTraceLevel) {
  if (level === "user_prompt") return 0
  if (level === "output_summary") return 2
  return 1
}

function publicationPrompt(workflowRun: WorkflowRun): string | null {
  const input = workflowRun.publication_invocation?.input
  if (typeof input === "string") return input.trim() || null
  if (input && typeof input === "object" && !Array.isArray(input)) {
    const prompt = (input as Record<string, unknown>).prompt
    if (typeof prompt === "string") return prompt.trim() || null
  }
  const fallback = workflowRun.invocation_prompt?.trim()
  if (!fallback) return null
  try {
    const parsed = JSON.parse(fallback) as unknown
    if (typeof parsed === "string") return parsed.trim() || null
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      const prompt = (parsed as Record<string, unknown>).prompt
      return typeof prompt === "string" ? prompt.trim() || null : null
    }
    return null
  } catch {
    return fallback
  }
}

function pushTraceEvent(
  events: PublicationTraceEvent[],
  state: PublicationTraceStreamState,
  publication: WorkflowPublicationConfig,
  workflowRun: WorkflowRun,
  nodeRun: NonNullable<WorkflowRun["node_runs"]>[number],
  level: PublicationTraceLevel,
  options: {
    key: string
    timestampMs: number
    message: string
    data?: unknown
  },
) {
  if (!options.message && options.data == null) return
  const key = `${level}:${options.key}`
  if (state.traceKeys.has(key)) return
  state.traceKeys.add(key)
  const nodeContext = publication.trace_context?.nodes[nodeRun.node_id]
  events.push({
    workflow_run_id: workflowRun.id,
    workflow_node_run_id: nodeRun.id,
    node_id: nodeRun.node_id,
    node_label: nodeContext?.node_label ?? nodeRun.node_id,
    agent_id: nodeRun.agent_id,
    agent_alias: nodeContext?.agent_alias ?? nodeRun.agent_id,
    level,
    sequence: 0,
    timestamp_ms: options.timestampMs,
    message: options.message,
    data: options.data,
  })
}

function completedTimestamp(
  nodeRun: NonNullable<WorkflowRun["node_runs"]>[number],
  workflowRun: WorkflowRun,
) {
  return nodeRun.completed_at_ms ?? workflowRun.completed_at_ms ?? Date.now()
}

function completionOutputMessage(output: unknown): string | null {
  if (typeof output === "string") return output
  if (!output || typeof output !== "object" || Array.isArray(output)) return null
  const message = (output as Record<string, unknown>).message
  if (typeof message === "string") return message
  try {
    return JSON.stringify(output)
  } catch {
    return String(output)
  }
}

function workflowFinalOutputMessage(
  workflowRun: WorkflowRun,
  nodeRun: NonNullable<WorkflowRun["node_runs"]>[number],
): string | null {
  if (workflowRun.completed_by_node_run_id && workflowRun.completed_by_node_run_id !== nodeRun.id) {
    return null
  }
  if (!workflowRun.completed_by_node_run_id && nodeRun.status !== "Completed") {
    return null
  }
  return completionOutputMessage(workflowRun.final_output)
}
