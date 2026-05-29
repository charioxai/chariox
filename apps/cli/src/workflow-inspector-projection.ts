import type { TextareaRenderable } from "@opentui/core"

import type {
  RuntimeSession,
  TranscriptEntry,
  WorkflowDefinition,
  WorkflowNodeDefinition,
  WorkflowRun,
} from "./cli-types.js"
import { workflowAgentDisplayLabel } from "./workflow-collaboration-labels.js"
import type { WorkflowComponentSelection } from "./workflow-component-selection.js"
import type { WorkflowNodeInstructionsEditor } from "./workflow-node-instructions-editor-controller.js"
import { resolveActiveWorkflowRun } from "./workflow-prompt-state.js"

export type WorkflowInspectorMode = "logs" | "trace" | "edit"

export type WorkflowInspectorProjection = {
  title: string
  mode?: WorkflowInspectorMode
  meta: string[]
  body?: string | null
  draft?: string
  placeholder?: string | null
  hint?: string | null
  onDraftChange?: ((draft: string) => void) | null
  onEditorRef?: ((editor: TextareaRenderable | null) => void) | null
  transcriptAgentId?: string | null
  transcriptEntries?: TranscriptEntry[]
}

type WorkflowInspectorProjectionInput = {
  session: RuntimeSession
  selectedWorkflowId: string | null
  selectedWorkflowNodeId: string | null
  selectedWorkflowComponent?: WorkflowComponentSelection | null
  inspectorMode: WorkflowInspectorMode
  nodeInstructionsEditor: WorkflowNodeInstructionsEditor | null
  agentPaneEntries: Record<string, TranscriptEntry[]>
  updateNodeInstructionsDraft: (draft: string) => void
  setNodeInstructionsInputRef: (editor: TextareaRenderable | null) => void
}

export function buildWorkflowInspectorProjection(
  input: WorkflowInspectorProjectionInput,
): WorkflowInspectorProjection | null {
  if (input.nodeInstructionsEditor) {
    return buildNodeInstructionsInspector(input)
  }
  if (input.inspectorMode === "trace") {
    return buildTraceInspector(input)
  }
  if (input.inspectorMode === "edit") {
    return buildEditInspector(input)
  }
  return buildLogsInspector(input)
}

function buildNodeInstructionsInspector(
  input: WorkflowInspectorProjectionInput,
): WorkflowInspectorProjection | null {
  const editor = input.nodeInstructionsEditor
  if (!editor) {
    return null
  }
  const workflow = input.session.workflows?.find((entry) => entry.id === editor.workflowId) ?? null
  const node = workflow?.nodes?.find((entry) => entry.id === editor.nodeId) ?? null
  const agent = node ? input.session.agents.find((entry) => entry.id === node.agent_id) ?? null : null
  const workflowLabel = workflow?.alias ? `${workflow.id} (${workflow.alias})` : editor.workflowId
  const agentLabel = node ? workflowAgentDisplayLabel(agent) : "unknown"
  return {
    title: "Node Instructions",
    mode: "edit",
    meta: [
      `Workflow: ${workflowLabel}`,
      `Node: ${node?.id ?? editor.nodeId}`,
      `Agent: ${agentLabel}`,
    ],
    draft: editor.draft ?? "",
    placeholder: "Type system instructions for this node",
    hint: "Use /workflow node instructions save to persist. /workflow node instructions close to discard.",
    onDraftChange: input.updateNodeInstructionsDraft,
    onEditorRef: input.setNodeInstructionsInputRef,
  }
}

function buildEditInspector(
  input: WorkflowInspectorProjectionInput,
): WorkflowInspectorProjection | null {
  const workflow = input.session.workflows?.find((entry) => entry.id === input.selectedWorkflowId) ?? null
  if (!workflow) {
    return null
  }
  const resolved = resolveSelectedComponent(workflow, input.selectedWorkflowNodeId, input.selectedWorkflowComponent ?? null)
  const selectedNode = resolved.node
  const selectedAgent = selectedNode
    ? input.session.agents.find((entry) => entry.id === selectedNode.agent_id) ?? null
    : null
  return {
    title: "Workflow Edit",
    mode: "edit",
    meta: [
      `Workflow: ${workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id}`,
      `Selected: ${formatComponentSelection(resolved)}`,
      `Agent: ${selectedNode ? workflowAgentDisplayLabel(selectedAgent) : "-"}`,
    ],
    body: buildEditBody(workflow, resolved, selectedNode),
    hint: resolved.selection.kind === "node"
      ? "Press Enter or use /workflow node instructions set to open the node editor."
      : "Use slash commands to update the selected workflow component.",
  }
}

