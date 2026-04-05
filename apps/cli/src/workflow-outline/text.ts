import type { WorkflowOutline, WorkflowOutlineEdgeItem, WorkflowOutlineNodeItem } from "./types.js"

export type OutlineLine = {
  content: string
  tone: "title" | "section" | "detail" | "muted"
  emphasis?: "bold"
}

export function buildWorkflowOutlineNodeLines(node: WorkflowOutlineNodeItem): OutlineLine[] {
  const lines: OutlineLine[] = [
    {
      content: `node ${node.id} • agent ${formatAgentLabel(node)}`,
      tone: "title",
      emphasis: "bold",
    },
  ]

  if (node.entryEndpoints.length > 0) {
    lines.push({ content: `entry endpoints ${node.entryEndpoints.length}`, tone: "section" })
    for (const endpoint of node.entryEndpoints) {
      lines.push({
        content: `  ${endpoint.id}${endpoint.alias ? ` (${endpoint.alias})` : ""}`,
        tone: "section",
      })
    }
  }

  lines.push({ content: `outgoing ${node.outgoingEdges.length}`, tone: "section" })
  if (node.outgoingEdges.length === 0) {
    lines.push({ content: "  none", tone: "muted" })
  } else {
    for (const edge of node.outgoingEdges) {
      lines.push({
        content: `  ${formatOutgoingEdge(edge)}`,
        tone: "muted",
      })
    }
  }

  lines.push({ content: `incoming ${node.incomingEdges.length}`, tone: "section" })
  if (node.incomingEdges.length === 0) {
    lines.push({ content: "  none", tone: "muted" })
  } else {
    for (const edge of node.incomingEdges) {
      lines.push({
        content: `  ${formatIncomingEdge(edge)}`,
        tone: "muted",
      })
    }
  }

  if (!node.selected) {
    return lines
  }

  lines.push({ content: `provider ${node.provider ?? "-"}`, tone: "detail" })
  lines.push({ content: `model ${node.model ?? "-"}`, tone: "detail" })
  lines.push({ content: `effort ${node.effort ?? "-"}`, tone: "detail" })
  lines.push({ content: `status ${String(node.runStatus ?? "idle").toLowerCase()}`, tone: "detail" })
  lines.push({ content: `failures ${node.failureCount}`, tone: node.failureCount > 0 ? "section" : "detail" })
  if (node.recentFailures.length > 0) {
    lines.push({ content: "recent failure events", tone: "section" })
    for (const failure of node.recentFailures) {
      lines.push({
        content: `  ${String(failure.kind).toLowerCase()} • ${failure.message}`,
        tone: "detail",
      })
    }
  }
  lines.push({ content: "instructions", tone: "section" })
  const instructionsLines = splitInstructions(node.instructions)
  for (const instructionLine of instructionsLines) {
    lines.push({
      content: `  ${instructionLine}`,
      tone: "detail",
    })
  }
  return lines
}

export function renderWorkflowOutlineToText(outline: WorkflowOutline) {
  const lines = [
    `workflow: ${outline.workflowAlias ? `${outline.workflowId} (${outline.workflowAlias})` : outline.workflowId}${outline.agentLabels.length > 0 ? `, agents: ${outline.agentLabels.join(", ")}` : ""}`,
    ...(outline.workflowRunId
      ? [`run: ${outline.workflowRunId} • status ${String(outline.workflowRunStatus).toLowerCase()}${outline.workflowFailureCount > 0 ? ` • failures ${outline.workflowFailureCount}` : ""}`]
      : []),
    `Tab cycles nodes • nodes ${outline.nodeCount} • endpoints ${outline.endpointCount} • edges ${outline.edgeCount}`,
  ]
  for (const node of outline.nodes) {
    lines.push("")
    lines.push(...buildWorkflowOutlineNodeLines(node).map((line) => line.content))
  }
  return lines.join("\n")
}

function formatAgentLabel(node: WorkflowOutlineNodeItem) {
  return node.agentAlias ? `${node.agentRef} (${node.agentAlias})` : node.agentRef
}

function formatAdjacentAgent(edge: WorkflowOutlineEdgeItem) {
  return edge.agentAlias ? `${edge.agentRef} (${edge.agentAlias})` : edge.agentRef
}

function formatOutgoingEdge(edge: WorkflowOutlineEdgeItem) {
  return `${edge.id} -> ${edge.nodeId} • agent ${formatAdjacentAgent(edge)}`
}

function formatIncomingEdge(edge: WorkflowOutlineEdgeItem) {
  return `${edge.id} <- ${edge.nodeId} • agent ${formatAdjacentAgent(edge)}`
}

function splitInstructions(instructions: string | null) {
  const normalized = instructions?.trim()
  if (!normalized) {
    return ["-"]
  }
  return normalized.split("\n")
}
