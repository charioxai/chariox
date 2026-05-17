import type { TextareaRenderable } from "@opentui/core"

import { formatAgentLabel } from "./agent-label.js"
import type {
  RuntimeSession,
  WorkflowRun,
} from "./cli-types.js"
import type { WorkflowNodeInstructionsEditor } from "./workflow-node-instructions-editor-controller.js"
import { resolveActiveWorkflowRun } from "./workflow-prompt-state.js"

export type WorkflowInspectorMode = "runtime" | "terminal"

export type WorkflowInspectorProjection = {
  title: string
  meta: string[]
  body?: string | null
  draft?: string
  placeholder?: string | null
  hint?: string | null
  onDraftChange?: ((draft: string) => void) | null
  onEditorRef?: ((editor: TextareaRenderable | null) => void) | null
}

type WorkflowInspectorProjectionInput = {
  session: RuntimeSession
  selectedWorkflowId: string | null
  selectedWorkflowNodeId: string | null
  inspectorMode: WorkflowInspectorMode
  nodeInstructionsEditor: WorkflowNodeInstructionsEditor | null
  updateNodeInstructionsDraft: (draft: string) => void
  setNodeInstructionsInputRef: (editor: TextareaRenderable | null) => void
}

export function buildWorkflowInspectorProjection(
  input: WorkflowInspectorProjectionInput,
): WorkflowInspectorProjection | null {
  if (input.nodeInstructionsEditor) {
    return buildNodeInstructionsInspector(input)
  }
  return input.inspectorMode === "terminal"
    ? buildTerminalInspector(input)
    : buildRuntimeInspector(input)
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
  const agentLabel = agent ? formatAgentLabel(agent) : node?.agent_id ?? "unknown"
  return {
    title: "Node Instructions",
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

function buildRuntimeInspector(
  input: WorkflowInspectorProjectionInput,
): WorkflowInspectorProjection | null {
  const workflow = input.session.workflows?.find((entry) => entry.id === input.selectedWorkflowId) ?? null
  if (!workflow) {
    return null
  }
  const selectedNode = workflow.nodes?.find((entry) => entry.id === input.selectedWorkflowNodeId) ?? null
  const selectedAgent = selectedNode
    ? input.session.agents.find((entry) => entry.id === selectedNode.agent_id) ?? null
    : null
  const workflowRun = resolveLatestWorkflowRun(workflow.id, input.session.workflow_runs ?? [])
  const workflowLabel = workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id
  const meta = [
    `Workflow: ${workflowLabel}`,
    `Selected node: ${selectedNode?.id ?? "-"}`,
    `Agent: ${selectedAgent ? formatAgentLabel(selectedAgent) : selectedNode?.agent_id ?? "-"}`,
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

function buildTerminalInspector(
  input: WorkflowInspectorProjectionInput,
): WorkflowInspectorProjection | null {
  const workflow = input.session.workflows?.find((entry) => entry.id === input.selectedWorkflowId) ?? null
  if (!workflow) {
    return null
  }
  const workflowLabel = workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id
  const consoleState = (input.session.workflow_consoles ?? []).find((entry) => entry.workflow_id === workflow.id) ?? null
  const body = (consoleState?.entries ?? []).map((entry) => entry.text ?? "").join("")
  return {
    title: "Workflow Terminal",
    meta: [
      `Workflow: ${workflowLabel}`,
      `Entries: ${consoleState?.entries?.length ?? 0}`,
    ],
    body: body.length > 0 ? body : "No workflow terminal output yet.",
    hint: "Use /workflow terminal [workflow-ref] to keep this console visible while the workflow runs.",
  }
}

function resolveLatestWorkflowRun(workflowId: string, workflowRuns: WorkflowRun[]) {
  return resolveActiveWorkflowRun(workflowId, workflowRuns)
    ?? [...workflowRuns]
      .filter((entry) => entry.workflow_id === workflowId)
      .sort((left, right) => right.created_at_ms - left.created_at_ms)[0]
    ?? null
}