function buildTraceInspector(
  input: WorkflowInspectorProjectionInput,
): WorkflowInspectorProjection | null {
  const workflow = input.session.workflows?.find((entry) => entry.id === input.selectedWorkflowId) ?? null
  if (!workflow) {
    return null
  }
  const resolved = resolveSelectedComponent(workflow, input.selectedWorkflowNodeId, input.selectedWorkflowComponent ?? null)
  const selectedNode = resolved.node
  const selectedAgent = selectedNode
    ? input.session.agents.find((entry) => entry.id === selectedNode.agent_id) ?? null
    : null
  const entries = selectedAgent ? input.agentPaneEntries[selectedAgent.id] ?? [] : []
  return {
    title: "Workflow Trace",
    mode: "trace",
    meta: [
      `Workflow: ${workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id}`,
      `Selected: ${formatComponentSelection(resolved)}`,
      `Agent: ${selectedNode ? workflowAgentDisplayLabel(selectedAgent) : "-"}`,
      `Entries: ${entries.length}`,
    ],
    body: selectedAgent
      ? entries.length > 0 ? null : "No trace entries for the selected agent yet."
      : "Select a workflow node, edge, or endpoint to show its backing agent trace.",
    transcriptAgentId: selectedAgent?.id ?? null,
    transcriptEntries: entries,
    hint: "The bottom prompt targets this selected workflow agent.",
  }
}

function buildRuntimeSummary(input: WorkflowInspectorProjectionInput): WorkflowInspectorProjection | null {
  const workflow = input.session.workflows?.find((entry) => entry.id === input.selectedWorkflowId) ?? null
  if (!workflow) {
    return null
  }
  const resolved = resolveSelectedComponent(workflow, input.selectedWorkflowNodeId, input.selectedWorkflowComponent ?? null)
  const selectedNode = resolved.node
  const selectedAgent = selectedNode
    ? input.session.agents.find((entry) => entry.id === selectedNode.agent_id) ?? null
    : null
  const workflowRun = resolveLatestWorkflowRun(workflow.id, input.session.workflow_runs ?? [])
  const workflowLabel = workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id
  const meta = [
    `Workflow: ${workflowLabel}`,
    `Selected: ${formatComponentSelection(resolved)}`,
    `Agent: ${selectedNode ? workflowAgentDisplayLabel(selectedAgent) : "-"}`,
    `Run: ${workflowRun?.id ?? "-"}`,
    `Run status: ${String(workflowRun?.status ?? "idle").toLowerCase()}`,
  ]
  const nodeRuns = workflowRun?.node_runs ?? []
  const selectedNodeRun = selectedNode
    ? [...nodeRuns].filter((entry) => entry.node_id === selectedNode.id).sort((left, right) => right.created_at_ms - left.created_at_ms)[0] ?? null
    : null
  const failureEvents = workflowRun?.failure_events ?? []
  const selectedNodeFailures = selectedNodeRun
    ? failureEvents.filter((entry) => entry.source_node_run_id === selectedNodeRun.id)
    : []
  const workflowWatchdogs = (input.session.workflow_watchdogs ?? [])
    .filter((entry) => entry.workflow_id === workflow.id)
    .sort((left, right) => left.next_run_at_ms - right.next_run_at_ms)
  const lines: string[] = []
  lines.push(`Watchdogs: ${workflowWatchdogs.length}`)
  if (workflowWatchdogs.length > 0) {
    lines.push("")
    lines.push("Watchdogs")
    for (const watchdog of workflowWatchdogs.slice(0, 8)) {
      lines.push(`- ${watchdog.id} endpoint=${watchdog.endpoint_id} every=${watchdog.interval_seconds}s policy=${watchdog.policy} enabled=${String(watchdog.enabled)}`)
      lines.push(`  next: ${new Date(watchdog.next_run_at_ms).toISOString()}`)
      if (watchdog.last_status) {
        lines.push(`  last: ${watchdog.last_status}`)
      }
      if (watchdog.pending_run) {
        lines.push("  pending: true")
      }
    }
  }
  lines.push("")
  lines.push(`Failure events: ${failureEvents.length}`)
  if (selectedNodeRun) {
    lines.push("")
    lines.push("Selected node run")
    lines.push(`- id: ${selectedNodeRun.id}`)
    lines.push(`- status: ${String(selectedNodeRun.status).toLowerCase()}`)
    lines.push(`- summary: ${selectedNodeRun.summary ?? "-"}`)
    if (selectedNodeRun.turn_envelope) {
      lines.push(`- turn state: ${selectedNodeRun.turn_envelope.state}`)
      lines.push(`- delivery token: ${selectedNodeRun.turn_envelope.delivery_token}`)
      if (selectedNodeRun.turn_envelope.mailbox_content) {
        lines.push("")
        lines.push("Mailbox snapshot")
        lines.push(selectedNodeRun.turn_envelope.mailbox_content)
      }
      if (selectedNodeRun.turn_envelope.handoff_payloads_json) {
        lines.push("")
        lines.push("Handoff snapshot")
        lines.push(selectedNodeRun.turn_envelope.handoff_payloads_json)
      }
      const runtimeToolCalls = selectedNodeRun.turn_envelope.runtime_tool_calls ?? []
      if (runtimeToolCalls.length > 0) {
        lines.push("")
        lines.push("Runtime tool calls")
        for (const call of runtimeToolCalls.slice(-10)) {
          lines.push(`- ${call.tool_name} @ ${new Date(call.timestamp_ms).toISOString()} ok=${String(call.ok)}`)
          lines.push(`  args: ${call.arguments_json}`)
          if (call.result_json) {
            lines.push(`  result: ${call.result_json}`)
          }
        }
      }
    }
  }
  if (selectedNodeFailures.length > 0) {
    lines.push("")
    lines.push("Selected node failure events")
    for (const failure of selectedNodeFailures) {
      lines.push(`- ${String(failure.kind).toLowerCase()} @ ${new Date(failure.timestamp_ms).toISOString()}`)
      lines.push(`  ${failure.message}`)
      if (failure.edge_ids.length > 0) {
        lines.push(`  edges: ${failure.edge_ids.join(", ")}`)
      }
    }
  } else if (failureEvents.length > 0) {
    lines.push("")
    lines.push("Recent workflow failure events")
    for (const failure of failureEvents.slice(-5).reverse()) {
      lines.push(`- ${String(failure.kind).toLowerCase()} @ ${new Date(failure.timestamp_ms).toISOString()}`)
      lines.push(`  ${failure.message}`)
    }
  } else {
    lines.push("")
    lines.push("No failure events recorded for the current workflow run.")
  }
  return {
    title: "Workflow Runtime",
    meta,
    body: lines.join("\n"),
    hint: "Use /workflow runs, /workflow cancel, and /workflow resume to manage the current run.",
  }
}

