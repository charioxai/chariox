import type {
  PublicationTraceEvent,
  PublicationTraceLevel,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"

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
  for (const nodeRun of workflowRun.node_runs ?? []) {
    const levels = new Set(policy[nodeRun.node_id] ?? [])
    if (levels.size === 0) continue
    if (levels.has("output_summary")) {
      pushTraceEvent(events, state, publication, workflowRun, nodeRun, "output_summary", {
        key: `summary:${nodeRun.id}:${nodeRun.summary ?? ""}:${nodeRun.completion?.summary ?? ""}`,
        timestampMs: completedTimestamp(nodeRun, workflowRun),
        message: nodeRun.completion?.summary ?? nodeRun.summary ?? "",
        data: {
          summary: nodeRun.summary ?? null,
          completion_summary: nodeRun.completion?.summary ?? null,
        },
      })
    }
    if (levels.has("assistant_messages")) {
      for (const message of workflowRun.messages ?? []) {
        if (message.source_node_run_id !== nodeRun.id) continue
        pushTraceEvent(events, state, publication, workflowRun, nodeRun, "assistant_messages", {
          key: `message:${message.id}`,
          timestampMs: message.created_at_ms,
          message: message.summary || message.handoff_payload,
          data: {
            message_id: message.id,
            message_type: message.message_type,
            summary: message.summary,
            handoff_payload: message.handoff_payload,
          },
        })
      }
      const outputMessage = nodeRun.completion?.output?.message
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
        pushTraceEvent(events, state, publication, workflowRun, nodeRun, "tool_use", {
          key: `tool:${nodeRun.id}:${index}:${toolCall.timestamp_ms}`,
          timestampMs: toolCall.timestamp_ms,
          message: `${toolCall.tool_name} ${toolCall.ok ? "ok" : "failed"}`,
          data: toolCall,
        })
      }
    }
  }
  return events
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
    sequence: state.nextSequence++,
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
