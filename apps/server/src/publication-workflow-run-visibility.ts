import type {
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"
import {
  visibleAssistantWorkflowMessage,
  visibleRuntimeToolCall,
} from "./publication-workflow-message-visibility.js"

export function visibleWorkflowInvocationResult(
  publication: WorkflowPublicationConfig,
  result: WorkflowInvocationResult,
): WorkflowInvocationResult {
  return result.workflow_run
    ? { ...result, workflow_run: visibleWorkflowRun(publication, result.workflow_run) }
    : result
}

export function visibleWorkflowRun(
  publication: WorkflowPublicationConfig,
  workflowRun: WorkflowRun,
): WorkflowRun {
  const policy = publication.trace_exposure?.nodes ?? {}
  const visibleRun: WorkflowRun = {
    id: workflowRun.id,
    status: workflowRun.status,
  }
  if (workflowRun.workflow_id !== undefined) visibleRun.workflow_id = workflowRun.workflow_id
  if (workflowRun.endpoint_id !== undefined) visibleRun.endpoint_id = workflowRun.endpoint_id
  if (workflowRun.publication_invocation !== undefined) visibleRun.publication_invocation = workflowRun.publication_invocation
  if (workflowRun.completed_by_node_run_id !== undefined) visibleRun.completed_by_node_run_id = workflowRun.completed_by_node_run_id
  if (workflowRun.created_at_ms !== undefined) visibleRun.created_at_ms = workflowRun.created_at_ms
  if (workflowRun.completed_at_ms !== undefined) visibleRun.completed_at_ms = workflowRun.completed_at_ms
  if (workflowRun.final_output !== undefined) visibleRun.final_output = workflowRun.final_output
  if (workflowRun.intermediate_outputs !== undefined) visibleRun.intermediate_outputs = workflowRun.intermediate_outputs
  if (workflowRun.node_runs !== undefined) {
    visibleRun.node_runs = workflowRun.node_runs.map((nodeRun) => {
      const levels = new Set(policy[nodeRun.node_id] ?? [])
      const visibleNodeRun: NonNullable<WorkflowRun["node_runs"]>[number] = {
        id: nodeRun.id,
        node_id: nodeRun.node_id,
        agent_id: nodeRun.agent_id,
        status: nodeRun.status,
      }
      if (nodeRun.completed_at_ms !== undefined) visibleNodeRun.completed_at_ms = nodeRun.completed_at_ms
      if (levels.has("output_summary")) {
        if (nodeRun.summary !== undefined) visibleNodeRun.summary = nodeRun.summary
        if (nodeRun.completion?.summary !== undefined) {
          visibleNodeRun.completion = { ...(visibleNodeRun.completion ?? {}), summary: nodeRun.completion.summary }
        }
      }
      if (levels.has("assistant_messages") && nodeRun.completion?.output !== undefined) {
        visibleNodeRun.completion = { ...(visibleNodeRun.completion ?? {}), output: nodeRun.completion.output }
      }
      if (levels.has("thinking") && nodeRun.thinking_traces !== undefined) visibleNodeRun.thinking_traces = nodeRun.thinking_traces
      if (levels.has("tool_use")) {
        visibleNodeRun.turn_envelope = {
          runtime_tool_calls: (nodeRun.turn_envelope?.runtime_tool_calls ?? []).map(visibleRuntimeToolCall),
        }
      }
      return visibleNodeRun
    })
  }
  if (workflowRun.messages !== undefined) {
    visibleRun.messages = workflowRun.messages
      .filter((message) => {
        if (!message.source_node_run_id) return false
        const nodeRun = workflowRun.node_runs?.find((candidate) => candidate.id === message.source_node_run_id)
        return Boolean(nodeRun && (policy[nodeRun.node_id] ?? []).includes("assistant_messages"))
      })
      .map(visibleAssistantWorkflowMessage)
  }
  const invocationInput = workflowRun.publication_invocation?.input
  const invocationPrompt = invocationInput && typeof invocationInput === "object" && !Array.isArray(invocationInput)
    ? (invocationInput as Record<string, unknown>).prompt
    : null
  if (typeof invocationPrompt === "string" && invocationPrompt.trim()) {
    const promptMessages = (workflowRun.node_runs ?? [])
      .filter((nodeRun) => (policy[nodeRun.node_id] ?? []).length > 0)
      .map((nodeRun) => ({
        id: `publication-prompt:${workflowRun.id}:${nodeRun.node_id}`,
        source_node_run_id: nodeRun.id,
        target_node_id: nodeRun.node_id,
        message_type: "user_prompt",
        summary: invocationPrompt.trim(),
        handoff_payload: "",
        created_at_ms: workflowRun.created_at_ms ?? 0,
      }))
    if (promptMessages.length > 0) {
      visibleRun.messages = [...promptMessages, ...(visibleRun.messages ?? [])]
    }
  }
  return visibleRun
}