function buildLogsInspector(
  input: WorkflowInspectorProjectionInput,
): WorkflowInspectorProjection | null {
  const workflow = input.session.workflows?.find((entry) => entry.id === input.selectedWorkflowId) ?? null
  if (!workflow) {
    return null
  }
  const workflowLabel = workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id
  const consoleState = (input.session.workflow_consoles ?? []).find((entry) => entry.workflow_id === workflow.id) ?? null
  const body = (consoleState?.entries ?? []).map((entry) => entry.text ?? "").join("")
  const runtimeSummary = buildRuntimeSummary(input)
  return {
    title: "Workflow Logs",
    mode: "logs",
    meta: [
      ...(runtimeSummary?.meta ?? [`Workflow: ${workflowLabel}`]),
      `Entries: ${consoleState?.entries?.length ?? 0}`,
    ],
    body: [
      body.length > 0 ? body : "No workflow logs captured yet.",
      runtimeSummary?.body ? `\n${runtimeSummary.body}` : "",
    ].join(""),
    hint: "Use /workflow run, /workflow runs, /workflow cancel, and /workflow resume to manage runs.",
  }
}

function resolveLatestWorkflowRun(workflowId: string, workflowRuns: WorkflowRun[]) {
  return resolveActiveWorkflowRun(workflowId, workflowRuns)
    ?? [...workflowRuns]
      .filter((entry) => entry.workflow_id === workflowId)
      .sort((left, right) => right.created_at_ms - left.created_at_ms)[0]
    ?? null
}

function resolveSelectedComponent(
  workflow: WorkflowDefinition,
  selectedNodeId: string | null,
  selection: WorkflowComponentSelection | null,
): { selection: WorkflowComponentSelection; node: WorkflowNodeDefinition | null } {
  if (selection?.kind === "workflow") {
    return { selection, node: null }
  }
  if (selection?.kind === "edge") {
    const edge = workflow.edges?.find((entry) => entry.id === selection.id) ?? null
    const node = edge ? workflow.nodes?.find((entry) => entry.id === edge.from_node_id) ?? null : null
    return { selection, node }
  }
  if (selection?.kind === "endpoint") {
    const endpoint = workflow.endpoints?.find((entry) => entry.id === selection.id) ?? null
    const node = endpoint ? workflow.nodes?.find((entry) => entry.id === endpoint.entry_node_id) ?? null : null
    return { selection, node }
  }
  if (selection?.kind === "node") {
    return {
      selection,
      node: workflow.nodes?.find((entry) => entry.id === selection.id) ?? null,
    }
  }
  if (selectedNodeId) {
    return {
      selection: { kind: "node", id: selectedNodeId },
      node: workflow.nodes?.find((entry) => entry.id === selectedNodeId) ?? null,
    }
  }
  return { selection: { kind: "workflow" }, node: null }
}

function formatComponentSelection(resolved: { selection: WorkflowComponentSelection; node: WorkflowNodeDefinition | null }) {
  const selection = resolved.selection
  if (selection.kind === "workflow") {
    return "workflow"
  }
  const backingNode = resolved.node ? ` -> node ${resolved.node.id}` : ""
  return `${selection.kind} ${selection.id}${selection.kind === "node" ? "" : backingNode}`
}

function buildEditBody(
  workflow: WorkflowDefinition,
  resolved: { selection: WorkflowComponentSelection; node: WorkflowNodeDefinition | null },
  selectedNode: WorkflowNodeDefinition | null,
) {
  const selection = resolved.selection
  if (selection.kind === "workflow") {
    return [
      "Editable workflow fields",
      `- alias: ${workflow.alias ?? "none"}`,
      `- flush-context: ${(workflow.flush_agent_context_before_run ?? true) ? "true" : "false"}`,
      `- run-output-schema: ${workflow.run_output_schema_ref ?? "none"}`,
      `- intermediate-output-schema: ${workflow.intermediate_output_schema_ref ?? "none"}`,
      "",
      "Use /workflow <workflow-ref> <alias> to rename.",
      "Use /workflow flush-context to update context flush policy.",
      "Use /workflow run-output-schema or /workflow intermediate-output-schema to edit schema refs.",
    ].join("\n")
  }
  if (selection.kind === "node") {
    if (!selectedNode) {
      return "Selected node no longer exists."
    }
    return [
      "Editable node fields",
      `- instructions: ${selectedNode.instructions?.trim() ? "configured" : "none"}`,
      `- intermediate-output-schema: ${selectedNode.intermediate_output_schema_ref ?? "none"}`,
      `- can-complete-run: ${String(selectedNode.can_complete_workflow_run ?? false)}`,
      `- can-emit-intermediate-output: ${String(selectedNode.can_emit_intermediate_run_output ?? false)}`,
      `- max-turns: ${selectedNode.max_turns ?? "none"}`,
      "",
      "Use /workflow node instructions set to edit instructions.",
      "Use /workflow node intermediate-output-schema to edit schema refs.",
      "Use /workflow node can-complete-run, can-emit-intermediate-output, or max-turns for runtime settings.",
    ].join("\n")
  }
  if (selection.kind === "edge") {
    const edge = workflow.edges?.find((entry) => entry.id === selection.id) ?? null
    if (!edge) {
      return "Selected edge no longer exists."
    }
    return [
      "Editable edge fields",
      `- from: ${edge.from_node_id}`,
      `- to: ${edge.to_node_id}`,
      `- handoff-schema: ${edge.handoff_schema_ref ?? "none"}`,
      `- validation-policy: ${edge.validation_policy ?? "default"}`,
      "",
      "Use /workflow edge remove and /workflow edge add --handoff-schema to replace an edge contract.",
    ].join("\n")
  }
  const endpoint = workflow.endpoints?.find((entry) => entry.id === selection.id) ?? null
  if (!endpoint) {
    return "Selected endpoint no longer exists."
  }
  return [
    "Editable endpoint fields",
    `- alias: ${endpoint.alias ?? "none"}`,
    `- entry-node: ${endpoint.entry_node_id}`,
    "",
    "Use /workflow endpoint alias to rename.",
    "Use /workflow endpoint bind to change the entry node.",
    "Use /workflow run <endpoint-ref> <prompt> to start a workflow run.",
  ].join("\n")
}
